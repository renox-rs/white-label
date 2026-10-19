//! The consumer of `PaymentReversed` for supplier bills (wl-invoicing §4
//! `unallocate`, #75 part 4): one `unallocate` per removed bill allocation,
//! `SupplierBillReopened` and the accountant notice, the follow-up task of a
//! refusal, a repeated delivery, and the bill page and tasks list.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::bill_machine::approve;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::payables::Payables;
use wl_invoicing::permissions as perm;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::task::{InvoicingTask, REVERSAL_TASK, TaskKind, TaskStatus};
use wl_invoicing::{
    BillAllocation, BillLineDraft, BillUnallocation, SupplierBill, SupplierBillStatus,
};
use wl_kernel::invoicing::{
    BillReopenReason, INVOICE_SUBJECT_TYPE, PayBill, SUPPLIER_BILL_SUBJECT_TYPE, SupplierBillId,
    SupplierBills, VatCategory,
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

async fn pay(t: &Booted, bill: &SupplierBill, payment: i64, allocation: i64, amount: i64) {
    Payables
        .pay(
            t.state(),
            PayBill::new(
                SupplierBillId(bill.id),
                Subject::new("payments.payment", payment),
                amount,
                format!("payments.allocation:{allocation}"),
            ),
        )
        .await
        .unwrap();
}

async fn reload(t: &Booted, bill: &SupplierBill) -> SupplierBill {
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
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

fn removed(allocation: i64, bill: &SupplierBill, amount: i64) -> RemovedAllocation {
    RemovedAllocation {
        allocation,
        target: Subject::new(SUPPLIER_BILL_SUBJECT_TYPE, bill.id),
        amount,
    }
}

async fn deliver(t: &Booted, event: &PaymentReversed) {
    t.state().emit(event.clone()).await.unwrap();
}

async fn tasks(t: &Booted, bill: &SupplierBill) -> Vec<InvoicingTask> {
    InvoicingTask::query()
        .where_(InvoicingTask::BILL_ID.eq(bill.id))
        .order_by(InvoicingTask::ID)
        .get(t.db())
        .await
        .unwrap()
}

fn users_of(request: &wl_kernel::notify::NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

#[renox::test]
async fn a_reversed_payment_reopens_the_bill_and_tells_each_accountant_once() {
    let t = boot().await;
    let zaid = t.accountant("Zaid").await;
    let amal = t.accountant("Amal2").await;
    let bill = approved(&t, "INV-1").await;
    pay(&t, &bill, 1, 11, GROSS).await;
    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Paid);

    deliver(&t, &reversed(1, vec![removed(11, &bill, GROSS)])).await;

    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::Approved, GROSS)
    );
    let rows = BillAllocation::of_bill(t.db(), bill.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_active());

    let events = t.bill_reopened.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].bill, SupplierBillId(bill.id));
    assert_eq!(events[0].supplier, SUPPLIER);
    assert_eq!(events[0].source, Subject::new("payments.payment", 1));
    assert_eq!((events[0].amount, events[0].balance), (GROSS, GROSS));
    assert_eq!(events[0].reason, BillReopenReason::PaymentReversed);
    assert_eq!(
        events[0].new_status,
        wl_kernel::invoicing::SupplierBillStatus::Approved
    );

    let notices = t.notices.of_type(REOPENED);
    assert_eq!(notices.len(), 1);
    let mut told = users_of(&notices[0]);
    told.sort_unstable();
    let mut expected = vec![zaid.id, amal.id];
    expected.sort_unstable();
    assert_eq!(told, expected, "each accountant, once");
    let text = notices[0].data["reason"].as_str().unwrap();
    assert!(text.contains("bank return"), "{text}");
    assert_eq!(notices[0].data["new_status"], "Approved");
    assert_eq!(
        notices[0].data["hint"],
        "Pay again or fix the bank details."
    );
    assert!(tasks(&t, &bill).await.is_empty());
}

#[renox::test]
async fn reversing_one_of_two_payments_leaves_the_other_applied() {
    let t = boot().await;
    let bill = approved(&t, "INV-2").await;
    pay(&t, &bill, 1, 11, 600_000).await;
    pay(&t, &bill, 2, 12, 450_000).await;

    deliver(&t, &reversed(2, vec![removed(12, &bill, 450_000)])).await;

    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::PartiallyPaid, 450_000)
    );
    let active: Vec<i64> = BillAllocation::of_bill(t.db(), bill.id)
        .await
        .unwrap()
        .iter()
        .filter(|row| row.is_active())
        .map(|row| row.amount)
        .collect();
    assert_eq!(active, [600_000]);
    assert_eq!(t.bill_reopened.all().len(), 1);
}

