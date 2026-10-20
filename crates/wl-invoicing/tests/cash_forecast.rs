//! The cash-flow forecast (wl-invoicing §5.14, `FIN.CFL.01`): the acceptance
//! checks of #91. Today is Wednesday 2027-01-13; week 1 is Monday 2027-01-11
//! to Sunday 2027-01-17 unless a test says otherwise.

mod support;

use renox::chrono::{Datelike, NaiveDate, Weekday};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_invoicing::cash_balance::{self, EnterBalance, OpeningCashBalance};
use wl_invoicing::cash_forecast::{self, Forecast, LineKind, MemoKind, Warning};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::duplicates::BillException;
use wl_invoicing::supplier_bill::{POSSIBLE_DUPLICATE, SupplierBill, SupplierBillStatus};
use wl_invoicing::vat_summary::VatSummaryStatus;
use wl_invoicing::{Allocation, BillLine, Invoice, WlInvoicing, permissions as perm};
use wl_kernel::cash_forecast::{
    CashForecastSource, ForecastFuture, ForecastLine, register_cash_forecast_source,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::payments::Direction;
use wl_kernel::{LegalEntity, Subject};

use support::{BLOCKED_PARTY, BLUE_DUNES, Booted, boot, boot_with, open_cheque};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// A booted app whose clock reads Wednesday 2027-01-13.
async fn booted() -> Booted {
    let t = boot().await;
    t.travel_to("2027-01-13", 8, 0);
    t
}

/// The forecast of `entities` computed now.
async fn grid(t: &Booted, entities: &[&LegalEntity]) -> Forecast {
    let entities: Vec<LegalEntity> = entities.iter().map(|e| (*e).clone()).collect();
    let inputs = t
        .app
        .at_travelled_time(async {
            cash_forecast_load::load(t.state(), &t.config(), &entities, today()).await
        })
        .await
        .unwrap();
    cash_forecast::assemble(&inputs)
}

/// An issued invoice of `entity` with `balance` fils open, due `due`.
async fn open_invoice(
    t: &Booted,
    entity: &LegalEntity,
    number: &str,
    due: NaiveDate,
    balance: i64,
) -> Invoice {
    invoice_in_state(t, entity, number, due, balance, InvoiceStatus::Issued).await
}

/// An invoice of `entity` created in `status` as a fixture.
async fn invoice_in_state(
    t: &Booted,
    entity: &LegalEntity,
    number: &str,
    due: NaiveDate,
    balance: i64,
    status: InvoiceStatus,
) -> Invoice {
    let mut invoice = Invoice {
        number: Some(number.to_owned()),
        legal_entity_id: Some(entity.id),
        party_id: BLUE_DUNES.0,
        currency: "AED".to_owned(),
        issue_date: Some(day(1, 1)),
        due_date: Some(due),
        gross_total: balance,
        balance,
        status,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    invoice
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    invoice
}

/// A supplier bill of `entity` with `balance` fils open, due `due`.
async fn open_bill(
    t: &Booted,
    entity: &LegalEntity,
    number: &str,
    due: NaiveDate,
    balance: i64,
    status: SupplierBillStatus,
) -> SupplierBill {
    let mut bill = SupplierBill {
        party_id: BLUE_DUNES.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: day(1, 2),
        due_date: Some(due),
        currency: "AED".to_owned(),
        gross_total: balance,
        balance,
        status,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    bill
}

/// A cash-flow viewer with the role in every entity.
async fn viewer(t: &Booted) -> User {
    t.helper("Cashier", &[perm::CASHFLOW_VIEW, perm::CASHFLOW_MANAGE])
        .await
}

/// A statement balance of AED 50,000 on 2027-01-10 and an incoming transfer
/// of AED 5,000 recorded on 2027-01-12 make an opening balance of AED 55,000.
#[renox::test]
async fn the_opening_balance_is_the_statement_plus_the_payments_after_it() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 10));
    t.refunds
        .bank_payment(e1.id, "BA1", Direction::In, 500_000, day(1, 12), None);
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.opening_total(), 5_500_000);
    assert_eq!(forecast.openings.len(), 1);
    assert_eq!(forecast.openings[0].movements, 500_000);
}

/// A payment dated on the balance day is already in the balance, even when it
/// was recorded later: it is not added again.
#[renox::test]
async fn a_payment_dated_on_the_balance_day_is_not_added() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 10));
    t.refunds
        .bank_payment(e1.id, "BA1", Direction::In, 200_000, day(1, 10), None);
    t.refunds
        .bank_payment(e1.id, "BA1", Direction::In, 500_000, day(1, 12), None);
    t.refunds
        .bank_payment(e1.id, "BA1", Direction::Out, 100_000, day(1, 13), None);
    assert_eq!(grid(&t, &[&e1]).await.opening_total(), 5_400_000);
}

