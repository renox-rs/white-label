//! The cash-flow forecast reads what other modules own through the kernel
//! `CashForecastSource` (wl-invoicing §5.14, §8, `FIN.CFL.01`): payroll runs
//! and final settlements (wl-hr), scheduled charges and payer bills. Neither
//! wl-hr's payroll nor the scheduled charges nor the payer bills exist in this
//! build, so a minimal provider declared here stands in for them through the
//! real trait: it holds the rules of its module (the WPS due date, the last
//! paid regular run, the 14-day deadline of a final settlement) and the
//! forecast must place its lines, add what it paid after a bank balance to
//! the opening, show its warnings and move a payer's receipts by the days its
//! documents were paid late. Checks of #91 points 5, 8, 11 and 15 and of #92
//! point 8. Today is Wednesday 2027-01-13; week 1 is Monday 2027-01-11 to
//! Sunday 2027-01-17. The registry of sources is process wide, so the
//! provider answers only for the entities these tests make (by code).

mod support;

use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use wl_invoicing::cash_forecast::{self, Forecast, LineKind, Warning};
use wl_invoicing::cash_forecast_load;
use wl_invoicing::{WlInvoicing, permissions as perm};
use wl_kernel::cash_forecast::{
    CashForecastSource, ForecastFuture, ForecastLine, register_cash_forecast_source,
};
use wl_kernel::documents::today;
use wl_kernel::payments::Direction;
use wl_kernel::{LegalEntity, PartyId, Subject};

use support::{BLUE_DUNES, Booted, boot, boot_with, open_cheque};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2027, m, d).unwrap()
}

/// A payroll run of the stand-in: the first day of its period, its net and the
/// day it was confirmed paid.
struct Run {
    period: NaiveDate,
    net: i64,
    paid_on: Option<NaiveDate>,
}

/// The provider: payroll runs and final settlements as wl-hr keeps them, and
/// the scheduled charges and payer bills of other modules.
struct Providers;

impl Providers {
    fn runs(code: &str) -> Vec<Run> {
        match code {
            // The last paid regular run is December's, net AED 90,000.
            "PAY1" => vec![Run {
                period: day(1, 1) - Days::new(31),
                net: 9_000_000,
                paid_on: Some(day(1, 2)),
            }],
            // The January run is paid on 2027-01-12 (and December's on the 02nd).
            "PAY2" => vec![
                Run {
                    period: day(1, 1) - Days::new(31),
                    net: 8_000_000,
                    paid_on: Some(day(1, 2)),
                },
                Run {
                    period: day(1, 1),
                    net: 9_000_000,
                    paid_on: Some(day(1, 12)),
                },
            ],
            _ => Vec::new(),
        }
    }

    /// The settlements approved and not paid: `(net, last working day)`.
    fn settlements(code: &str) -> Vec<(i64, NaiveDate)> {
        match code {
            // Last working day 01-20: the legal deadline is 14 days later.
            "SETL" => vec![(1_200_000, day(1, 20))],
            // The contract ended 2026-12-20: the deadline (01-03) has passed.
            "LATE" => vec![(700_000, NaiveDate::from_ymd_opt(2026, 12, 20).unwrap())],
            _ => Vec::new(),
        }
    }

    /// The day the WPS file of the period starting `period` is due: the first
    /// day of the next month.
    fn wps_due(period: NaiveDate) -> NaiveDate {
        let (year, month) = if period.month() == 12 {
            (period.year() + 1, 1)
        } else {
            (period.year(), period.month() + 1)
        };
        NaiveDate::from_ymd_opt(year, month, 1).unwrap()
    }

    /// The payer bills of a payer: how late each was paid.
    fn payer_delays(code: &str) -> Vec<i64> {
        match code {
            "PAYER1" => vec![15, 30, 45],
            "PAYER2" => vec![15, 30],
            _ => Vec::new(),
        }
    }
}

