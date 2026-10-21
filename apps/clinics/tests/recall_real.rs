//! A recalled lot traced to patients and the neutral notices they get
//! (BL-CLN-14, #362: `CLN.ACL.01`, `STK.ACL.05`), against the **real**
//! wl-inventory: the recall, its hold and its trace call the clinic's
//! `TracePartyResolver` (visit to patient, or to the guardian of a dependant);
//! the lines come from the consumables issued to real visits. The notices go
//! through the kernel's notifier; the give-up uses wl-notify's own staff-task
//! reaction.

mod support;

use renox::auth::permissions;
use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_inventory::{
    Lot, LotStatus, NoticeStatus, PendingIssue, PendingStatus, Recall, RecallLine, RecallStatus,
};
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_kernel::tickets::Assignee;
use wl_kernel::{PartyId, Subject};

use clinics::permissions::{
    RECALLS_CALL, RECALLS_NOTIFY, RECALLS_TRACE, RECEPTIONIST_ROLE, VISITS_VIEW,
};
use clinics::recall_notice::{NOTICE_TYPE, RecallNotice, RecallNoticeStatus};
use clinics::{ConsumableIssue, Visit};
use support::Messages;
use support::desk::{Desk, desk_with_stock, desk_with_stock_and_notify};
use support::stock::{Store, elsewhere, receive, store, used_filler, used_filler_with};

const P1: PartyId = PartyId(70);
const P2: PartyId = PartyId(71);
const P3: PartyId = PartyId(72);
const P4: PartyId = PartyId(73);
const GUARDIAN: PartyId = PartyId(74);
const P5: PartyId = PartyId(75);
static P2_EMAIL: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| support::Guardians::email_of(P2));
const REASON: &str = "Contamination found by the supplier";

/// The names the pages show.
fn name_patients(d: &Desk) {
    d.t.guardians.name(P1.0, "Amal Rashid");
    d.t.guardians.name(P2.0, "Omar Saleh");
    d.t.guardians.name(P3.0, "Noor Haddad");
    d.t.guardians.name(P4.0, "Lina Karim");
    d.t.guardians.name(GUARDIAN.0, "Karim Aziz");
    d.t.guardians.name(P5.0, "Sami Farouk");
}

/// The clinic manager: reads the trace, sends the notices, opens recalls.
async fn manager(d: &Desk) -> User {
    d.t.user(
        "Mia",
        &[
            RECALLS_TRACE,
            RECALLS_NOTIFY,
            VISITS_VIEW,
            wl_inventory::permissions::RECALLS_MANAGE,
        ],
    )
    .await
}

/// Opens a recall of lot `lot` of the filler through `POST /recalls`.
async fn open_recall(d: &Desk, item: i64, lot: &str) -> Recall {
    let mia = manager(d).await;
    let item = item.to_string();
    d.post(
        &mia,
        "/recalls",
        &[
            ("source", "supplier"),
            ("reason", REASON),
            ("items", item.as_str()),
            ("lot_numbers", lot),
        ],
    )
    .await
    .assert_ok();
    let recall = Recall::query()
        .order_by(Recall::ID)
        .get(d.t.db())
        .await
        .unwrap()
        .pop()
        .expect("a recall");
    assert_eq!(recall.status, RecallStatus::Traced);
    recall
}

/// The notices asked of the notifier.
fn notices(d: &Desk) -> Vec<NotifyRequest> {
    d.t.messages
        .sent()
        .into_iter()
        .filter(|request| request.type_key == NOTICE_TYPE)
        .collect()
}

/// The lines of the recall, oldest first.
async fn lines(d: &Desk, recall: &Recall) -> Vec<RecallLine> {
    RecallLine::query()
        .where_(RecallLine::RECALL_ID.eq(recall.id))
        .order_by(RecallLine::ID)
        .get(d.t.db())
        .await
        .unwrap()
}