/// A manual balance of AED 60,000 dated 2027-01-11 replaces the statement
/// balance (it is later); on the same day the statement wins; a voided one is
/// never used.
#[renox::test]
async fn the_later_balance_wins_and_a_voided_one_is_never_used() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 10));
    t.refunds
        .bank_payment(e1.id, "BA1", Direction::In, 500_000, day(1, 12), None);
    let manual = t
        .app
        .at_travelled_time(cash_balance::enter(
            t.db(),
            None,
            &EnterBalance {
                legal_entity_id: e1.id,
                bank_account: "BA1".into(),
                amount: 6_000_000,
                as_of: day(1, 11),
                note: None,
            },
        ))
        .await
        .unwrap();
    assert_eq!(grid(&t, &[&e1]).await.opening_total(), 6_500_000);
    assert_eq!(
        grid(&t, &[&e1]).await.openings[0].balance.map(|b| b.as_of),
        Some(day(1, 11))
    );

    // Voided: back to the statement.
    t.app
        .at_travelled_time(cash_balance::void(
            t.db(),
            manual.id,
            None,
            "entered in error",
        ))
        .await
        .unwrap();
    assert_eq!(grid(&t, &[&e1]).await.opening_total(), 5_500_000);

    // On the statement's own day the statement wins.
    t.app
        .at_travelled_time(cash_balance::enter(
            t.db(),
            None,
            &EnterBalance {
                legal_entity_id: e1.id,
                bank_account: "BA1".into(),
                amount: 9_000_000,
                as_of: day(1, 10),
                note: Some("same day".into()),
            },
        ))
        .await
        .unwrap();
    assert_eq!(grid(&t, &[&e1]).await.opening_total(), 5_500_000);
}

/// A balance is never dated in the future, never edited, and is voided only
/// with a reason (and once).
#[renox::test]
async fn an_opening_cash_balance_is_checked_and_voided_with_a_reason() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let balance = |as_of: NaiveDate, account: &str| EnterBalance {
        legal_entity_id: e1.id,
        bank_account: account.into(),
        amount: 100,
        as_of,
        note: None,
    };
    assert!(
        t.app
            .at_travelled_time(cash_balance::enter(
                t.db(),
                None,
                &balance(day(1, 14), "BA1")
            ))
            .await
            .is_err()
    );
    assert!(
        t.app
            .at_travelled_time(cash_balance::enter(t.db(), None, &balance(day(1, 13), " ")))
            .await
            .is_err()
    );
    let kept = t
        .app
        .at_travelled_time(cash_balance::enter(
            t.db(),
            None,
            &balance(day(1, 13), "BA1"),
        ))
        .await
        .unwrap();
    assert!(
        t.app
            .at_travelled_time(cash_balance::void(t.db(), kept.id, None, "  "))
            .await
            .is_err()
    );
    t.app
        .at_travelled_time(cash_balance::void(t.db(), kept.id, None, "wrong bank"))
        .await
        .unwrap();
    assert!(
        t.app
            .at_travelled_time(cash_balance::void(t.db(), kept.id, None, "again"))
            .await
            .is_err()
    );
    // The voided balance stays in the history.
    let history = cash_balance::history(t.db(), e1.id).await.unwrap();
    assert_eq!(history.len(), 1);
    assert!(history[0].is_voided());
    assert_eq!(history[0].void_reason.as_deref(), Some("wrong bank"));
}

/// Receipts and payments fall in the week of their date; a past date counts in
/// week 1.
#[renox::test]
async fn invoices_and_bills_fall_in_the_week_of_their_due_date() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    open_invoice(&t, &e1, "INV-1", day(1, 25), 1_050_000).await;
    open_invoice(&t, &e1, "INV-2", day(1, 2), 300_000).await;
    open_bill(
        &t,
        &e1,
        "SB-5",
        day(2, 10),
        1_200_000,
        SupplierBillStatus::Approved,
    )
    .await;
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.weeks.len(), 13);
    assert_eq!(
        (forecast.weeks[0].start, forecast.weeks[0].end),
        (day(1, 11), day(1, 17))
    );
    // INV-1 is in week 3 (2027-01-25 to 2027-01-31), INV-2 in week 1.
    assert_eq!(forecast.receipts(2), 1_050_000);
    assert_eq!(forecast.receipts(0), 300_000);
    // SB-5 is a payment in week 5.
    assert_eq!(forecast.payments(4), 1_200_000);
    assert_eq!(forecast.balance(4), 1_050_000 + 300_000 - 1_200_000);
}

/// A received cheque dated 2027-02-01 that pays an invoice counts in week 4
/// only, by the cheque date; the invoice does not count again.
#[renox::test]
async fn a_post_dated_cheque_counts_by_its_date_and_not_with_its_invoice() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let rent = open_invoice(&t, &e1, "INV-9", day(1, 20), 800_000).await;
    t.forecast.open_cheque(
        e1.id,
        open_cheque(
            7,
            "CHQ-7",
            Direction::In,
            day(2, 1),
            800_000,
            &[(rent.subject(), 800_000)],
        ),
    );
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.receipts(3), 800_000);
    assert_eq!(forecast.receipts(1), 0);
    assert_eq!(forecast.lines.len(), 1);
    assert_eq!(forecast.lines[0].kind, LineKind::ChequeIn);
}

