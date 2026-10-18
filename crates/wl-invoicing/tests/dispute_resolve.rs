//! Resolving an invoice dispute (wl-invoicing §5.10, #67 part 4): `upheld`
//! with its grace period, `credited` in full or in part, the `FIN.ACL.07`
//! approval, and the notices and events that follow. Users are named by id
//! (`DisputeBy::User`), since a test cannot build an `AuthUser`.

mod support;

use renox::prelude::*;
use wl_invoicing::dispute::{self, Dispute};
use wl_invoicing::dispute_resolve::{self, ResolveInput};
use wl_invoicing::permissions as perm;
use wl_invoicing::{CreditNote, CreditNoteKind, Invoice};
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalPolicy, ApprovalStatus};
use wl_kernel::invoicing::{
    CreditNoteStatus, DisputeBy, DisputeOutcome, DisputeRequest, DraftLine, InvoiceDraft,
    InvoiceId, InvoiceStatus, VatCategory,
};

use support::{BLUE_DUNES, Booted, boot};

/// Gross 2,100.00, 840.00 and 420.00: AED 3,360.00 in all.
const LINES: [(i64, VatCategory); 3] = [
    (200_000, VatCategory::Standard),
    (80_000, VatCategory::Standard),
    (40_000, VatCategory::Standard),
];
const GROSS: i64 = 336_000;

/// A user who may dispute but not credit.
async fn disputer(t: &Booted) -> User {
    t.helper("Dora", &[perm::VIEW, perm::ISSUE, perm::DISPUTE])
        .await
}

/// A user who may also credit.
async fn crediter(t: &Booted) -> User {
    t.helper(
        "Cleo",
        &[perm::VIEW, perm::ISSUE, perm::DISPUTE, perm::CREDIT],
    )
    .await
}

/// Another holder of `invoices.credit`, to approve.
async fn approver(t: &Booted) -> User {
    t.helper("Erin", &[perm::VIEW, perm::CREDIT]).await
}

/// Sets the `FIN.ACL.07` limit (fils): a credit above it needs an approval.
async fn set_limit(t: &Booted, fils: i64) {
    ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("invoicing.dispute_credit"))
        .update(t.db(), &[("threshold", &fils)])
        .await
        .unwrap();
}

/// An issued invoice, disputed by Dora on `lines` (all lines when empty)
/// with `amount`.
async fn disputed(t: &Booted, lines: &[i32], amount: Option<i64>) -> (Invoice, Dispute) {
    let clerk = t.user_a().await;
    let dora = disputer(t).await;
    let invoice = t.issued(&LINES, None, &clerk).await;
    let mut request = DisputeRequest::new(
        InvoiceId(invoice.id),
        "wrong hours",
        DisputeBy::User { id: dora.id },
    )
    .lines(lines.iter().copied());
    if let Some(amount) = amount {
        request = request.amount(amount);
    }
    let id = dispute::raise(t.state(), request).await.unwrap();
    let invoice = t.reload(&invoice).await;
    assert!(invoice.disputed);
    (invoice, Dispute::find(t.db(), id).await.unwrap().unwrap())
}

async fn stored(t: &Booted, dispute: &Dispute) -> Dispute {
    Dispute::find(t.db(), dispute.id).await.unwrap().unwrap()
}

fn is_422<T>(result: Result<T>) -> bool {
    matches!(
        result,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    )
}

/// `upheld` clears the flag, sets the grace period on the dispute and the
/// invoice, leaves the due date and status alone, tells the party and emits
/// `InvoiceDisputeResolved` once.
#[renox::test]
async fn upheld_clears_the_flag_and_starts_the_grace_period() {
    let t = boot().await;
    let (invoice, dispute) = disputed(&t, &[2, 3], None).await;
    let dora = disputer(&t).await;

    let done = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Hours verified"),
        DisputeBy::User { id: dora.id },
    )
    .await
    .unwrap();
    assert!(done.is_resolved());

    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.dispute_id, None);
    assert_eq!(after.dispute_grace_until, Some(t.day(7)));
    assert_eq!(after.due_date, invoice.due_date);
    assert_eq!(after.status, invoice.status);
    assert_eq!(after.balance, invoice.balance);

    let dispute = stored(&t, &dispute).await;
    assert_eq!(dispute.outcome, Some(DisputeOutcome::Upheld));
    assert_eq!(dispute.resolution_note.as_deref(), Some("Hours verified"));
    assert_eq!(dispute.grace_until, Some(t.day(7)));
    assert_eq!(dispute.resolved_by_user_id, Some(dora.id));
    assert!(dispute.resolved_at.is_some());
    assert!(!dispute.is_open());

    let events = t.resolved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, DisputeOutcome::Upheld);
    assert_eq!(events[0].credited_amount, None);
    assert_eq!(events[0].credit_note, None);
    let notices = t.notices.of_type("invoicing.dispute_resolved");
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].data["note"], "Hours verified");
    assert!(
        notices[0].data["result"]
            .as_str()
            .unwrap()
            .starts_with("The invoice stands.")
    );
    // Nothing more to resolve.
    let again = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Again"),
        DisputeBy::User { id: dora.id },
    )
    .await;
    assert!(is_422(again));
}

