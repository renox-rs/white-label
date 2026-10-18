//! Unallocating a payment from an open invoice (wl-invoicing §4
//! `unallocate`, #70 part 2): the reopened state, the removed and split
//! allocation rows, the log, the idempotency key and the refusals. The
//! money comes in through `Invoicer::allocate` and goes out through
//! `Invoicer::unallocate`, as `wl-payments` does.

mod support;

use renox::prelude::*;
use wl_invoicing::{Allocation, Invoice, Unallocation, UnallocationOutcome};
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    InvoiceId, InvoiceStatus, ReopenReason, Reopened, Unallocate, VatCategory,
};

use support::{Booted, boot};

/// AED 1,000.00 plus 5% VAT.
const LINE: (i64, VatCategory) = (100_000, VatCategory::Standard);
const GROSS: i64 = 105_000;

fn payment(n: i64) -> Subject {
    Subject::new("payments.payment", n)
}

fn request(invoice: &Invoice, n: i64, amount: i64, key: &str) -> Unallocate {
    Unallocate::new(
        InvoiceId(invoice.id),
        payment(n),
        amount,
        key,
        ReopenReason::PaymentReversed,
    )
    .reversal_reason("bank_return")
}

async fn unallocate(t: &Booted, request: Unallocate) -> Result<Reopened> {
    wl_kernel::invoicing::invoicer(t.state())
        .unallocate(t.state(), request)
        .await
}

async fn rows(t: &Booted, invoice: &Invoice) -> Vec<Allocation> {
    Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .order_by(Allocation::ID)
        .get(t.db())
        .await
        .unwrap()
}

async fn logs(t: &Booted, invoice: &Invoice) -> Vec<Unallocation> {
    Unallocation::query()
        .where_(Unallocation::INVOICE_ID.eq(invoice.id))
        .order_by(Unallocation::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// A `paid` invoice due in `due` days, paid by payment 1 in full.
async fn paid(t: &Booted, due: i64) -> Invoice {
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(due)), &a).await;
    t.pay(&invoice, 1, GROSS).await;
    let invoice = t.reload(&invoice).await;
    assert_eq!(invoice.status, InvoiceStatus::Paid);
    invoice
}

#[renox::test]
async fn a_reversed_payment_reopens_a_paid_invoice_as_issued() {
    let t = boot().await;
    let invoice = paid(&t, 20).await;
    let lines = wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap();

    let reopened = unallocate(&t, request(&invoice, 1, GROSS, "payments.allocation:1"))
        .await
        .unwrap();
    assert_eq!(
        reopened,
        Reopened {
            new_status: InvoiceStatus::Issued,
            balance: GROSS,
            skipped: false
        }
    );
    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance, after.allocated_total),
        (InvoiceStatus::Issued, GROSS, 0)
    );

    // The allocation is removed, not deleted.
    let allocations = rows(&t, &invoice).await;
    assert_eq!(allocations.len(), 1);
    let removed = &allocations[0];
    assert!(!removed.is_active());
    assert_eq!(removed.removed_reason.as_deref(), Some("payment_reversed"));
    assert_eq!(
        removed.removed_key.as_deref(),
        Some("payments.allocation:1")
    );

    // The log has one done row.
    let log = logs(&t, &invoice).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, UnallocationOutcome::Done);
    assert_eq!(log[0].transition.as_deref(), Some("unallocate"));
    assert_eq!(log[0].balance_after, Some(GROSS));
    assert_eq!(log[0].reversal_reason.as_deref(), Some("bank_return"));
    assert_eq!(t.history(&after).await.last().unwrap(), "unallocate");

    // Lines, totals, number and snapshots are as they were.
    assert_eq!(after.number, invoice.number);
    assert_eq!(after.gross_total, invoice.gross_total);
    assert_eq!(after.vat_total, invoice.vat_total);
    assert_eq!(after.party_snapshot, invoice.party_snapshot);
    assert_eq!(after.issuer_snapshot, invoice.issuer_snapshot);
    let lines_after = wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap();
    assert_eq!(
        renox::serde_json::to_value(&lines).unwrap(),
        renox::serde_json::to_value(&lines_after).unwrap()
    );
}