/// Of an invoice with AED 2,000 disputed of AED 6,000 open, AED 4,000 counts
/// and AED 2,000 is on the memo line.
#[renox::test]
async fn the_disputed_part_is_a_memo_line() {
    use wl_invoicing::dispute::Dispute;
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let mut invoice = open_invoice(&t, &e1, "INV-3", day(1, 20), 600_000).await;
    let mut dispute = Dispute {
        invoice_id: invoice.id,
        reason: "wrong rate".into(),
        amount: 200_000,
        raised_at: renox::db::now(),
        ..Default::default()
    };
    dispute.insert(t.db()).await.unwrap();
    invoice.disputed = true;
    invoice.dispute_id = Some(dispute.id);
    invoice.save(t.db()).await.unwrap();
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.receipts(1), 400_000);
    assert_eq!(forecast.memo(MemoKind::Disputed, 1), 200_000);

    // The setting that counts disputed amounts.
    let counting = boot_with(WlInvoicing::new().cash_disputed_counted(true)).await;
    counting.travel_to("2027-01-13", 8, 0);
    let e = counting.entity("PGP").await;
    let mut other = open_invoice(&counting, &e, "INV-3", day(1, 20), 600_000).await;
    let mut d = Dispute {
        invoice_id: other.id,
        reason: "x".into(),
        amount: 200_000,
        raised_at: renox::db::now(),
        ..Default::default()
    };
    d.insert(counting.db()).await.unwrap();
    other.disputed = true;
    other.dispute_id = Some(d.id);
    other.save(counting.db()).await.unwrap();
    assert_eq!(grid(&counting, &[&e]).await.receipts(1), 600_000);
}

/// A bank account the client names but that has no balance shows a warning and
/// counts as zero.
#[renox::test]
async fn an_account_without_a_balance_warns_and_counts_zero() {
    let t = boot_with(WlInvoicing::new().cash_account("PGP", "BA2")).await;
    t.travel_to("2027-01-13", 8, 0);
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 10));
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.opening_total(), 5_000_000);
    assert!(forecast.warnings.contains(&Warning::NoBalance {
        legal_entity_id: e1.id,
        account: "BA2".into(),
    }));
}

/// A balance dated 10 days ago is used and warned as stale.
#[renox::test]
async fn a_stale_balance_is_used_with_a_warning() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 3));
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.opening_total(), 5_000_000);
    assert!(
        forecast
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::Stale { as_of, .. } if *as_of == day(1, 3)))
    );
}

/// Q4 2026 VAT summary `prepared` with a net payable of AED 14,000 (due
/// 2027-01-28) is a payment in week 3; without a summary it is an estimate.
#[renox::test]
async fn the_vat_payable_is_a_payment_at_the_return_due_date() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let mut invoice = invoice_in_state(&t, &e1, "INV-V", day(1, 20), 0, InvoiceStatus::Paid).await;
    invoice.date_of_supply = Some(NaiveDate::from_ymd_opt(2026, 12, 15).unwrap());
    invoice.issue_date = invoice.date_of_supply;
    invoice.vat_total = 1_400_000;
    invoice.vat_total_aed = Some(1_400_000);
    invoice.save(t.db()).await.unwrap();

    let estimate = grid(&t, &[&e1]).await;
    assert_eq!(estimate.payments(2), 1_400_000);
    assert!(
        estimate
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Vat && l.estimate)
    );

    t.summary(
        &e1,
        NaiveDate::from_ymd_opt(2026, 12, 15).unwrap(),
        VatSummaryStatus::Prepared,
    )
    .await;
    let prepared = grid(&t, &[&e1]).await;
    assert_eq!(prepared.payments(2), 1_400_000);
    assert!(
        prepared
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Vat && !l.estimate)
    );
}

/// A payment that names the VAT summary as its reference removes the line.
#[renox::test]
async fn a_vat_payment_naming_the_summary_removes_the_line() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let mut invoice = invoice_in_state(&t, &e1, "INV-V", day(1, 20), 0, InvoiceStatus::Paid).await;
    invoice.date_of_supply = Some(NaiveDate::from_ymd_opt(2026, 12, 15).unwrap());
    invoice.vat_total_aed = Some(1_400_000);
    invoice.save(t.db()).await.unwrap();
    let summary = t
        .summary(
            &e1,
            NaiveDate::from_ymd_opt(2026, 12, 15).unwrap(),
            VatSummaryStatus::Prepared,
        )
        .await;
    t.forecast.statement(e1.id, "BA1", 5_000_000, day(1, 10));
    t.refunds.bank_payment(
        e1.id,
        "BA1",
        Direction::Out,
        1_400_000,
        day(1, 12),
        Some(&format!("invoicing.vat_summary:{}", summary.id)),
    );
    let forecast = grid(&t, &[&e1]).await;
    assert!(forecast.lines.iter().all(|l| l.kind != LineKind::Vat));
    // The payment is in the opening balance instead.
    assert_eq!(forecast.opening_total(), 5_000_000 - 1_400_000);
}

