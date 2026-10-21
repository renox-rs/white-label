//! `record_death` (wl-hr §4 *Employee*, §5.4 *Death*, §10, issue #199): HR
//! records that an employee died in service, with the date of death and the
//! death certificate (or the authority's notice); another user with
//! `employees.exit.approve` approves (`HRM.ACL.07`, [`crate::exits`]); only
//! then the transition runs.
//!
//! - **The effects** ([`apply`]), in the transition's transaction: the date of
//!   death is kept on the employee and the portal account is closed; the
//!   payroll lines in runs not yet `submitted` are held with the reason
//!   *death* ([`payroll::hold_for_death_in`], also the ones an absence case or
//!   report held: nothing releases them) and the salary each owes up to the
//!   date of death ([`payroll::owed_until`]) moves into the final settlement as
//!   unpaid salary, so it is paid once: the line is marked *settled in the
//!   final settlement* and `HRM.RUL.12` never proposes an off-cycle run for it
//!   ([`crate::release`] releases only absence case and absence report holds);
//!   leave requests `requested` or `approved` that have not started are
//!   cancelled by the system; the final settlement is drafted with the payee
//!   *estate*, or the open one (from `notice` or `absconded`) gets the reason
//!   *death* and the payee *estate* and no second is drafted; the *cancellation*
//!   visa case opens from the *death* variant of the step template (it starts
//!   from the death certificate).
//! - **After the commit** ([`after_commit`]): every pending and future message
//!   to the employee is stopped through the notifier contract; the HR task list
//!   (visa cancellation, insurance claim, repatriation) opens, once each; and
//!   `EmployeeDeceased` is emitted for the apps to end the deployments.
//!
//! Decisions. The date of death is optional in the request and defaults to the
//! day of the approval (the guard of the existing request needs the document,
//! and an earlier issue accepts a request without the date); a date in the
//! future is refused. A settlement that was already `approved` takes the salary
//! too, with a new statement version, because the amount owed cannot wait for a
//! second approval; `pay` re-reads the net. Who receives the estate's payment
//! is `⚠ open: lawyer (L-25)`: the representative and the document giving the
//! right are recorded on the settlement and `pay` is refused until they are.

use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::notify::{self, NotifyRecipient};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::workforce::EmployeeDeceased;

use crate::config::config_of;
use crate::employee::Employee;
use crate::exits::ExitInput;
use crate::leave::{LeaveRequest, LeaveStatus};
use crate::settlement::{FinalSettlement, PAYEE_ESTATE, REASON_DEATH, SettlementStatus};
use crate::settlement_offset::recalculate_in;
use crate::texts::english;
use crate::{notices, payroll, settlement, visa};

/// Why the messages to the employee stop.
pub const STOP_REASON: &str = "employee_deceased";

/// The tasks of the HR task list, with the key part of each.
pub const TASKS: [&str; 3] = ["visa_cancellation", "insurance_claim", "repatriation"];

/// The source key of the HR task `task` of employee `id`.
pub fn task_key(id: i64, task: &str) -> String {
    format!("hr.death:{id}:{task}")
}

/// The effect of `record_death`, inside the transition's transaction.
pub async fn apply(tx: &mut Transaction, employee: &Employee, input: &ExitInput) -> Result {
    let id = employee.id;
    let day = input.death_date.unwrap_or_else(wl_kernel::documents::today);
    let closed = employee
        .portal_closed_at
        .or(employee.user_id.map(|_| now()));
    Employee::query()
        .where_(Employee::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("date_of_death", &Some(day)),
                ("portal_closed_at", &closed),
                ("updated_at", &now()),
            ],
        )
        .await?;
    cancel_unstarted_leave_in(tx, id).await?;

    let held = payroll::hold_for_death_in(&mut *tx, id).await?;
    let drafted = settlement::draft_or_relabel_in(&mut *tx, id, REASON_DEATH, PAYEE_ESTATE).await?;
    FinalSettlement::query()
        .where_(FinalSettlement::ID.eq(drafted.id))
        .update(
            &mut *tx,
            &[("last_working_day", &Some(day)), ("updated_at", &now())],
        )
        .await?;
    let mut moved = 0;
    for (line, period) in held {
        let owed = payroll::owed_until(line.net_amount, &period, day);
        payroll::PayrollLine::query()
            .where_(payroll::PayrollLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("settled_in_settlement", &Some(drafted.id)),
                    ("settled_amount", &owed),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        moved += owed;
    }
    if moved > 0 {
        let bump = i64::from(drafted.status == SettlementStatus::Approved);
        FinalSettlement::query()
            .where_(FinalSettlement::ID.eq(drafted.id))
            .update(
                &mut *tx,
                &[
                    ("unpaid_salary", &(drafted.unpaid_salary + moved)),
                    ("statement_version", &(drafted.statement_version + bump)),
                    ("updated_at", &now()),
                ],
            )
            .await?;
        recalculate_in(&mut *tx, drafted.id).await?;
    }
    visa::open_death_in(&mut *tx, id).await?;
    Ok(())
}

