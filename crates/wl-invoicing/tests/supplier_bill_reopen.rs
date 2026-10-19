//! Acceptance checks of reopening a supplier bill when a payment is reversed
//! (#75, AC1 to AC8). The consumer, `unallocate` and the notices have their
//! own files; this one covers the acceptance criteria end to end, in
//! particular the ones no other test states: a cheque cancelled before it
//! clears, a debit note alone, the refused states, the untouched approval
//! data, a mixed event and the absence of any user action.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::{approve, void};
use wl_invoicing::bill_submit::submit;
use wl_invoicing::bill_unallocate::run;
use wl_invoicing::credit_notes::By;
use wl_invoicing::payables::Payables;
use wl_invoicing::supplier_bill::{self, BillDraft, SupplierBillStatus};
use wl_invoicing::task::InvoicingTask;
use wl_invoicing::{BillAllocation, BillLineDraft, BillUnallocation, SupplierBill};
use wl_kernel::invoicing::{
    BillReopenReason, INVOICE_SUBJECT_TYPE, InvoiceStatus, PayBill, SUPPLIER_BILL_SUBJECT_TYPE,
    SupplierBillId, SupplierBills, UnallocateBill, VatCategory,
};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{
    PaymentId, PaymentReversed, PaymentSource, RemovedAllocation, ReversalReason,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

const SUPPLIER: PartyId = PartyId(2);
/// AED 10,500.00.
const GROSS: i64 = 1_050_000;
const REOPENED: &str = "invoicing.supplier_bill_reopened";

fn payment(id: i64) -> Subject {
    Subject::new("payments.payment", id)
}

fn note(id: i64) -> Subject {
    Subject::new("invoicing.debit_note", id)
}

async fn draft(t: &Booted, number: &str) -> SupplierBill {
    let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let draft = BillDraft::new(SUPPLIER, number, day).line(BillLineDraft::new(
        "Pump repair",
        1_000_000,
        VatCategory::Standard,
    ));
    supplier_bill::create(t.db(), &draft, Some(7))
        .await
        .unwrap()
}

async fn approved(t: &Booted, number: &str) -> SupplierBill {
    let bill = draft(t, number).await;
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

async fn pay(t: &Booted, bill: &SupplierBill, source: Subject, amount: i64, key: &str) {
    Payables
        .pay(
            t.state(),
            PayBill::new(SupplierBillId(bill.id), source, amount, key),
        )
        .await
        .unwrap();
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

fn reversed(payment: i64, removed: Vec<RemovedAllocation>) -> PaymentReversed {
    PaymentReversed {
        payment: PaymentId(payment),
        party: SUPPLIER,
        legal_entity_id: 1,
        reason: ReversalReason::BankReturn,
        source: PaymentSource::Manual,
        statement_line: None,
        removed,
        prepaid: None,
        reversed_on: NaiveDate::from_ymd_opt(2026, 12, 10).unwrap(),
    }
}

fn removed_of(allocation: i64, subject_type: &str, id: i64, amount: i64) -> RemovedAllocation {
    RemovedAllocation {
        allocation,
        target: Subject::new(subject_type, id),
        amount,
    }
}

async fn logs(t: &Booted, bill: &SupplierBill) -> usize {
    BillUnallocation::query()
        .where_(BillUnallocation::BILL_ID.eq(bill.id))
        .get(t.db())
        .await
        .unwrap()
        .len()
}

#[renox::test]
async fn ac1_a_paid_bill_is_reopened_by_a_bank_return() {
    let t = boot().await;
    let zaid = t.accountant("Zaid").await;
    let amal = t.accountant("Amal2").await;
    let bill = approved(&t, "AC-1").await;
    pay(&t, &bill, payment(1), GROSS, "payments.allocation:11").await;
    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Paid);

    let event = reversed(
        1,
        vec![removed_of(11, SUPPLIER_BILL_SUBJECT_TYPE, bill.id, GROSS)],
    );
    t.state().emit(event).await.unwrap();

    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::Approved, GROSS)
    );
    let rows = BillAllocation::of_bill(t.db(), bill.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_active());

    let notices = t.notices.of_type(REOPENED);
    let mut told: Vec<i64> = notices
        .iter()
        .flat_map(|n| {
            n.recipients.iter().filter_map(|r| match r {
                NotifyRecipient::User { id } => Some(*id),
                _ => None,
            })
        })
        .collect();
    told.sort_unstable();
    let mut expected = vec![zaid.id, amal.id];
    expected.sort_unstable();
    assert_eq!(told, expected, "each accountant once");
    assert!(
        notices[0].data["reason"]
            .as_str()
            .unwrap()
            .contains("bank return")
    );

    let emitted = t.bill_reopened.all();
    assert_eq!(emitted.len(), 1);
    assert_eq!(format!("{:?}", emitted[0].new_status), "Approved");
}