/// A received cheque that bounced against an overdue invoice: its AED 7,000 is
/// on the memo line and not in week 1.
#[renox::test]
async fn a_bounced_cheques_invoice_is_a_memo_line() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let invoice = open_invoice(&t, &e1, "INV-5", day(1, 1), 700_000).await;
    t.forecast.bounced_cheque(
        e1.id,
        open_cheque(
            8,
            "CHQ-8",
            Direction::In,
            day(1, 1),
            700_000,
            &[(invoice.subject(), 700_000)],
        ),
    );
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.receipts(0), 0);
    assert_eq!(forecast.memo(MemoKind::BouncedCheque, 0), 700_000);
}

/// A supplier bill that is matched (not approved) is a payment marked *not
/// yet approved*; one held as a possible duplicate is on the memo line.
#[renox::test]
async fn bills_not_approved_are_marked_and_a_duplicate_is_a_memo_line() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    open_bill(
        &t,
        &e1,
        "SB-6",
        day(2, 3),
        500_000,
        SupplierBillStatus::Matched,
    )
    .await;
    let held = open_bill(
        &t,
        &e1,
        "SB-7",
        day(2, 3),
        900_000,
        SupplierBillStatus::Exception,
    )
    .await;
    let mut reason = BillException {
        bill_id: held.id,
        reason_code: POSSIBLE_DUPLICATE.to_owned(),
        ..Default::default()
    };
    reason.insert(t.db()).await.unwrap();
    let forecast = grid(&t, &[&e1]).await;
    // 2027-02-03 is in week 4.
    assert_eq!(forecast.payments(3), 500_000);
    assert!(
        forecast
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Bill && l.not_approved)
    );
    assert_eq!(forecast.memo(MemoKind::DuplicateBill, 3), 900_000);

    // The setting that leaves them out.
    let strict = boot_with(WlInvoicing::new().cash_unapproved_bills_counted(false)).await;
    strict.travel_to("2027-01-13", 8, 0);
    let e = strict.entity("PGP").await;
    open_bill(
        &strict,
        &e,
        "SB-6",
        day(2, 3),
        500_000,
        SupplierBillStatus::Matched,
    )
    .await;
    assert_eq!(grid(&strict, &[&e]).await.payments(3), 0);
}

/// A receivable of a blocked party is on the memo line, not a receipt.
#[renox::test]
async fn a_blocked_partys_receivable_is_a_memo_line() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let mut invoice = open_invoice(&t, &e1, "INV-B", day(1, 20), 300_000).await;
    invoice.party_id = BLOCKED_PARTY.0;
    invoice.save(t.db()).await.unwrap();
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.receipts(1), 0);
    assert_eq!(forecast.memo(MemoKind::BlockedParty, 1), 300_000);
}

/// Approved or failed refunds not paid are payments in week 1.
#[renox::test]
async fn unpaid_refunds_are_payments_in_week_one() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    t.refunds.unpaid_refund(e1.id, 90_000, false);
    t.refunds.unpaid_refund(e1.id, 10_000, true);
    assert_eq!(grid(&t, &[&e1]).await.payments(0), 100_000);
}

/// A foreign-currency invoice without a stored rate is left out and listed
/// under the warnings.
#[renox::test]
async fn a_foreign_amount_without_a_rate_is_left_out_with_a_warning() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let mut invoice = open_invoice(&t, &e1, "INV-US", day(1, 20), 100_000).await;
    invoice.currency = "USD".into();
    invoice.save(t.db()).await.unwrap();
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.receipts(1), 0);
    assert!(
        forecast
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::Unpriced { document } if *document == invoice.subject()))
    );
    // With a rate it counts in AED.
    invoice.exchange_rate_micro = Some(3_672_500);
    invoice.save(t.db()).await.unwrap();
    assert_eq!(grid(&t, &[&e1]).await.receipts(1), 367_250);
}

/// Months of the horizon with no approved budget are a warning.
#[renox::test]
async fn months_without_an_approved_budget_are_a_warning() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let forecast = grid(&t, &[&e1]).await;
    assert!(forecast.warnings.iter().any(
        |w| matches!(w, Warning::NoBudget { months, .. } if months.first() == Some(&day(1, 1)))
    ));
}

/// With week start Sunday, week 1 of Wednesday 2027-01-13 is 2027-01-10 to
/// 2027-01-16.
#[renox::test]
async fn the_week_starts_on_the_day_the_setting_names() {
    let t = boot_with(WlInvoicing::new().cash_week_start(Weekday::Sun)).await;
    t.travel_to("2027-01-13", 8, 0);
    let e1 = t.entity("PGP").await;
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(
        (forecast.weeks[0].start, forecast.weeks[0].end),
        (day(1, 10), day(1, 16))
    );
    let short = boot_with(WlInvoicing::new().cash_forecast_weeks(4)).await;
    short.travel_to("2027-01-13", 8, 0);
    let e = short.entity("PGP").await;
    assert_eq!(grid(&short, &[&e]).await.weeks.len(), 4);
}

/// A source adds payroll (a total only) and a final settlement; one standing
/// for a document the forecast counts is dropped.
struct Hr;

