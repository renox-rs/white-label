//! Reopening a supplier bill (wl-invoicing §4 `unallocate`, #75 part 3):
//! the money comes back, the bill returns to `approved` or
//! `partially_paid`, rows are marked removed and never deleted, the key
//! makes a repeat a no-op, and a refusal is logged.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::bill_unallocate::run;
use wl_invoicing::credit_notes::By;
use wl_invoicing::payables::Payables;
use wl_invoicing::supplier_bill::{self, BillDraft, SupplierBillStatus};
use wl_invoicing::unallocation::UnallocationOutcome;
use wl_invoicing::{BillAllocation, BillLineDraft, BillUnallocation, SupplierBill};
use wl_kernel::invoicing::{
    BillReopenReason, PayBill, SupplierBillId, SupplierBills, UnallocateBill, VatCategory,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

const SUPPLIER: PartyId = PartyId(2);
/// AED 10,500.00.
const GROSS: i64 = 1_050_000;

fn payment(id: i64) -> Subject {
    Subject::new("payments.payment", id)
}

fn note(id: i64) -> Subject {
    Subject::new("invoicing.debit_note", id)
}

async fn approved(t: &Booted, number: &str) -> SupplierBill {
    let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let draft = BillDraft::new(SUPPLIER, number, day).line(BillLineDraft::new(
        "Pump repair",
        1_000_000,
        VatCategory::Standard,
    ));
    let bill = supplier_bill::create(t.db(), &draft, Some(7))
        .await
        .unwrap();
    submit(t.state(), By::System(Some(7)), bill.id)
        .await
        .unwrap();
    approve(t.state(), By::System(Some(8)), bill.id, "")
        .await
        .unwrap()
}

async fn reload(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
}

fn pay_of(bill: &SupplierBill, source: Subject, amount: i64, key: &str) -> PayBill {
    PayBill::new(SupplierBillId(bill.id), source, amount, key)
}

fn undo(bill: &SupplierBill, source: Subject, amount: i64, key: &str) -> UnallocateBill {
    UnallocateBill::new(
        SupplierBillId(bill.id),
        source,
        amount,
        key,
        BillReopenReason::PaymentReversed,
    )
    .reversal_reason("bank_return")
}

async fn pay(t: &Booted, bill: &SupplierBill, source: Subject, amount: i64, key: &str) {
    Payables
        .pay(t.state(), pay_of(bill, source, amount, key))
        .await
        .unwrap();
}

#[renox::test]
async fn a_paid_bill_reverses_to_approved_and_keeps_its_removed_row() {
    let t = boot().await;
    let bill = approved(&t, "INV-1").await;
    pay(&t, &bill, payment(1), GROSS, "payments.allocation:1").await;
    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Paid);

    let before = reload(&t, &bill).await;
    let done = Payables
        .unallocate(
            t.state(),
            undo(&bill, payment(1), GROSS, "payments.allocation:1"),
        )
        .await
        .unwrap();
    assert!(!done.skipped);
    assert_eq!(done.balance, GROSS);
    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::Approved);
    assert_eq!((after.allocated_total, after.balance), (0, GROSS));

    // Nothing but the money side changed.
    let mut a = renox::serde_json::to_value(&before).unwrap();
    let mut b = renox::serde_json::to_value(&after).unwrap();
    for field in ["status", "allocated_total", "balance", "updated_at"] {
        a[field] = renox::serde_json::Value::Null;
        b[field] = renox::serde_json::Value::Null;
    }
    assert_eq!(a, b);

    let rows = BillAllocation::of_bill(t.db(), bill.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_active());
    assert_eq!(rows[0].removed_reason.as_deref(), Some("payment_reversed"));
    assert_eq!(
        rows[0].removed_key.as_deref(),
        Some("payments.allocation:1")
    );

    let log = BillUnallocation::by_key(t.db(), "payments.allocation:1")
        .await
        .unwrap()
        .expect("logged");
    assert_eq!(log.outcome, UnallocationOutcome::Done);
    assert_eq!(log.transition.as_deref(), Some("unallocate"));
    assert_eq!(log.balance_after, Some(GROSS));
    assert_eq!(log.reversal_reason.as_deref(), Some("bank_return"));
}

