//! `exit` of an absconded or deceased employee (wl-hr §4 *Employee*, §5.4,
//! §6 *Events emitted*, issue #200): the end of employment without the
//! standard guards. An absconded worker never signs and a deceased one cannot,
//! so the exit needs no signed acknowledgement, and for an absconded worker no
//! payment either.
//!
//! - **The checks** ([`problem_in`]) read the database, so they are the first
//!   effect of the transition, in its transaction (a guard cannot read it).
//!   From `absconded` or `deceased`: the *cancellation* visa case is
//!   `completed`, or the authority's cancellation reference is recorded
//!   ([`record_cancellation_reference`]); and the final settlement is
//!   `approved`, `paid` or `closed` for an absconded worker (`disputed` blocks
//!   the exit) or `paid` or `closed` for a deceased one (the estate was
//!   paid; `disputed` blocks it too). From `notice` (the standard exit, which
//!   the base owns; only the checks the spec names are here): the settlement is
//!   `paid` or `closed` and a *cancellation* case, when one exists, is
//!   `completed`.
//! - **The effect** sets the exit route (`standard`, `absconding` or `death`)
//!   and the day. The portal is read-only from that day and closes
//!   [`crate::HrConfig::portal_days_after_exit`] days later (default 60,
//!   `POR.ACL.01`): [`crate::workforce`] answers `read_only` and
//!   `access_disabled` from the day, so no job is needed.
//! - **After the commit** `EmployeeExited` carries the exit date and the route
//!   ([`after_commit`]).

use renox::db::{Transaction, now};
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeExited;

use crate::employee::{Employee, EmployeeStatus};
use crate::exits;
use crate::settlement::{FinalSettlement, SettlementStatus};
use crate::texts::english;
use crate::visa::{TYPE_CANCELLATION, VisaCase, VisaCaseStatus};

/// The `exit` transition.
pub const EXIT: &str = "exit";

/// The exit route of the standard exit from `notice`.
pub const ROUTE_STANDARD: &str = "standard";
/// The exit route of an absconded employee.
pub const ROUTE_ABSCONDING: &str = "absconding";
/// The exit route of a deceased employee.
pub const ROUTE_DEATH: &str = "death";

/// The audit action of a recorded cancellation reference.
pub const AUDIT_REFERENCE: &str = "hr.cancellation_reference";

fn text(key: &str) -> String {
    english(&format!("wl_hr.exit.{key}"), &[])
}

fn refusal(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The exit route of an employee who leaves from `status`, or `None` for a
/// state `exit` does not leave.
pub fn route_of(status: EmployeeStatus) -> Option<&'static str> {
    match status {
        EmployeeStatus::Notice => Some(ROUTE_STANDARD),
        EmployeeStatus::Absconded => Some(ROUTE_ABSCONDING),
        EmployeeStatus::Deceased => Some(ROUTE_DEATH),
        _ => None,
    }
}

/// Why `employee` cannot exit now, in English, or `None` when the database
/// allows it.
pub async fn problem_in(tx: &mut Transaction, employee: &Employee) -> Result<Option<String>> {
    let Some(route) = route_of(employee.status) else {
        return Ok(Some(text("wrong_state")));
    };
    let cases = VisaCase::of_type_in(&mut *tx, employee.id, TYPE_CANCELLATION).await?;
    let completed = cases
        .iter()
        .any(|case| case.status == VisaCaseStatus::Completed);
    let reference = employee
        .cancellation_reference
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty());
    if route == ROUTE_STANDARD {
        // The standard exit: a cancellation case, when there is one, is done.
        let live = cases.iter().any(VisaCase::is_live);
        if live && !completed {
            return Ok(Some(text("cancellation_required")));
        }
    } else if !completed && !reference {
        return Ok(Some(text("cancellation_required")));
    }
    let settlements = FinalSettlement::of_employee_in(&mut *tx, employee.id).await?;
    let current: Vec<&FinalSettlement> = settlements
        .iter()
        .filter(|s| s.status != SettlementStatus::Cancelled)
        .collect();
    if current
        .iter()
        .any(|s| s.status == SettlementStatus::Disputed)
    {
        return Ok(Some(text("settlement_disputed")));
    }
    let (allowed, key): (&[SettlementStatus], &str) = if route == ROUTE_ABSCONDING {
        (
            &[
                SettlementStatus::Approved,
                SettlementStatus::Paid,
                SettlementStatus::Closed,
            ],
            "settlement_approved_required",
        )
    } else {
        (
            &[SettlementStatus::Paid, SettlementStatus::Closed],
            "settlement_paid_required",
        )
    };
    if !current.iter().any(|s| allowed.contains(&s.status)) {
        return Ok(Some(text(key)));
    }
    Ok(None)
}

/// The effect of `exit`, inside the transition's transaction: the checks once
/// more, then the exit route and the day.
pub async fn apply(tx: &mut Transaction, employee: &Employee) -> Result {
    if let Some(message) = problem_in(&mut *tx, employee).await? {
        return Err(refusal(message));
    }
    let route = route_of(employee.status).unwrap_or(ROUTE_STANDARD);
    Employee::query()
        .where_(Employee::ID.eq(employee.id))
        .update(
            &mut *tx,
            &[
                ("exit_route", &Some(route.to_owned())),
                ("exited_on", &Some(today())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// After the commit of `exit` for employee `id`: emits `EmployeeExited`.
pub async fn after_commit(state: &AppState, id: i64) -> Result {
    let Some(employee) = Employee::find(&state.db, id).await? else {
        return Ok(());
    };
    state
        .emit(EmployeeExited {
            employee: employee.employee_id(),
            exit_date: employee.exited_on.unwrap_or_else(today),
            route: employee
                .exit_route
                .clone()
                .unwrap_or_else(|| ROUTE_STANDARD.to_owned()),
        })
        .await
}

/// `by` (a holder of `employees.manage`) records the authority's reference of
/// the cancellation of `employee_id`'s residence visa, for an employee whose
/// *cancellation* case was not completed in the system (the authority did it).
/// It replaces an earlier reference and is audited.
///
/// # Errors
///
/// `Forbidden` without `employees.manage`; 422 for a blank reference or an
/// employee who is not in `notice`, `absconded` or `deceased`.
pub async fn record_cancellation_reference(
    state: &AppState,
    by: &AuthUser,
    employee_id: i64,
    reference: &str,
) -> Result<Employee> {
    let employee = Employee::find_or_404(&state.db, employee_id).await?;
    if !exits::holds_manage(state, by.id, &employee).await? {
        return Err(Error::Forbidden);
    }
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(refusal(text("reference_required")));
    }
    if route_of(employee.status).is_none() {
        return Err(refusal(text("wrong_state")));
    }
    Employee::query()
        .where_(Employee::ID.eq(employee_id))
        .update(
            &state.db,
            &[
                ("cancellation_reference", &Some(reference.to_owned())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    renox::audit::record(
        &state.db,
        renox::audit::Entry::new(AUDIT_REFERENCE)
            .user(by.id)
            .subject(Employee::TABLE, employee_id)
            .data(json!({ "reference": reference })),
    )
    .await?;
    Employee::find_or_404(&state.db, employee_id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_follows_the_state_the_employee_leaves() {
        assert_eq!(route_of(EmployeeStatus::Notice), Some("standard"));
        assert_eq!(route_of(EmployeeStatus::Absconded), Some("absconding"));
        assert_eq!(route_of(EmployeeStatus::Deceased), Some("death"));
        assert_eq!(route_of(EmployeeStatus::Active), None);
    }
}
