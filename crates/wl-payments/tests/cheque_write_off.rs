//! BL-PAY-13 (#106): a bounced cheque is written off through the invoice
//! write-off of every invoice it pays. The cheque becomes `written_off` only
//! when `InvoiceWrittenOff` has arrived for all of them; a request that is
//! rejected leaves the cheque where it is. The credit hold is not lifted by
//! this module: `ChequeBounceResolved` (written off) lets wl-sales decide.

mod support;

use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalEnd;
use wl_kernel::cheques::{ChequeOutcome, ChequeStatus, RegisterCheque};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus};
use wl_payments::cheque_write_off::{ChequeWriteOff, WriteOffStatus};
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::{Cheque, FollowupTask};

use support::{BLUE_DUNES, Booted, boot};

async fn tasks_of(t: &Booted, cheque: &Cheque, kind: FollowupKind) -> Vec<FollowupTask> {
    FollowupTask::of_cheque(t.db(), cheque.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|task| task.kind == kind)
        .collect()
}

/// A bounced cheque of `amount` fils paying `invoice`.
async fn bounced_for(
    t: &Booted,
    staff: &renox::auth::User,
    number: &str,
    amount: i64,
    invoice: InvoiceId,
) -> Cheque {
    t.bounced_cheque(staff, BLUE_DUNES, number, amount, Some(invoice), "")
        .await
}

/// AC1: a bounced cheque open AED 10,000.00 linked to INV-5 with a balance of
/// AED 10,000.00: `write_off` with the reason "untraceable" creates an invoice
/// write-off request and the cheque stays `bounced`; when user B approves it
/// and `InvoiceWrittenOff` arrives, the cheque is `written_off` and
/// `ChequeBounceResolved` (written off) is emitted.
#[renox::test]
async fn the_cheque_is_written_off_when_the_invoice_write_off_is_approved() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.invoices.require_approval(true);
    let inv5 = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = bounced_for(&t, &a, "5001", 1_000_000, inv5).await;
    assert_eq!(cheque.open_amount, 1_000_000);

    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();

    // One request, waiting: the cheque is not written off before the approval.
    assert_eq!(t.invoices.write_offs(), vec![inv5]);
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
    assert!(t.cheque_resolved.all().is_empty());
    let rows = ChequeWriteOff::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, WriteOffStatus::Pending);
    assert_eq!(rows[0].amount, 1_000_000);
    assert_eq!(rows[0].reason, "untraceable");
    assert_eq!(rows[0].requested_by, Some(a.id));
    assert!(rows[0].approval_request_id.is_some());

    // User B approves: the invoice is written off and the event arrives.
    t.approve_invoice_write_off(inv5).await;

    let done = t.cheque(cheque.id).await;
    assert_eq!(done.status, ChequeStatus::WrittenOff);
    // The records are kept as evidence.
    assert_eq!(done.open_amount, 1_000_000);
    assert_eq!(done.return_certificate.as_deref(), Some("CERT-1"));
    let rows = ChequeWriteOff::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(rows[0].status, WriteOffStatus::WrittenOff);
    let resolved = t.cheque_resolved.all();
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].outcome, ChequeOutcome::WrittenOff);
    assert_eq!(resolved[0].cheque, cheque.cheque_id());
    // The follow-up task of the bounce is done.
    let follow_up = tasks_of(&t, &done, FollowupKind::FollowUpCheque).await;
    assert_eq!(follow_up[0].status, FollowupStatus::Done);
}

/// The invoice write-off that needs no approval is made at once, and the
/// cheque with it.
#[renox::test]
async fn a_write_off_within_the_limit_writes_the_cheque_off_at_once() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-8", 300_000);
    let cheque = bounced_for(&t, &a, "5002", 300_000, inv).await;

    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();

    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::WrittenOff);
    assert_eq!(t.invoices.status_of(inv), Some(InvoiceStatus::WrittenOff));
    assert_eq!(t.cheque_resolved.all().len(), 1);
}

/// AC2: a linked charge not yet invoiced is invoiced first, then written off.
#[renox::test]
async fn a_charge_not_yet_invoiced_is_invoiced_first() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let entity = t.entity("PGP").await;
    t.charges.register(7, BLUE_DUNES, 500_000);
    let request = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "5003", today(), 500_000)
        .link(Subject::new("invoicing.charge", 7));
    let cheque = t.register_cheque(request).await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "insufficient_funds", "CERT-1", "")
        .await
        .assert_ok();
    let cheque = t.cheque(cheque.id).await;

    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();

    // The link points at the invoice made from the charge.
    let linked = t.cheque(cheque.id).await.linked(t.db()).await.unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].subject_type, "invoicing.invoice");
    let invoice = InvoiceId(linked[0].subject_id);
    assert_eq!(t.invoices.write_offs(), vec![invoice]);
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::WrittenOff);
}

