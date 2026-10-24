//! The leave request services (wl-hr §4 *Leave request*, §5.2, base of
//! BL-HRM-02, issue #192): `request`, `approve`, `reject`, `cancel` and
//! `complete_ended`, each of which checks who may act and then runs the
//! transition of [`crate::leave_machine`].
//!
//! Who acts (§3, §6):
//!
//! - `request`: the employee on their own account, or a holder of
//!   `leave.approve` in the employee's scopes (HR records it).
//! - `approve` and `reject`: a holder of `leave.approve` in the employee's
//!   scopes.
//! - `cancel`: the employee on their own account, or a holder of
//!   `leave.approve`; the start date must not be reached.
//!
//! After the commit a listener turns `approve` into the kernel's
//! `LeaveApproved` and `cancel` into `LeaveCancelled` (the calendars of
//! `wl-scheduling` and the rosters follow them) and moves the due dates of
//! the employee's open onboarding steps ([`crate::leave::sync_due_dates`]).
//!
//! Decision: the balance model (entitlement, accrual, carry-over) is a later
//! issue, so `approve` marks days unpaid only where this base can know them:
//! all days of `unpaid` leave and the days of sick leave beyond the paid
//! bands ([`sick_unpaid_days`]). Annual leave is never short here.

use renox::auth::permissions::{users_with_permission_in, users_with_role_in};
use renox::chrono::{Datelike, Months, NaiveDate};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::calendar::{BusinessCalendar, calendar};
use wl_kernel::documents::today;
use wl_kernel::workforce::{LeaveApproved, LeaveCancelled};

use crate::assignees;
use crate::config::config_of;
use crate::employee::Employee;
use crate::leave::{self, LeaveRequest, LeaveStatus};
use crate::leave_machine::{APPROVE, CANCEL, COMPLETE, LeaveInput, REJECT};
use crate::permissions::LEAVE_APPROVE;
use crate::texts::english;

/// The leave types the module knows (§7 *Leave policies*).
pub const LEAVE_TYPES: [&str; 8] = [
    "annual",
    "sick",
    "maternity",
    "parental",
    "bereavement",
    "hajj",
    "study",
    "unpaid",
];

/// Working days of sick leave paid in a year: 15 at full pay and 30 at half
/// pay (ER Art. 31); the rest of the 90 are unpaid.
pub const PAID_SICK_DAYS: i64 = 45;

/// Months of probation, in which sick leave is unpaid (Art. 31).
pub const PROBATION_MONTHS: u32 = 6;

/// A 422 with the text of `key`.
pub(crate) fn refuse(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, english(key, &[]))
}

/// The working days from `from` to `to`, both ends included.
pub fn working_days(calendar: &BusinessCalendar, from: NaiveDate, to: NaiveDate) -> i64 {
    let mut count = 0;
    let mut day = from;
    while day <= to {
        if calendar.is_working_day(day) {
            count += 1;
        }
        let Some(next) = day.succ_opt() else { break };
        day = next;
    }
    count
}

/// The working days of `days` that are unpaid sick leave: all of them in the
/// probation, else what exceeds the paid days still left after `taken`
/// working days of sick leave earlier in the year.
pub fn sick_unpaid_days(taken: i64, days: i64, probation: bool) -> i64 {
    if probation {
        return days.max(0);
    }
    let paid_left = (PAID_SICK_DAYS - taken).max(0);
    (days - paid_left).max(0)
}

