//! Unblocking a party (wl-parties §4 `unblock`): the acceptance checks
//! AC1–AC9 of #35. The routes act as the user (`acting_as`), as the back
//! office does; a test-only subscription store and its listeners stand for
//! the module that pauses agreements on `PartyBlocked`.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use renox::state_machine::{self, StateMachine};
use wl_kernel::{PartyBlocked, PartyKind, PartyStatus, PartyUnblocked};
use wl_parties::review_tasks::{ReviewTask, ReviewTaskItem, ReviewTaskStatus};
use wl_parties::{
    BlockRecord, Party, UnblockBasis, UnblockEvidence, WlParties, block_by_system,
    permissions as perm,
};

use support::{Booted, boot, boot_custom};

const AL_NOOR: &str = "Al Noor Trading LLC";
const GULF_BUILD: &str = "Gulf Build Materials LLC";
const NEUTRAL: &str = "This hold must be lifted by another authorised user";
const PDF: &[u8] = b"%PDF-1.7 sanctions screening result";

async fn records(t: &Booted, party: &Party) -> Vec<BlockRecord> {
    BlockRecord::for_party(t.db(), party.id).await.unwrap()
}

/// `user` blocks `party` through the real route.
async fn block(t: &Booted, user: &User, party: &Party, code: &str, note: &str) {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(
            &format!("/parties/{}/block", party.id),
            &[("reason_code", code), ("note", note)],
        )
        .await
        .assert_ok();
}

/// `user` posts the unblock sheet: the note, a basis when given and files.
async fn unblock(
    t: &Booted,
    user: &User,
    party: &Party,
    note: &str,
    basis: Option<&str>,
    files: &[(&str, &[u8])],
) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    let mut fields = vec![("note", note)];
    if let Some(basis) = basis {
        fields.push(("basis", basis));
    }
    let files: Vec<(&str, &str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| ("files", *name, *bytes))
        .collect();
    t.app
        .htmx()
        .post_multipart(&format!("/parties/{}/unblock", party.id), &fields, &files)
        .await
}

async fn history(t: &Booted, party: &Party) -> Vec<state_machine::StateTransition> {
    state_machine::history(t.db(), Party::TABLE, party.id)
        .await
        .unwrap()
}

/// The deputy MLRO: blocks and sees restricted reasons.
async fn deputy(t: &Booted) -> User {
    t.user("Dina", &[perm::VIEW, perm::BLOCK, perm::BLOCK_REASON_VIEW])
        .await
}

/// User C: blocks and unblocks, never sees restricted reasons.
async fn user_c(t: &Booted) -> User {
    t.user("Chen", &[perm::VIEW, perm::BLOCK]).await
}

/// AC1: A cannot lift their own block; B can, with a reason.
#[renox::test]
async fn another_user_lifts_a_block_with_a_reason() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    block(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;

    unblock(&t, &a, &al_noor, "paid in full", None, &[])
        .await
        .assert_status(422)
        .assert_see(NEUTRAL);
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Blocked);
    assert!(t.unblocked().is_empty());

    unblock(&t, &b, &al_noor, "paid in full", None, &[])
        .await
        .assert_ok();
    let party = t.reload(&al_noor).await;
    assert_eq!(party.status, PartyStatus::Active);
    assert_eq!(party.hold_reason, None);
    let records = records(&t, &party).await;
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(!record.is_open());
    assert_eq!(record.unblocked_by, Some(b.id));
    assert!(record.unblocked_at.is_some());
    assert_eq!(record.unblock_note.as_deref(), Some("paid in full"));
    assert_eq!(record.unblock_basis, None);

    let history = history(&t, &party).await;
    let row = history.last().expect("a history row");
    assert_eq!(row.transition, "unblock");
    assert_eq!(row.actor_id, Some(b.id));
    assert_eq!(row.reason.as_deref(), Some("paid in full"));

    assert_eq!(
        t.unblocked(),
        [PartyUnblocked {
            party: party.party_id(),
            display_reason: "non_payment".to_owned(),
        }]
    );
}

/// AC2: a blank note is refused (422) and a user without `parties.block`
/// gets 403; nothing changes and nothing is emitted.
#[renox::test]
async fn a_blank_note_and_a_missing_permission_are_refused() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    block(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;

    for note in ["", "   "] {
        unblock(&t, &b, &al_noor, note, None, &[])
            .await
            .assert_status(422);
    }
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    unblock(&t, &viewer, &al_noor, "paid in full", None, &[])
        .await
        .assert_forbidden();

    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Blocked);
    assert!(records(&t, &al_noor).await[0].is_open());
    assert!(t.unblocked().is_empty());
}