/// A charge nothing can invoice refuses the write-off and asks for it to be
/// invoiced first.
#[renox::test]
async fn a_charge_that_cannot_be_invoiced_refuses_the_write_off() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let entity = t.entity("PGP").await;
    let request = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "5004", today(), 500_000)
        .link(Subject::new("invoicing.charge", 99));
    let cheque = t.register_cheque(request).await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "insufficient_funds", "CERT-1", "")
        .await
        .assert_ok();
    let cheque = t.cheque(cheque.id).await;

    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_status(422);
    assert!(t.invoices.write_offs().is_empty());
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
}

/// AC3: when the invoice request is rejected, the cheque stays `bounced`; the
/// accountant is told with a task, and the user may try again.
#[renox::test]
async fn a_rejected_invoice_write_off_leaves_the_cheque_bounced() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let accountant = t.accountant("Cara").await;
    t.invoices.require_approval(true);
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = bounced_for(&t, &a, "5005", 1_000_000, inv).await;
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();

    t.end_invoice_write_off(inv, ApprovalEnd::Rejected).await;

    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
    assert!(t.cheque_resolved.all().is_empty());
    let rows = ChequeWriteOff::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(rows[0].status, WriteOffStatus::Failed);
    assert_eq!(rows[0].failure.as_deref(), Some("rejected"));
    let tasks = tasks_of(&t, &cheque, FollowupKind::WriteOffFailed).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].role, "accountant");
    let told = t
        .notices
        .recipients(wl_payments::notices::CHEQUE_TASK)
        .into_iter()
        .any(
            |r| matches!(r, wl_kernel::notify::NotifyRecipient::User { id } if id == accountant.id),
        );
    assert!(told);

    // A new request is possible once the first one failed.
    t.write_off_cheque(&a, &cheque, "untraceable again")
        .await
        .assert_ok();
    assert_eq!(t.invoices.write_offs().len(), 2);
    t.approve_invoice_write_off(inv).await;
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::WrittenOff);
}

/// The party pays while the request waits: the request fails because the
/// balance changed, and the cheque stays.
#[renox::test]
async fn a_request_that_fails_because_the_party_paid_leaves_the_cheque() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.invoices.require_approval(true);
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = bounced_for(&t, &a, "5006", 1_000_000, inv).await;
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();

    t.invoices.set_balance(inv, 400_000);
    t.end_invoice_write_off(inv, ApprovalEnd::Failed).await;

    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
    let rows = ChequeWriteOff::of_cheque(t.db(), cheque.id).await.unwrap();
    assert_eq!(rows[0].status, WriteOffStatus::Failed);
    assert!(rows[0].failure.as_deref().unwrap().contains("changed"));
}

/// AC4: open AED 10,000.00 linked to INV-5 with a balance of AED 12,500.00 is
/// refused with a message to write the invoice off directly.
#[renox::test]
async fn balances_that_differ_from_the_open_amount_are_refused() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_250_000);
    let cheque = bounced_for(&t, &a, "5007", 1_000_000, inv).await;

    // The message (checked in the module's unit tests) tells the user to
    // write the invoice off directly.
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_status(422);
    assert!(t.invoices.write_offs().is_empty());
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
    assert!(
        ChequeWriteOff::of_cheque(t.db(), cheque.id)
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC5: open AED 10,000.00 linked to INV-5 (AED 6,000.00) and INV-6 (AED
/// 4,000.00): two requests; after only INV-5 is approved the cheque is still
/// `bounced`; after INV-6 it is `written_off`.
#[renox::test]
async fn several_invoices_are_written_off_one_request_each() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.invoices.require_approval(true);
    let inv5 = t.invoices.add(BLUE_DUNES, "INV-5", 600_000);
    let inv6 = t.invoices.add(BLUE_DUNES, "INV-6", 400_000);
    let entity = t.entity("PGP").await;
    let request = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "5008", today(), 1_000_000)
        .link(inv5.subject())
        .link(inv6.subject());
    let cheque = t.register_cheque(request).await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "insufficient_funds", "CERT-1", "")
        .await
        .assert_ok();
    let cheque = t.cheque(cheque.id).await;

    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();
    assert_eq!(t.invoices.write_offs(), vec![inv5, inv6]);

    t.approve_invoice_write_off(inv5).await;
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
    assert!(t.cheque_resolved.all().is_empty());

    t.approve_invoice_write_off(inv6).await;
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::WrittenOff);
    assert_eq!(t.cheque_resolved.all().len(), 1);
}