#[renox::test]
async fn ac2_reversing_one_of_two_payments_leaves_the_other_applied() {
    let t = boot().await;
    let bill = approved(&t, "AC-2").await;
    pay(&t, &bill, payment(1), 600_000, "payments.allocation:1").await;
    pay(&t, &bill, payment(2), 450_000, "payments.allocation:2").await;

    t.state()
        .emit(reversed(
            2,
            vec![removed_of(2, SUPPLIER_BILL_SUBJECT_TYPE, bill.id, 450_000)],
        ))
        .await
        .unwrap();

    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!(after.balance, 450_000);
    let active: Vec<_> = BillAllocation::of_bill(t.db(), bill.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.is_active())
        .map(|r| r.amount)
        .collect();
    assert_eq!(active, [600_000]);
}

#[renox::test]
async fn ac3_the_same_event_twice_changes_nothing_the_second_time() {
    let t = boot().await;
    let _ = t.accountant("Zaid").await;
    let bill = approved(&t, "AC-3").await;
    pay(&t, &bill, payment(1), GROSS, "payments.allocation:11").await;
    let event = reversed(
        1,
        vec![removed_of(11, SUPPLIER_BILL_SUBJECT_TYPE, bill.id, GROSS)],
    );

    t.state().emit(event.clone()).await.unwrap();
    let notices = t.notices.of_type(REOPENED).len();
    t.state().emit(event).await.unwrap();

    assert_eq!(logs(&t, &bill).await, 1);
    assert_eq!(t.notices.of_type(REOPENED).len(), notices);
    assert_eq!(notices, 1);
    assert_eq!(t.bill_reopened.all().len(), 1);
    assert_eq!(reload(&t, &bill).await.balance, GROSS);
}

#[renox::test]
async fn ac4_a_cheque_cancelled_before_it_clears_leaves_the_bill_alone() {
    let t = boot().await;
    let _ = t.accountant("Zaid").await;
    let bill = approved(&t, "AC-4").await;
    // An issued cheque creates its outgoing payment only when it clears, so
    // wl-payments allocates nothing and removes nothing: the only event a
    // cancellation can send names no bill allocation.
    t.state().emit(reversed(5, Vec::new())).await.unwrap();

    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::Approved, GROSS)
    );
    assert_eq!(logs(&t, &bill).await, 0);
    assert!(
        BillAllocation::of_bill(t.db(), bill.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.bill_reopened.all().is_empty());
    assert!(t.notices.of_type(REOPENED).is_empty());
}

#[renox::test]
async fn ac5_a_debit_note_alone_reopens_to_approved_and_with_a_payment_to_partially_paid() {
    let t = boot().await;
    let bill = approved(&t, "AC-5").await;
    pay(&t, &bill, note(1), 31_500, "invoicing.debit_note:1").await;
    let part = reload(&t, &bill).await;
    assert_eq!(part.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!(part.balance, GROSS - 31_500);

    let request = UnallocateBill::new(
        SupplierBillId(bill.id),
        note(1),
        31_500,
        "invoicing.debit_note:1:cancel",
        BillReopenReason::DebitNoteCancelled,
    );
    run(t.state(), request).await.unwrap();
    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::Approved, GROSS)
    );

    // With a payment that remains the result is partially_paid.
    let other = approved(&t, "AC-5b").await;
    pay(&t, &other, note(2), 31_500, "invoicing.debit_note:2").await;
    pay(&t, &other, payment(3), 200_000, "payments.allocation:3").await;
    let request = UnallocateBill::new(
        SupplierBillId(other.id),
        note(2),
        31_500,
        "invoicing.debit_note:2:cancel",
        BillReopenReason::DebitNoteCancelled,
    );
    run(t.state(), request).await.unwrap();
    let after = reload(&t, &other).await;
    assert_eq!(after.status, SupplierBillStatus::PartiallyPaid);
    assert_eq!(after.balance, GROSS - 200_000);
}