/// AC3: a sanctions block needs the permission, another user, a basis and a
/// file; nothing about it leaks.
#[renox::test]
async fn a_sanctions_block_is_lifted_with_evidence_and_stays_neutral() {
    let t = boot().await;
    let gulf = t.party(GULF_BUILD).await;
    let mlro = t.mlro().await;
    block(&t, &mlro, &gulf, "sanctions", "List match on the owner").await;
    let (c, deputy) = (user_c(&t).await, deputy(&t).await);

    // C does not hold `parties.block_reason.view`: the same message as a
    // conflict of duties, and no word about sanctions.
    let evidence = [("m.pdf", PDF)];
    let refused = unblock(
        &t,
        &c,
        &gulf,
        "match found false",
        Some("false_match"),
        &evidence,
    )
    .await;
    refused.assert_status(422).assert_see(NEUTRAL);
    assert!(!refused.text().contains("sanctions"));

    unblock(&t, &deputy, &gulf, "match found false", None, &evidence)
        .await
        .assert_status(422)
        .assert_see("Choose why the hold is lifted");
    unblock(
        &t,
        &deputy,
        &gulf,
        "match found false",
        Some("false_match"),
        &[],
    )
    .await
    .assert_status(422)
    .assert_see("Attach at least one evidence file");
    assert_eq!(t.reload(&gulf).await.status, PartyStatus::Blocked);
    assert!(t.unblocked().is_empty());

    unblock(
        &t,
        &deputy,
        &gulf,
        "match found false",
        Some("false_match"),
        &evidence,
    )
    .await
    .assert_ok();
    let party = t.reload(&gulf).await;
    assert_eq!(party.status, PartyStatus::Active);
    let record = records(&t, &party).await.remove(0);
    assert_eq!(record.unblocked_by, Some(deputy.id));
    assert_eq!(record.unblock_basis, Some(UnblockBasis::FalseMatch));
    assert_eq!(record.unblock_note.as_deref(), Some("match found false"));
    let files = UnblockEvidence::for_record(t.db(), record.id)
        .await
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].file_name, "m.pdf");
    assert!(
        files[0]
            .storage_key
            .contains(&format!("parties/{}/unblock/{}/", party.id, record.id))
    );
    assert!(
        t.app
            .state()
            .storage
            .exists(&files[0].storage_key)
            .await
            .unwrap()
    );

    let history = history(&t, &party).await;
    let row = history.last().expect("a history row");
    assert_eq!(row.transition, "unblock");
    assert_eq!(row.reason.as_deref(), Some("on_hold"));
    let events = t.unblocked();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].display_reason, "on_hold");
    let leaked = format!(
        "{} {} {:?}",
        row.reason.clone().unwrap_or_default(),
        row.metadata,
        events
    )
    .to_lowercase();
    for word in ["sanctions", "match found false", "false_match"] {
        assert!(!leaked.contains(word), "{word} in {leaked}");
    }
}

/// AC4: a sanctions block made for the MLRO's `confirm_match` is lifted by
/// the deputy, not by the MLRO.
#[renox::test]
async fn the_reviewer_who_ran_the_match_cannot_lift_it() {
    let t = boot().await;
    let gulf = t.party(GULF_BUILD).await;
    let (mlro, deputy) = (t.mlro().await, deputy(&t).await);
    block_by_system(
        t.app.state(),
        &gulf,
        "sanctions",
        "Confirmed match",
        Some(mlro.id),
    )
    .await
    .unwrap();
    assert_eq!(records(&t, &gulf).await[0].blocked_by, Some(mlro.id));

    let evidence = [("m.pdf", PDF)];
    unblock(&t, &mlro, &gulf, "delisted", Some("delisted"), &evidence)
        .await
        .assert_status(422)
        .assert_see(NEUTRAL);
    assert_eq!(t.reload(&gulf).await.status, PartyStatus::Blocked);
    unblock(&t, &deputy, &gulf, "delisted", Some("delisted"), &evidence)
        .await
        .assert_ok();
    let party = t.reload(&gulf).await;
    assert_eq!(party.status, PartyStatus::Active);
    let record = records(&t, &party).await.remove(0);
    assert_eq!(record.unblocked_by, Some(deputy.id));
    assert_eq!(record.unblock_basis, Some(UnblockBasis::Delisted));
}

