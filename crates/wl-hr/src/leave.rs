//! Leave as onboarding needs it (wl-hr §3 *Onboarding*, §4 *Leave request*,
//! issue #218): the approved days of leave move the due dates of the open
//! steps of the employee.
//!
//! The record is here; its lifecycle (`approve`, `reject`, `cancel`,
//! `complete`) is [`crate::leave_machine`] and the services that run it are
//! [`crate::leave_service`]. Each change calls [`sync_due_dates`], so a leave
//! shortened, extended or cancelled moves the due dates again. The daily run
//! of `HRM.RUL.17` ([`crate::reminders`]) calls it too, so no change is
//! missed.

use renox::chrono::{Days, NaiveDate};
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::employee::{Employee, EmployeeStatus};
use crate::onboarding::{
    Onboarding, OnboardingStatus, OnboardingStep, StepStatus, due_date as plus_days,
};

/// Where a leave request stands (§4 *Leave request*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LeaveStatus {
    /// Waiting for approval.
    #[default]
    Requested,
    /// Approved; not finished.
    Approved,
    /// The leave period is over.
    Completed,
    /// Not approved.
    Rejected,
    /// Withdrawn before it started.
    Cancelled,
}

impl LeaveStatus {
    /// Whether the leave counts as approved leave: `approved` and
    /// `completed`.
    pub fn is_approved(self) -> bool {
        matches!(self, LeaveStatus::Approved | LeaveStatus::Completed)
    }
}

/// One absence of an employee.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_leave_requests",
    state_machine,
    index(employee_id, status)
)]
pub struct LeaveRequest {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// `annual`, `sick`, `unpaid`, ...
    pub leave_type: String,
    /// The first day of leave.
    pub from_date: NaiveDate,
    /// The last day of leave, included.
    pub to_date: NaiveDate,
    /// Where it stands; changed only by the machine.
    pub status: LeaveStatus,
    /// Working days of the leave that are unpaid (beyond the balance or the
    /// paid bands of sick leave).
    pub unpaid_days: i64,
    /// The user who asked, when a user did.
    pub requested_by: Option<i64>,
    /// The user who approved, rejected or cancelled it.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// The reason given with a rejection or a cancellation.
    pub decision_reason: Option<String>,
    /// Whether it was recorded as approved at once (`record_sick`).
    pub recorded_sick: bool,
    /// The number of the stored sick note.
    pub sick_note_document_id: Option<i64>,
    /// The day the sick note was attached.
    pub sick_note_attached_on: Option<NaiveDate>,
    /// The last day the sick note may follow (`record_sick`).
    pub sick_note_due_on: Option<NaiveDate>,
    /// The HR task raised when the note did not follow in time.
    pub sick_note_task_id: Option<i64>,
    /// The day that task was raised.
    pub sick_note_task_raised_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The subject type of a leave request.
pub const LEAVE_SUBJECT_TYPE: &str = "hr.leave_request";

impl LeaveRequest {
    /// The leave request as a [`Subject`](wl_kernel::Subject):
    /// `hr.leave_request:7`.
    pub fn subject(&self) -> wl_kernel::Subject {
        wl_kernel::Subject::new(LEAVE_SUBJECT_TYPE, self.id)
    }

    /// The employee as an id.
    pub fn employee(&self) -> wl_kernel::workforce::EmployeeId {
        wl_kernel::workforce::EmployeeId(self.employee_id)
    }