impl CashForecastSource for Hr {
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            // Only the entity the test makes (the registry is process wide).
            let entity = LegalEntity::find(&state.db, legal_entity_id).await?;
            if entity.map(|e| e.code) != Some("HRCO".to_owned()) {
                return Ok(Vec::new());
            }
            Ok(vec![
                ForecastLine::new("Payroll", "payroll", day(2, 1), 9_000_000, Direction::Out)
                    .party(BLUE_DUNES)
                    .total_only(),
                ForecastLine::new(
                    "Final settlement",
                    "settlement",
                    day(2, 3),
                    1_200_000,
                    Direction::Out,
                )
                .total_only(),
                ForecastLine::new("Bill again", "bill", day(2, 3), 77, Direction::Out)
                    .document(Subject::new("invoicing.supplier_bill", 1)),
            ])
        })
    }
}

#[renox::test]
async fn the_lines_of_a_source_join_the_grid_as_totals() {
    register_cash_forecast_source("test.hr", Hr);
    let t = booted().await;
    let co = t.new_entity("HRCO").await;
    let bill = open_bill(
        &t,
        &co,
        "SB-1",
        day(2, 10),
        1_000,
        SupplierBillStatus::Approved,
    )
    .await;
    assert_eq!(bill.id, 1);
    let forecast = grid(&t, &[&co]).await;
    // Payroll of 2027-02-01 is in week 4, the settlement of 2027-02-03 too.
    assert_eq!(forecast.payments(3), 10_200_000);
    assert!(
        forecast
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Source)
            .all(|l| l.total_only)
    );
    // The line standing for the counted bill is dropped.
    assert!(forecast.lines.iter().all(|l| l.amount != 77));

    // The cell shows the total, not the party, to a user without payroll rights.
    let viewer = t
        .scoped_user("Hrviewer", &[perm::CASHFLOW_VIEW], co.id)
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get(&format!(
            "/cash-forecast/cell?entity={}&week=4&side=out",
            co.id
        ))
        .await
        .assert_ok()
        .assert_see("Payroll")
        .assert_dont_see("Blue Dunes");
}

/// A user scoped to one entity opening all entities sees that entity only; an
/// entity outside the scope is refused; a user without the permission is too.
#[renox::test]
async fn a_user_sees_the_entities_of_the_scope() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let e2 = t.entity("PGFZ").await;
    open_invoice(&t, &e1, "INV-E1", day(1, 20), 100_000).await;
    open_invoice(&t, &e2, "INV-E2", day(1, 20), 700_000).await;
    let scoped = t.scoped_user("Scoper", &[perm::CASHFLOW_VIEW], e1.id).await;
    t.app.acting_as(&scoped);
    t.app
        .get("/cash-forecast")
        .await
        .assert_ok()
        .assert_see("Projected balance");
    t.app
        .get("/cash-forecast/cell?week=2&side=in")
        .await
        .assert_ok()
        .assert_see("INV-E1")
        .assert_dont_see("INV-E2");
    t.app
        .get(&format!("/cash-forecast?entity={}", e2.id))
        .await
        .assert_forbidden();

    let nobody = t.helper("Nobody", &[perm::VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/cash-forecast").await.assert_forbidden();

    // Both entities for a user who sees both: the sum.
    let admin = viewer(&t).await;
    t.app.acting_as(&admin);
    let both = grid(&t, &[&e1, &e2]).await;
    assert_eq!(both.receipts(1), 800_000);
}

/// Opening the forecast stores nothing; entering and voiding a balance needs
/// `cashflow.manage`.
#[renox::test]
async fn opening_the_forecast_writes_nothing_and_balances_need_manage() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    let reader = t.helper("Reader", &[perm::CASHFLOW_VIEW]).await;
    let manager = viewer(&t).await;
    t.app.acting_as(&reader);
    t.app.get("/cash-forecast").await.assert_ok();
    assert_eq!(OpeningCashBalance::query().count(t.db()).await.unwrap(), 0);

    let id = e1.id.to_string();
    let form = [
        ("legal_entity_id", id.as_str()),
        ("bank_account", "BA1"),
        ("amount", "60,000.00"),
        ("as_of", "2027-01-11"),
        ("note", ""),
    ];
    t.app
        .htmx()
        .post("/cash-balances", &form)
        .await
        .assert_forbidden();

    t.app.acting_as(&manager);
    let future = [
        ("legal_entity_id", id.as_str()),
        ("bank_account", "BA1"),
        ("amount", "60000"),
        ("as_of", "2027-01-20"),
        ("note", ""),
    ];
    t.app
        .htmx()
        .post("/cash-balances", &future)
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post("/cash-balances", &form)
        .await
        .assert_redirect("/cash-forecast");
    let rows = OpeningCashBalance::query().get(t.db()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount, 6_000_000);

    let path = format!("/cash-balances/{}/void", rows[0].id);
    t.app
        .htmx()
        .post(&path, &[("reason", " ")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&path, &[("reason", "wrong day")])
        .await
        .assert_redirect("/cash-forecast");
    assert!(
        OpeningCashBalance::find(t.db(), rows[0].id)
            .await
            .unwrap()
            .unwrap()
            .is_voided()
    );
}

/// Clicking the week 3 receipts cell lists INV-1.
#[renox::test]
async fn a_cell_lists_the_documents_behind_it() {
    let t = booted().await;
    let e1 = t.entity("PGP").await;
    open_invoice(&t, &e1, "INV-1", day(1, 25), 1_050_000).await;
    let user = viewer(&t).await;
    t.app.acting_as(&user);
    t.app
        .get("/cash-forecast/cell?week=3&side=in")
        .await
        .assert_ok()
        .assert_see("INV-1")
        .assert_see("10,500.00");
    t.app
        .get("/cash-forecast/cell?week=4&side=in")
        .await
        .assert_ok()
        .assert_dont_see("INV-1");
}

// ---- Budget fill and days late (#92) ----------------------------------------

/// A booted app whose clock reads `date` (a Monday, for the weeks to line up)
/// after the approved budget `lines` of the entity PGP for `year` were made
/// with the real clock.
async fn booted_with_budget(
    module: WlInvoicing,
    year: i64,
    lines: &[(&str, usize, i64)],
    date: &str,
) -> Booted {
    let t = boot_with(module).await;
    if !lines.is_empty() {
        t.approved_budget("PGP", year, lines).await;
    }
    t.travel_to(date, 8, 0);
    t
}

/// The budget fill lines of `forecast` dated in `month` of `year`, in fils.
fn fill_in(forecast: &Forecast, year: i32, month: u32) -> i64 {
    forecast
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::BudgetFill)
        .filter(|l| l.date.year() == year && l.date.month() == month)
        .map(|l| l.amount)
        .sum()
}