/// Done when 1: the lot issued to the visits of two patients; opening the
/// recall lists each visit with its patient, the day and the branch, and
/// nothing of why they came or what the product is.
#[renox::test]
async fn opening_a_recall_lists_the_visits_and_their_patients() {
    let d = desk_with_stock().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    let v1 = used_filler(&d, P1, 1, 9, 1, "L1").await;
    let v2 = used_filler(&d, P2, 2, 10, 1, "L1").await;

    let recall = open_recall(&d, s.item.id, "L1").await;
    let listed = lines(&d, &recall).await;
    assert_eq!(listed.len(), 2, "{listed:?}");
    let mut parties: Vec<(Option<Subject>, Option<i64>)> = listed
        .iter()
        .map(|line| (line.used_by(), line.party_id))
        .collect();
    parties.sort_by_key(|(subject, _)| subject.as_ref().map(|s| s.subject_id));
    assert_eq!(
        parties,
        vec![
            (Some(v1.subject()), Some(P1.0)),
            (Some(v2.subject()), Some(P2.0))
        ]
    );

    let mia = manager(&d).await;
    let page = d
        .get(&mia, &format!("/clinics/recalls/{}", recall.id))
        .await;
    page.assert_ok()
        .assert_see("Amal Rashid")
        .assert_see("Omar Saleh")
        .assert_see(&v1.visit_date.to_string())
        .assert_see(&v2.visit_date.to_string())
        .assert_see("test.branch 2");
    let text = page.text();
    for secret in ["Chest pain", "angina", "Filler", "filler", REASON] {
        assert!(!text.contains(secret), "the trace list shows `{secret}`");
    }
    // Each view is audited (`CLN.ACL.01`).
    let log = renox::audit::for_subject(d.t.db(), "inventory.recall", recall.id, 10)
        .await
        .unwrap();
    assert_eq!(
        log.iter()
            .filter(|e| e.action == clinics::recalls::VIEWED)
            .count(),
        1
    );
}

/// Done when 6: a receptionist without health data access cannot open the
/// trace list, and a viewer of the schedule cannot either.
#[renox::test]
async fn a_receptionist_cannot_open_the_trace_list() {
    let d = desk_with_stock().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    used_filler(&d, P1, 1, 9, 1, "L1").await;
    let recall = open_recall(&d, s.item.id, "L1").await;
    let path = format!("/clinics/recalls/{}", recall.id);

    d.get(&d.rita, &path).await.assert_forbidden();
    let vera = d.t.viewer().await;
    d.get(&vera, &path).await.assert_forbidden();
    d.post(&d.rita, &format!("{path}/notices"), &[("x", "y")])
        .await
        .assert_forbidden();
    let log = renox::audit::for_subject(d.t.db(), "inventory.recall", recall.id, 10)
        .await
        .unwrap();
    assert!(
        log.iter().all(|e| e.action != clinics::recalls::VIEWED),
        "a refused view is not a view"
    );
}

/// The audit entries of `action` about the notice `notice`, as
/// (the user, the data).
async fn audited(d: &Desk, notice: i64, action: &str) -> Vec<(Option<i64>, String)> {
    renox::audit::for_subject(d.t.db(), "clinics.recall_notice", notice, 20)
        .await
        .unwrap()
        .into_iter()
        .filter(|entry| entry.action == action)
        .map(|entry| (entry.user_id, entry.data.to_string()))
        .collect()
}

/// Moves the clock of the app `minutes` ahead: what follows is recorded
/// after what went before, by the clock, not by a rewritten column.
fn pass(d: &Desk, minutes: u64) {
    d.t.app.travel(std::time::Duration::from_secs(minutes * 60));
}

/// The visit of `patient` that used `units` of filler from `lot`, checked out
/// at the clock of the app.
async fn used(d: &Desk, patient: PartyId, hour: u32, units: i64, lot: &str) -> Visit {
    d.t.app
        .at_travelled_time(used_filler(d, patient, 1, hour, units, lot))
        .await
}

/// A receipt of `quantity` units of `lot` at the store, and the pending issues
/// it covers posted, at the clock of the app.
async fn arrives(d: &Desk, s: &Store, lot: &Lot, quantity: i64) {
    d.t.app
        .at_travelled_time(async {
            receive(d, s, lot, quantity).await;
            wl_inventory::pending_issue::post_on_arrival(
                d.t.state(),
                s.store.id,
                &[s.item.id],
                "receipt",
            )
            .await
            .unwrap();
        })
        .await;
}

