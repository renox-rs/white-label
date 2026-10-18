//! A recovery whose payment is reversed, end to end through the real
//! `PaymentReversed` event (#70 AC5 and AC8): the invoice goes back to
//! `written_off`, the VAT side is undone, and the document is untouched.
//! The finer cases are in `unallocate_recovery.rs`.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_invoicing::vat_summary::{VatSummary, VatSummaryStatus};
use wl_invoicing::{
    AdjustmentKind, AdjustmentStatus, CreditNote, Invoice, Recovery, UnallocationOutcome,
    VatAdjustment, bad_debt_relief,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{INVOICE_SUBJECT_TYPE, InvoiceId, InvoiceStatus, ReopenReason};
use wl_kernel::payments::{
    PaymentId, PaymentReversed, PaymentSource, RemovedAllocation, ReversalReason,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a date")
}

fn reversed(invoice: &Invoice, amount: i64) -> PaymentReversed {
    PaymentReversed {
        payment: PaymentId(7),
        party: PartyId(1),
        legal_entity_id: 1,
        reason: ReversalReason::BankReturn,
        source: PaymentSource::Manual,
        statement_line: None,
        removed: vec![RemovedAllocation {
            allocation: 41,
            target: Subject::new(INVOICE_SUBJECT_TYPE, invoice.id),
            amount,
        }],
        prepaid: None,
        reversed_on: day(2026, 12, 10),
    }
}

/// The issued document as it is stored, and the write-off record.
async fn document(t: &Booted, invoice: &Invoice) -> Value {
    let invoice = t.reload(invoice).await;
    let lines = wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap();
    json!({
        "number": invoice.number,
        "lines": lines,
        "net_total": invoice.net_total,
        "vat_total": invoice.vat_total,
        "gross_total": invoice.gross_total,
        "party_snapshot": invoice.party_snapshot,
        "issuer_snapshot": invoice.issuer_snapshot,
        "write_off_amount": invoice.write_off_amount,
        "write_off_date": invoice.write_off_date,
        "write_off_reason_code": invoice.write_off_reason_code,
        "write_off_notice_on": invoice.write_off_notice_on,
    })
}

async fn raised(t: &Booted, invoice: &Invoice) -> Vec<VatAdjustment> {
    VatAdjustment::query()
        .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRelief))
        .where_(VatAdjustment::SOURCE_ID.eq(invoice.id))
        .where_(VatAdjustment::REVERSES_RECOVERY_ID.is_not_null())
        .get(t.db())
        .await
        .unwrap()
}

