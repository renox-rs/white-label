//! What `cancel_onboarding`, `withdraw_notice` and `reinstate` check in the
//! database and change on the employee (wl-hr §4 *Employee*, §5.4, §10; issues
//! #195, #196, #198). The guard on the transition checks the reason and the
//! documents ([`crate::exits::missing`]); a guard cannot read the database, so
//! the rest is [`problem_in`], run at the request, again at the final approval
//! (the kernel's final-check hook) and a last time as the first effect of the
//! transition, in its transaction. What reaches other records follows the
//! commit, in the listener of [`crate::employee_machine`]: the settlement and
//! the cancellation case are cancelled by the system and the event is emitted.
//!
//! - **`cancel_onboarding`.** No *new* visa case is still live (it is `cancelled`
//!   or `rejected`). When a work permit or entry permit was issued (a step of the
//!   *new* case is done), a *cancellation* case is `completed` or *cancellation
//!   not needed* was recorded on the *new* case. When the employee has
//!   attendance, a final settlement with the reason *onboarding cancelled* is
//!   `paid` (or `closed`); without attendance none is needed. The attendance is
//!   read at the request ([`crate::exits::ExitInput::has_attendance`]) and, as
//!   the check at the approval has only the database, kept up to date from
//!   `AttendanceRecorded` in [`Employee::first_attended_on`]
//!   ([`on_attendance_recorded`]): a check-in between the request and the
//!   approval counts at the approval. The effect sets the exit
//!   route and closes the portal account; fees and onboarding costs stay on the
//!   employee as a cost object and nothing is recovered from the person (Art.
//!   6(4)): no adjustment or receivable is made.
//! - **`withdraw_notice` and `reinstate`.** The *cancellation* case is not
//!   `completed`, and the employee's settlement is not `paid`, `closed` or
//!   `disputed` and has no payment recorded against it (otherwise the notice
//!   stands, or the worker is rehired as a new employee). The effect clears the
//!   last working day; `reinstate` also marks the absence report `withdrawn`
//!   with the reference and the day the worker returned, and clears the exit
//!   route.

use renox::chrono::NaiveDate;
use renox::db::{Transaction, now};
use renox::prelude::*;

use wl_kernel::attendance::AttendanceRecorded;

use crate::absence::AbsenceReport;
use crate::employee::Employee;
use crate::employee_machine::{CANCEL_ONBOARDING, REINSTATE, WITHDRAW_NOTICE};
use crate::exits::ExitInput;
use crate::settlement::{FinalSettlement, REASON_ONBOARDING_CANCELLED, SettlementStatus};
use crate::settlement_payment::paid_total_in;
use crate::texts::english;
use crate::visa::{self, TYPE_CANCELLATION, TYPE_NEW, VisaCase, VisaCaseStatus};

/// The exit route of an employee whose onboarding was cancelled.
pub const ROUTE_ONBOARDING_CANCELLED: &str = "onboarding_cancelled";

/// Whether `transition` has checks that read the database.
pub fn has_checks(transition: &str) -> bool {
    [CANCEL_ONBOARDING, WITHDRAW_NOTICE, REINSTATE].contains(&transition)
}

fn text(key: &str) -> String {
    english(&format!("wl_hr.exits.{key}"), &[])
}