/// The fill counted in week `week` (from 0), in fils.
fn fill_week(forecast: &Forecast, week: usize) -> i64 {
    forecast
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::BudgetFill && l.week == week)
        .map(|l| l.amount)
        .sum()
}

/// A bill of `entity` not yet approved with one line of `net` fils coded to
/// the cost code `code`, dated `date`.
async fn bill_with_line(
    t: &Booted,
    entity: &LegalEntity,
    number: &str,
    date: NaiveDate,
    net: i64,
    code: &str,
    status: SupplierBillStatus,
) -> SupplierBill {
    let mut bill = SupplierBill {
        party_id: BLUE_DUNES.0,
        legal_entity_id: Some(entity.id),
        supplier_invoice_number: number.to_owned(),
        number_key: number.to_lowercase(),
        bill_date: date,
        due_date: Some(date + renox::chrono::Days::new(30)),
        currency: "AED".to_owned(),
        gross_total: net,
        balance: net,
        status,
        ..Default::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    bill.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = BillLine {
        bill_id: bill.id,
        position: 1,
        description: "Rent".to_owned(),
        quantity_milli: 1_000,
        unit_price: net,
        cost_code: Some(code.to_owned()),
        net_amount_aed: Some(net),
        vat_amount_aed: Some(0),
        ..Default::default()
    };
    line.insert(t.db()).await.unwrap();
    bill
}

/// Rent and utilities AED 31,000 for March 2027, no actual, no bills, on Monday
/// 2027-03-01: AED 1,000 a day, AED 7,000 in each of the weeks of 1, 8, 15 and
/// 22 March and AED 3,000 in the week of 29 March.
#[renox::test]
async fn a_budget_line_is_spread_evenly_over_the_days_of_its_month() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[("rent_utilities", 3, 3_100_000)],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    let forecast = grid(&t, &[&e1]).await;
    for week in 0..4 {
        assert_eq!(fill_week(&forecast, week), 700_000, "week {}", week + 1);
        assert_eq!(forecast.payments(week), 700_000);
    }
    assert_eq!(fill_week(&forecast, 4), 300_000);
    assert_eq!(fill_week(&forecast, 5), 0);
    let line = &forecast.lines[0];
    assert_eq!(
        (line.kind, line.category.as_str()),
        (LineKind::BudgetFill, "rent_utilities")
    );
    assert_eq!(line.label, "Rent and utilities");
    assert_eq!(line.direction, Direction::Out);
}

/// An approved bill of AED 25,000 excl. VAT for rent dated 2027-03-01 is in the
/// March actual: the fill is AED 6,000 and the April fill is not reduced.
#[renox::test]
async fn an_approved_bill_is_in_the_actual_and_not_subtracted_twice() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[
            ("rent_utilities", 3, 3_100_000),
            ("rent_utilities", 4, 3_000_000),
        ],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    t.approved_bill(
        &e1,
        "SB-RENT",
        day(3, 1),
        &[(
            "Rent",
            2_500_000,
            "rent",
            wl_kernel::invoicing::VatCategory::Standard,
        )],
    )
    .await;
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(fill_in(&forecast, 2027, 3), 600_000);
    assert_eq!(fill_in(&forecast, 2027, 4), 3_000_000);
}

