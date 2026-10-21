//! BL-ATT-09 (#186): an adjustment timesheet of origin `client_dispute` is
//! linked to an open invoice dispute (wl-attendance §4 *Correcting a locked
//! timesheet*, §5.7 point 4, §5.8; `HRM.ACL.08`).

mod support;

use renox::prelude::*;
use wl_attendance::dispute::AdjustmentEvidence;
use wl_attendance::timesheet::{AdjustmentOrigin, Affects, ApproverKind, TimesheetKind};
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetStatus};
use wl_kernel::invoicing::{
    DisputeBy, DisputeOutcome, InvoiceDisputeResolved, InvoiceDisputed, InvoiceId,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, at, boot, locked_eight_hours, minus_eight_hours, post_to, project};

const UPHELD_NOTICE: &str = "attendance.adjustment_dispute_upheld";

fn dispute(id: i64) -> Subject {
    Subject::new("invoicing.dispute", id)
}

/// The form of a `client_dispute` adjustment of -8 h linked to `link`.
fn dispute_form<'a>(link: &'a str, entries: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("reason", "client and business agreed to cut 8 h"),
        ("origin", "client_dispute"),
        ("linked", link),
        ("entries", entries),
    ]
}

/// A resolved dispute 7 of invoice 1, as wl-invoicing emits it.
fn resolved(outcome: DisputeOutcome, credit_note: Option<i64>) -> InvoiceDisputeResolved {
    InvoiceDisputeResolved {
        invoice: InvoiceId(1),
        party: PartyId(1),
        dispute: 7,
        outcome,
        credited_amount: credit_note.map(|_| 20_000),
        credit_note: credit_note.map(|id| Subject::new("invoicing.credit_note", id)),
    }
}

/// Opens J on `original` as `creator`, linked to dispute 7, and returns it.
async fn open_j(t: &Booted, creator: &User, original: &Timesheet) -> Timesheet {
    let entries = minus_eight_hours();
    post_to(
        t,
        creator,
        original,
        "adjustments",
        &dispute_form("invoicing.dispute:7", &entries),
    )
    .await
    .assert_ok();
    original.adjusted_by(t.db()).await.unwrap().pop().unwrap()
}

/// AC1: Given invoice INV-1 built from locked timesheet T and flagged
/// disputed, when A opens a `client_dispute` adjustment J (-8 h) linked to
/// INV-1's dispute, J is `open` and `affects` defaults to `billing`; no
/// adjustment or task was created automatically when the dispute was raised.
#[renox::test]
async fn att09_a_client_dispute_adjustment_is_opened_by_a_user_and_defaults_to_billing() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let original =
        locked_eight_hours(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;

    // The invoice is disputed: wl-invoicing raises the event.
    t.disputes.open(7, 1);
    t.state()
        .emit(InvoiceDisputed {
            invoice: InvoiceId(1),
            party: PartyId(1),
            source: original.subject(),
            dispute: 7,
            amount: 20_000,
            lines: Vec::new(),
            ticket: None,
            raised_by: DisputeBy::System,
        })
        .await
        .unwrap();
    assert!(
        original.adjusted_by(t.db()).await.unwrap().is_empty(),
        "nothing is opened when the dispute is raised"
    );
    assert!(t.tasks.opened().is_empty());

    let j = open_j(&t, &a_user, &original).await;
    assert_eq!(j.status, TimesheetStatus::Open);
    assert_eq!(j.kind, TimesheetKind::Adjustment);
    assert_eq!(j.origin, Some(AdjustmentOrigin::ClientDispute));
    assert_eq!(j.affects, Some(Affects::Billing));
    assert_eq!(j.linked(), Some(dispute(7)));
    assert_eq!(j.created_by, Some(a_user.id));
    assert_eq!(j.adjusts_id, Some(original.id));
    assert!(
        t.tasks.opened().is_empty(),
        "the dispute ticket is the task"
    );
}