impl CashForecastSource for Providers {
    fn lines<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        from: NaiveDate,
        to: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            let Some(entity) = LegalEntity::find(&state.db, legal_entity_id).await? else {
                return Ok(Vec::new());
            };
            let code = entity.code.as_str();
            let mut lines = Vec::new();

            // Payroll per period at the WPS due date: the net of the period's
            // regular run while it is unpaid; with no run yet, the net of the
            // entity's last paid regular run; with none, nothing.
            let runs = Self::runs(code);
            let last_paid = runs
                .iter()
                .filter(|r| r.paid_on.is_some())
                .max_by_key(|r| r.period);
            for months in 0..4u32 {
                let period = from
                    .with_day(1)
                    .unwrap()
                    .checked_sub_months(renox::chrono::Months::new(1))
                    .unwrap()
                    .checked_add_months(renox::chrono::Months::new(months))
                    .unwrap();
                let due = Self::wps_due(period);
                if due < from || due > to {
                    continue;
                }
                let net = match runs.iter().find(|r| r.period == period) {
                    Some(run) if run.paid_on.is_some() => continue,
                    Some(run) => run.net,
                    None => match last_paid {
                        Some(run) => run.net,
                        None => continue,
                    },
                };
                lines.push(
                    ForecastLine::new("Payroll", "payroll", due, net, Direction::Out).total_only(),
                );
            }

            // Approved final settlements not yet paid: at the legal deadline,
            // 14 days after the last working day.
            for (net, last_day) in Self::settlements(code) {
                lines.push(
                    ForecastLine::new(
                        "Final settlement",
                        "settlement",
                        last_day + Days::new(14),
                        net,
                        Direction::Out,
                    )
                    .total_only(),
                );
            }

            // A scheduled charge not yet invoiced, and the payer bill of a
            // payer.
            if code == "CHG1" {
                lines.push(
                    ForecastLine::new(
                        "Scheduled charge CH-3",
                        "scheduled_charge",
                        day(1, 25),
                        800_000,
                        Direction::In,
                    )
                    .party(BLUE_DUNES)
                    .document(Subject::new("invoicing.scheduled_charge", 3)),
                );
            }
            if code.starts_with("PAYER") {
                lines.push(
                    ForecastLine::new(
                        "Payer bill PB-4",
                        "payer_bill",
                        day(3, 1),
                        500_000,
                        Direction::In,
                    )
                    .party(BLUE_DUNES),
                );
            }
            Ok(lines)
        })
    }

    fn paid_after<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        account: &'a str,
        since: NaiveDate,
    ) -> ForecastFuture<'a, Vec<ForecastLine>> {
        Box::pin(async move {
            let Some(entity) = LegalEntity::find(&state.db, legal_entity_id).await? else {
                return Ok(Vec::new());
            };
            // The entity's payroll account is BA1.
            if account != "BA1" {
                return Ok(Vec::new());
            }
            Ok(Self::runs(&entity.code)
                .into_iter()
                .filter_map(|run| {
                    let paid = run.paid_on.filter(|paid| *paid > since)?;
                    Some(ForecastLine::new(
                        "Payroll paid",
                        "payroll",
                        paid,
                        run.net,
                        Direction::Out,
                    ))
                })
                .collect())
        })
    }

    fn warnings<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
    ) -> ForecastFuture<'a, Vec<String>> {
        Box::pin(async move {
            let Some(entity) = LegalEntity::find(&state.db, legal_entity_id).await? else {
                return Ok(Vec::new());
            };
            // An entity of this provider's (payroll) with no paid regular run.
            if entity.code == "PAY0" {
                return Ok(vec![
                    "No payroll history: the next payroll is not forecast".to_owned(),
                ]);
            }
            Ok(Vec::new())
        })
    }

    fn paid_late<'a>(
        &'a self,
        state: &'a AppState,
        legal_entity_id: i64,
        parties: &'a [PartyId],
        _since: NaiveDate,
        _until: NaiveDate,
    ) -> ForecastFuture<'a, Vec<(PartyId, i64)>> {
        Box::pin(async move {
            let Some(entity) = LegalEntity::find(&state.db, legal_entity_id).await? else {
                return Ok(Vec::new());
            };
            Ok(Self::payer_delays(&entity.code)
                .into_iter()
                .filter(|_| parties.contains(&BLUE_DUNES))
                .map(|days| (BLUE_DUNES, days))
                .collect())
        })
    }
}

