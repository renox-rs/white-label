//! `HRM.RUL.09` (wl-hr §6 *Automations*, §5.4 step 4, §7, §10 *Final
//! settlement*, issue #215): the final settlement is due within 14 days of the
//! end of the contract (Art. 53 ✔ [S1]). HR and the accountant are told N days
//! (default 10) after the last working day; on the deadline the MD is told it
//! is overdue. For a `disputed` settlement the reminder is to pay the
//! undisputed amount.
//!
//! - **Condition.** The settlement is `draft` or `approved`, or `disputed`
//!   with less than the undisputed amount paid (whether it was disputed before
//!   or after `pay`). `paid`, `closed` and `cancelled` settlements are never
//!   alerted. A settlement without a last working day has no deadline and is
//!   skipped.
//! - **Days.** Calendar days, Asia/Dubai, the daily job [`TASK`] at 08:00.
//! - **Idempotency.** One alert per point per settlement: the days are kept on
//!   the row (`alerted_on`, `overdue_alerted_on`), set after the notice went
//!   out. A day the job missed is caught up on the next run (both alerts when
//!   both are due).
//! - **Who.** HR and the accountant are the holders of the `hr_role` and the
//!   `accountant_role`, the MD the holders of the `md_role`.

use renox::chrono::{Duration, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::config::config_of;
use crate::employee::Employee;
use crate::notices::{self, SETTLEMENT_DUE, SETTLEMENT_OVERDUE, SETTLEMENT_UNDISPUTED_DUE};
use crate::settlement::{FinalSettlement, SettlementStatus};
use crate::settlement_events::users_of;
use crate::settlement_payment::paid_total;

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "hr:settlement-deadline";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "08:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run of the job did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Settlements HR and the accountant were told about.
    pub alerted: usize,
    /// Settlements the MD was told are overdue.
    pub overdue: usize,
    /// Settlements that failed; the job moves on.
    pub failed: usize,
}

/// Whether `settlement` is one `HRM.RUL.09` watches: unpaid, or disputed with
/// less than the undisputed amount paid.
async fn watched(state: &AppState, settlement: &FinalSettlement) -> Result<bool> {
    Ok(match settlement.status {
        SettlementStatus::Draft | SettlementStatus::Approved => true,
        SettlementStatus::Disputed => {
            paid_total(&state.db, settlement.id).await? < settlement.undisputed_amount
        }
        _ => false,
    })
}

/// Runs `HRM.RUL.09` for `day` (Asia/Dubai).
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let mut report = Report::default();
    let open = FinalSettlement::query()
        .where_(FinalSettlement::STATUS.is_in([
            SettlementStatus::Draft,
            SettlementStatus::Approved,
            SettlementStatus::Disputed,
        ]))
        .where_(FinalSettlement::LAST_WORKING_DAY.is_not_null())
        .order_by(FinalSettlement::ID)
        .get(&state.db)
        .await?;
    for settlement in open {
        match alert(state, settlement.id, day, &mut report).await {
            Ok(()) => {}
            Err(error) => {
                report.failed += 1;
                eprintln!("{TASK}: settlement {}: {error:?}", settlement.id);
            }
        }
    }
    Ok(report)
}

async fn alert(state: &AppState, id: i64, day: NaiveDate, report: &mut Report) -> Result {
    // Read it again: the list may be older than a payment.
    let Some(settlement) = FinalSettlement::find(&state.db, id).await? else {
        return Ok(());
    };
    let Some(last) = settlement.last_working_day else {
        return Ok(());
    };
    if !watched(state, &settlement).await? {
        return Ok(());
    }
    let config = config_of(state);
    let due = last + Duration::days(config.settlement_alert_days);
    let deadline = last + Duration::days(config.settlement_deadline_days);
    let Some(employee) = Employee::find(&state.db, settlement.employee_id).await? else {
        return Ok(());
    };
    let name = notices::name_of(state, &employee).await;
    if day >= due && settlement.alerted_on.is_none() {
        let users = users_of(state, &[&config.hr_role, &config.accountant_role]).await?;
        let kind = if settlement.status == SettlementStatus::Disputed {
            SETTLEMENT_UNDISPUTED_DUE
        } else {
            SETTLEMENT_DUE
        };
        notices::settlement_notice(state, kind, &settlement, &name, Some(deadline), &users).await?;
        FinalSettlement::query()
            .where_(FinalSettlement::ID.eq(id))
            .update(
                &state.db,
                &[("alerted_on", &Some(day)), ("updated_at", &now())],
            )
            .await?;
        report.alerted += 1;
    }
    if day >= deadline && settlement.overdue_alerted_on.is_none() {
        let users = users_of(state, &[&config.md_role]).await?;
        notices::settlement_notice(
            state,
            SETTLEMENT_OVERDUE,
            &settlement,
            &name,
            Some(deadline),
            &users,
        )
        .await?;
        FinalSettlement::query()
            .where_(FinalSettlement::ID.eq(id))
            .update(
                &state.db,
                &[("overdue_alerted_on", &Some(day)), ("updated_at", &now())],
            )
            .await?;
        report.overdue += 1;
    }
    Ok(())
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Tell HR, the accountant and the MD of final settlements near or past the legal deadline \
         (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = match args.value("--date") {
                None => today(),
                Some(text) => text.parse().map_err(|_| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("--date must be YYYY-MM-DD, got `{text}`"),
                    )
                })?,
            };
            let report = run(&state, on).await?;
            println!(
                "{} settlements alerted, {} overdue, {} failed.",
                report.alerted, report.overdue, report.failed
            );
            Ok(())
        },
    );
}