/// One invoice approved and the other rejected: the cheque stays, the
/// approved write-off stays, and the user may write the rest off.
#[renox::test]
async fn an_approved_write_off_stays_when_the_other_one_fails() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.invoices.require_approval(true);
    let inv5 = t.invoices.add(BLUE_DUNES, "INV-5", 600_000);
    let inv6 = t.invoices.add(BLUE_DUNES, "INV-6", 400_000);
    let entity = t.entity("PGP").await;
    let request = RegisterCheque::new(BLUE_DUNES, entity.id, "ENBD", "5009", today(), 1_000_000)
        .link(inv5.subject())
        .link(inv6.subject());
    let cheque = t.register_cheque(request).await;
    let cheque = t.deposit_cheque(&a, &cheque).await;
    t.bounce_cheque(&a, &cheque, "insufficient_funds", "CERT-1", "")
        .await
        .assert_ok();
    let cheque = t.cheque(cheque.id).await;
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();

    t.approve_invoice_write_off(inv5).await;
    t.end_invoice_write_off(inv6, ApprovalEnd::Rejected).await;
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
    assert_eq!(t.invoices.status_of(inv5), Some(InvoiceStatus::WrittenOff));

    // Only INV-6 is requested again: INV-5 is written off already.
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();
    assert_eq!(t.invoices.write_offs(), vec![inv5, inv6, inv6]);
    t.approve_invoice_write_off(inv6).await;
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::WrittenOff);
}

/// A second request while one waits is refused.
#[renox::test]
async fn a_request_is_not_made_twice_while_it_waits() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    t.invoices.require_approval(true);
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = bounced_for(&t, &a, "5010", 1_000_000, inv).await;
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_ok();
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_status(422);
    assert_eq!(t.invoices.write_offs().len(), 1);
}

/// A reason is required; a disputed invoice refuses the write-off; the
/// permission `cheques.manage` is needed.
#[renox::test]
async fn a_reason_is_required_and_a_disputed_invoice_refuses() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let viewer = t
        .user("Vera", &[wl_payments::permissions::CHEQUES_VIEW])
        .await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = bounced_for(&t, &a, "5011", 1_000_000, inv).await;

    t.write_off_cheque(&a, &cheque, "  ")
        .await
        .assert_status(422);
    t.write_off_cheque(&viewer, &cheque, "untraceable")
        .await
        .assert_forbidden();

    t.invoices.set_disputed(inv, true);
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_status(422);
    assert!(t.invoices.write_offs().is_empty());
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::Bounced);
}

/// A cheque in a legal case is written off too, and its case task is closed.
#[renox::test]
async fn a_cheque_in_a_legal_case_can_be_written_off() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = bounced_for(&t, &a, "5012", 1_000_000, inv).await;
    t.refer_cheque(&a, &cheque, "civil_execution", None)
        .await
        .assert_ok();
    assert_eq!(t.cheque(cheque.id).await.status, ChequeStatus::LegalCase);

    t.write_off_cheque(&a, &cheque, "uneconomic to pursue")
        .await
        .assert_ok();

    let done = t.cheque(cheque.id).await;
    assert_eq!(done.status, ChequeStatus::WrittenOff);
    // The case is kept as evidence.
    assert!(done.case_type.is_some());
    let case = tasks_of(&t, &done, FollowupKind::LegalCase).await;
    assert_eq!(case[0].status, FollowupStatus::Done);
}

/// Only an owed cheque is written off.
#[renox::test]
async fn a_cheque_that_is_not_bounced_is_not_written_off() {
    let t = boot().await;
    let a = t.cheque_staff("Amal").await;
    let inv = t.invoices.add(BLUE_DUNES, "INV-5", 1_000_000);
    let cheque = t
        .received_cheque(BLUE_DUNES, "5013", 1_000_000, Some(inv))
        .await;
    t.write_off_cheque(&a, &cheque, "untraceable")
        .await
        .assert_status(422);
    assert!(t.invoices.write_offs().is_empty());
}
