//! Reopening an invoice when a payment is reversed (wl-invoicing §4
//! `unallocate`, #70): the acceptance checks AC1-AC4, AC7, AC8 and AC9. The
//! real `PaymentReversed` event is emitted as wl-payments does after a
//! reversal; the notices go to a recording `Notifier`.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_invoicing::reminders::{self, AFTER_DUE};
use wl_invoicing::task::{InvoicingTask, REVERSAL_TASK, TaskKind, TaskStatus};
use wl_invoicing::vat_summary::VatSummaryStatus;
use wl_invoicing::{
    Allocation, CreditNote, Invoice, ReminderLog, Unallocation, UnallocationOutcome,
};
use wl_kernel::approvals;
use wl_kernel::invoicing::{
    INVOICE_SUBJECT_TYPE, InvoiceId, InvoiceStatus, ReopenReason, Unallocate, VatCategory,
};
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

async fn allocations(t: &Booted, invoice: &Invoice) -> Vec<Allocation> {
    Allocation::query()
        .where_(Allocation::INVOICE_ID.eq(invoice.id))
        .order_by(Allocation::ID)
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

/// The issued document as it is stored: number, lines, totals, VAT and the
/// snapshots (A11: there is no stored PDF yet).
async fn document(t: &Booted, invoice: &Invoice) -> Value {
    let invoice = t.reload(invoice).await;
    let lines = wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap();
    assert!(invoice.number.is_some() && !lines.is_empty());
    json!({
        "number": invoice.number,
        "lines": lines,
        "net_total": invoice.net_total,
        "vat_total": invoice.vat_total,
        "gross_total": invoice.gross_total,
        "date_of_supply": invoice.date_of_supply,
        "due_date": invoice.due_date,
        "party_snapshot": invoice.party_snapshot,
        "issuer_snapshot": invoice.issuer_snapshot,
    })
}

/// AC1, AC7 and AC8: a `paid` invoice of AED 1,050.00 whose only payment is
/// reversed with `bank_return` is `issued` again with its allocation marked
/// removed; the owner and each `receivables` user get one notice, the event
/// is emitted once, and the same delivery twice changes nothing more.
#[renox::test]
async fn a_reversed_payment_reopens_the_invoice_once() {
    let t = boot().await;
    let owner = t.user_a().await;
    let clerk = t.user_b().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &owner).await;
    t.pay(&invoice, 1, GROSS).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);
    let before = document(&t, &invoice).await;
    let event = reversed(1, vec![removed(11, &invoice, GROSS)]);

    deliver(&t, &event).await;

    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance, after.allocated_total),
        (InvoiceStatus::Issued, GROSS, 0)
    );
    let rows = allocations(&t, &invoice).await;
    assert_eq!(rows.len(), 1, "the allocation is kept");
    assert!(!rows[0].is_active());
    assert_eq!(
        rows[0].removed_key.as_deref(),
        Some("payments.allocation:11")
    );

    let notices = t.notices.of_type(REOPENED);
    let mut told: Vec<i64> = notices.iter().flat_map(users_of).collect();
    told.sort_unstable();
    // The owner is a receivables user too, and is told once.
    let mut expected = vec![owner.id, clerk.id];
    expected.sort_unstable();
    assert_eq!(told, expected, "each of them is told once");
    assert!(notices.iter().all(|n| {
        n.data["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("bank return"))
    }));

    let events = t.invoice_reopened.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, ReopenReason::PaymentReversed);
    assert_eq!(events[0].new_status, InvoiceStatus::Issued);
    assert_eq!(events[0].amount, GROSS);
    assert_eq!(events[0].balance, GROSS);
    assert_eq!(events[0].reversal_reason.as_deref(), Some("bank_return"));

    // AC8: the document is byte-identical, and nothing was credited.
    assert_eq!(document(&t, &invoice).await, before);
    assert_eq!(CreditNote::query().count(t.db()).await.unwrap(), 0);

    // AC7: the same event again.
    deliver(&t, &event).await;
    assert_eq!(logs(&t, &invoice).await.len(), 1);
    assert_eq!(allocations(&t, &invoice).await.len(), 1);
    assert_eq!(t.notices.of_type(REOPENED).len(), 2);
    assert_eq!(t.invoice_reopened.all().len(), 1);
    assert!(tasks(&t, &invoice).await.is_empty());
    assert_eq!(t.reload(&invoice).await.balance, GROSS);

    // The kernel call with the same key is skipped as well.
    let again = wl_kernel::invoicing::invoicer(t.state())
        .unallocate(
            t.state(),
            Unallocate::new(
                InvoiceId(invoice.id),
                Subject::new("payments.payment", 1),
                GROSS,
                "payments.allocation:11",
                ReopenReason::PaymentReversed,
            ),
        )
        .await
        .unwrap();
    assert!(again.skipped);
}