    /// The calendar days of the leave, both ends included; 0 when the dates
    /// are the wrong way round.
    pub fn days(&self) -> i64 {
        ((self.to_date - self.from_date).num_days() + 1).max(0)
    }
}

/// The approved leave of `employee` that has started on `today`, oldest
/// first, as `(from, to)` pairs.
pub async fn started_leave(
    db: &Db,
    employee: i64,
    today: NaiveDate,
) -> Result<Vec<(NaiveDate, NaiveDate)>> {
    Ok(LeaveRequest::query()
        .where_(LeaveRequest::EMPLOYEE_ID.eq(employee))
        .where_(LeaveRequest::STATUS.is_in([LeaveStatus::Approved, LeaveStatus::Completed]))
        .order_by(LeaveRequest::FROM_DATE)
        .get(db)
        .await?
        .into_iter()
        .filter(|leave| leave.from_date <= today && leave.days() > 0)
        .map(|leave| (leave.from_date, leave.to_date))
        .collect())
}

/// `original` plus the days of the `leave` periods (sorted by start) that
/// begin on or before the due date as it moves, and that did not end before
/// `started_on`. A leave that begins after the due date does not move it.
pub fn shifted_due(
    original: NaiveDate,
    started_on: NaiveDate,
    leave: &[(NaiveDate, NaiveDate)],
) -> NaiveDate {
    let mut due = original;
    for (from, to) in leave {
        if *to < started_on || *to < *from {
            continue;
        }
        if *from <= due {
            let days = u64::try_from((*to - *from).num_days() + 1).unwrap_or(0);
            due = due.checked_add_days(Days::new(days)).unwrap_or(due);
        }
    }
    due
}

/// Moves the due dates of the open steps of the `in_progress` onboardings of
/// `employee_id` to their original due date (start date plus due days) plus
/// the days of approved leave that has started on `today`, as recorded now.
/// Nothing moves while the employee is `absconded`. Returns how many steps
/// moved.
pub async fn sync_due_dates(state: &AppState, employee_id: i64, today: NaiveDate) -> Result<usize> {
    let Some(employee) = Employee::find(&state.db, employee_id).await? else {
        return Ok(0);
    };
    if employee.status == EmployeeStatus::Absconded {
        return Ok(0);
    }
    let leave = started_leave(&state.db, employee_id, today).await?;
    let mut moved = 0;
    for onboarding in Onboarding::query()
        .where_(Onboarding::EMPLOYEE_ID.eq(employee_id))
        .where_(Onboarding::STATUS.eq(OnboardingStatus::InProgress))
        .get(&state.db)
        .await?
    {
        for step in OnboardingStep::query()
            .where_(OnboardingStep::ONBOARDING_ID.eq(onboarding.id))
            .where_(OnboardingStep::STATUS.eq(StepStatus::Open))
            .get(&state.db)
            .await?
        {
            let original = plus_days(onboarding.started_on, step.due_days);
            let due = shifted_due(original, onboarding.started_on, &leave);
            if due != step.due_date {
                OnboardingStep::query()
                    .where_(OnboardingStep::ID.eq(step.id))
                    .update(&state.db, &[("due_date", &due), ("updated_at", &now())])
                    .await?;
                moved += 1;
            }
        }
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, d).unwrap()
    }

    #[test]
    fn ten_days_of_leave_from_the_eighth_move_the_ninth_to_the_nineteenth() {
        let leave = [(day(11, 8), day(11, 17))];
        assert_eq!(shifted_due(day(11, 9), day(11, 2), &leave), day(11, 19));
    }

    #[test]
    fn a_shorter_leave_moves_it_less_and_none_puts_it_back() {
        assert_eq!(
            shifted_due(day(11, 9), day(11, 2), &[(day(11, 8), day(11, 13))]),
            day(11, 15)
        );
        assert_eq!(shifted_due(day(11, 9), day(11, 2), &[]), day(11, 9));
    }

    #[test]
    fn leave_after_the_due_date_or_before_the_start_does_not_move_it() {
        assert_eq!(
            shifted_due(day(11, 9), day(11, 2), &[(day(11, 12), day(11, 20))]),
            day(11, 9)
        );
        assert_eq!(
            shifted_due(day(11, 9), day(11, 5), &[(day(10, 20), day(11, 1))]),
            day(11, 9)
        );
    }

    #[test]
    fn a_second_leave_that_begins_inside_the_moved_period_counts() {
        let leave = [(day(11, 8), day(11, 10)), (day(11, 11), day(11, 12))];
        // 9 + 3 = 12; the second begins on the 11th, before the 12th.
        assert_eq!(shifted_due(day(11, 9), day(11, 2), &leave), day(11, 14));
    }

    #[test]
    fn only_approved_and_completed_leave_counts() {
        assert!(LeaveStatus::Approved.is_approved());
        assert!(LeaveStatus::Completed.is_approved());
        assert!(!LeaveStatus::Requested.is_approved());
        assert!(!LeaveStatus::Cancelled.is_approved());
        let leave = LeaveRequest {
            from_date: day(11, 8),
            to_date: day(11, 17),
            ..LeaveRequest::default()
        };
        assert_eq!(leave.days(), 10);
    }
}