/// Whether `user` holds `leave.approve` in one of the employee's scopes
/// (globally, when it has none).
pub async fn holds_approve(state: &AppState, user: i64, employee: &Employee) -> Result<bool> {
    let scopes = assignees::scopes_of(&state.db, &employee.scopes(&state.db).await?).await?;
    for scope in &scopes {
        if users_with_permission_in(&state.db, LEAVE_APPROVE, scope)
            .await?
            .iter()
            .any(|u| u.id == user)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `user` holds the manager role in one of the employee's scopes
/// (§3 *The employee's manager*). Unlike the `manager` owner of an
/// onboarding step, HR does not stand in for a missing manager.
pub async fn is_manager_of(state: &AppState, user: i64, employee: &Employee) -> Result<bool> {
    let role = config_of(state).manager_role;
    let scopes = assignees::scopes_of(&state.db, &employee.scopes(&state.db).await?).await?;
    for scope in &scopes {
        if users_with_role_in(&state.db, &role, scope)
            .await?
            .iter()
            .any(|u| u.id == user)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The working days of sick leave the employee has approved in the year of
/// `on`, other than request `except`.
async fn sick_taken(state: &AppState, employee: i64, on: NaiveDate, except: i64) -> Result<i64> {
    let cal = calendar(state);
    let mut taken = 0;
    for other in LeaveRequest::query()
        .where_(LeaveRequest::EMPLOYEE_ID.eq(employee))
        .where_(LeaveRequest::LEAVE_TYPE.eq("sick".to_owned()))
        .where_(LeaveRequest::STATUS.is_in([LeaveStatus::Approved, LeaveStatus::Completed]))
        .get(&state.db)
        .await?
    {
        if other.id != except && other.from_date.year() == on.year() {
            taken += working_days(&cal, other.from_date, other.to_date);
        }
    }
    Ok(taken)
}

/// The unpaid working days of `leave`, as `approve` marks them.
pub async fn unpaid_days_of(state: &AppState, leave: &LeaveRequest) -> Result<i64> {
    let cal = calendar(state);
    let days = working_days(&cal, leave.from_date, leave.to_date);
    match leave.leave_type.as_str() {
        "unpaid" => Ok(days),
        "sick" => {
            let employee = Employee::find_or_404(&state.db, leave.employee_id).await?;
            let probation = employee
                .hire_date
                .checked_add_months(Months::new(PROBATION_MONTHS))
                .is_some_and(|end| leave.from_date < end);
            let taken = sick_taken(state, leave.employee_id, leave.from_date, leave.id).await?;
            Ok(sick_unpaid_days(taken, days, probation))
        }
        _ => Ok(0),
    }
}

/// The kernel event of an approved `leave`.
pub fn approved_event(leave: &LeaveRequest) -> LeaveApproved {
    LeaveApproved {
        leave: leave.subject(),
        employee: leave.employee(),
        leave_type: leave.leave_type.clone(),
        from: leave.from_date,
        to: leave.to_date,
        unpaid: leave.leave_type == "unpaid",
    }
}

/// The kernel event of a cancelled `leave`.
pub fn cancelled_event(leave: &LeaveRequest) -> LeaveCancelled {
    LeaveCancelled {
        employee: leave.employee(),
        request: leave.id,
        leave_type: leave.leave_type.clone(),
        from: leave.from_date,
        to: leave.to_date,
    }
}

/// `by` asks for leave of `employee_id` from `from` to `to` (the request
/// starts `requested`).
///
/// # Errors
///
/// `Forbidden` for a user who is neither the employee nor a holder of
/// `leave.approve`; 422 for an unknown type or dates the wrong way round.
pub async fn request(
    state: &AppState,
    by: i64,
    employee_id: i64,
    leave_type: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<LeaveRequest> {
    let employee = Employee::find(&state.db, employee_id)
        .await?
        .ok_or(Error::NotFound)?;
    if employee.user_id != Some(by) && !holds_approve(state, by, &employee).await? {
        return Err(Error::Forbidden);
    }
    let leave_type = leave_type.trim().to_lowercase();
    if !LEAVE_TYPES.contains(&leave_type.as_str()) {
        return Err(refuse("wl_hr.leave.problem.type"));
    }
    if to < from {
        return Err(refuse("wl_hr.leave.problem.dates"));
    }
    let mut tx = state.db.begin().await?;
    let leave = LeaveRequest::create(
        &mut tx,
        LeaveRequest {
            employee_id,
            leave_type,
            from_date: from,
            to_date: to,
            status: LeaveStatus::Requested,
            requested_by: Some(by),
            ..LeaveRequest::default()
        },
    )
    .await?;
    // Found from now on (BL-AGT-11).
    crate::search::index(&mut tx, crate::leave::LEAVE_SUBJECT_TYPE, leave.id).await?;
    tx.commit().await?;
    Ok(leave)
}

/// Loads request `id` and its employee.
async fn load(state: &AppState, id: i64) -> Result<(LeaveRequest, Employee)> {
    let leave = LeaveRequest::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let employee = Employee::find_or_404(&state.db, leave.employee_id).await?;
    Ok((leave, employee))
}

/// `by` approves request `id`.
///
/// # Errors
///
/// `Forbidden` without `leave.approve` in the employee's scopes; 422 when
/// the request is not `requested` or its sick note is missing.
pub async fn approve(state: &AppState, by: i64, id: i64) -> Result<LeaveRequest> {
    let (leave, employee) = load(state, id).await?;
    if !holds_approve(state, by, &employee).await? {
        return Err(Error::Forbidden);
    }
    let input = LeaveInput {
        by: Some(by),
        today: None,
        has_document: leave.sick_note_document_id.is_some(),
        unpaid_days: unpaid_days_of(state, &leave).await?,
    };
    let _outcome = leave
        .transition(APPROVE)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    LeaveRequest::find_or_404(&state.db, id).await
}

/// `by` rejects request `id` for `reason`.
///
/// # Errors
///
/// `Forbidden` without `leave.approve` in the employee's scopes; 422 without
/// a reason or when the request is not `requested`.
pub async fn reject(state: &AppState, by: i64, id: i64, reason: &str) -> Result<LeaveRequest> {
    let (leave, employee) = load(state, id).await?;
    if !holds_approve(state, by, &employee).await? {
        return Err(Error::Forbidden);
    }
    let _outcome = leave
        .transition(REJECT)
        .by_system()
        .reason(reason.trim())
        .input(&LeaveInput {
            by: Some(by),
            ..LeaveInput::default()
        })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    LeaveRequest::find_or_404(&state.db, id).await
}

/// `by` cancels request `id` before it starts.
///
/// # Errors
///
/// `Forbidden` for a user who is neither the employee nor a holder of
/// `leave.approve`; 422 when the start date is reached or the request is
/// settled.
pub async fn cancel(state: &AppState, by: i64, id: i64, reason: &str) -> Result<LeaveRequest> {
    let (leave, employee) = load(state, id).await?;
    if employee.user_id != Some(by) && !holds_approve(state, by, &employee).await? {
        return Err(Error::Forbidden);
    }
    let _outcome = leave
        .transition(CANCEL)
        .by_system()
        .reason(reason.trim())
        .input(&LeaveInput {
            by: Some(by),
            ..LeaveInput::default()
        })
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    LeaveRequest::find_or_404(&state.db, id).await
}

/// `complete` for every approved leave that ended before `day`. Returns how
/// many completed; a request that is not `approved` any more is skipped.
pub async fn complete_ended(state: &AppState, day: NaiveDate) -> Result<usize> {
    let mut done = 0;
    for leave in LeaveRequest::query()
        .where_(LeaveRequest::STATUS.eq(LeaveStatus::Approved))
        .where_(LeaveRequest::TO_DATE.lt(day))
        .order_by(LeaveRequest::ID)
        .get(&state.db)
        .await?
    {
        let ran = leave
            .transition(COMPLETE)
            .by_system()
            .input(&LeaveInput {
                today: Some(day),
                ..LeaveInput::default()
            })
            .run(state)
            .await;
        match ran {
            Ok(_outcome) => done += 1,
            Err(renox::state_machine::TransitionError::NotFrom(_)) => {}
            Err(error) => return Err(error.into_error()),
        }
    }
    Ok(done)
}

/// Registers the listener that turns `approve` and `cancel` into the
/// kernel's events and moves the due dates of onboarding steps.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<LeaveRequest>, state: AppState| async move {
            if event.transition != APPROVE && event.transition != CANCEL {
                return Ok(());
            }
            let Some(leave) = LeaveRequest::find(&state.db, event.key).await? else {
                return Ok(());
            };
            if event.transition == APPROVE {
                state.emit(approved_event(&leave)).await?;
            } else {
                state.emit(cancelled_event(&leave)).await?;
            }
            leave::sync_due_dates(&state, leave.employee_id, today())
                .await
                .map(|_| ())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sick_days_beyond_the_paid_bands_are_unpaid() {
        assert_eq!(sick_unpaid_days(0, 2, false), 0);
        assert_eq!(sick_unpaid_days(44, 3, false), 2);
        assert_eq!(sick_unpaid_days(45, 1, false), 1);
        assert_eq!(sick_unpaid_days(60, 5, false), 5);
    }

    #[test]
    fn all_sick_days_in_the_probation_are_unpaid() {
        assert_eq!(sick_unpaid_days(0, 2, true), 2);
        assert_eq!(sick_unpaid_days(0, 0, true), 0);
    }

    #[test]
    fn working_days_skip_the_weekend() {
        let cal = BusinessCalendar::default();
        let d = |day: u32| NaiveDate::from_ymd_opt(2026, 11, day).unwrap();
        // Friday 6 to Monday 9 Nov: two working days.
        assert_eq!(working_days(&cal, d(6), d(9)), 2);
        assert_eq!(working_days(&cal, d(2), d(3)), 2);
        assert_eq!(working_days(&cal, d(7), d(8)), 0);
    }
}