#[renox::test]
async fn reversing_one_of_two_payments_leaves_the_bill_partially_paid() {
    let t = boot().await;
    let bill = approved(&t, "INV-2").await;
    pay(&t, &bill, payment(1), 600_000, "payments.allocation:1").await;
    pay(&t, &bill, payment(2), 450_000, "payments.allocation:2").await;
    let done = Payables
        .unallocate(
            t.state(),
            undo(&bill, payment(2), 450_000, "payments.allocation:2"),
        )
        .await
        .unwrap();
    assert_eq!(done.balance, 450_000);
    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!((after.allocated_total, after.balance), (600_000, 450_000));
    let active: Vec<_> = BillAllocation::of_bill(t.db(), bill.id)
        .await
        .unwrap()
        .into_iter()
        .filter(BillAllocation::is_active)
        .map(|r| (r.source(), r.amount))
        .collect();
    assert_eq!(active, [(payment(1), 600_000)]);
    let log = BillUnallocation::by_key(t.db(), "payments.allocation:2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(log.transition.as_deref(), Some("unallocate_partially"));
}

#[renox::test]
async fn a_part_of_a_larger_row_is_split_and_the_rest_stays_active() {
    let t = boot().await;
    let bill = approved(&t, "INV-3").await;
    pay(&t, &bill, payment(1), 600_000, "payments.allocation:1").await;
    run(
        t.state(),
        undo(&bill, payment(1), 100_000, "payments.allocation:9"),
    )
    .await
    .unwrap();
    let rows = BillAllocation::of_bill(t.db(), bill.id).await.unwrap();
    let seen: Vec<_> = rows.iter().map(|r| (r.amount, r.is_active())).collect();
    assert_eq!(seen, [(600_000, false), (500_000, true)]);
    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!((after.allocated_total, after.balance), (500_000, 550_000));
}

#[renox::test]
async fn a_debit_note_that_stays_applied_keeps_the_bill_partially_paid() {
    let t = boot().await;
    let bill = approved(&t, "INV-4").await;
    pay(&t, &bill, note(1), 31_500, "invoicing.debit_note:1").await;
    pay(
        &t,
        &bill,
        payment(1),
        GROSS - 31_500,
        "payments.allocation:1",
    )
    .await;
    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Paid);
    run(
        t.state(),
        undo(&bill, payment(1), GROSS - 31_500, "payments.allocation:1"),
    )
    .await
    .unwrap();
    // The debit note still counts as applied.
    assert_eq!(
        reload(&t, &bill).await.status,
        SupplierBillStatus::PartiallyPaid
    );
    // Cancelling the note then reopens the bill fully.
    let request = UnallocateBill::new(
        SupplierBillId(bill.id),
        note(1),
        31_500,
        "invoicing.debit_note:1:cancel",
        BillReopenReason::DebitNoteCancelled,
    );
    let done = run(t.state(), request).await.unwrap();
    assert_eq!(done.balance, GROSS);
    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Approved);
}

#[renox::test]
async fn the_same_key_twice_changes_nothing_the_second_time() {
    let t = boot().await;
    let bill = approved(&t, "INV-5").await;
    pay(&t, &bill, payment(1), GROSS, "payments.allocation:1").await;
    // A repeated payment key is skipped too.
    let again = Payables
        .pay(
            t.state(),
            pay_of(&bill, payment(1), GROSS, "payments.allocation:1"),
        )
        .await
        .unwrap();
    assert!(again.skipped);
    assert_eq!(
        BillAllocation::of_bill(t.db(), bill.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let request = undo(&bill, payment(1), GROSS, "payments.allocation:1");
    let first = run(t.state(), request.clone()).await.unwrap();
    let second = run(t.state(), request).await.unwrap();
    assert!(!first.skipped);
    assert!(second.skipped);
    assert_eq!(second.balance, GROSS);
    let log = BillUnallocation::query().get(t.db()).await.unwrap();
    assert_eq!(log.len(), 1);
}

#[renox::test]
async fn too_much_is_refused_logged_failed_and_the_bill_is_unchanged() {
    let t = boot().await;
    let bill = approved(&t, "INV-6").await;
    pay(&t, &bill, payment(1), 600_000, "payments.allocation:1").await;
    for (amount, key) in [(600_001, "k:above"), (0, "k:zero")] {
        let error = run(t.state(), undo(&bill, payment(1), amount, key))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
        ));
    }
    // Money from a source that applied nothing.
    assert!(
        run(t.state(), undo(&bill, payment(7), 100, "k:other"))
            .await
            .is_err()
    );
    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!((after.allocated_total, after.balance), (600_000, 450_000));
    let failed = BillUnallocation::by_key(t.db(), "k:above")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.outcome, UnallocationOutcome::Failed);
    assert!(failed.message.unwrap().contains("Only"));
    assert!(failed.transition.is_none());
    // A refused key is not run again.
    assert!(
        run(t.state(), undo(&bill, payment(1), 100, "k:above"))
            .await
            .unwrap()
            .skipped
    );
}

#[renox::test]
async fn an_approved_a_draft_and_an_unknown_bill_cannot_be_reopened() {
    let t = boot().await;
    let bill = approved(&t, "INV-7").await;
    assert!(
        run(t.state(), undo(&bill, payment(1), 100, "k:1"))
            .await
            .is_err()
    );
    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Approved);

    let day = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
    let draft = BillDraft::new(SUPPLIER, "INV-8", day).line(BillLineDraft::new(
        "Filter",
        1_000,
        VatCategory::Standard,
    ));
    let other = supplier_bill::create(t.db(), &draft, Some(7))
        .await
        .unwrap();
    assert!(
        run(t.state(), undo(&other, payment(1), 100, "k:2"))
            .await
            .is_err()
    );

    let missing = UnallocateBill::new(
        SupplierBillId(9_999),
        payment(1),
        100,
        "k:3",
        BillReopenReason::PaymentReversed,
    );
    assert!(matches!(
        run(t.state(), missing).await,
        Err(Error::NotFound)
    ));
}

#[renox::test]
async fn unallocate_is_system_only_so_no_user_is_offered_it() {
    let machine = <SupplierBill as renox::state_machine::StateMachine>::machine();
    for name in ["unallocate", "unallocate_partially"] {
        assert!(machine.find(name).expect(name).is_system());
    }
}