/// Done when 2: a pending issue of the item at the store, recorded and then
/// posted with a lot assigned on arrival while the recalled lot was on hand,
/// is listed "lot uncertain" for its patient; one posted before the lot ever
/// arrived is not listed. The order is made by real receipts, pending issues
/// and the clock, not by rewriting dates.
#[renox::test]
async fn a_pending_issue_inside_the_lot_window_is_listed_uncertain() {
    let d = desk_with_stock().await;
    name_patients(&d);
    // Nothing at the store: both visits become pending issues.
    let s = store(&d, 0, 0).await;
    let earlier = used(&d, P5, 9, 1, "").await;
    let inside = used(&d, P3, 10, 1, "").await;
    for visit in [&earlier, &inside] {
        let rows = ConsumableIssue::of_visit(d.t.db(), visit.id).await.unwrap();
        assert!(rows.iter().all(ConsumableIssue::is_uncertain), "{rows:?}");
    }
    // L2 arrives first: it covers the oldest pending issue only (P5's). That
    // was before the recalled lot L1 existed at the store.
    pass(&d, 1);
    arrives(&d, &s, &s.l2, 1).await;
    // L1, the recalled lot, arrives a minute later and covers P3's.
    pass(&d, 1);
    arrives(&d, &s, &s.l1, 10).await;
    pass(&d, 1);

    let recall = open_recall(&d, s.item.id, "L1").await;
    let listed = lines(&d, &recall).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].used_by(), Some(inside.subject()));
    assert_eq!(listed[0].party_id, Some(P3.0));
    assert_eq!(listed[0].kind.as_str(), "uncertain");

    let mia = manager(&d).await;
    let page = d
        .get(&mia, &format!("/clinics/recalls/{}", recall.id))
        .await;
    page.assert_ok()
        .assert_see("Noor Haddad")
        .assert_see("Lot uncertain");
    assert!(!page.text().contains("Sami Farouk"), "{}", page.text());
}

/// The pending issues of `visit`, all of them posted on arrival.
async fn posted_on_arrival(d: &Desk, visit: &Visit) -> bool {
    let pending: Vec<PendingIssue> = PendingIssue::query()
        .where_(PendingIssue::SOURCE_ID.eq(visit.id))
        .get(d.t.db())
        .await
        .unwrap();
    !pending.is_empty()
        && pending
            .iter()
            .all(|p| p.status == PendingStatus::Posted && p.assigned_on_arrival)
}

/// The end of the lot's window, by a hold: the recalled lot is held, and a
/// pending issue recorded after the hold (the visit found nothing to take
/// because the lot is held) and posted later with another lot is not listed,
/// also when the trace runs again; the visit that took the lot before the
/// hold is.
#[renox::test]
async fn a_pending_issue_recorded_after_the_hold_is_not_listed() {
    let d = desk_with_stock().await;
    name_patients(&d);
    let s = store(&d, 0, 0).await;
    pass(&d, 1);
    arrives(&d, &s, &s.l1, 10).await;
    pass(&d, 1);
    let before = used(&d, P1, 9, 1, "L1").await;
    pass(&d, 1);

    // The recall holds L1: its window ends here.
    let recall = open_recall(&d, s.item.id, "L1").await;
    let held = Lot::find_or_404(d.t.db(), s.l1.id).await.unwrap();
    assert_eq!(held.status, LotStatus::OnHold);
    pass(&d, 1);

    // After the hold nothing can be taken: the visit's issue is pending, and
    // L2 covers it when it arrives.
    let after = used(&d, P3, 10, 1, "").await;
    let rows = ConsumableIssue::of_visit(d.t.db(), after.id).await.unwrap();
    assert!(rows.iter().all(ConsumableIssue::is_uncertain), "{rows:?}");
    pass(&d, 1);
    arrives(&d, &s, &s.l2, 1).await;
    assert!(posted_on_arrival(&d, &after).await);

    let mia = manager(&d).await;
    d.post(
        &mia,
        &format!("/recalls/{}/trace", recall.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    let listed = lines(&d, &recall).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].used_by(), Some(before.subject()));
    assert_eq!(listed[0].kind.as_str(), "confirmed");
    let page = d
        .get(&mia, &format!("/clinics/recalls/{}", recall.id))
        .await;
    page.assert_ok().assert_see("Amal Rashid");
    assert!(!page.text().contains("Noor Haddad"), "{}", page.text());
}

