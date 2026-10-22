//! Billing the hours of a deployment (09 Manpower §4.3 *Time to bill*, §7.2
//! `MPW.RUL.05`, `MPW.ACL.16`; BL-MPW-06, BL-MPW-11).
//!
//! The vertical knows which contract a deployment bills under, its role and
//! shift, the days it ran and the days it was suspended; wl-invoicing knows the
//! rate cards. The two meet in the kernel's
//! [`UsageRecorder`](wl_kernel::usage::UsageRecorder), and the hours of a
//! timesheet come from wl-attendance through the kernel's
//! [`Timesheets`](wl_kernel::timesheets::Timesheets) (the entries per day,
//! billable ones only).
//!
//! - **Approved hours become usage.** [`bill_approved`] listens to
//!   `TimesheetApproved` of a regular timesheet in a deployment: it describes
//!   the timesheet (contract, role, shift, the days the deployment ran, the
//!   [`SuspensionSpan`]s) and the recorder prices each day by the rate card
//!   version in force on the work date (`MPW.ACL.16`). It runs whether the
//!   contract is `active` or `ended` (the last timesheet of a contract that
//!   ended is billed too, BL-MPW-06), and again for the same timesheet it
//!   replaces the records not invoiced and touches none that is, so a retry
//!   bills nothing twice. Nothing invoiced, and nothing in a locked timesheet,
//!   is repriced by it. Adjustment timesheets are wl-invoicing's own.
//! - **Suspended days are not billed** (BL-MPW-11): an hour or day rate bills
//!   attendance only (and no hours are recorded for a suspended day: the
//!   recorder drops them if there are); a month rate is the rate times the
//!   days of the month the deployment ran and was not suspended, over the
//!   calendar days of the month (3,000 x 20/30 = 2,000 for a third of the
//!   month suspended); no minimum hours are charged for them.
//! - **Hours no rate card prices** (no card, no accepted version in force that
//!   day, no line for the role and shift) are not billed; the accountants are
//!   told which days of which timesheet.
//! - **`MPW.RUL.05`** ([`run`], daily at 03:00 Asia/Dubai): at the end of a
//!   contract's billing cycle (the first day of the month for a monthly
//!   cycle, every Monday for a weekly one, every second Monday from the
//!   contract's first week for a fortnightly one), and every day for a contract
//!   that is `ended` (its last timesheets are billed after their approval,
//!   whenever it comes), the unbilled usage of the client becomes a draft
//!   invoice per issuing entity and currency, and the accountants are told
//!   (`UsageRecorder::bill`). Decision: billing is per client (the recorder
//!   bills a party), so a client with two contracts is billed for both at the
//!   first cycle end of either. A second run finds nothing unbilled and does
//!   nothing.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_role;
use renox::chrono::{Datelike, NaiveDate, Weekday};
use renox::prelude::*;
use wl_kernel::attendance::TimesheetApproved;
use wl_kernel::documents::today;
use wl_kernel::parties::directory;
use wl_kernel::timesheets::timesheets;
use wl_kernel::usage::{BilledInvoice, DaySpan, TimesheetBilling, UsageRecorded, usage_recorder};
use wl_kernel::{PartyId, Subject, role_tasks};

use crate::contract::{BillingCycle, ClientContract, ContractStatus};
use crate::deployment::{self, Deployment};
use crate::deployments::worker_name;
use crate::notices;
use crate::seed;
use crate::suspension::SuspensionSpan;
use crate::texts::{english, money};

/// The name of the daily scheduled task of `MPW.RUL.05`.
pub const TASK: &str = "manpower:billing-daily";

/// The accountants: the holders of the role, else whom the kernel picks.
async fn accountants(state: &AppState) -> Result<Vec<i64>> {
    let mut ids: Vec<i64> = users_with_role(&state.db, seed::ACCOUNTANT)
        .await?
        .iter()
        .map(|user| user.id)
        .collect();
    if ids.is_empty()
        && let Some(user) = role_tasks::assignee(&state.db, seed::ACCOUNTANT).await?
    {
        ids.push(user);
    }
    Ok(ids)
}

