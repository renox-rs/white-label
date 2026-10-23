//! The daily run of the fleet contracts (08 §7.1 *Fleet contract*, §7.2
//! `AUT.RUL.04` and `AUT.RUL.05`): at 00:30 Asia/Dubai it flags the contracts
//! whose renewal is due, ends the contracts whose last day of service has
//! passed (the contract is valid through its end date, so the first run after
//! it ends it), terminates the ones whose scheduled termination took effect,
//! requests the bookings of the services that fall due, and on day 1 of the
//! month drafts the consolidated invoice of the month before.
//!
//! One contract that fails does not stop the others: its refusal is reported
//! in [`DailyRun::failed`] and tried again the next day. What follows an end
//! or a termination (bookings cancelled, the last invoice, the fee, the fleet
//! contact told) is done by the listener of the transition
//! ([`crate::fleet_end`], BL-AUT-13 and BL-AUT-12); an end that stopped half
//! way is finished at the end of the run ([`DailyRun::wound_down`]). A contract
//! that is suspended is ended and invoiced like any other in force, but books
//! nothing new.

use renox::chrono::{Datelike, NaiveDate};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::fleet_billing;
use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::fleet_contract_machine::{FLAG_RENEWAL, TERMINATE_SYSTEM};
use crate::fleet_contracts::{due_transition, run_system};
use crate::fleet_end;
use crate::fleet_services;

/// The scheduled task and command.
pub const TASK: &str = "auto:fleet";
/// The time of day it runs.
pub const TASK_AT: &str = "00:30";
/// The time zone of [`TASK_AT`].
pub const TASK_ZONE: &str = "Asia/Dubai";
/// The day of the month the consolidated invoice is drafted (`AUT.RUL.05`).
pub const INVOICE_DAY: u32 = 1;

/// What a run did, by contract id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DailyRun {
    /// The contracts flagged as due for renewal.
    pub flagged: Vec<i64>,
    /// The contracts that ended because their last day passed.
    pub ended: Vec<i64>,
    /// The contracts whose scheduled termination took effect.
    pub terminated: Vec<i64>,
    /// The bookings requested for services that fell due.
    pub booked: usize,
    /// The contracts whose monthly invoice was drafted.
    pub invoiced: Vec<i64>,
    /// The contracts that had ended and whose last steps were done now (the
    /// bookings, the last invoice, the fee, the notice of an end that stopped
    /// half way).
    pub wound_down: Vec<i64>,
    /// The contracts a step was refused for, with why.
    pub failed: Vec<(i64, String)>,
}

/// Runs the day `day`.
///
/// # Errors
///
/// The database's errors.
pub async fn run_daily_on(state: &AppState, day: NaiveDate) -> Result<DailyRun> {
    let mut run = DailyRun::default();
    let contracts = FleetContract::query()
        .where_(FleetContract::STATUS.is_in([
            FleetStatus::Active,
            FleetStatus::Suspended,
            FleetStatus::RenewalDue,
        ]))
        .order_by(FleetContract::ID)
        .get(&state.db)
        .await?;
    for contract in contracts {
        if let Some((name, input)) = due_transition(&contract, day) {
            let reason = (name == TERMINATE_SYSTEM).then(|| {
                contract
                    .termination_reason
                    .clone()
                    .unwrap_or_else(|| name.to_owned())
            });
            match run_system(state, &contract, name, reason.as_deref(), input).await {
                Ok(_) if name == FLAG_RENEWAL => run.flagged.push(contract.id),
                Ok(_) if name == TERMINATE_SYSTEM => run.terminated.push(contract.id),
                Ok(_) => run.ended.push(contract.id),
                Err(error) => run.failed.push((contract.id, error.to_string())),
            }
        }
        let current = FleetContract::find_or_404(&state.db, contract.id).await?;
        if current.status == FleetStatus::Active {
            match fleet_services::request_bookings(state, &current, day).await {
                Ok(booked) => {
                    run.booked += booked.booked.len();
                    for (plan, why) in booked.failed {
                        run.failed.push((current.id, format!("plan {plan}: {why}")));
                    }
                }
                Err(error) => run.failed.push((current.id, error.to_string())),
            }
        }
        if day.day() == INVOICE_DAY && current.status.is_in_force() {
            match fleet_billing::invoice_previous_month(state, &current, day).await {
                Ok(Some(_)) => run.invoiced.push(current.id),
                Ok(None) => {}
                Err(error) => run.failed.push((current.id, error.to_string())),
            }
        }
    }
    match fleet_end::retry_all(state).await {
        Ok(done) => run.wound_down = done,
        Err(error) => run.failed.push((0, error.to_string())),
    }
    Ok(run)
}

/// Runs today (Asia/Dubai).
///
/// # Errors
///
/// As for [`run_daily_on`].
pub async fn run_daily(state: &AppState) -> Result<DailyRun> {
    run_daily_on(state, today()).await
}

/// Registers the daily task and its command.
pub(crate) fn register(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Run the fleet contracts of the day: renewals, ends, terminations, the bookings of the services that fall due and the monthly invoice",
        |_args, state: AppState| async move {
            let run = run_daily(&state).await?;
            println!(
                "{} flagged, {} ended, {} terminated, {} bookings, {} invoices, {} refused.",
                run.flagged.len(),
                run.ended.len(),
                run.terminated.len(),
                run.booked,
                run.invoiced.len(),
                run.failed.len()
            );
            Ok(())
        },
    );
}