/// AC5: with two open records every "blocked by" user and every user
/// without the restricted permission is refused; bad files are refused and
/// nothing is stored.
#[renox::test]
async fn every_open_record_counts_and_bad_files_are_refused() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let (a, b, mlro, deputy) = (
        t.user_a().await,
        t.user_b().await,
        t.mlro().await,
        deputy(&t).await,
    );
    block(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;
    // A second record, as a later issue adds it without a transition.
    BlockRecord::open(
        al_noor.id,
        "aml_concern",
        true,
        "Cash deposits",
        Some(mlro.id),
    )
    .insert(t.db())
    .await
    .unwrap();

    for user in [&a, &mlro, &b] {
        unblock(&t, user, &al_noor, "cleared", None, &[])
            .await
            .assert_status(422)
            .assert_see(NEUTRAL);
    }
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Blocked);

    let too_large = vec![b'x'; 20 * 1024 * 1024 + 1];
    for (name, bytes) in [("malware.exe", PDF), ("huge.pdf", too_large.as_slice())] {
        unblock(&t, &deputy, &al_noor, "cleared", None, &[(name, bytes)])
            .await
            .assert_status(422);
    }
    assert!(
        UnblockEvidence::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(records(&t, &al_noor).await.iter().all(BlockRecord::is_open));
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Blocked);

    unblock(&t, &deputy, &al_noor, "cleared", None, &[])
        .await
        .assert_ok();
    let party = t.reload(&al_noor).await;
    assert_eq!(party.status, PartyStatus::Active);
    let records = records(&t, &party).await;
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.unblocked_by == Some(deputy.id)));
    assert_eq!(t.unblocked()[0].display_reason, "on_hold");
}

/// What a test-only subscription is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubStatus {
    Active,
    Paused,
    Ended,
}

/// A test-only subscription model: `(party, status)` rows, in memory.
type Subscriptions = Arc<Mutex<Vec<(i64, SubStatus)>>>;

fn statuses(subs: &Subscriptions, party: i64) -> Vec<SubStatus> {
    subs.lock()
        .unwrap()
        .iter()
        .filter(|(owner, _)| *owner == party)
        .map(|(_, status)| *status)
        .collect()
}

/// AC6: the owning module pauses on `PartyBlocked` and resumes only what it
/// paused and what is still paused on `PartyUnblocked`.
#[renox::test]
async fn a_module_resumes_what_it_paused_and_not_what_was_ended() {
    let subs: Subscriptions = Arc::new(Mutex::new(Vec::new()));
    let (on_block, on_unblock) = (Arc::clone(&subs), Arc::clone(&subs));
    let t = boot_custom(WlParties::new(), true, move |app| {
        app.listen(move |event: PartyBlocked, _state: AppState| {
            let subs = Arc::clone(&on_block);
            async move {
                for (owner, status) in subs.lock().unwrap().iter_mut() {
                    if *owner == event.party.0 && *status == SubStatus::Active {
                        *status = SubStatus::Paused;
                    }
                }
                Ok(())
            }
        })
        .listen(move |event: PartyUnblocked, _state: AppState| {
            let subs = Arc::clone(&on_unblock);
            async move {
                for (owner, status) in subs.lock().unwrap().iter_mut() {
                    if *owner == event.party.0 && *status == SubStatus::Paused {
                        *status = SubStatus::Active;
                    }
                }
                Ok(())
            }
        })
    })
    .await;
    let al_noor = t.party(AL_NOOR).await;
    let gulf = t.party(GULF_BUILD).await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    *subs.lock().unwrap() = vec![
        (al_noor.id, SubStatus::Active),
        (al_noor.id, SubStatus::Active),
        (gulf.id, SubStatus::Active),
    ];

    block(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;
    assert_eq!(statuses(&subs, al_noor.id), [SubStatus::Paused; 2]);
    // The decider ends the second one while the party is blocked.
    subs.lock().unwrap()[1].1 = SubStatus::Ended;

    unblock(&t, &b, &al_noor, "paid in full", None, &[])
        .await
        .assert_ok();
    assert_eq!(
        statuses(&subs, al_noor.id),
        [SubStatus::Active, SubStatus::Ended]
    );
    assert_eq!(statuses(&subs, gulf.id), [SubStatus::Active]);
}

/// AC7: an open review task of a booking list gets the note and goes back to
/// the clinic manager role, still open.
#[renox::test]
async fn an_open_review_task_goes_back_to_its_decider() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    block(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;
    let mut task = ReviewTask::new(
        al_noor.id,
        "booking",
        Some(("clinics", 3)),
        Some("clinic_manager"),
        "non_payment",
    );
    task.assigned_user = Some(a.id);
    task.insert(t.db()).await.unwrap();
    ReviewTaskItem::pending(task.id, "scheduling.booking", 12, "Booking list, 3 visits")
        .insert(t.db())
        .await
        .unwrap();

    unblock(&t, &b, &al_noor, "paid in full", None, &[])
        .await
        .assert_ok();
    let task = ReviewTask::find(t.db(), task.id).await.unwrap().unwrap();
    assert_eq!(task.status, ReviewTaskStatus::Open);
    assert_eq!(task.assigned_user, None);
    assert_eq!(task.decider_role.as_deref(), Some("clinic_manager"));
    let notes = task.notes(t.db()).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].body, "Party unblocked");
    assert_eq!(task.items(t.db()).await.unwrap().len(), 1);
}