async fn booted_with(module: WlInvoicing) -> Booted {
    register_cash_forecast_source("test.providers", Providers);
    let t = boot_with(module).await;
    t.travel_to("2027-01-13", 8, 0);
    t
}

async fn booted() -> Booted {
    register_cash_forecast_source("test.providers", Providers);
    let t = boot().await;
    t.travel_to("2027-01-13", 8, 0);
    t
}

/// The forecast of `entity` computed now.
async fn grid(t: &Booted, entity: &LegalEntity) -> Forecast {
    let entities = vec![entity.clone()];
    let inputs = t
        .app
        .at_travelled_time(async {
            cash_forecast_load::load(t.state(), &t.config(), &entities, today()).await
        })
        .await
        .unwrap();
    cash_forecast::assemble(&inputs)
}

/// The lines of the sources in a forecast.
fn from_sources(forecast: &Forecast) -> Vec<&cash_forecast::Line> {
    forecast
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::Source)
        .collect()
}

/// Payroll for E1: the WPS file is due 2027-02-01 and the last paid regular
/// run had a net of AED 90,000, so a payment of AED 90,000 is in week 4, a
/// total only. The periods after it repeat it at their own WPS due dates.
#[renox::test]
async fn payroll_is_a_payment_of_the_last_paid_run_at_the_wps_due_date() {
    let t = booted().await;
    let co = t.new_entity("PAY1").await;
    let forecast = grid(&t, &co).await;
    assert_eq!(forecast.payments(3), 9_000_000);
    let payroll = from_sources(&forecast);
    assert!(payroll.iter().all(|l| l.total_only));
    assert_eq!(payroll[0].date, day(2, 1));
    assert_eq!(payroll[0].week, 3);
    assert_eq!(payroll[0].category, "payroll");
    // The next period's WPS due date (03-01) is in week 8 (index 7); nothing in between.
    assert_eq!(forecast.payments(4), 0);
    assert_eq!(forecast.payments(6), 0);
    assert_eq!(forecast.payments(7), 9_000_000);
    assert!(
        forecast
            .warnings
            .iter()
            .all(|w| !matches!(w, Warning::Source { .. }))
    );
}

/// An entity with no payroll history gets no payroll line and a warning the
/// grid shows.
#[renox::test]
async fn an_entity_without_payroll_history_skips_payroll_with_a_warning() {
    let t = booted().await;
    let co = t.new_entity("PAY0").await;
    let forecast = grid(&t, &co).await;
    assert!(from_sources(&forecast).is_empty());
    assert_eq!(forecast.payments(3), 0);
    assert!(forecast.warnings.iter().any(|w| matches!(
        w,
        Warning::Source { message, .. } if message.contains("No payroll history")
    )));

    let viewer = t
        .scoped_user("Payviewer", &[perm::CASHFLOW_VIEW], co.id)
        .await;
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/cash-forecast?entity={}", co.id))
        .await
        .assert_ok()
        .assert_see("No payroll history");
}