#[renox::test]
async fn ac6_too_much_is_refused_with_a_log_a_task_and_a_notice() {
    let t = boot().await;
    let _ = t.accountant("Zaid").await;
    let bill = approved(&t, "AC-6").await;
    pay(&t, &bill, payment(1), 400_000, "payments.allocation:11").await;

    t.state()
        .emit(reversed(
            1,
            vec![removed_of(11, SUPPLIER_BILL_SUBJECT_TYPE, bill.id, 500_000)],
        ))
        .await
        .unwrap();

    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::PartiallyPaid, GROSS - 400_000)
    );
    let log = BillUnallocation::by_key(t.db(), "payments.allocation:11")
        .await
        .unwrap()
        .expect("logged");
    assert_eq!(log.outcome, wl_invoicing::UnallocationOutcome::Failed);
    let open = InvoicingTask::query()
        .where_(InvoicingTask::BILL_ID.eq(bill.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].role, "accountant");
    assert_eq!(
        t.notices.of_type(wl_invoicing::task::REVERSAL_TASK).len(),
        1
    );

    // The direct call is refused too, and nothing changes.
    assert!(
        run(t.state(), undo(&bill, payment(1), 500_000, "k:direct"))
            .await
            .is_err()
    );
    assert_eq!(reload(&t, &bill).await.balance, GROSS - 400_000);
}

#[renox::test]
async fn ac6_a_void_and_an_exception_bill_are_refused() {
    let t = boot().await;
    let voided = draft(&t, "AC-6v").await;
    void(t.state(), By::System(Some(7)), voided.id, "wrong supplier")
        .await
        .unwrap();
    assert_eq!(reload(&t, &voided).await.status, SupplierBillStatus::Void);

    let first = approved(&t, "AC-6d").await;
    assert_eq!(first.status, SupplierBillStatus::Approved);
    let second = draft(&t, "ac-6d ").await;
    let held = submit(t.state(), By::System(Some(7)), second.id)
        .await
        .unwrap();
    assert_eq!(held.status, SupplierBillStatus::Exception);

    for (n, bill) in [&voided, &held].into_iter().enumerate() {
        let before = reload(&t, bill).await;
        assert!(
            run(t.state(), undo(bill, payment(1), 100, &format!("k:{n}")))
                .await
                .is_err()
        );
        let after = reload(&t, bill).await;
        assert_eq!(
            (before.status, before.balance, before.allocated_total),
            (after.status, after.balance, after.allocated_total)
        );
    }
}

#[renox::test]
async fn ac7_a_reopen_changes_only_the_money_side_and_invoices_in_the_same_event_still_reopen() {
    let t = boot().await;
    let a = t.user_a().await;
    let _ = t.accountant("Zaid").await;
    let bill = approved(&t, "AC-7").await;
    pay(&t, &bill, payment(1), GROSS, "payments.allocation:11").await;
    let invoice = t
        .issued(&[(100_000, VatCategory::Standard)], Some(t.day(20)), &a)
        .await;
    t.pay(&invoice, 1, 105_000).await;
    let before = reload(&t, &bill).await;
    assert!(before.approved_by.is_some());

    t.state()
        .emit(reversed(
            1,
            vec![
                removed_of(11, SUPPLIER_BILL_SUBJECT_TYPE, bill.id, GROSS),
                removed_of(12, INVOICE_SUBJECT_TYPE, invoice.id, 105_000),
            ],
        ))
        .await
        .unwrap();

    let after = reload(&t, &bill).await;
    assert_eq!(after.status, SupplierBillStatus::Approved);
    let mut x = renox::serde_json::to_value(&before).unwrap();
    let mut y = renox::serde_json::to_value(&after).unwrap();
    for field in ["status", "allocated_total", "balance", "updated_at"] {
        x[field] = renox::serde_json::Value::Null;
        y[field] = renox::serde_json::Value::Null;
    }
    assert_eq!(x, y);

    let reopened = t.reload(&invoice).await;
    assert_eq!(reopened.status, InvoiceStatus::Issued);
    assert_eq!(reopened.balance, 105_000);
}

#[renox::test]
async fn ac8_no_user_action_runs_unallocate() {
    let t = boot().await;
    let zaid = t.accountant("Zaid").await;
    let bill = approved(&t, "AC-8").await;
    pay(&t, &bill, payment(1), GROSS, "payments.allocation:11").await;

    let machine = <SupplierBill as renox::state_machine::StateMachine>::machine();
    for name in ["unallocate", "unallocate_partially"] {
        assert!(machine.find(name).expect(name).is_system());
    }
    t.app.acting_as(&zaid);
    for name in ["unallocate", "unallocate_partially"] {
        let response = t
            .app
            .htmx()
            .post(
                &format!("/supplier-bills/{}/transitions/{name}", bill.id),
                &[("reason", "try")],
            )
            .await;
        response.assert_status(403);
    }
    let after = reload(&t, &bill).await;
    assert_eq!((after.status, after.balance), (SupplierBillStatus::Paid, 0));
    assert_eq!(logs(&t, &bill).await, 0);
}