/// AC9: `credited` needs `invoices.credit` besides `invoices.dispute`; the
/// note is required; the system may only uphold; and `upheld` still works
/// from `paid` (AC11).
#[renox::test]
async fn who_may_resolve_and_with_what() {
    let t = boot().await;
    let (invoice, _) = disputed(&t, &[], Some(100_000)).await;
    let dora = disputer(&t).await;
    let by = DisputeBy::User { id: dora.id };

    let credited = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("x", 50_000),
        by,
    )
    .await;
    assert!(matches!(credited, Err(Error::Forbidden)));
    let nobody = t.helper("Vera2", &[perm::VIEW]).await;
    let viewer = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("x"),
        DisputeBy::User { id: nobody.id },
    )
    .await;
    assert!(matches!(viewer, Err(Error::Forbidden)));
    let party = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("x"),
        DisputeBy::Party { id: BLUE_DUNES },
    )
    .await;
    assert!(matches!(party, Err(Error::Forbidden)));
    let blank =
        dispute_resolve::resolve(t.state(), invoice.id, ResolveInput::upheld("  "), by).await;
    assert!(is_422(blank));
    assert!(t.reload(&invoice).await.disputed);

    // The whole balance is paid while disputed: `paid`, flag stays.
    t.pay(&invoice, 1, GROSS).await;
    let paid = t.reload(&invoice).await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert!(paid.disputed);
    dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Paid anyway"),
        DisputeBy::System,
    )
    .await
    .unwrap();
    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.status, InvoiceStatus::Paid);
}

/// AC6: with the limit raised above the amount, `credited` issues a partial
/// credit note at once; the balance drops, the flag clears, the party is told
/// the credit note number, and the event carries the credit note.
#[renox::test]
async fn a_credit_below_the_limit_is_issued_at_once() {
    let t = boot().await;
    set_limit(&t, 1_000_000).await;
    let (invoice, dispute) = disputed(&t, &[2, 3], None).await;
    let cleo = crediter(&t).await;

    let done = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed on the hours", 120_000),
        DisputeBy::User { id: cleo.id },
    )
    .await
    .unwrap();
    assert!(done.is_resolved());

    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.balance, GROSS - 120_000);
    assert_eq!(after.status, InvoiceStatus::PartiallyPaid);
    let note = CreditNote::find(t.db(), done.credit_note.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(note.kind, CreditNoteKind::Partial);
    assert_eq!(note.gross_total, 120_000);
    // Lines 2 and 3, in position order: 840.00 and 360.00 with VAT.
    let lines = wl_invoicing::credit_notes::lines_of(t.db(), note.id)
        .await
        .unwrap();
    let grosses: Vec<i64> = lines.iter().map(|l| l.gross_amount).collect();
    assert_eq!(grosses, [84_000, 36_000]);
    assert_eq!(note.status, CreditNoteStatus::Applied);
    assert!(note.number.is_some());

    let dispute = stored(&t, &dispute).await;
    assert_eq!(dispute.outcome, Some(DisputeOutcome::Credited));
    assert_eq!(dispute.agreed_amount, Some(120_000));
    assert_eq!(dispute.credit_note_id, Some(note.id));
    assert_eq!(dispute.resolved_by_user_id, Some(cleo.id));
    assert_eq!(dispute.grace_until, None);

    let events = t.resolved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, DisputeOutcome::Credited);
    assert_eq!(events[0].credited_amount, Some(120_000));
    assert_eq!(events[0].credit_note, Some(note.subject()));
    let notices = t.notices.of_type("invoicing.dispute_resolved");
    assert_eq!(notices.len(), 1);
    let result = notices[0].data["result"].as_str().unwrap();
    assert!(result.contains(note.number.as_deref().unwrap()), "{result}");
    assert!(result.contains("AED 1,200.00"));
}