/// Leave requests `requested` or `approved` that have not started are
/// cancelled by the system.
async fn cancel_unstarted_leave_in(tx: &mut Transaction, employee_id: i64) -> Result {
    LeaveRequest::query()
        .where_(LeaveRequest::EMPLOYEE_ID.eq(employee_id))
        .where_(LeaveRequest::STATUS.is_in([LeaveStatus::Requested, LeaveStatus::Approved]))
        .where_(LeaveRequest::FROM_DATE.gt(wl_kernel::documents::today()))
        .update(
            &mut *tx,
            &[
                ("status", &LeaveStatus::Cancelled.as_str().to_owned()),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// After the commit of `record_death` for employee `id`: stops the messages,
/// opens the HR tasks and emits `EmployeeDeceased`.
pub async fn after_commit(state: &AppState, id: i64) -> Result {
    let Some(employee) = Employee::find(&state.db, id).await? else {
        return Ok(());
    };
    let mut recipients = vec![NotifyRecipient::Party {
        id: employee.party(),
    }];
    if let Some(user) = employee.user_id {
        recipients.push(NotifyRecipient::User { id: user });
    }
    notify::notifier(state)
        .stop_messages_to(state, recipients, STOP_REASON)
        .await?;
    open_tasks(state, &employee).await?;
    let date = employee
        .date_of_death
        .unwrap_or_else(wl_kernel::documents::today);
    state
        .emit(EmployeeDeceased {
            employee: employee.employee_id(),
            date,
        })
        .await
}

/// The HR task list, once each: the visa cancellation, the insurance claim
/// and the repatriation.
async fn open_tasks(state: &AppState, employee: &Employee) -> Result {
    let config = config_of(state);
    let name = notices::name_of(state, employee).await;
    for task in TASKS {
        let title = english(&format!("wl_hr.death.task_{task}"), &[("name", &name)]);
        let description = english(&format!("wl_hr.death.task_{task}_description"), &[]);
        let item = RoleTask::new(
            employee.subject(),
            &config.hr_role,
            &title,
            &task_key(employee.id, task),
        )
        .description(&description);
        role_tasks::open(state, item).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::chrono::NaiveDate;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn the_salary_owed_is_pro_rata_over_the_calendar_days_of_the_period() {
        assert_eq!(payroll::owed_until(300_000, "2026-11", day(10)), 100_000);
        assert_eq!(payroll::owed_until(300_000, "2026-11", day(30)), 300_000);
        assert_eq!(payroll::owed_until(300_000, "2026-11", day(1)), 10_000);
    }

    #[test]
    fn nothing_is_owed_for_a_period_after_the_death_and_all_for_one_before() {
        assert_eq!(payroll::owed_until(300_000, "2026-12", day(10)), 0);
        assert_eq!(payroll::owed_until(300_000, "2026-10", day(10)), 300_000);
        assert_eq!(payroll::owed_until(300_000, "not a period", day(10)), 0);
    }

    #[test]
    fn the_hr_task_list_has_one_key_per_task() {
        let keys: Vec<String> = TASKS.iter().map(|t| task_key(7, t)).collect();
        assert_eq!(
            keys,
            [
                "hr.death:7:visa_cancellation",
                "hr.death:7:insurance_claim",
                "hr.death:7:repatriation"
            ]
        );
    }
}