/// The timesheet of `event` as the recorder takes it: the deployment's
/// contract, role and shift, the days it ran and the days it was suspended,
/// and the billable hours. `None` for something that is not a regular
/// timesheet of a deployment, or whose hours are not there to bill.
///
/// # Errors
///
/// The database's errors.
pub async fn describe(
    state: &AppState,
    event: &TimesheetApproved,
) -> Result<Option<(Deployment, TimesheetBilling)>> {
    if event.kind != "regular" {
        return Ok(None);
    }
    let Some(context) = event
        .context
        .as_ref()
        .filter(|context| context.subject_type == deployment::SUBJECT_TYPE)
    else {
        return Ok(None);
    };
    let Some(found) = Deployment::find(&state.db, context.subject_id).await? else {
        return Ok(None);
    };
    let Some(hours) = timesheets(state)
        .billable_hours(state, &event.timesheet)
        .await?
    else {
        return Ok(None);
    };
    let spans = SuspensionSpan::of_deployment(&state.db, found.id).await?;
    let billing = TimesheetBilling {
        timesheet: event.timesheet.clone(),
        contract: Subject::new(crate::contract::SUBJECT_TYPE, found.client_contract_id),
        role: found.role.clone(),
        shift_type: found.shift_type.clone(),
        period_from: hours.period_from,
        period_to: hours.period_to,
        deployed: DaySpan::new(found.start_date, found.end_date),
        suspended: spans.iter().filter_map(SuspensionSpan::days).collect(),
        hours: hours.lines,
    };
    Ok(Some((found, billing)))
}

/// Records the approved hours of `event` as usage and tells the accountants of
/// the days no rate card prices. `None` when the event is not for a
/// deployment's regular timesheet.
///
/// # Errors
///
/// The database's and the recorder's errors.
pub async fn bill_approved(
    state: &AppState,
    event: &TimesheetApproved,
) -> Result<Option<UsageRecorded>> {
    let Some((found, billing)) = describe(state, event).await? else {
        return Ok(None);
    };
    let recorded = usage_recorder(state)
        .record_timesheet(state, &billing)
        .await?;
    if !recorded.unpriced.is_empty() {
        tell_unpriced(state, &found, &billing, &recorded).await?;
    }
    Ok(Some(recorded))
}

/// Tells the accountants which days of the timesheet are not billed.
async fn tell_unpriced(
    state: &AppState,
    found: &Deployment,
    billing: &TimesheetBilling,
    recorded: &UsageRecorded,
) -> Result {
    let worker = worker_name(state, "en", found.employee()).await;
    let days = recorded
        .unpriced
        .iter()
        .map(NaiveDate::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 6] = [
        ("worker", &worker),
        ("role", &found.role),
        ("shift", &found.shift_type),
        ("timesheet", &billing.timesheet),
        ("contract", &billing.contract),
        ("days", &days),
    ];
    let users = accountants(state).await?;
    notices::tell(
        state,
        billing.timesheet.clone(),
        &billing.timesheet.to_string(),
        &users,
        &english("manpower.billing.unpriced_title", &params),
        &english("manpower.billing.unpriced_body", &params),
    )
    .await?;
    Ok(())
}

/// Registers the listener of `TimesheetApproved`.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: TimesheetApproved, state: AppState| async move {
        bill_approved(&state, &event).await.map(|_| ())
    });
}

/// The Monday of the week of `day`.
fn monday_of(day: NaiveDate) -> NaiveDate {
    day - renox::chrono::Duration::days(i64::from(day.weekday().num_days_from_monday()))
}

/// Whether a billing cycle of `contract` ended the day before `on`.
pub fn cycle_ended(contract: &ClientContract, on: NaiveDate) -> bool {
    match contract.billing_cycle {
        BillingCycle::Monthly => on.day() == 1,
        BillingCycle::Weekly => on.weekday() == Weekday::Mon,
        BillingCycle::Fortnightly => {
            on.weekday() == Weekday::Mon
                && (monday_of(on) - monday_of(contract.start_date))
                    .num_weeks()
                    .rem_euclid(2)
                    == 0
        }
    }
}

