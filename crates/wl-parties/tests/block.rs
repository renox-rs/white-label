//! Blocking a party with a reason code (wl-parties §4 `block`, `PTY.ACL.02`,
//! no tipping off): the acceptance checks AC4–AC9 and AC11 of #34.

mod support;

use renox::prelude::*;
use renox::state_machine;
use wl_kernel::PartyStatus;
use wl_parties::{BlockRecord, Party, block_by_system, permissions as perm};

use support::{Booted, boot};

const AL_NOOR: &str = "Al Noor Trading LLC";
const GULF_BUILD: &str = "Gulf Build Materials LLC";

/// The party's Block records, newest first.
async fn records(t: &Booted, party: &Party) -> Vec<BlockRecord> {
    BlockRecord::for_party(t.db(), party.id).await.unwrap()
}

/// `POST /parties/{id}/block` as htmx, as the block sheet sends it.
async fn post_block(
    t: &Booted,
    party: &Party,
    code: &str,
    note: &str,
) -> renox::testing::TestResponse {
    t.app
        .htmx()
        .post(
            &format!("/parties/{}/block", party.id),
            &[("reason_code", code), ("note", note)],
        )
        .await
}

/// AC4: user A blocks Al Noor Trading for non-payment.
#[renox::test]
async fn a_user_blocks_a_party_for_non_payment() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let a = t.user_a().await;
    t.app.acting_as(&a);
    let page = format!("/parties/{}", al_noor.id);
    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see(&format!("/parties/{}/block", al_noor.id));
    post_block(&t, &al_noor, "non_payment", "3 bounced cheques")
        .await
        .assert_ok();

    let party = t.reload(&al_noor).await;
    assert_eq!(party.status, PartyStatus::Blocked);
    assert_eq!(party.hold_reason.as_deref(), Some("non_payment"));
    let records = records(&t, &party).await;
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.reason_code, "non_payment");
    assert!(!record.restricted);
    assert_eq!(record.note, "3 bounced cheques");
    assert_eq!(record.blocked_by, Some(a.id));
    assert!(record.is_open());

    let blocked = t.blocked();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].party, party.party_id());
    assert_eq!(blocked[0].display_reason, "non_payment");

    let history = state_machine::history(t.db(), Party::TABLE, party.id)
        .await
        .unwrap();
    let row = history.last().expect("a history row");
    assert_eq!(row.transition, "block");
    assert_eq!(row.actor_id, Some(a.id));
    assert_eq!(row.reason.as_deref(), Some("non_payment"));

    t.app
        .get(&page)
        .await
        .assert_ok()
        .assert_see("On hold — Non-payment")
        .assert_see("3 bounced cheques")
        // Blocked: the block action is gone.
        .assert_dont_see(&format!("/parties/{}/block", al_noor.id));
    t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_see("On hold — Non-payment");
}

/// AC5: the MLRO blocks with `aml_concern`; only the Block record knows.
#[renox::test]
async fn a_restricted_block_leaves_only_on_hold_outside_the_record() {
    let t = boot().await;
    let gulf = t.party(GULF_BUILD).await;
    let mlro = t.mlro().await;
    t.app.acting_as(&mlro);
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_see(r#"value="aml_concern""#)
        .assert_see(r#"value="sanctions""#);
    post_block(&t, &gulf, "aml_concern", "Unusual cash deposits")
        .await
        .assert_ok();

    let party = t.reload(&gulf).await;
    assert_eq!(party.status, PartyStatus::Blocked);
    assert_eq!(party.hold_reason.as_deref(), Some("on_hold"));
    let blocked = t.blocked();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].display_reason, "on_hold");

    let history = state_machine::history(t.db(), Party::TABLE, party.id)
        .await
        .unwrap();
    let row = history.last().expect("a history row");
    assert_eq!(row.reason.as_deref(), Some("on_hold"));
    let meta = row.metadata.to_string();
    assert!(!meta.contains("aml_concern"), "{meta}");
    assert!(!meta.contains("Unusual cash"), "{meta}");

    let records = records(&t, &party).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason_code, "aml_concern");
    assert!(records[0].restricted);
    assert_eq!(records[0].note, "Unusual cash deposits");
    assert_eq!(records[0].blocked_by, Some(mlro.id));

    // The MLRO sees the real reason in the Block records.
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_see("Block records")
        .assert_see("AML concern")
        .assert_see("Unusual cash deposits");

    // User B sees only "On hold": card, list and search.
    let b = t.user_b().await;
    t.app.acting_as(&b);
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_see("On hold")
        .assert_dont_see("aml_concern")
        .assert_dont_see("AML concern")
        .assert_dont_see("Unusual cash deposits");
    for uri in ["/parties", "/parties?search=Gulf"] {
        t.app
            .get(uri)
            .await
            .assert_ok()
            .assert_see("Gulf Build Materials")
            .assert_see("On hold")
            .assert_dont_see("aml_concern")
            .assert_dont_see("AML concern");
    }
}

