//! Same-day sick leave (wl-hr §4 *Leave request* `record_sick`, §5.2 step 3,
//! §7 *Sick note due*, BL-HRM-02, issue #192).
//!
//! An employee who calls in sick today is recorded directly as approved
//! sick leave by `leave.approve` or by the employee's manager (§3): the
//! request is inserted as `approved`, `LeaveApproved` goes out at once (so
//! today's bookings and rosters move) and the sick note may follow.
//!
//! - **Guards:** the leave starts today (Asia/Dubai); a manager of another
//!   scope without `leave.approve` is refused; the note is not required.
//! - **Due date:** the note is due by 23:59 on the Nth working day after the
//!   start date (N = [`HrConfig::sick_note_days`](crate::HrConfig), default
//!   2; the start date is not counted). The HR task is raised by [`run`]
//!   at 00:00 the next day, once per request ([`TASK`] runs it daily).
//! - **Task:** one per request, found by its source key; attaching the note
//!   ([`attach_note`]) resolves it.
//! - **Unpaid days:** as `approve` marks them, from the sick pay bands
//!   ([`crate::leave_service::sick_unpaid_days`]).
//!
//! Decision: the sick note is a stored document the caller names by its
//! number; the document kinds and e-signature of wl-documents are not
//! needed to attach it.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::calendar::{BusinessCalendar, calendar};
use wl_kernel::documents::today;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};

use crate::assignees;
use crate::config::config_of;
use crate::employee::Employee;
use crate::leave::{self, LeaveRequest, LeaveStatus};
use crate::leave_service::{
    approved_event, complete_ended, holds_approve, is_manager_of, refuse, unpaid_days_of,
};
use crate::notices::name_of;
use crate::texts::english;

/// The leave type of a same-day sick leave.
pub const SICK: &str = "sick";

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "hr:leave-daily";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "00:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// The key of the HR task for the missing sick note of request `id`.
pub fn source_key(id: i64) -> String {
    format!("hr.sick_note:{id}")
}

/// The last day the sick note may follow a leave that starts on `start`:
/// `days` working days later; the start date itself is not counted.
pub fn note_due_on(calendar: &BusinessCalendar, start: NaiveDate, days: i64) -> NaiveDate {
    let mut day = start;
    for _ in 0..days.max(0) {
        day = calendar.next_working_day(day);
    }
    day
}