/// The end of the lot's window, by its balance: the recalled lot is used up
/// (its last unit leaves), and a pending issue recorded after that, posted
/// later with another lot, is not listed when the recall is opened; the visit
/// that took the last unit is.
#[renox::test]
async fn a_pending_issue_recorded_after_the_balance_reached_zero_is_not_listed() {
    let d = desk_with_stock().await;
    name_patients(&d);
    let s = store(&d, 0, 0).await;
    pass(&d, 1);
    arrives(&d, &s, &s.l1, 1).await;
    pass(&d, 1);
    let last = used(&d, P1, 9, 1, "L1").await;
    pass(&d, 1);

    // L1 is used up: the next visit finds nothing and is pending.
    let after = used(&d, P3, 10, 1, "").await;
    let rows = ConsumableIssue::of_visit(d.t.db(), after.id).await.unwrap();
    assert!(rows.iter().all(ConsumableIssue::is_uncertain), "{rows:?}");
    pass(&d, 1);
    arrives(&d, &s, &s.l2, 1).await;
    assert!(posted_on_arrival(&d, &after).await);
    pass(&d, 1);

    let recall = open_recall(&d, s.item.id, "L1").await;
    let listed = lines(&d, &recall).await;
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0].used_by(), Some(last.subject()));
    assert_eq!(listed[0].kind.as_str(), "confirmed");
    let mia = manager(&d).await;
    let page = d
        .get(&mia, &format!("/clinics/recalls/{}", recall.id))
        .await;
    page.assert_ok().assert_see("Amal Rashid");
    assert!(!page.text().contains("Noor Haddad"), "{}", page.text());
}

/// Done when 3: a patient with two visits gets one notice listing both days,
/// with the clinic's name and phone and no product, reason or clinical
/// detail; sending again, when everyone was told, is refused.
#[renox::test]
async fn a_patient_gets_one_neutral_notice_for_all_their_visits() {
    let d = desk_with_stock().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    let first = used_filler(&d, P1, 1, 9, 1, "L1").await;
    let second = used_filler(&d, P1, 2, 9, 1, "L1").await;
    used_filler(&d, P2, 1, 10, 1, "L1").await;
    let recall = open_recall(&d, s.item.id, "L1").await;
    assert_eq!(lines(&d, &recall).await.len(), 3);

    let mia = manager(&d).await;
    d.post(
        &mia,
        &format!("/clinics/recalls/{}/notices", recall.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();

    let sent = notices(&d);
    assert_eq!(sent.len(), 2, "one per patient: {sent:?}");
    let to_p1 = sent
        .iter()
        .find(|r| r.recipients == vec![NotifyRecipient::Party { id: P1 }])
        .expect("P1 was told");
    let body = Messages::body(to_p1);
    assert!(body.contains("Palm Grove Clinic LLC"), "{body}");
    assert!(body.contains("+971 4 555 0100"), "{body}");
    assert!(body.contains(&first.visit_date.to_string()), "{body}");
    assert!(body.contains(&second.visit_date.to_string()), "{body}");
    for secret in ["Filler", "filler", "L1", REASON, "Chest pain", "recall"] {
        assert!(!body.contains(secret), "the notice says `{secret}`: {body}");
    }
    assert!(!to_p1.data.to_string().contains("Filler"), "{}", to_p1.data);

    // The lines are told in the recall too.
    assert!(
        lines(&d, &recall)
            .await
            .iter()
            .all(|line| line.notice_status == NoticeStatus::Sent)
    );
    let made = RecallNotice::of_recall(d.t.db(), recall.id).await.unwrap();
    assert_eq!(made.len(), 2);
    assert!(made.iter().all(|n| n.status == RecallNoticeStatus::Sent));

    // Who sent each notice is audited, with the recall and the number of
    // visits, nothing about the patient's health.
    for notice in &made {
        let entries = audited(&d, notice.id, clinics::recalls::SENT).await;
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].0, Some(mia.id));
        for secret in ["Filler", "filler", "L1", REASON, "Chest pain"] {
            assert!(!entries[0].1.contains(secret), "{}", entries[0].1);
        }
        assert!(
            entries[0].1.contains(&format!("\"recall\":{}", recall.id)),
            "{}",
            entries[0].1
        );
    }

    let again = d
        .post(
            &mia,
            &format!("/clinics/recalls/{}/notices", recall.id),
            &[("x", "y")],
        )
        .await;
    again.assert_status(422);
    assert_eq!(notices(&d).len(), 2, "nobody is told twice");
    for notice in &made {
        assert_eq!(
            audited(&d, notice.id, clinics::recalls::SENT).await.len(),
            1,
            "a refused send is not a send"
        );
    }
}