#[renox::test]
async fn the_same_event_twice_changes_nothing() {
    let t = boot().await;
    t.accountant("Zaid").await;
    let bill = approved(&t, "INV-3").await;
    pay(&t, &bill, 1, 11, GROSS).await;
    let event = reversed(1, vec![removed(11, &bill, GROSS)]);

    deliver(&t, &event).await;
    deliver(&t, &event).await;

    assert_eq!(
        BillUnallocation::query()
            .where_(BillUnallocation::BILL_ID.eq(bill.id))
            .get(t.db())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(t.notices.of_type(REOPENED).len(), 1);
    assert_eq!(t.bill_reopened.all().len(), 1);
    assert_eq!(reload(&t, &bill).await.balance, GROSS);
}

#[renox::test]
async fn an_event_without_bill_allocations_changes_nothing() {
    let t = boot().await;
    let bill = approved(&t, "INV-4").await;
    pay(&t, &bill, 1, 11, GROSS).await;
    let invoice = RemovedAllocation {
        allocation: 12,
        target: Subject::new(INVOICE_SUBJECT_TYPE, 99),
        amount: 5_000,
    };

    deliver(&t, &reversed(5, vec![invoice])).await;

    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Paid);
    assert!(t.bill_reopened.all().is_empty());
    assert!(t.notices.of_type(REOPENED).is_empty());
}

#[renox::test]
async fn an_amount_above_what_is_applied_is_refused_with_a_task_and_a_notice() {
    let t = boot().await;
    let zaid = t.accountant("Zaid").await;
    let bill = approved(&t, "INV-5").await;
    pay(&t, &bill, 1, 11, 400_000).await;
    let event = reversed(1, vec![removed(11, &bill, 500_000)]);

    deliver(&t, &event).await;
    // Delivered again: still one task and one notice.
    deliver(&t, &event).await;

    let after = reload(&t, &bill).await;
    assert_eq!(
        (after.status, after.balance),
        (SupplierBillStatus::PartiallyPaid, GROSS - 400_000)
    );
    assert!(t.bill_reopened.all().is_empty());
    let log = BillUnallocation::by_key(t.db(), "payments.allocation:11")
        .await
        .unwrap()
        .expect("logged");
    assert_eq!(log.outcome, wl_invoicing::UnallocationOutcome::Failed);

    let open = tasks(&t, &bill).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, TaskKind::BillReversalRefused);
    assert_eq!(open[0].status, TaskStatus::Open);
    assert_eq!(open[0].invoice_id, None);
    assert_eq!(open[0].role, "accountant");
    assert!(
        open[0]
            .message
            .starts_with("Reversal of AED 5,000.00 refused: ")
    );
    assert!(
        open[0].message.contains("only AED 4,000.00"),
        "{}",
        open[0].message
    );

    let notices = t.notices.of_type(REVERSAL_TASK);
    assert_eq!(notices.len(), 1);
    assert_eq!(users_of(&notices[0]), [zaid.id]);
    assert_eq!(notices[0].subject, bill.subject());
}

#[renox::test]
async fn a_bill_that_is_not_paid_is_refused_and_becomes_a_task() {
    let t = boot().await;
    let bill = approved(&t, "INV-6").await;
    // Not paid at all: approved is not a from-state of `unallocate`.
    deliver(&t, &reversed(1, vec![removed(11, &bill, 1_000)])).await;

    assert_eq!(reload(&t, &bill).await.status, SupplierBillStatus::Approved);
    assert_eq!(tasks(&t, &bill).await.len(), 1);
    assert!(t.bill_reopened.all().is_empty());
}

#[renox::test]
async fn the_bill_page_and_the_tasks_list_show_the_reversal() {
    let t = boot().await;
    let viewer = t
        .user("Vera", &[perm::SUPPLIER_BILLS_VIEW, perm::RECEIVABLES_VIEW])
        .await;
    let bill = approved(&t, "INV-7").await;
    pay(&t, &bill, 1, 11, GROSS).await;
    deliver(&t, &reversed(1, vec![removed(11, &bill, GROSS)])).await;
    // A refused reversal, for the tasks list.
    deliver(&t, &reversed(2, vec![removed(12, &bill, 777_700)])).await;

    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_ok()
        .assert_see("payments.payment:1")
        .assert_see("Removed")
        .assert_see("payment reversed")
        .assert_see("Reopened")
        .assert_see("bank return")
        .assert_see("Open tasks")
        .assert_see("Reversal of AED 7,777.00 refused");
    t.app
        .get("/invoices/tasks")
        .await
        .assert_ok()
        .assert_see("INV-7")
        .assert_see("Reversal of AED 7,777.00 refused");
    let task = tasks(&t, &bill).await.remove(0);
    t.app
        .get(&format!("/invoices/tasks/{}", task.id))
        .await
        .assert_redirect(&format!("/supplier-bills/{}", bill.id));
}