/// The refusal as an HTTP 422.
fn refusal(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// Why `transition` cannot run for `employee_id` now, in English, or `None`
/// when the database allows it.
pub async fn problem_in(
    tx: &mut Transaction,
    employee_id: i64,
    transition: &str,
    input: &ExitInput,
) -> Result<Option<String>> {
    match transition {
        CANCEL_ONBOARDING => cancel_onboarding_problem(tx, employee_id, input).await,
        WITHDRAW_NOTICE | REINSTATE => {
            let cancellations =
                VisaCase::of_type_in(&mut *tx, employee_id, TYPE_CANCELLATION).await?;
            if cancellations
                .iter()
                .any(|case| case.status == VisaCaseStatus::Completed)
            {
                return Ok(Some(text("cancellation_completed")));
            }
            settlement_problem_in(tx, employee_id).await
        }
        _ => Ok(None),
    }
}

async fn cancel_onboarding_problem(
    tx: &mut Transaction,
    employee_id: i64,
    input: &ExitInput,
) -> Result<Option<String>> {
    let new_cases = VisaCase::of_type_in(&mut *tx, employee_id, TYPE_NEW).await?;
    if new_cases.iter().any(VisaCase::is_live) {
        return Ok(Some(text("new_case_open")));
    }
    if visa::permit_issued_in(&mut *tx, employee_id).await? {
        let completed = VisaCase::of_type_in(&mut *tx, employee_id, TYPE_CANCELLATION)
            .await?
            .iter()
            .any(|case| case.status == VisaCaseStatus::Completed);
        let not_needed = new_cases
            .iter()
            .any(|case| case.cancellation_not_needed_on.is_some());
        if !completed && !not_needed {
            return Ok(Some(text("cancellation_required")));
        }
    }
    let attended_since_request = Employee::find(&mut *tx, employee_id)
        .await?
        .is_some_and(|employee| employee.first_attended_on.is_some());
    if input.has_attendance == Some(true) || attended_since_request {
        let settled = FinalSettlement::of_employee_in(&mut *tx, employee_id)
            .await?
            .iter()
            .any(|s| {
                s.reason == REASON_ONBOARDING_CANCELLED
                    && matches!(s.status, SettlementStatus::Paid | SettlementStatus::Closed)
            });
        if !settled {
            return Ok(Some(text("settlement_required")));
        }
    }
    Ok(None)
}

/// A settlement that stops `withdraw_notice` and `reinstate`: `paid`,
/// `closed` or `disputed`, or one with a payment recorded against it. A
/// cancelled one does not count.
async fn settlement_problem_in(tx: &mut Transaction, employee_id: i64) -> Result<Option<String>> {
    for settlement in FinalSettlement::of_employee_in(&mut *tx, employee_id).await? {
        match settlement.status {
            SettlementStatus::Paid | SettlementStatus::Closed => {
                return Ok(Some(text("settlement_paid")));
            }
            SettlementStatus::Disputed => return Ok(Some(text("settlement_disputed"))),
            SettlementStatus::Draft | SettlementStatus::Approved => {
                if paid_total_in(&mut *tx, settlement.id).await? > 0 {
                    return Ok(Some(text("settlement_part_paid")));
                }
            }
            SettlementStatus::Cancelled => {}
        }
    }
    Ok(None)
}

/// `AttendanceRecorded`: remembers the first day the employee has an
/// attendance record (from the hire date on), for the check of
/// `cancel_onboarding` at its approval.
pub async fn on_attendance_recorded(state: &AppState, event: &AttendanceRecorded) -> Result {
    let Some(employee) = Employee::find(&state.db, event.employee.0).await? else {
        return Ok(());
    };
    if employee.first_attended_on.is_some() || event.date < employee.hire_date {
        return Ok(());
    }
    Employee::query()
        .where_(Employee::ID.eq(employee.id))
        .update(&state.db, &[("first_attended_on", &Some(event.date))])
        .await?;
    Ok(())
}

/// The effect of `cancel_onboarding`, `withdraw_notice` and `reinstate` in the
/// transition's transaction: the checks once more, then the changes on the
/// employee.
pub async fn apply(
    tx: &mut Transaction,
    employee: &Employee,
    transition: &str,
    input: &ExitInput,
) -> Result {
    if let Some(message) = problem_in(&mut *tx, employee.id, transition, input).await? {
        return Err(refusal(message));
    }
    let id = employee.id;
    match transition {
        CANCEL_ONBOARDING => {
            let closed = employee.user_id.map(|_| now());
            Employee::query()
                .where_(Employee::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("exit_route", &Some(ROUTE_ONBOARDING_CANCELLED.to_owned())),
                        ("portal_closed_at", &closed),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        WITHDRAW_NOTICE => {
            Employee::query()
                .where_(Employee::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("last_working_day", &None::<NaiveDate>),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        REINSTATE => {
            let reference = input
                .withdrawal_reference
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .to_owned();
            AbsenceReport::query()
                .where_(AbsenceReport::EMPLOYEE_ID.eq(id))
                .where_(AbsenceReport::STATUS.eq("valid"))
                .update(
                    &mut *tx,
                    &[
                        ("status", &"withdrawn".to_owned()),
                        ("withdrawal_reference", &Some(reference)),
                        ("returned_on", &input.return_date),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            Employee::query()
                .where_(Employee::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("last_working_day", &None::<NaiveDate>),
                        ("exit_route", &None::<String>),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_exits_have_database_checks() {
        assert!(has_checks(CANCEL_ONBOARDING));
        assert!(has_checks(WITHDRAW_NOTICE));
        assert!(has_checks(REINSTATE));
        assert!(!has_checks("report_absconding"));
    }
}