/// A bill matched (not approved) for rent dated 2027-03-01 leaves a March fill
/// of AED 6,000; one dated 2027-02-26 leaves AED 31,000.
#[renox::test]
async fn a_bill_not_yet_approved_covers_the_month_of_its_bill_date() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[("rent_utilities", 3, 3_100_000)],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    bill_with_line(
        &t,
        &e1,
        "SB-F",
        day(2, 26),
        2_500_000,
        "rent",
        SupplierBillStatus::Matched,
    )
    .await;
    assert_eq!(fill_in(&grid(&t, &[&e1]).await, 2027, 3), 3_100_000);
    bill_with_line(
        &t,
        &e1,
        "SB-M",
        day(3, 1),
        2_500_000,
        "rent",
        SupplierBillStatus::Matched,
    )
    .await;
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(fill_in(&forecast, 2027, 3), 600_000);
    // The bills themselves are payments by their due dates, by bill date + 30.
    assert!(
        forecast
            .lines
            .iter()
            .any(|l| l.kind == LineKind::Bill && l.not_approved)
    );
}

/// With AED 33,000 posted against a budget of AED 31,000 there is no fill.
#[renox::test]
async fn the_fill_is_never_negative() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[("rent_utilities", 3, 3_100_000)],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    t.approved_bill(
        &e1,
        "SB-BIG",
        day(3, 1),
        &[(
            "Rent",
            3_300_000,
            "rent",
            wl_kernel::invoicing::VatCategory::Exempt,
        )],
    )
    .await;
    let forecast = grid(&t, &[&e1]).await;
    assert!(
        forecast
            .lines
            .iter()
            .all(|l| l.kind != LineKind::BudgetFill)
    );
}

/// Without an approved budget there is no fill and the forecast says so.
#[renox::test]
async fn without_an_approved_budget_there_is_no_fill_and_a_warning() {
    let t = booted_with_budget(WlInvoicing::new(), 2027, &[], "2027-03-01").await;
    let e1 = t.entity("PGP").await;
    let forecast = grid(&t, &[&e1]).await;
    assert!(
        forecast
            .lines
            .iter()
            .all(|l| l.kind != LineKind::BudgetFill)
    );
    assert!(forecast.warnings.iter().any(
        |w| matches!(w, Warning::NoBudget { months, .. } if months.first() == Some(&day(3, 1)))
    ));
    let user = viewer(&t).await;
    t.app.acting_as(&user);
    t.app
        .get("/cash-forecast")
        .await
        .assert_ok()
        .assert_see("no approved budget for the months");
}

/// On 2027-11-15 with an approved 2027 budget and none for 2028, December has
/// fill and January and February 2028 show the warning and no fill.
#[renox::test]
async fn a_year_without_a_budget_gets_the_warning_and_no_fill() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[
            ("rent_utilities", 11, 3_000_000),
            ("rent_utilities", 12, 3_100_000),
        ],
        "2027-11-15",
    )
    .await;
    let e1 = t.entity("PGP").await;
    let forecast = grid(&t, &[&e1]).await;
    // 16 days of November (15 to 30) and all of December.
    assert_eq!(fill_in(&forecast, 2027, 11), 3_000_000);
    assert_eq!(fill_in(&forecast, 2027, 12), 3_100_000);
    assert!(
        forecast
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::BudgetFill)
            .all(|l| l.date < NaiveDate::from_ymd_opt(2028, 1, 1).unwrap())
    );
    let months: Vec<NaiveDate> = forecast
        .warnings
        .iter()
        .find_map(|w| match w {
            Warning::NoBudget { months, .. } => Some(months.clone()),
            _ => None,
        })
        .expect("the warning");
    assert_eq!(
        months,
        vec![
            NaiveDate::from_ymd_opt(2028, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2028, 2, 1).unwrap()
        ]
    );
}

/// An income category fills receipts; owner drawings fill a total only.
#[renox::test]
async fn income_fill_is_a_receipt_and_drawings_fill_is_a_total() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[
            ("sales_services", 3, 3_100_000),
            ("owner_drawings", 3, 620_000),
        ],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    let forecast = grid(&t, &[&e1]).await;
    assert_eq!(forecast.receipts(0), 700_000);
    assert_eq!(forecast.payments(0), 140_000);
    let drawings = forecast
        .lines
        .iter()
        .find(|l| l.category == "owner_drawings")
        .expect("a drawings fill");
    assert!(drawings.total_only);
    assert!(
        forecast
            .lines
            .iter()
            .filter(|l| l.category == "sales_services")
            .all(|l| !l.total_only && l.direction == Direction::In)
    );
}

/// The grid shows a row for the fill and the cell opens its lines.
#[renox::test]
async fn the_grid_shows_a_fill_row_per_category_and_its_cell() {
    let t = booted_with_budget(
        WlInvoicing::new(),
        2027,
        &[("rent_utilities", 3, 3_100_000)],
        "2027-03-01",
    )
    .await;
    let user = viewer(&t).await;
    t.app.acting_as(&user);
    t.app
        .get("/cash-forecast")
        .await
        .assert_ok()
        .assert_see("Budget fill: Rent and utilities")
        .assert_see("7,000");
    t.app
        .get("/cash-forecast/cell?week=5&side=fill:rent_utilities")
        .await
        .assert_ok()
        .assert_see("Rent and utilities")
        .assert_see("3,000.00");
}