/// AC2: payments of AED 600.00 and AED 450.00, only the second reversed:
/// `partially_paid` with balance 450.00 and the first allocation active.
#[renox::test]
async fn reversing_one_of_two_payments_leaves_the_other_allocated() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&invoice, 1, 60_000).await;
    t.pay(&invoice, 2, 45_000).await;

    deliver(&t, &reversed(2, vec![removed(12, &invoice, 45_000)])).await;

    let after = t.reload(&invoice).await;
    assert_eq!(
        (after.status, after.balance, after.allocated_total),
        (InvoiceStatus::PartiallyPaid, 45_000, 60_000)
    );
    let rows = allocations(&t, &invoice).await;
    assert!(rows[0].is_active() && rows[0].amount == 60_000);
    assert!(!rows[1].is_active());
    assert_eq!(
        t.invoice_reopened.all()[0].new_status,
        InvoiceStatus::PartiallyPaid
    );
}

/// The steps logged for `invoice`, oldest first.
async fn steps(t: &Booted, invoice: &Invoice) -> Vec<i64> {
    ReminderLog::query()
        .where_(ReminderLog::INVOICE_ID.eq(invoice.id))
        .order_by(ReminderLog::STEP)
        .get(t.db())
        .await
        .unwrap()
        .iter()
        .map(|log| log.step)
        .collect()
}

/// AC3: an invoice due 9 days ago, paid and reversed, is `overdue`; the
/// next run sends the 7-day step once, and the 14-day step goes out on its
/// day. A disputed invoice gets nothing.
#[renox::test]
async fn a_reopened_overdue_invoice_is_reminded_again() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.issued(&[LINE], Some(t.day(-9)), &a).await;
    t.pay(&invoice, 1, GROSS).await;
    let disputed = t.issued(&[LINE], Some(t.day(-9)), &a).await;
    t.pay(&disputed, 2, GROSS).await;
    Invoice::query()
        .where_(Invoice::ID.eq(disputed.id))
        .update(t.db(), &[("disputed", &true)])
        .await
        .unwrap();
    // Nothing is due while both are paid.
    assert_eq!(reminders::run(t.state(), t.day(0)).await.unwrap(), 0);

    deliver(&t, &reversed(1, vec![removed(11, &invoice, GROSS)])).await;
    deliver(&t, &reversed(2, vec![removed(12, &disputed, GROSS)])).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Overdue);
    assert_eq!(t.reload(&disputed).await.status, InvoiceStatus::Overdue);
    assert_eq!(
        t.invoice_reopened.all()[0].new_status,
        InvoiceStatus::Overdue
    );

    assert_eq!(reminders::run(t.state(), t.day(0)).await.unwrap(), 1);
    let sent = t.notices.of_type(AFTER_DUE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, invoice.subject());
    assert_eq!(sent[0].data["days"], 7);
    assert_eq!(steps(&t, &invoice).await, [7]);

    // Two days later nothing new is due; five days later the 14-day step.
    assert_eq!(reminders::run(t.state(), t.day(2)).await.unwrap(), 0);
    assert_eq!(reminders::run(t.state(), t.day(5)).await.unwrap(), 1);
    assert_eq!(steps(&t, &invoice).await, [7, 14]);
    assert!(steps(&t, &disputed).await.is_empty());
}

/// Writes `invoice` off through the approval of a second user.
async fn write_off(t: &Booted, invoice: &Invoice) {
    let clerk = t.clerk("Wendy").await;
    let approver = t.clerk("Wade").await;
    t.post_write_off(invoice, &clerk, "insolvent", "")
        .await
        .assert_ok();
    let request = t.pending(invoice).await.remove(0);
    approvals::approve(t.state(), request.id, &approver, "")
        .await
        .unwrap()
        .expect("the approval runs");
}