/// `by` records the sick leave of `employee_id` from `from` (which must be
/// today) to `to` as approved at once.
///
/// # Errors
///
/// `Forbidden` for a user who neither holds `leave.approve` in the
/// employee's scopes nor is their manager; 422 when `from` is not today or
/// `to` is before `from`.
pub async fn record(
    state: &AppState,
    by: i64,
    employee_id: i64,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<LeaveRequest> {
    let employee = Employee::find(&state.db, employee_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !holds_approve(state, by, &employee).await? && !is_manager_of(state, by, &employee).await? {
        return Err(Error::Forbidden);
    }
    if from != today() {
        return Err(refuse("wl_hr.sick.problem.not_today"));
    }
    if to < from {
        return Err(refuse("wl_hr.leave.problem.dates"));
    }
    let due = note_due_on(&calendar(state), from, config_of(state).sick_note_days);
    let mut request = LeaveRequest {
        employee_id,
        leave_type: SICK.to_owned(),
        from_date: from,
        to_date: to,
        status: LeaveStatus::Approved,
        requested_by: Some(by),
        decided_by: Some(by),
        decided_at: Some(now()),
        recorded_sick: true,
        sick_note_due_on: Some(due),
        ..LeaveRequest::default()
    };
    request.unpaid_days = unpaid_days_of(state, &request).await?;
    let mut tx = state.db.begin().await?;
    request
        .insert_in_state(&mut tx, LeaveStatus::Approved, "record_sick")
        .await?;
    tx.commit().await?;
    state.emit(approved_event(&request)).await?;
    leave::sync_due_dates(state, employee_id, today()).await?;
    LeaveRequest::find_or_404(&state.db, request.id).await
}

/// `by` attaches the sick note `document` to request `id`; the open HR task
/// for the missing note is resolved.
///
/// # Errors
///
/// `Forbidden` for a user who is not the employee, a holder of
/// `leave.approve` in their scopes or their manager; 422 for a request that
/// is not sick leave or is rejected or cancelled.
pub async fn attach_note(
    state: &AppState,
    by: i64,
    id: i64,
    document: i64,
) -> Result<LeaveRequest> {
    let request = LeaveRequest::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let employee = Employee::find_or_404(&state.db, request.employee_id).await?;
    if employee.user_id != Some(by)
        && !holds_approve(state, by, &employee).await?
        && !is_manager_of(state, by, &employee).await?
    {
        return Err(Error::Forbidden);
    }
    if request.leave_type != SICK
        || matches!(
            request.status,
            LeaveStatus::Rejected | LeaveStatus::Cancelled
        )
    {
        return Err(refuse("wl_hr.sick.problem.not_sick"));
    }
    LeaveRequest::query()
        .where_(LeaveRequest::ID.eq(id))
        .update(
            &state.db,
            &[
                ("sick_note_document_id", &Some(document)),
                ("sick_note_attached_on", &Some(today())),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if let Some(task) = request.sick_note_task_id {
        let note = english("wl_hr.sick.task_resolved", &[]);
        task_creator(state)
            .resolve_from_source(state, task.into(), &note)
            .await?;
    }
    leave::sync_due_dates(state, request.employee_id, today()).await?;
    LeaveRequest::find_or_404(&state.db, id).await
}

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// HR tasks raised for a sick note that did not follow.
    pub raised: usize,
    /// Approved leave that ended and completed.
    pub completed: usize,
}

/// The daily run at `day` (Asia/Dubai): raises the HR task of every
/// same-day sick leave whose note was due by the end of the day before and is
/// still missing, once per request, and completes the approved leave that
/// ended before `day`.
pub async fn run(state: &AppState, day: NaiveDate) -> Result<Report> {
    let completed = complete_ended(state, day).await?;
    let mut raised = 0;
    for request in LeaveRequest::query()
        .where_(LeaveRequest::RECORDED_SICK.eq(true))
        .where_(LeaveRequest::STATUS.is_in([LeaveStatus::Approved, LeaveStatus::Completed]))
        .where_(LeaveRequest::SICK_NOTE_DOCUMENT_ID.is_null())
        .where_(LeaveRequest::SICK_NOTE_TASK_RAISED_ON.is_null())
        .where_(LeaveRequest::SICK_NOTE_DUE_ON.lt(day))
        .order_by(LeaveRequest::ID)
        .get(&state.db)
        .await?
    {
        if raise_task(state, &request, day).await? {
            raised += 1;
        }
    }
    Ok(Report { raised, completed })
}

/// Opens the HR task of `request`, or finds the one still open under its
/// key; records it on the request. `false` when ticketing is off.
async fn raise_task(state: &AppState, request: &LeaveRequest, day: NaiveDate) -> Result<bool> {
    let employee = Employee::find_or_404(&state.db, request.employee_id).await?;
    let key = source_key(request.id);
    let creator = task_creator(state);
    let task = match creator.find_open(state, &key).await? {
        Some(found) => found,
        None => {
            let name = name_of(state, &employee).await;
            let title = english("wl_hr.sick.task_title", &[("employee", &name)]);
            let description = english(
                "wl_hr.sick.task_description",
                &[
                    ("employee", &name),
                    ("from", &request.from_date),
                    (
                        "due",
                        &request.sick_note_due_on.unwrap_or(request.from_date),
                    ),
                ],
            );
            let scopes = employee.scopes(&state.db).await?;
            let mut ticket = OpenTicket::new(request.subject(), TASK_CATEGORY, &title)
                .description(&description)
                .source_key(&key);
            if let Some(hr) = assignees::hr_holders(state, &scopes).await?.first() {
                ticket = ticket.assign_to(Assignee::user(*hr));
            }
            match creator.open(state, ticket).await? {
                Some(opened) => opened,
                None => return Ok(false),
            }
        }
    };
    LeaveRequest::query()
        .where_(LeaveRequest::ID.eq(request.id))
        .update(
            &state.db,
            &[
                ("sick_note_task_id", &Some(task.id.0)),
                ("sick_note_task_raised_on", &Some(day)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(true)
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
        "Raise the HR task of a missing sick note and complete leave that ended \
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
                "{} sick note tasks raised, {} leave requests completed.",
                report.raised, report.completed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn the_note_is_due_on_the_second_working_day_after_the_start() {
        let cal = BusinessCalendar::default();
        // Monday 2 Nov: Tuesday and Wednesday count.
        assert_eq!(note_due_on(&cal, day(2), 2), day(4));
        // Thursday 5 Nov: Friday, then Monday.
        assert_eq!(note_due_on(&cal, day(5), 2), day(9));
        assert_eq!(note_due_on(&cal, day(2), 0), day(2));
    }

    #[test]
    fn the_key_names_the_request() {
        assert_eq!(source_key(7), "hr.sick_note:7");
    }
}