/// AC8: a party left `blocked` with an open record (as a restore from
/// `archived` leaves it, fixture) is lifted like any other.
#[renox::test]
async fn a_restored_blocked_party_is_lifted() {
    let t = boot().await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    let mut party = Party {
        code: "P-90001".to_owned(),
        kind: PartyKind::Company,
        display_name: "Restored Trading LLC".to_owned(),
        legal_name_en: "Restored Trading LLC".to_owned(),
        status: PartyStatus::Blocked,
        hold_reason: Some("non_payment".to_owned()),
        ..Party::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    party
        .insert_in_state(&mut tx, PartyStatus::Blocked, "fixture")
        .await
        .unwrap();
    BlockRecord::open(party.id, "non_payment", false, "Before archive", Some(a.id))
        .insert(&mut tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    unblock(&t, &b, &party, "paid in full", None, &[])
        .await
        .assert_ok();
    let party = t.reload(&party).await;
    assert_eq!(party.status, PartyStatus::Active);
    let records = records(&t, &party).await;
    assert!(!records[0].is_open());
    assert_eq!(records[0].unblocked_by, Some(b.id));
    assert_eq!(t.unblocked().len(), 1);
}

/// AC9: unblocking an active party is refused and emits nothing; the page
/// shows the action and the closed record; the evidence download has the
/// record's visibility.
#[renox::test]
async fn the_page_shows_the_action_and_the_evidence_to_those_who_may() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let gulf = t.party(GULF_BUILD).await;
    let (a, b) = (t.user_a().await, t.user_b().await);
    let (mlro, deputy) = (t.mlro().await, deputy(&t).await);

    // Active: refused (`NotFrom`), no event.
    let refused = unblock(&t, &b, &al_noor, "paid in full", None, &[]).await;
    assert!(!refused.status.is_success(), "{}", refused.status);
    assert!(t.unblocked().is_empty());
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Active);

    // B sees the action on a blocked party and the closed record afterwards.
    block(&t, &a, &al_noor, "non_payment", "3 bounced cheques").await;
    let page = format!("/parties/{}", al_noor.id);
    let action = format!("/parties/{}/unblock", al_noor.id);
    t.app.acting_as(&b);
    t.app.get(&page).await.assert_ok().assert_see(&action);
    unblock(&t, &b, &al_noor, "paid in full", None, &[])
        .await
        .assert_ok();
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("Bilal")
        .assert_see("paid in full")
        .assert_dont_see(&action);

    // Sanctions: the deputy sees the evidence file, B gets 404.
    block(&t, &mlro, &gulf, "sanctions", "List match on the owner").await;
    unblock(
        &t,
        &deputy,
        &gulf,
        "match found false",
        Some("false_match"),
        &[("m.pdf", PDF)],
    )
    .await
    .assert_ok();
    let record = records(&t, &gulf).await.remove(0);
    let file = UnblockEvidence::for_record(t.db(), record.id)
        .await
        .unwrap()
        .remove(0);
    let download = format!(
        "/parties/{}/block-records/{}/evidence/{}",
        gulf.id, record.id, file.id
    );
    t.app.acting_as(&deputy);
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_see("m.pdf")
        .assert_see("Match found false");
    let got = t.app.get(&download).await;
    got.assert_ok();
    assert_eq!(got.body.as_ref(), PDF);
    t.app.acting_as(&b);
    t.app.get(&download).await.assert_not_found();
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_dont_see("m.pdf");
}