/// Customer C with invoices paid 10, 20 and 30 days late moves INV-9 due
/// 2027-03-01 to 2027-03-21; payments are not moved.
#[renox::test]
async fn days_late_moves_a_customers_receipts_by_its_average() {
    let t = booted_with_budget(
        WlInvoicing::new().cash_days_late(true),
        2027,
        &[],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    for (i, late) in [10_i64, 20, 30].iter().enumerate() {
        let due = day(1, 1);
        let paid =
            invoice_in_state(&t, &e1, &format!("INV-P{i}"), due, 0, InvoiceStatus::Paid).await;
        let mut allocation = Allocation::new(
            paid.id,
            &Subject::new("payments.payment", 40 + i as i64),
            1_000,
            None,
        );
        allocation.allocated_at = (due + renox::chrono::Days::new(*late as u64))
            .and_hms_opt(8, 0, 0)
            .unwrap()
            .and_utc();
        allocation.insert(t.db()).await.unwrap();
    }
    open_invoice(&t, &e1, "INV-9", day(3, 1), 500_000).await;
    open_bill(
        &t,
        &e1,
        "SB-5",
        day(3, 1),
        200_000,
        SupplierBillStatus::Approved,
    )
    .await;
    let forecast = grid(&t, &[&e1]).await;
    let line = forecast
        .lines
        .iter()
        .find(|l| l.label == "INV-9")
        .expect("INV-9");
    assert_eq!(line.date, day(3, 21));
    assert_eq!(forecast.receipts(2), 500_000);
    assert_eq!(forecast.payments(0), 200_000);

    // Off: the due date.
    let plain = booted_with_budget(WlInvoicing::new(), 2027, &[], "2027-03-01").await;
    let e = plain.entity("PGP").await;
    open_invoice(&plain, &e, "INV-9", day(3, 1), 500_000).await;
    let forecast = grid(&plain, &[&e]).await;
    assert_eq!(forecast.lines[0].date, day(3, 1));
}

/// Fewer than three paid invoices: no adjustment; written-off invoices are not
/// in the average.
#[renox::test]
async fn days_late_needs_three_paid_invoices() {
    let t = booted_with_budget(
        WlInvoicing::new().cash_days_late(true),
        2027,
        &[],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    for (i, late) in [10_i64, 20].iter().enumerate() {
        let due = day(1, 1);
        let paid =
            invoice_in_state(&t, &e1, &format!("INV-P{i}"), due, 0, InvoiceStatus::Paid).await;
        let mut allocation = Allocation::new(
            paid.id,
            &Subject::new("payments.payment", 40 + i as i64),
            1_000,
            None,
        );
        allocation.allocated_at = (due + renox::chrono::Days::new(*late as u64))
            .and_hms_opt(8, 0, 0)
            .unwrap()
            .and_utc();
        allocation.insert(t.db()).await.unwrap();
    }
    invoice_in_state(&t, &e1, "INV-W", day(1, 1), 0, InvoiceStatus::WrittenOff).await;
    open_invoice(&t, &e1, "INV-9", day(3, 1), 500_000).await;
    let forecast = grid(&t, &[&e1]).await;
    let line = forecast
        .lines
        .iter()
        .find(|l| l.label == "INV-9")
        .expect("INV-9");
    assert_eq!(line.date, day(3, 1));
}

/// A receipt moved beyond the horizon drops out of the grid and is listed as
/// after week 13.
#[renox::test]
async fn a_receipt_moved_beyond_the_horizon_is_listed_after_it() {
    let t = booted_with_budget(
        WlInvoicing::new().cash_days_late(true),
        2027,
        &[],
        "2027-03-01",
    )
    .await;
    let e1 = t.entity("PGP").await;
    for (i, late) in [20_i64, 20, 20].iter().enumerate() {
        let due = day(1, 1);
        let paid =
            invoice_in_state(&t, &e1, &format!("INV-P{i}"), due, 0, InvoiceStatus::Paid).await;
        let mut allocation = Allocation::new(
            paid.id,
            &Subject::new("payments.payment", 40 + i as i64),
            1_000,
            None,
        );
        allocation.allocated_at = (due + renox::chrono::Days::new(*late as u64))
            .and_hms_opt(8, 0, 0)
            .unwrap()
            .and_utc();
        allocation.insert(t.db()).await.unwrap();
    }
    // Week 13 ends 2027-05-30; due 2027-05-28 + 20 days is 2027-06-17.
    open_invoice(&t, &e1, "INV-LATE", day(5, 28), 500_000).await;
    let forecast = grid(&t, &[&e1]).await;
    assert!(forecast.lines.is_empty());
    assert_eq!(forecast.after.len(), 1);
    assert_eq!(forecast.after[0].date, day(6, 17));
    let user = viewer(&t).await;
    t.app.acting_as(&user);
    t.app
        .get("/cash-forecast")
        .await
        .assert_ok()
        .assert_see("After week 13")
        .assert_see("INV-LATE");
}