/// AC4: AED 200.00 is paid, the balance of AED 850.00 is written off and
/// recovered in full, so the invoice is `paid`. The payment from before the
/// write-off is reversed: refused, logged, one task, the accountant told.
#[renox::test]
async fn a_payment_from_before_a_write_off_is_refused_after_a_recovery() {
    let t = boot().await;
    let accountant = t.accountant("Zaid").await;
    let a = t.user_a().await;
    let invoice = t.overdue_invoice(&a).await;
    t.pay(&invoice, 1, 20_000).await;
    write_off(&t, &invoice).await;
    let written = t.reload(&invoice).await;
    assert_eq!(written.status, InvoiceStatus::WrittenOff);
    assert_eq!(written.balance, 85_000);
    t.pay(&invoice, 7, 85_000).await;
    let before = t.reload(&invoice).await;
    assert_eq!(before.status, InvoiceStatus::Paid);
    assert_eq!(before.write_off_recovered_amount, 85_000);

    let event = reversed(1, vec![removed(11, &invoice, 20_000)]);
    deliver(&t, &event).await;
    deliver(&t, &event).await;

    let after = t.reload(&invoice).await;
    assert_eq!(
        (
            after.status,
            after.balance,
            after.allocated_total,
            after.write_off_amount,
            after.write_off_recovered_amount,
        ),
        (
            before.status,
            before.balance,
            before.allocated_total,
            before.write_off_amount,
            before.write_off_recovered_amount,
        )
    );
    let log = logs(&t, &invoice).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, UnallocationOutcome::Failed);
    let open = tasks(&t, &invoice).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, TaskKind::ReversalAfterWriteOff);
    assert_eq!(open[0].status, TaskStatus::Open);
    assert_eq!(open[0].role, "accountant");
    let notices = t.notices.of_type(REVERSAL_TASK);
    assert_eq!(notices.len(), 1);
    assert_eq!(users_of(&notices[0]), [accountant.id]);
    assert!(t.invoice_reopened.all().is_empty());
    assert!(
        allocations(&t, &invoice)
            .await
            .iter()
            .all(Allocation::is_active)
    );
}

/// AC9: more than is allocated from the payment is refused with a
/// `reversal_refused` task and an unchanged invoice; `unallocate*` is never
/// offered and no route runs it; it works while the VAT summary is locked.
#[renox::test]
async fn a_refusal_leaves_a_task_and_a_locked_vat_period_does_not_matter() {
    let t = boot().await;
    let admin = t.admin_a().await;
    let a = admin.clone();
    let invoice = t.issued(&[LINE], Some(t.day(20)), &a).await;
    t.pay(&invoice, 1, 60_000).await;

    deliver(&t, &reversed(1, vec![removed(11, &invoice, 70_000)])).await;
    let same = t.reload(&invoice).await;
    assert_eq!(
        (same.status, same.balance, same.allocated_total),
        (InvoiceStatus::PartiallyPaid, 45_000, 60_000)
    );
    let log = logs(&t, &invoice).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, UnallocationOutcome::Failed);
    let open = tasks(&t, &invoice).await;
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].kind, TaskKind::ReversalRefused);
    assert!(
        open[0].message.contains("AED 600.00"),
        "{}",
        open[0].message
    );

    // Never offered, and no route.
    for by in [&a, &admin] {
        let offered = t.available(&same, by).await;
        assert!(!offered.contains("unallocate"), "{offered}");
    }
    t.app.acting_as(&admin);
    for path in ["unallocate", "reopen", "unallocate_partially"] {
        let response = t
            .app
            .post(&format!("/invoices/{}/{path}", invoice.id), &[])
            .await;
        assert!(
            response.status.is_client_error(),
            "{path}: {}",
            response.status
        );
    }
    assert_eq!(t.reload(&invoice).await.balance, 45_000);

    // The supply period's VAT summary is locked.
    let pgp = t.entity("PGP").await;
    let supply = NaiveDate::from_ymd_opt(2026, 3, 15).unwrap();
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(t.db(), &[("date_of_supply", &Some(supply))])
        .await
        .unwrap();
    let before = document(&t, &invoice).await;
    let summary = t.summary(&pgp, supply, VatSummaryStatus::Locked).await;
    assert_eq!(summary.status, VatSummaryStatus::Locked);
    deliver(&t, &reversed(1, vec![removed(12, &invoice, 60_000)])).await;
    let reopened = t.reload(&invoice).await;
    assert_eq!(
        (reopened.status, reopened.balance),
        (InvoiceStatus::Issued, GROSS)
    );
    assert_eq!(document(&t, &invoice).await, before);
    assert_eq!(
        t.summary_now(summary.id).await.status,
        VatSummaryStatus::Locked
    );
}