#[renox::test]
async fn the_same_key_twice_changes_nothing() {
    let t = boot().await;
    let invoice = paid(&t, 20).await;
    let first = request(&invoice, 1, GROSS, "payments.allocation:1");
    unallocate(&t, first.clone()).await.unwrap();

    let again = unallocate(&t, first).await.unwrap();
    assert!(again.skipped);
    assert_eq!(
        (again.new_status, again.balance),
        (InvoiceStatus::Issued, GROSS)
    );
    assert_eq!(logs(&t, &invoice).await.len(), 1);
    assert_eq!(rows(&t, &invoice).await.len(), 1);
    assert_eq!(t.reload(&invoice).await.balance, GROSS);
}

#[renox::test]
async fn reversing_one_of_two_payments_leaves_the_invoice_partly_paid() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&invoice, 1, 60_000).await;
    t.pay(&invoice, 2, 45_000).await;

    let reopened = unallocate(&t, request(&invoice, 2, 45_000, "payments.allocation:2"))
        .await
        .unwrap();
    assert_eq!(reopened.new_status, InvoiceStatus::PartiallyPaid);
    assert_eq!(reopened.balance, 45_000);
    let allocations = rows(&t, &invoice).await;
    assert!(allocations[0].is_active() && allocations[0].amount == 60_000);
    assert!(!allocations[1].is_active());
    assert_eq!(
        logs(&t, &invoice).await[0].transition.as_deref(),
        Some("unallocate_partially")
    );
}

#[renox::test]
async fn a_part_of_one_allocation_splits_its_row() {
    let t = boot().await;
    let invoice = paid(&t, 20).await;

    let reopened = unallocate(&t, request(&invoice, 1, 30_000, "payments.allocation:1"))
        .await
        .unwrap();
    assert_eq!(reopened.new_status, InvoiceStatus::PartiallyPaid);
    assert_eq!(reopened.balance, 30_000);
    let allocations = rows(&t, &invoice).await;
    assert_eq!(allocations.len(), 2);
    assert!(!allocations[0].is_active());
    assert_eq!(allocations[0].amount, GROSS);
    assert!(allocations[1].is_active());
    assert_eq!(allocations[1].amount, GROSS - 30_000);
    let after = t.reload(&invoice).await;
    assert_eq!(after.allocated_total, GROSS - 30_000);

    // The rest can be reversed later, whole.
    let rest = unallocate(
        &t,
        request(&invoice, 1, GROSS - 30_000, "payments.allocation:9"),
    )
    .await
    .unwrap();
    assert_eq!(rest.new_status, InvoiceStatus::Issued);
    assert_eq!(rest.balance, GROSS);
}

#[renox::test]
async fn an_invoice_past_its_due_date_reopens_as_overdue() {
    let t = boot().await;
    let a = t.user_a().await;
    let overdue = t.overdue_invoice(&a).await;
    t.pay(&overdue, 1, GROSS).await;
    assert_eq!(t.reload(&overdue).await.status, InvoiceStatus::Paid);

    let reopened = unallocate(&t, request(&overdue, 1, GROSS, "payments.allocation:1"))
        .await
        .unwrap();
    assert_eq!(reopened.new_status, InvoiceStatus::Overdue);
    assert_eq!(
        logs(&t, &overdue).await[0].transition.as_deref(),
        Some("unallocate_overdue")
    );
}