/// AC7: above the limit the credit waits for an approval: a pending request,
/// a draft credit note, the invoice stays disputed. The requester cannot
/// approve; another holder of `invoices.credit` does and the credit follows.
#[renox::test]
async fn an_approved_credit_is_issued_and_settles_the_dispute() {
    let t = boot().await;
    let (invoice, dispute) = disputed(&t, &[2, 3], None).await;
    let cleo = crediter(&t).await;
    let erin = approver(&t).await;

    let done = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed", 120_000),
        DisputeBy::User { id: cleo.id },
    )
    .await
    .unwrap();
    let request_id = done.pending_request.expect("a request is pending");
    assert!(!done.is_resolved());
    let draft = CreditNote::find(t.db(), done.credit_note.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.status, CreditNoteStatus::Draft);
    assert_eq!(draft.number, None);
    assert!(t.reload(&invoice).await.disputed);
    assert_eq!(t.reload(&invoice).await.balance, GROSS);
    let waiting = stored(&t, &dispute).await;
    assert!(waiting.is_open());
    assert_eq!(waiting.approval_request_id, Some(request_id));
    assert_eq!(waiting.credit_note_id, Some(draft.id));
    assert!(t.resolved.all().is_empty());

    // A new resolve while the request waits is refused.
    let again = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Changed my mind"),
        DisputeBy::User { id: cleo.id },
    )
    .await;
    assert!(is_422(again));

    // The requester cannot approve.
    let own = approvals::approve(t.state(), request_id, &cleo, "").await;
    assert!(matches!(own, Err(Error::Forbidden)));

    let decided = approvals::approve(t.state(), request_id, &erin, "ok")
        .await
        .unwrap();
    assert!(decided.is_some());

    let after = t.reload(&invoice).await;
    assert!(!after.disputed);
    assert_eq!(after.balance, GROSS - 120_000);
    let note = CreditNote::find(t.db(), draft.id).await.unwrap().unwrap();
    assert_eq!(note.status, CreditNoteStatus::Applied);
    assert!(note.number.is_some());
    let dispute = stored(&t, &dispute).await;
    assert_eq!(dispute.outcome, Some(DisputeOutcome::Credited));
    assert_eq!(dispute.resolved_by_user_id, Some(cleo.id));
    assert_eq!(dispute.credit_note_id, Some(draft.id));

    let events = t.resolved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].credited_amount, Some(120_000));
    assert_eq!(t.notices.of_type("invoicing.dispute_resolved").len(), 1);
}

/// AC7: a rejected request leaves the flag; a new resolve discards the old
/// draft first and can go on.
#[renox::test]
async fn a_rejected_credit_leaves_the_dispute_open_for_a_new_resolve() {
    let t = boot().await;
    let (invoice, dispute) = disputed(&t, &[2, 3], None).await;
    let cleo = crediter(&t).await;
    let erin = approver(&t).await;
    let by = DisputeBy::User { id: cleo.id };

    let first = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed", 120_000),
        by,
    )
    .await
    .unwrap();
    let request_id = first.pending_request.unwrap();
    approvals::reject(t.state(), request_id, &erin, "too much")
        .await
        .unwrap();
    let request = approvals::ApprovalRequest::find(t.db(), request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Rejected);
    assert!(t.reload(&invoice).await.disputed);
    assert!(stored(&t, &dispute).await.is_open());
    assert!(t.resolved.all().is_empty());

    // Resolve again, for less: the old draft is discarded.
    let second = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Agreed on less", 84_000),
        by,
    )
    .await
    .unwrap();
    assert!(second.pending_request.is_some());
    assert_ne!(second.credit_note, first.credit_note);
    let old = CreditNote::find(t.db(), first.credit_note.unwrap())
        .await
        .unwrap();
    assert!(old.is_none(), "the old draft is gone");

    // The new request is pending, so any further resolve is refused.
    let third =
        dispute_resolve::resolve(t.state(), invoice.id, ResolveInput::upheld("Fine"), by).await;
    assert!(is_422(third));
}