/// AC6: user B cannot read the restricted record nor pick a restricted code.
#[renox::test]
async fn others_cannot_see_or_use_a_restricted_reason() {
    let t = boot().await;
    let gulf = t.party(GULF_BUILD).await;
    block_by_system(t.app.state(), &gulf, "aml_concern", "Flagged", None)
        .await
        .unwrap();
    let record = records(&t, &gulf).await.remove(0);

    let b = t.user_b().await;
    t.app.acting_as(&b);
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_dont_see("Block records")
        .assert_dont_see("Flagged");
    t.app
        .get(&format!("/parties/{}/block-records/{}", gulf.id, record.id))
        .await
        .assert_not_found();

    // The dialog offers B no restricted code.
    let al_noor = t.party(AL_NOOR).await;
    t.app
        .get(&format!("/parties/{}", al_noor.id))
        .await
        .assert_ok()
        .assert_see(r#"value="non_payment""#)
        .assert_see(r#"value="on_hold""#)
        .assert_dont_see("sanctions")
        .assert_dont_see("aml_concern");
    // A forged post is refused and nothing changes.
    post_block(&t, &al_noor, "aml_concern", "forged")
        .await
        .assert_invalid("reason_code");
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Active);
    assert!(records(&t, &al_noor).await.is_empty());

    // The MLRO reads the record.
    let mlro = t.mlro().await;
    t.app.acting_as(&mlro);
    t.app
        .get(&format!("/parties/{}/block-records/{}", gulf.id, record.id))
        .await
        .assert_ok()
        .assert_json_path("reason_code", "aml_concern");
    // Not under another party.
    t.app
        .get(&format!(
            "/parties/{}/block-records/{}",
            al_noor.id, record.id
        ))
        .await
        .assert_not_found();
}

/// AC7: a blank note, an unknown code, an already blocked party and a user
/// without `parties.block` are all refused.
#[renox::test]
async fn bad_blocks_are_refused() {
    let t = boot().await;
    let al_noor = t.party(AL_NOOR).await;
    let a = t.user_a().await;
    t.app.acting_as(&a);

    post_block(&t, &al_noor, "non_payment", "   ")
        .await
        .assert_invalid("note");
    post_block(&t, &al_noor, "no_such_code", "a note")
        .await
        .assert_invalid("reason_code");
    assert_eq!(t.reload(&al_noor).await.status, PartyStatus::Active);
    assert!(records(&t, &al_noor).await.is_empty());
    assert!(t.blocked().is_empty());

    post_block(&t, &al_noor, "non_payment", "3 bounced cheques")
        .await
        .assert_ok();
    // Already blocked: `NotFrom`, no second record.
    let blocked = t.reload(&al_noor).await;
    let again = wl_parties::block_by_system(t.app.state(), &blocked, "other", "again", None)
        .await
        .unwrap_err();
    assert!(
        matches!(again, state_machine::TransitionError::NotFrom(_)),
        "{again:?}"
    );
    let res = post_block(&t, &blocked, "other", "again").await;
    assert!(
        res.status.is_client_error(),
        "a second block answers {}",
        res.status
    );
    assert_eq!(records(&t, &al_noor).await.len(), 1);
    assert_eq!(t.blocked().len(), 1);

    // Without `parties.block`.
    let gulf = t.party(GULF_BUILD).await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/parties/{}", gulf.id))
        .await
        .assert_ok()
        .assert_dont_see(&format!("/parties/{}/block", gulf.id));
    post_block(&t, &gulf, "non_payment", "a note")
        .await
        .assert_forbidden();
    assert_eq!(t.reload(&gulf).await.status, PartyStatus::Active);
}

/// AC8: nothing is sent or queued for the party's contacts.
#[renox::test]
async fn the_party_is_not_told() {
    let t = boot().await;
    t.app.fake_notifications();
    let al_noor = t.party(AL_NOOR).await;
    let gulf = t.party(GULF_BUILD).await;
    let a = t.user_a().await;
    let mlro = t.mlro().await;
    t.app.acting_as(&a);
    post_block(&t, &al_noor, "non_payment", "3 bounced cheques")
        .await
        .assert_ok();
    t.app.acting_as(&mlro);
    post_block(&t, &gulf, "sanctions", "Listed")
        .await
        .assert_ok();

    t.app.assert_nothing_notified();
    assert!(t.app.queued_jobs().await.is_empty());
    t.app.run_all_jobs().await;
    assert!(t.app.sent_mail().is_empty(), "{:?}", t.app.sent_mail());
}

/// AC9: user B's export shows "On hold" and no restricted code; the filter
/// offers none.
#[renox::test]
async fn an_export_shows_on_hold_for_a_restricted_block() {
    let t = boot().await;
    let gulf = t.party(GULF_BUILD).await;
    let al_noor = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    t.app.acting_as(&mlro);
    post_block(&t, &gulf, "aml_concern", "Unusual cash deposits")
        .await
        .assert_ok();
    post_block(&t, &al_noor, "non_payment", "3 bounced cheques")
        .await
        .assert_ok();

    let b = t.user_b().await;
    t.app.acting_as(&b);
    let csv = t.app.get("/parties?export=csv").await.assert_ok().text();
    let gulf_line = csv
        .lines()
        .find(|line| line.contains("Gulf Build Materials"))
        .expect("the blocked party is exported");
    assert!(gulf_line.contains("On hold"), "{gulf_line}");
    let al_noor_line = csv
        .lines()
        .find(|line| line.contains("Al Noor Trading"))
        .expect("Al Noor is exported");
    assert!(
        al_noor_line.contains("On hold — Non-payment"),
        "{al_noor_line}"
    );
    for hidden in ["aml_concern", "sanctions", "AML concern", "Sanctions"] {
        assert!(!csv.contains(hidden), "the export shows {hidden}");
    }

    // The list's hold reason filter offers B no restricted code.
    t.app
        .get("/parties")
        .await
        .assert_ok()
        .assert_dont_see("sanctions")
        .assert_dont_see("aml_concern");
    // Filtering on `on_hold` finds the restricted block.
    t.app
        .get("/parties?in.hold_reason=on_hold")
        .await
        .assert_ok()
        .assert_see("Gulf Build Materials")
        .assert_dont_see("Al Noor Trading");

    // Without `parties.export` there is no export.
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    t.app.acting_as(&viewer);
    t.app.get("/parties?export=csv").await.assert_forbidden();
}

/// AC11: the system blocks for sanctions with no user.
#[renox::test]
async fn the_system_blocks_for_sanctions() {
    let t = boot().await;
    let gulf = t.party(GULF_BUILD).await;
    block_by_system(
        t.app.state(),
        &gulf,
        "sanctions",
        "Confirmed list match",
        None,
    )
    .await
    .unwrap();

    let party = t.reload(&gulf).await;
    assert_eq!(party.status, PartyStatus::Blocked);
    assert_eq!(party.hold_reason.as_deref(), Some("on_hold"));
    let records = records(&t, &party).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reason_code, "sanctions");
    assert!(records[0].restricted);
    assert_eq!(records[0].blocked_by, None);
    assert_eq!(records[0].note, "Confirmed list match");
    let blocked = t.blocked();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].display_reason, "on_hold");
    let history = state_machine::history(t.db(), Party::TABLE, party.id)
        .await
        .unwrap();
    let row = history.last().expect("a history row");
    assert_eq!(row.transition, "block_by_system");
    assert_eq!(row.actor_id, None);
    assert_eq!(row.reason.as_deref(), Some("on_hold"));
    assert!(!row.metadata.to_string().contains("sanctions"));

    // A reviewer's run keeps who caused it.
    let al_noor = t.party(AL_NOOR).await;
    let mlro = t.mlro().await;
    block_by_system(t.app.state(), &al_noor, "sanctions", "Match", Some(mlro.id))
        .await
        .unwrap();
    assert_eq!(
        BlockRecord::for_party(t.db(), al_noor.id).await.unwrap()[0].blocked_by,
        Some(mlro.id)
    );
}