/// A statement of AED 150,000 on 2027-01-10 and the January run, net AED
/// 90,000, confirmed paid on 2027-01-12 from the payroll account: the opening
/// balance is reduced by AED 90,000. A run paid on or before the statement
/// day is in the balance already, and the paid January run is no payment.
#[renox::test]
async fn a_payroll_run_paid_after_the_balance_reduces_the_opening() {
    let t = booted().await;
    let co = t.new_entity("PAY2").await;
    t.forecast.statement(co.id, "BA1", 15_000_000, day(1, 10));
    let forecast = grid(&t, &co).await;
    assert_eq!(forecast.openings.len(), 1);
    assert_eq!(forecast.openings[0].movements, -9_000_000);
    assert_eq!(forecast.opening_total(), 6_000_000);
    // The January period has its (paid) run: no payroll at 02-01; February
    // repeats the last paid regular run at 03-01.
    assert_eq!(forecast.payments(3), 0);
    assert_eq!(forecast.payments(7), 9_000_000);

    // A statement dated after the payment already holds it.
    t.forecast.statement(co.id, "BA1", 6_000_000, day(1, 12));
    let later = grid(&t, &co).await;
    assert_eq!(later.opening_total(), 6_000_000);
    assert_eq!(later.openings[0].movements, 0);
}

/// An approved final settlement of AED 12,000 with the last working day on
/// 2027-01-20 is a payment of AED 12,000 on its legal deadline, 2027-02-03,
/// in week 4. One whose deadline has passed counts in week 1.
#[renox::test]
async fn a_final_settlement_is_paid_at_its_legal_deadline_or_in_week_one() {
    let t = booted().await;
    let co = t.new_entity("SETL").await;
    let forecast = grid(&t, &co).await;
    let lines = from_sources(&forecast);
    assert_eq!(lines.len(), 1);
    assert_eq!((lines[0].date, lines[0].week), (day(2, 3), 3));
    assert_eq!(forecast.payments(3), 1_200_000);
    assert!(lines[0].total_only);

    let late = t.new_entity("LATE").await;
    let overdue = grid(&t, &late).await;
    assert_eq!(overdue.payments(0), 700_000);
    assert_eq!(from_sources(&overdue)[0].week, 0);
}

/// A received post-dated cheque of AED 8,000 dated 2027-02-01 is linked to
/// scheduled charge CH-3 (not invoiced): the amount counts in week 4 through
/// the cheque only, and the charge's own line of 2027-01-25 is dropped.
#[renox::test]
async fn a_cheque_linked_to_a_scheduled_charge_counts_through_the_cheque_only() {
    let t = booted().await;
    let co = t.new_entity("CHG1").await;

    // Without the cheque the charge counts at its due date, in week 3.
    let alone = grid(&t, &co).await;
    assert_eq!(alone.receipts(2), 800_000);
    assert_eq!(alone.receipts(3), 0);

    t.forecast.open_cheque(
        co.id,
        open_cheque(
            7,
            "CHQ-7",
            Direction::In,
            day(2, 1),
            800_000,
            &[(Subject::new("invoicing.scheduled_charge", 3), 800_000)],
        ),
    );
    let forecast = grid(&t, &co).await;
    assert_eq!(forecast.receipts(3), 800_000);
    assert_eq!(forecast.receipts(2), 0);
    assert!(
        forecast
            .lines
            .iter()
            .all(|l| l.kind != LineKind::Source || l.category != "scheduled_charge")
    );
}

/// Payer P has three payer bills paid 15, 30 and 45 days late: an open payer
/// bill due 2027-03-01 is expected on 2027-03-31 (the average is 30 days).
/// With two paid bills there is no average; with the setting off the due date
/// stands.
#[renox::test]
async fn a_payers_receipts_move_by_the_days_its_bills_were_paid_late() {
    let t = booted_with(WlInvoicing::new().cash_days_late(true)).await;
    let payer = t.new_entity("PAYER1").await;
    let forecast = grid(&t, &payer).await;
    let bill = from_sources(&forecast);
    assert_eq!(bill.len(), 1);
    assert_eq!(bill[0].date, day(3, 31));
    assert_eq!(forecast.receipts(bill[0].week), 500_000);

    let two = t.new_entity("PAYER2").await;
    let unmoved = grid(&t, &two).await;
    assert_eq!(from_sources(&unmoved)[0].date, day(3, 1));

    let off = booted().await;
    let payer = off.new_entity("PAYER1").await;
    assert_eq!(from_sources(&grid(&off, &payer).await)[0].date, day(3, 1));
}