/// AC2: when J is approved, the credit note is raised from it and INV-1's
/// dispute is resolved `credited` naming J, J links to the credit note (and
/// the credit note links to J on wl-invoicing's side).
#[renox::test]
async fn att09_the_credited_dispute_links_the_adjustment_to_the_credit_note() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let approver = t.user_with("Bilal", "approver").await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let original =
        locked_eight_hours(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    t.disputes.open(7, 1);
    let j = open_j(&t, &a_user, &original).await;
    post_to(&t, &keeper, &j, "submit", &[("x", "y")])
        .await
        .assert_ok();
    post_to(&t, &approver, &j, "approve", &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(t.reload_sheet(&j).await.credit_note_id, None);

    // The approval told billing; billing raised the credit note and resolved
    // the dispute.
    t.disputes.settle(7, DisputeOutcome::Credited);
    for _ in 0..2 {
        t.state()
            .emit(resolved(DisputeOutcome::Credited, Some(55)))
            .await
            .unwrap();
    }
    let linked = t.reload_sheet(&j).await;
    assert_eq!(linked.credit_note_id, Some(55));
    assert_eq!(linked.status, TimesheetStatus::Approved);
    // The original keeps its own consumers and stays as it was.
    assert_eq!(
        t.reload_sheet(&original).await.status,
        TimesheetStatus::Locked
    );
    assert_eq!(t.reload_sheet(&original).await.credit_note_id, None);
}

/// AC3, first half: given INV-1's dispute is resolved `upheld` with no
/// adjustment, no adjustment exists for T and none is created.
#[renox::test]
async fn att09_a_dispute_upheld_with_no_adjustment_creates_nothing() {
    let t = boot().await;
    let original =
        locked_eight_hours(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    t.disputes.open(7, 1);
    t.disputes.settle(7, DisputeOutcome::Upheld);
    t.state()
        .emit(resolved(DisputeOutcome::Upheld, None))
        .await
        .unwrap();
    assert!(original.adjusted_by(t.db()).await.unwrap().is_empty());
    assert!(t.tasks.opened().is_empty());
    assert!(t.notices.of_type(UPHELD_NOTICE).is_empty());
}

/// AC3, second half: given J was opened and the dispute then ends `upheld`,
/// J's `submit` is refused and A is notified, once.
#[renox::test]
async fn att09_an_adjustment_cannot_be_submitted_after_its_dispute_is_upheld() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let original =
        locked_eight_hours(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    t.disputes.open(7, 1);
    let j = open_j(&t, &a_user, &original).await;

    t.disputes.settle(7, DisputeOutcome::Upheld);
    for _ in 0..2 {
        t.state()
            .emit(resolved(DisputeOutcome::Upheld, None))
            .await
            .unwrap();
    }
    post_to(&t, &keeper, &j, "submit", &[("x", "y")])
        .await
        .assert_status(422);
    let stored = t.reload_sheet(&j).await;
    assert_eq!(stored.status, TimesheetStatus::Open);
    assert!(stored.dispute_upheld_at.is_some());

    let told = t.notices.of_type(UPHELD_NOTICE);
    assert_eq!(told.len(), 1, "A is told once, not once per event");
    assert_eq!(support::users_of(&told[0]), vec![a_user.id]);
    assert_eq!(told[0].subject, j.subject());
    // No payroll or billing effect: nothing approved, nothing posted.
    assert!(t.approved.all().is_empty());
    assert!(t.posted.all().is_empty());
}

/// An adjustment that was already approved when its dispute ends `upheld` is
/// left as it is and its creator is not told.
#[renox::test]
async fn att09_an_approved_adjustment_is_left_alone_when_the_dispute_ends_upheld() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let approver = t.user_with("Bilal", "approver").await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let original =
        locked_eight_hours(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    t.disputes.open(7, 1);
    let j = open_j(&t, &a_user, &original).await;
    post_to(&t, &keeper, &j, "submit", &[("x", "y")])
        .await
        .assert_ok();
    post_to(&t, &approver, &j, "approve", &[("x", "y")])
        .await
        .assert_ok();

    t.state()
        .emit(resolved(DisputeOutcome::Upheld, None))
        .await
        .unwrap();
    let stored = t.reload_sheet(&j).await;
    assert_eq!(stored.status, TimesheetStatus::Approved);
    assert!(stored.dispute_upheld_at.is_none());
    assert!(t.notices.of_type(UPHELD_NOTICE).is_empty());
}

/// AC4: choosing `both` without attendance evidence is refused; with
/// evidence it is accepted and the evidence is kept. A client dispute that
/// affects `payroll` needs it too; evidence of another worker or of an
/// unknown kind is refused.
#[renox::test]
async fn att09_pay_is_cut_only_with_attendance_evidence() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let worker = t.employee(1, &[]);
    let other = t.employee(2, &[]);
    let original = locked_eight_hours(&t, worker, &project(5), ApproverKind::Internal).await;
    t.disputes.open(7, 1);
    let site = t.location("Site 5").await;
    let mine = t
        .fixture(
            worker,
            &site,
            at(2, 7, 0),
            Some(at(2, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    let theirs = t
        .fixture(
            other,
            &site,
            at(2, 7, 0),
            Some(at(2, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    let entries = minus_eight_hours();
    let theirs_text = format!("attendance.record:{}", theirs.id);
    for (affects, evidence) in [
        ("both", ""),
        ("payroll", ""),
        // Another worker's record, a kind that is no evidence, nonsense.
        ("both", theirs_text.as_str()),
        ("both", "construction.project:5"),
        ("both", "nonsense"),
        // A record that does not exist.
        ("both", "attendance.record:9999"),
    ] {
        let mut form = dispute_form("invoicing.dispute:7", &entries);
        form.push(("affects", affects));
        form.push(("evidence", evidence));
        post_to(&t, &a_user, &original, "adjustments", &form)
            .await
            .assert_status(422);
    }
    assert!(original.adjusted_by(t.db()).await.unwrap().is_empty());

    // Billing only needs none; both needs the owner's record or a document.
    let evidence = format!("attendance.record:{}, wl_documents.document:12", mine.id);
    let mut form = dispute_form("invoicing.dispute:7", &entries);
    form.push(("affects", "both"));
    form.push(("evidence", evidence.as_str()));
    post_to(&t, &a_user, &original, "adjustments", &form)
        .await
        .assert_ok();
    let list = original.adjusted_by(t.db()).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].affects, Some(Affects::Both));
    let kept = AdjustmentEvidence::of_timesheet(t.db(), list[0].id)
        .await
        .unwrap();
    let subjects: Vec<Subject> = kept.iter().map(AdjustmentEvidence::subject).collect();
    assert_eq!(
        subjects,
        vec![
            Subject::new("attendance.record", mine.id),
            Subject::new("wl_documents.document", 12)
        ]
    );
    assert!(kept.iter().all(|e| e.attached_by == Some(a_user.id)));
}

/// AC5: a `client_dispute` adjustment without a linked dispute or ticket is
/// refused; a link to a dispute that is already resolved or does not exist is
/// refused; a linked ticket is allowed.
#[renox::test]
async fn att09_the_link_must_be_an_open_dispute_or_a_ticket() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let original =
        locked_eight_hours(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    let entries = minus_eight_hours();
    t.disputes.open(7, 1);
    t.disputes.open(8, 1);
    t.disputes.settle(8, DisputeOutcome::Upheld);

    // Nothing linked.
    post_to(
        &t,
        &a_user,
        &original,
        "adjustments",
        &[
            ("reason", "agreed"),
            ("origin", "client_dispute"),
            ("entries", entries.as_str()),
        ],
    )
    .await
    .assert_status(422);
    // Resolved, and unknown.
    for link in ["invoicing.dispute:8", "invoicing.dispute:99"] {
        post_to(
            &t,
            &a_user,
            &original,
            "adjustments",
            &dispute_form(link, &entries),
        )
        .await
        .assert_status(422);
    }
    assert!(original.adjusted_by(t.db()).await.unwrap().is_empty());

    // The open dispute, and a ticket.
    for link in ["invoicing.dispute:7", "ticketing.ticket:31"] {
        post_to(
            &t,
            &a_user,
            &original,
            "adjustments",
            &dispute_form(link, &entries),
        )
        .await
        .assert_ok();
    }
    let list = original.adjusted_by(t.db()).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].linked(), Some(dispute(7)));
    assert_eq!(list[1].linked(), Some(Subject::new("ticketing.ticket", 31)));
    assert!(list.iter().all(|j| j.affects == Some(Affects::Billing)));
}