/// Includes the recovery adjustment that the recovery of `invoice` made.
async fn include_recovery(t: &Booted, invoice: &Invoice, summary: &VatSummary, by: &User) {
    let adjustment = VatAdjustment::query()
        .where_(VatAdjustment::KIND.eq(AdjustmentKind::BadDebtRecovery))
        .where_(VatAdjustment::SOURCE_ID.eq(invoice.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("the recovery adjustment");
    t.post_decision(summary, &adjustment, "include", by, "ok")
        .await
        .assert_ok();
}

/// AC5 and AC8: written off for AED 1,050.00, the relief included, then
/// recovered in full; the recovery payment is reversed. The invoice is
/// `written_off` with nothing recovered, a new relief of AED 50.00 is
/// proposed on the open summary, the accountant is told and the event is
/// emitted. With no open summary the relief waits as `pending` and the next
/// `FIN.RUL.14` run proposes it.
#[renox::test]
async fn a_reversed_recovery_payment_writes_the_invoice_off_again() {
    let t = boot().await;
    let accountant = t.accountant("Zaid").await;
    let pgp = t.entity("PGP").await;
    let supply = day(2026, 3, 15);
    let first_day = day(2026, 9, 16);
    t.summary(&pgp, supply, VatSummaryStatus::Locked).await;
    let q3 = t.summary(&pgp, first_day, VatSummaryStatus::Open).await;
    let now = t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    let (invoice, relief) = t.written_off(None, supply, Some(day(2026, 7, 1))).await;
    bad_debt_relief::run(t.state(), first_day).await.unwrap();
    let relief = t.adjustment(relief.id).await;
    t.post_decision(&q3, &relief, "include", &accountant, "Meets the rules")
        .await
        .assert_ok();
    let before = document(&t, &invoice).await;

    t.pay(&invoice, 7, 105_000).await;
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Paid);
    include_recovery(&t, &invoice, &now, &accountant).await;

    let event = reversed(&invoice, 105_000);
    t.state().emit(event.clone()).await.unwrap();

    let after = t.reload(&invoice).await;
    assert_eq!(after.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        (after.write_off_recovered_amount, after.balance),
        (0, 105_000)
    );
    assert_eq!(document(&t, &invoice).await, before);
    assert_eq!(CreditNote::query().count(t.db()).await.unwrap(), 0);

    let new_relief = raised(&t, &invoice).await;
    assert_eq!(new_relief.len(), 1);
    assert_eq!(new_relief[0].amount, 5_000);
    assert_eq!(new_relief[0].status, AdjustmentStatus::Proposed);
    assert_eq!(new_relief[0].summary_id, Some(now.id));
    assert!(Recovery::of_invoice(t.db(), invoice.id).await.unwrap()[0].is_reversed());

    let events = t.invoice_reopened.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, ReopenReason::RecoveryReversed);
    assert_eq!(events[0].new_status, InvoiceStatus::WrittenOff);
    assert_eq!(events[0].invoice, InvoiceId(invoice.id));
    let told = t.notices.of_type("invoicing.recovery_reversed");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].data["amount"], "AED 1,050.00");

    // The same delivery again changes nothing.
    t.state().emit(event).await.unwrap();
    assert_eq!(t.invoice_reopened.all().len(), 1);
    assert_eq!(raised(&t, &invoice).await.len(), 1);
    let log = wl_invoicing::Unallocation::query()
        .where_(wl_invoicing::Unallocation::INVOICE_ID.eq(invoice.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, UnallocationOutcome::Done);
}

/// AC5: with no open summary the new relief is `pending`; the next
/// `FIN.RUL.14` run after one exists proposes it.
#[renox::test]
async fn without_an_open_summary_the_new_relief_waits_to_be_proposed() {
    let t = boot().await;
    let accountant = t.accountant("Zaid").await;
    let pgp = t.entity("PGP").await;
    let supply = day(2026, 3, 15);
    let first_day = day(2026, 9, 16);
    t.summary(&pgp, supply, VatSummaryStatus::Locked).await;
    let q3 = t.summary(&pgp, first_day, VatSummaryStatus::Open).await;
    let now = t.summary(&pgp, today(), VatSummaryStatus::Open).await;
    let (invoice, relief) = t.written_off(None, supply, Some(day(2026, 7, 1))).await;
    bad_debt_relief::run(t.state(), first_day).await.unwrap();
    let relief = t.adjustment(relief.id).await;
    t.post_decision(&q3, &relief, "include", &accountant, "Meets the rules")
        .await
        .assert_ok();
    t.pay(&invoice, 7, 105_000).await;
    include_recovery(&t, &invoice, &now, &accountant).await;
    // Today's summary is closed for good.
    VatSummary::query()
        .where_(VatSummary::ID.eq(now.id))
        .update(t.db(), &[("status", &"locked")])
        .await
        .unwrap();

    t.state().emit(reversed(&invoice, 105_000)).await.unwrap();
    let waiting = raised(&t, &invoice).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].status, AdjustmentStatus::Pending);
    assert_eq!(waiting[0].summary_id, None);

    VatSummary::query()
        .where_(VatSummary::ID.eq(now.id))
        .update(t.db(), &[("status", &"open")])
        .await
        .unwrap();
    let ran = bad_debt_relief::run(t.state(), today()).await.unwrap();
    assert_eq!(ran.proposed, 1);
    let proposed = t.adjustment(waiting[0].id).await;
    assert_eq!(proposed.status, AdjustmentStatus::Proposed);
    assert_eq!(proposed.summary_id, Some(now.id));
}