/// AC10: the whole gross of an unpaid invoice credits it in full; more than
/// the disputed lines can still be credited by is refused; a timesheet line
/// refuses `credited`.
#[renox::test]
async fn full_credit_and_the_refusals() {
    let t = boot().await;
    set_limit(&t, 10_000_000).await;
    let (invoice, _) = disputed(&t, &[3], None).await;
    let cleo = crediter(&t).await;
    let by = DisputeBy::User { id: cleo.id };

    // Line 3 is worth 420.00 with VAT.
    let over = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("x", 42_001),
        by,
    )
    .await;
    assert!(is_422(over));
    let none = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput {
            agreed_amount: None,
            ..ResolveInput::credited("x", 1)
        },
        by,
    )
    .await;
    assert!(is_422(none));
    assert!(t.reload(&invoice).await.disputed);

    let done = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Billed twice", GROSS),
        by,
    )
    .await
    .unwrap();
    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::Credited);
    assert_eq!(after.balance, 0);
    assert!(!after.disputed);
    let note = CreditNote::find(t.db(), done.credit_note.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(note.kind, CreditNoteKind::Full);
    assert_eq!(note.gross_total, GROSS);
    assert_eq!(note.status, CreditNoteStatus::Applied);
    assert_eq!(t.resolved.all()[0].credited_amount, Some(GROSS));
}

/// A full credit above the limit waits for the approval and then cancels the
/// invoice.
#[renox::test]
async fn an_approved_full_credit_cancels_the_invoice() {
    let t = boot().await;
    let (invoice, dispute) = disputed(&t, &[], Some(50_000)).await;
    let cleo = crediter(&t).await;
    let erin = approver(&t).await;

    let done = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::credited("Wrong customer", GROSS),
        DisputeBy::User { id: cleo.id },
    )
    .await
    .unwrap();
    let draft = CreditNote::find(t.db(), done.credit_note.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(draft.kind, CreditNoteKind::Full);
    approvals::approve(t.state(), done.pending_request.unwrap(), &erin, "")
        .await
        .unwrap();

    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::Credited);
    assert_eq!(after.balance, 0);
    assert!(!after.disputed);
    assert_eq!(
        stored(&t, &dispute).await.outcome,
        Some(DisputeOutcome::Credited)
    );
    assert_eq!(t.resolved.all().len(), 1);
}

/// AC10: a disputed line billed from a timesheet refuses `credited` until an
/// adjustment timesheet can be checked (wl-attendance, #86).
#[renox::test]
async fn a_timesheet_line_refuses_a_credit() {
    let t = boot().await;
    set_limit(&t, 10_000_000).await;
    let clerk = t.user_a().await;
    let draft = t
        .draft_of(
            InvoiceDraft::new(BLUE_DUNES, Subject::new("property.lease", 9)).line(
                DraftLine::new("Hours", 100_000, VatCategory::Standard)
                    .origin(Subject::new("attendance.timesheet", 3)),
            ),
        )
        .await;
    let invoice = t.issue(&draft, &clerk).await;
    let dora = disputer(&t).await;
    dispute::raise(
        t.state(),
        DisputeRequest::new(
            InvoiceId(invoice.id),
            "wrong hours",
            DisputeBy::User { id: dora.id },
        )
        .lines([1]),
    )
    .await
    .unwrap();
    let cleo = crediter(&t).await;
    let refused = dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput {
            adjustment_timesheet: Some(Subject::new("attendance.timesheet", 4)),
            ..ResolveInput::credited("x", 50_000)
        },
        DisputeBy::User { id: cleo.id },
    )
    .await;
    match refused {
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)) => {
            assert!(message.contains("adjustment timesheet"), "{message}");
        }
        other => panic!("expected a 422, got {other:?}"),
    }
    assert!(t.reload(&invoice).await.disputed);
    // Upheld is not affected.
    dispute_resolve::resolve(
        t.state(),
        invoice.id,
        ResolveInput::upheld("Checked"),
        DisputeBy::User { id: cleo.id },
    )
    .await
    .unwrap();
    assert!(!t.reload(&invoice).await.disputed);
}