/// Done when 5: for a dependant the notice goes to the guardian, and the
/// trace names the patient.
#[renox::test]
async fn a_dependants_notice_goes_to_the_guardian() {
    let d = desk_with_stock().await;
    name_patients(&d);
    d.t.guardians.set(P4, GUARDIAN);
    let s = store(&d, 10, 0).await;
    let visit = used_filler(&d, P4, 1, 9, 1, "L1").await;
    let recall = open_recall(&d, s.item.id, "L1").await;
    let listed = lines(&d, &recall).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].used_by(), Some(visit.subject()));
    assert_eq!(listed[0].party_id, Some(GUARDIAN.0));

    let mia = manager(&d).await;
    let page = d
        .get(&mia, &format!("/clinics/recalls/{}", recall.id))
        .await;
    page.assert_ok()
        .assert_see("Lina Karim")
        .assert_see("Karim Aziz");
    d.post(
        &mia,
        &format!("/clinics/recalls/{}/notices", recall.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    let sent = notices(&d);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].recipients,
        vec![NotifyRecipient::Party { id: GUARDIAN }]
    );
}

/// A front desk user of one branch.
async fn desk_of(d: &Desk, name: &str, branch: &Subject) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(d.t.db(), name, &email, "password123")
        .await
        .unwrap();
    permissions::define_role(d.t.db(), RECEPTIONIST_ROLE, &[RECALLS_CALL])
        .await
        .unwrap();
    user.assign_role_in(
        d.t.db(),
        RECEPTIONIST_ROLE,
        &Scope::new(&branch.subject_type, branch.subject_id),
    )
    .await
    .unwrap();
    user
}

