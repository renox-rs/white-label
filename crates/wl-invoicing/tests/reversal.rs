//! The consumer of `PaymentReversed` (wl-invoicing §4 `unallocate`, #70 part
//! 3): one `unallocate` per removed invoice allocation, the notice to the
//! owner and the `receivables` role, the follow-up task of a refusal and a
//! repeated delivery. The event is emitted as wl-payments does after a
//! reversal.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::reversal;
use wl_invoicing::task::{self, InvoicingTask, REVERSAL_TASK, TaskKind, TaskStatus};
use wl_invoicing::{Invoice, Unallocation, UnallocationOutcome};
use wl_kernel::invoicing::{INVOICE_SUBJECT_TYPE, InvoiceStatus, VatCategory};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{
    PaymentId, PaymentReversed, PaymentSource, RemovedAllocation, ReversalReason,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

/// AED 1,000.00 plus 5% VAT.
const LINE: (i64, VatCategory) = (100_000, VatCategory::Standard);
const GROSS: i64 = 105_000;
const REOPENED: &str = "invoicing.invoice_reopened";

fn reversed(payment: i64, removed: Vec<RemovedAllocation>) -> PaymentReversed {
    PaymentReversed {
        payment: PaymentId(payment),
        party: PartyId(1),
        legal_entity_id: 1,
        reason: ReversalReason::BankReturn,
        source: PaymentSource::Manual,
        statement_line: None,
        removed,
        prepaid: None,
        reversed_on: NaiveDate::from_ymd_opt(2026, 12, 10).unwrap(),
    }
}

fn removed(allocation: i64, invoice: &Invoice, amount: i64) -> RemovedAllocation {
    RemovedAllocation {
        allocation,
        target: Subject::new(INVOICE_SUBJECT_TYPE, invoice.id),
        amount,
    }
}

async fn deliver(t: &Booted, event: &PaymentReversed) {
    t.state().emit(event.clone()).await.unwrap();
}

async fn tasks(t: &Booted, invoice: &Invoice) -> Vec<InvoicingTask> {
    InvoicingTask::query()
        .where_(InvoicingTask::INVOICE_ID.eq(invoice.id))
        .order_by(InvoicingTask::ID)
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
async fn a_reversed_payment_reopens_its_invoice_and_tells_the_owner_once() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&invoice, 1, GROSS).await;

    deliver(&t, &reversed(1, vec![removed(11, &invoice, GROSS)])).await;

    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance),
        (InvoiceStatus::Issued, GROSS)
    );
    let log = logs(&t, &invoice).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].key, "payments.allocation:11");
    assert_eq!(log[0].reversal_reason.as_deref(), Some("bank_return"));
    assert!(tasks(&t, &invoice).await.is_empty());

    let notices = t.notices.of_type(REOPENED);
    assert!(!notices.is_empty());
    let mut told: Vec<i64> = notices.iter().flat_map(users_of).collect();
    told.sort_unstable();
    let before = told.len();
    told.dedup();
    assert_eq!(before, told.len(), "nobody is told twice");
    assert!(told.contains(&a.id));
    assert!(told.contains(&b.id));
    let text = notices[0].data["reason"].as_str().unwrap();
    assert!(text.contains("bank return"), "{text}");
    assert_eq!(notices[0].data["new_status"], "Issued");
}

#[renox::test]
async fn the_same_event_twice_changes_nothing() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&invoice, 1, GROSS).await;
    let event = reversed(1, vec![removed(11, &invoice, GROSS)]);

    deliver(&t, &event).await;
    let notices = t.notices.of_type(REOPENED).len();
    deliver(&t, &event).await;

    assert_eq!(logs(&t, &invoice).await.len(), 1);
    assert_eq!(t.notices.of_type(REOPENED).len(), notices);
    assert_eq!(t.reload(&invoice).await.balance, GROSS);
}

#[renox::test]
async fn one_failing_allocation_does_not_stop_the_others() {
    let t = boot().await;
    let a = t.user_a().await;
    let first = t.issued(&[LINE], Some(t.day(20)), &a).await;
    let second = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&second, 1, 60_000).await;

    // The first invoice has nothing allocated from the payment.
    deliver(
        &t,
        &reversed(
            1,
            vec![removed(11, &first, 50_000), removed(12, &second, 60_000)],
        ),
    )
    .await;

    assert_eq!(t.reload(&second).await.status, InvoiceStatus::Issued);
    assert_eq!(
        logs(&t, &first).await[0].outcome,
        UnallocationOutcome::Failed
    );
    let open = tasks(&t, &first).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, TaskKind::ReversalRefused);
    assert_eq!(open[0].status, TaskStatus::Open);
    assert_eq!(open[0].role, "accountant");
    assert_eq!(open[0].amount, 50_000);
}

#[renox::test]
async fn a_payment_from_before_a_write_off_becomes_a_task_for_the_accountant() {
    let t = boot().await;
    let accountant = t.accountant("Amal").await;
    let (invoice, _relief) = t.written_off(None, t.day(-60), None).await;
    let before = t.reload(&invoice).await;
    assert_eq!(before.status, InvoiceStatus::WrittenOff);
    let event = reversed(1, vec![removed(11, &invoice, 5_000)]);

    deliver(&t, &event).await;
    deliver(&t, &event).await;

    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance, after.write_off_amount),
        (before.status, before.balance, before.write_off_amount)
    );
    let log = logs(&t, &invoice).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, UnallocationOutcome::Failed);
    let open = tasks(&t, &invoice).await;
    assert_eq!(open.len(), 1, "no second task");
    assert_eq!(open[0].kind, TaskKind::ReversalAfterWriteOff);
    assert_eq!(open[0].key, "payments.allocation:11");
    assert_eq!(task::open_of(t.db(), invoice.id).await.unwrap().len(), 1);

    let notices = t.notices.of_type(REVERSAL_TASK);
    assert_eq!(notices.len(), 1);
    assert_eq!(users_of(&notices[0]), [accountant.id]);
    assert!(t.notices.of_type(REOPENED).is_empty());
}

#[renox::test]
async fn allocations_to_other_targets_and_unknown_invoices_are_left_alone() {
    let t = boot().await;
    let other = RemovedAllocation {
        allocation: 5,
        target: Subject::new("property.deposit", 2),
        amount: 1_000,
    };
    let missing = RemovedAllocation {
        allocation: 6,
        target: Subject::new(INVOICE_SUBJECT_TYPE, 9_999),
        amount: 1_000,
    };
    reversal::handle(t.state(), &reversed(1, vec![other, missing]))
        .await
        .unwrap();
    let all = Unallocation::query().get(t.db()).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].outcome, UnallocationOutcome::Failed);
    assert!(InvoicingTask::query().get(t.db()).await.unwrap().is_empty());
}