/// Whether `contract` is billed on `on`: its cycle ended, or it has `ended`
/// and its last timesheets are billed as they are approved.
pub fn is_due(contract: &ClientContract, on: NaiveDate) -> bool {
    contract.status == ContractStatus::Ended || cycle_ended(contract, on)
}

/// Tells the accountants of a draft invoice.
async fn tell_drafted(state: &AppState, client: PartyId, invoice: &BilledInvoice) -> Result {
    let name = directory(state)
        .display_name(&state.db, client, "en")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{}", client.0));
    let total = money(invoice.net_total);
    let count = invoice.lines;
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] =
        [("client", &name), ("total", &total), ("count", &count)];
    let users = accountants(state).await?;
    notices::tell(
        state,
        invoice.invoice.clone(),
        &invoice.invoice.to_string(),
        &users,
        &english("manpower.billing.title", &params),
        &english("manpower.billing.body", &params),
    )
    .await?;
    Ok(())
}

/// Runs `MPW.RUL.05` on `on`: the clients whose contract is due have their
/// unbilled usage drafted as invoices, and the accountants are told. Returns
/// the drafts.
///
/// # Errors
///
/// The first error of a client's billing; the other clients are billed.
pub async fn run(state: &AppState, on: NaiveDate) -> Result<Vec<BilledInvoice>> {
    let contracts = ClientContract::query()
        .where_(ClientContract::STATUS.is_in([
            ContractStatus::Active,
            ContractStatus::OnHold,
            ContractStatus::Terminating,
            ContractStatus::Ended,
        ]))
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?;
    let clients: BTreeSet<i64> = contracts
        .iter()
        .filter(|contract| is_due(contract, on))
        .map(|contract| contract.client_party_id)
        .collect();
    let recorder = usage_recorder(state);
    let mut drafted = Vec::new();
    let mut failed: Option<Error> = None;
    for client in clients {
        let client = PartyId(client);
        match recorder.bill(state, client).await {
            Ok(invoices) => {
                for invoice in invoices {
                    if let Err(error) = tell_drafted(state, client, &invoice).await {
                        failed.get_or_insert(error);
                    }
                    drafted.push(invoice);
                }
            }
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    match failed {
        Some(error) => Err(error),
        None => Ok(drafted),
    }
}

/// The daily run at 03:00 Asia/Dubai.
///
/// # Errors
///
/// As [`run`].
pub async fn daily(state: &AppState) -> Result {
    run(state, today()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn contract(cycle: BillingCycle, status: ContractStatus) -> ClientContract {
        ClientContract {
            billing_cycle: cycle,
            status,
            start_date: day(2026, 11, 3),
            ..ClientContract::default()
        }
    }

    #[test]
    fn a_monthly_cycle_ends_on_the_first_of_the_next_month() {
        let monthly = contract(BillingCycle::Monthly, ContractStatus::Active);
        assert!(cycle_ended(&monthly, day(2026, 12, 1)));
        assert!(!cycle_ended(&monthly, day(2026, 12, 2)));
    }

    #[test]
    fn a_weekly_cycle_ends_on_monday_and_a_fortnightly_one_every_second_monday() {
        let weekly = contract(BillingCycle::Weekly, ContractStatus::Active);
        assert!(cycle_ended(&weekly, day(2026, 12, 7)));
        assert!(!cycle_ended(&weekly, day(2026, 12, 8)));
        let fortnightly = contract(BillingCycle::Fortnightly, ContractStatus::Active);
        // The contract starts in the week of Monday 2 November.
        assert!(cycle_ended(&fortnightly, day(2026, 11, 16)));
        assert!(!cycle_ended(&fortnightly, day(2026, 11, 23)));
        assert!(cycle_ended(&fortnightly, day(2026, 11, 30)));
    }

    #[test]
    fn an_ended_contract_is_billed_on_any_day() {
        let ended = contract(BillingCycle::Monthly, ContractStatus::Ended);
        assert!(is_due(&ended, day(2026, 12, 4)));
        let active = contract(BillingCycle::Monthly, ContractStatus::Active);
        assert!(!is_due(&active, day(2026, 12, 4)));
    }
}