#[renox::test]
async fn more_than_is_allocated_is_refused_and_logged() {
    let t = boot().await;
    let invoice = paid(&t, 20).await;

    let refused = unallocate(&t, request(&invoice, 1, GROSS + 1, "payments.allocation:1")).await;
    assert!(refused.is_err());
    let other = unallocate(&t, request(&invoice, 7, 100, "payments.allocation:2")).await;
    assert!(other.is_err());

    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance),
        (InvoiceStatus::Paid, invoice.balance)
    );
    assert!(rows(&t, &invoice).await.iter().all(Allocation::is_active));
    let log = logs(&t, &invoice).await;
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|l| l.outcome == UnallocationOutcome::Failed));
    assert!(
        log[0]
            .message
            .as_deref()
            .unwrap()
            .contains("Only AED 1,050.00")
    );

    // A refused key is not run again.
    let again = unallocate(&t, request(&invoice, 1, 100, "payments.allocation:1"))
        .await
        .unwrap();
    assert!(again.skipped);
    assert_eq!(t.reload(&invoice).await.balance, invoice.balance);
}

#[renox::test]
async fn an_amount_of_zero_and_an_unknown_invoice_are_refused() {
    let t = boot().await;
    let invoice = paid(&t, 20).await;
    assert!(
        unallocate(&t, request(&invoice, 1, 0, "k:zero"))
            .await
            .is_err()
    );
    let ghost = Unallocate::new(
        InvoiceId(9_999),
        payment(1),
        100,
        "k:ghost",
        ReopenReason::PaymentReversed,
    );
    assert!(unallocate(&t, ghost).await.is_err());
    let failed = Unallocation::by_key(t.db(), "k:ghost").await.unwrap();
    assert_eq!(failed.unwrap().outcome, UnallocationOutcome::Failed);
}

#[renox::test]
async fn an_issued_invoice_has_nothing_to_unallocate() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    assert!(
        unallocate(&t, request(&invoice, 1, 100, "k:issued"))
            .await
            .is_err()
    );
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Issued);
    assert_eq!(
        logs(&t, &invoice).await[0].outcome,
        UnallocationOutcome::Failed
    );
}

#[renox::test]
async fn a_reversed_recovery_is_not_run_here() {
    let t = boot().await;
    let invoice = paid(&t, 20).await;
    let recovery = Unallocate::new(
        InvoiceId(invoice.id),
        payment(1),
        GROSS,
        "k:recovery",
        ReopenReason::RecoveryReversed,
    );
    assert!(unallocate(&t, recovery).await.is_err());
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);
}

#[renox::test]
async fn unallocate_all_removes_every_source_and_reopens_the_invoice() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&invoice, 1, 60_000).await;
    t.pay(&invoice, 2, 45_000).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);

    let removed = wl_kernel::invoicing::invoicer(t.state())
        .unallocate_all(
            t.state(),
            InvoiceId(invoice.id),
            ReopenReason::AllocationMoved,
        )
        .await
        .unwrap();
    assert_eq!(removed, vec![(payment(1), 60_000), (payment(2), 45_000)]);
    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance, after.allocated_total),
        (InvoiceStatus::Issued, GROSS, 0)
    );
    assert!(rows(&t, &invoice).await.iter().all(|r| !r.is_active()));

    // Nothing is left to remove.
    let again = wl_kernel::invoicing::invoicer(t.state())
        .unallocate_all(
            t.state(),
            InvoiceId(invoice.id),
            ReopenReason::AllocationMoved,
        )
        .await
        .unwrap();
    assert!(again.is_empty());
}

#[renox::test]
async fn unallocate_all_on_an_overdue_due_date_leaves_it_overdue() {
    let t = boot().await;
    let a = t.user_a().await;
    let overdue = t.overdue_invoice(&a).await;
    t.pay(&overdue, 1, 40_000).await;
    t.pay(&overdue, 2, 65_000).await;
    let removed = wl_kernel::invoicing::invoicer(t.state())
        .unallocate_all(
            t.state(),
            InvoiceId(overdue.id),
            ReopenReason::AllocationMoved,
        )
        .await
        .unwrap();
    assert_eq!(removed.len(), 2);
    let after = t.reload(&overdue).await;
    assert_eq!(
        (after.status, after.balance),
        (InvoiceStatus::Overdue, GROSS)
    );
}