/// Done when 4: the real wl-notify sends the notices; when the mailbox of one
/// patient refuses it for good the message is abandoned, the rule gives up
/// (its own `MessageGaveUp`), the line is not told again, wl-notify's give-up
/// task goes to the front desk of the visit's branch only, due the next
/// working day, and the front desk of that branch records the call, which
/// marks the line sent. The other patient's notice stays sent.
#[renox::test]
async fn a_notice_that_gives_up_becomes_a_call_for_the_front_desk_of_the_branch() {
    let d = desk_with_stock_and_notify().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    used_filler(&d, P1, 1, 9, 1, "L1").await;
    let v2 = used_filler(&d, P2, 1, 10, 1, "L1").await;
    // The other branch's desk has the lower id and must not get the call.
    let elsewhere = desk_of(&d, "Zed", &support::other_branch()).await;
    let fay = desk_of(&d, "Fay", &support::branch()).await;
    d.t.guardians.reachable(P1);
    d.t.guardians.reachable(P2);
    d.t.outbox.refuse(&P2_EMAIL, "550", "mailbox unknown");
    let recall = open_recall(&d, s.item.id, "L1").await;
    let mia = manager(&d).await;
    d.post(
        &mia,
        &format!("/clinics/recalls/{}/notices", recall.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    let notice = RecallNotice::of_recall(d.t.db(), recall.id)
        .await
        .unwrap()
        .into_iter()
        .find(|n| n.party_id == P2.0)
        .expect("P2's notice");
    assert_eq!(notice.visit_ids.0, vec![v2.id]);

    // The notice reached P1; P2's mailbox refuses it for good: the real
    // wl-notify run abandons the message, finds no other channel, gives up,
    // and its own reaction opens the staff task.
    let sent = d.t.outbox.sent();
    assert_eq!(sent.len(), 0, "nothing left before the jobs run");
    let mut guard = 0;
    while d.t.app.run_jobs().await > 0 && guard < 20 {
        guard += 1;
    }
    let sent = d.t.outbox.sent();
    assert_eq!(sent.len(), 2, "one attempt per patient: {sent:?}");
    let to_p1 = sent
        .iter()
        .find(|out| out.address == support::PATIENT_EMAIL)
        .expect("P1's notice reached the provider");
    assert!(
        to_p1.body.contains("Palm Grove Clinic LLC"),
        "{}",
        to_p1.body
    );
    let told = RecallNotice::of_recall(d.t.db(), recall.id).await.unwrap();
    assert!(
        told.iter()
            .find(|n| n.party_id == P1.0)
            .is_some_and(|n| n.status == RecallNoticeStatus::Sent)
    );
    let abandoned = wl_notify::Message::query()
        .where_(wl_notify::Message::TYPE_KEY.eq(NOTICE_TYPE))
        .where_(wl_notify::Message::STATUS.eq(wl_notify::MessageStatus::Abandoned))
        .get(d.t.db())
        .await
        .unwrap();
    assert_eq!(abandoned.len(), 1, "only P2's message gave up");
    assert_eq!(abandoned[0].address.as_deref(), Some(P2_EMAIL.as_str()));
    let gave_up_at = abandoned[0].updated_at.expect("the give-up is stamped");

    let now = RecallNotice::find_or_404(d.t.db(), notice.id)
        .await
        .unwrap();
    assert_eq!(now.status, RecallNoticeStatus::GaveUp);
    let line = RecallLine::find_or_404(d.t.db(), now.line_ids.0[0])
        .await
        .unwrap();
    assert_eq!(line.notice_status, NoticeStatus::Pending);
    let tasks = d.t.tasks.opened();
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    assert_eq!(tasks[0].assignee, Some(Assignee::user(fay.id)));
    assert_ne!(tasks[0].assignee, Some(Assignee::user(elsewhere.id)));
    assert_eq!(
        tasks[0].due_at,
        Some(wl_kernel::calendar::calendar(d.t.state()).add_working_days(gave_up_at, 1))
    );
    for text in [&tasks[0].title, &tasks[0].description] {
        for secret in ["Filler", "filler", "L1", REASON] {
            assert!(!text.contains(secret), "the task says `{secret}`: {text}");
        }
    }

    // The front desk of the branch sees the call; the other branch does not.
    // Each view of a notice is audited for the user who saw it.
    d.get(&fay, "/clinics/recall-calls")
        .await
        .assert_ok()
        .assert_see("Omar Saleh");
    let other = d.get(&elsewhere, "/clinics/recall-calls").await;
    other.assert_ok();
    assert!(!other.text().contains("Omar Saleh"), "{}", other.text());
    let viewed = audited(&d, notice.id, clinics::recalls::CALL_VIEWED).await;
    assert_eq!(
        viewed.iter().map(|(user, _)| *user).collect::<Vec<_>>(),
        vec![Some(fay.id)],
        "Fay saw it, Zed did not"
    );
    d.get(&d.rita, "/clinics/recall-calls")
        .await
        .assert_forbidden();
    assert_eq!(
        audited(&d, notice.id, clinics::recalls::CALL_VIEWED)
            .await
            .len(),
        1,
        "a refused view is not a view"
    );
    let path = format!("/clinics/recall-calls/{}", notice.id);
    d.post(&elsewhere, &path, &[("outcome", "Reached")])
        .await
        .assert_forbidden();
    d.post(&fay, &path, &[("outcome", " ")])
        .await
        .assert_status(422);
    d.post(
        &fay,
        &path,
        &[("outcome", "Reached; will come in on Sunday")],
    )
    .await
    .assert_ok();
    let done = RecallNotice::find_or_404(d.t.db(), notice.id)
        .await
        .unwrap();
    assert_eq!(done.status, RecallNoticeStatus::Called);
    assert_eq!(
        done.call_outcome.as_deref(),
        Some("Reached; will come in on Sunday")
    );
    assert_eq!(done.called_by, Some(fay.id));
    let line = RecallLine::find_or_404(d.t.db(), done.line_ids.0[0])
        .await
        .unwrap();
    assert_eq!(line.notice_status, NoticeStatus::Sent);
}

/// The notice to a patient who reads Arabic is the Arabic text of the type,
/// filled by the real wl-notify: the clinic's name, the phone number in
/// Arabic, the visit days; no English phrase, no product, lot, reason or
/// anything clinical, and no template placeholder left over.
#[renox::test]
async fn the_arabic_notice_is_filled_and_neutral() {
    let d = desk_with_stock_and_notify().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    let first = used_filler(&d, P2, 1, 9, 1, "L1").await;
    let second = used_filler(&d, P2, 2, 9, 1, "L1").await;
    d.t.guardians.reachable_in(P2, "ar");
    let recall = open_recall(&d, s.item.id, "L1").await;
    let mia = manager(&d).await;
    d.post(
        &mia,
        &format!("/clinics/recalls/{}/notices", recall.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    while d.t.app.run_jobs().await > 0 {}

    let sent = d.t.outbox.sent();
    assert_eq!(sent.len(), 1, "{sent:?}");
    let out = &sent[0];
    assert_eq!(out.address, support::Guardians::email_of(P2));
    assert_eq!(out.language, "ar");
    assert_eq!(out.title, "رسالة من Palm Grove Clinic LLC");
    let body = &out.body;
    assert!(
        body.starts_with("Palm Grove Clinic LLC: يرجى الاتصال بنا على الرقم +971 4 555 0100"),
        "{body}"
    );
    assert!(body.contains(&first.visit_date.to_string()), "{body}");
    assert!(body.contains(&second.visit_date.to_string()), "{body}");
    assert!(!body.contains("{{"), "a placeholder is left: {body}");
    for secret in [
        "Filler",
        "filler",
        "L1",
        REASON,
        "Chest pain",
        "angina",
        "recall",
        "call us",
        "سحب",
    ] {
        assert!(!body.contains(secret), "the notice says `{secret}`: {body}");
    }
    assert!(!out.title.contains("Filler"), "{}", out.title);
}

/// A patient seen at two branches gets one notice for each branch (not one
/// notice whose call task lies in the branch of the first visit): when both
/// give up, the call of each goes to the front desk of its own branch, and
/// each branch's call list shows the days of that branch only.
#[renox::test]
async fn a_patient_seen_at_two_branches_is_called_from_each_branch() {
    let d = desk_with_stock_and_notify().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    let e = elsewhere(&d, &s, 10).await;
    let first = used_filler(&d, P1, 1, 9, 1, "L1").await;
    let second = used_filler_with(&d, &e.doctor, &e.service, P1, 2, 9, 1, "L1").await;
    assert_ne!(first.visit_date, second.visit_date);
    let fay = desk_of(&d, "Fay", &support::branch()).await;
    let zed = desk_of(&d, "Zed", &support::other_branch()).await;
    d.t.guardians.reachable(P1);
    d.t.outbox
        .refuse(support::PATIENT_EMAIL, "550", "mailbox unknown");

    let recall = open_recall(&d, s.item.id, "L1").await;
    let mia = manager(&d).await;
    d.post(
        &mia,
        &format!("/clinics/recalls/{}/notices", recall.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    let made = RecallNotice::of_recall(d.t.db(), recall.id).await.unwrap();
    assert_eq!(made.len(), 2, "one notice for each branch: {made:?}");
    let at_first = made.iter().find(|n| n.visit_ids.0 == vec![first.id]);
    let at_second = made.iter().find(|n| n.visit_ids.0 == vec![second.id]);
    let (at_first, at_second) = (
        at_first.expect("a notice for the visit of branch 1"),
        at_second.expect("a notice for the visit of branch 2"),
    );
    assert_eq!(at_first.visit_dates, first.visit_date.to_string());
    assert_eq!(at_second.visit_dates, second.visit_date.to_string());

    while d.t.app.run_jobs().await > 0 {}
    for notice in [at_first, at_second] {
        let now = RecallNotice::find_or_404(d.t.db(), notice.id)
            .await
            .unwrap();
        assert_eq!(now.status, RecallNoticeStatus::GaveUp);
    }
    let tasks = d.t.tasks.opened();
    assert_eq!(tasks.len(), 2, "{tasks:?}");
    let task_of = |notice: &RecallNotice| {
        tasks
            .iter()
            .find(|task| task.subject == notice.subject())
            .unwrap_or_else(|| panic!("no task for {notice:?}: {tasks:?}"))
    };
    assert_eq!(task_of(at_first).assignee, Some(Assignee::user(fay.id)));
    assert_eq!(task_of(at_second).assignee, Some(Assignee::user(zed.id)));

    // Each desk sees the call of its own branch only.
    let fay_page = d.get(&fay, "/clinics/recall-calls").await;
    fay_page
        .assert_ok()
        .assert_see(&first.visit_date.to_string());
    assert!(
        !fay_page.text().contains(&second.visit_date.to_string()),
        "{}",
        fay_page.text()
    );
    let zed_page = d.get(&zed, "/clinics/recall-calls").await;
    zed_page
        .assert_ok()
        .assert_see(&second.visit_date.to_string());
    assert!(
        !zed_page.text().contains(&first.visit_date.to_string()),
        "{}",
        zed_page.text()
    );
    let path = format!("/clinics/recall-calls/{}", at_second.id);
    d.post(&fay, &path, &[("outcome", "Reached")])
        .await
        .assert_forbidden();
    d.post(&zed, &path, &[("outcome", "Reached")])
        .await
        .assert_ok();
}

/// A manager who holds `recalls.trace` in one branch only opens the trace and
/// reads the rows of that branch; one who holds it in a branch with no row of
/// the recall is refused; the holder everywhere reads all rows. Each view is
/// audited.
#[renox::test]
async fn a_branch_manager_reads_the_trace_rows_of_her_branch() {
    let d = desk_with_stock().await;
    name_patients(&d);
    let s = store(&d, 10, 0).await;
    let e = elsewhere(&d, &s, 10).await;
    used_filler(&d, P2, 1, 9, 1, "L1").await;
    used_filler_with(&d, &e.doctor, &e.service, P3, 2, 9, 1, "L1").await;
    let recall = open_recall(&d, s.item.id, "L1").await;
    let path = format!("/clinics/recalls/{}", recall.id);

    let scoped = |name: &'static str, branch: Subject| {
        let d = &d;
        async move {
            let user = User::register(
                d.t.db(),
                name,
                &format!("{}@example.com", name.to_lowercase()),
                "password123",
            )
            .await
            .unwrap();
            permissions::define_role(d.t.db(), "branch_manager", &[RECALLS_TRACE])
                .await
                .unwrap();
            user.assign_role_in(
                d.t.db(),
                "branch_manager",
                &Scope::new(&branch.subject_type, branch.subject_id),
            )
            .await
            .unwrap();
            user
        }
    };
    let mona = scoped("Mona", support::branch()).await;
    let page = d.get(&mona, &path).await;
    page.assert_ok().assert_see("Omar Saleh");
    assert!(!page.text().contains("Noor Haddad"), "{}", page.text());
    // The send is the global manager's, not the branch manager's.
    d.post(&mona, &format!("{path}/notices"), &[("x", "y")])
        .await
        .assert_forbidden();

    let nobody = scoped("Nora", Subject::new("test.branch", 4)).await;
    d.get(&nobody, &path).await.assert_forbidden();

    let mia = manager(&d).await;
    let all = d.get(&mia, &path).await;
    all.assert_ok()
        .assert_see("Omar Saleh")
        .assert_see("Noor Haddad");

    let log = renox::audit::for_subject(d.t.db(), "inventory.recall", recall.id, 10)
        .await
        .unwrap();
    let viewers: Vec<_> = log
        .iter()
        .filter(|entry| entry.action == clinics::recalls::VIEWED)
        .filter_map(|entry| entry.user_id)
        .collect();
    assert_eq!(viewers.len(), 2, "{viewers:?}");
    assert!(viewers.contains(&mona.id) && viewers.contains(&mia.id));
}
