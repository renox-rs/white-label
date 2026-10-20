//! The deadline task contract (wl-ticketing §4.4 *Task*, §6 `Tasks`): how a
//! vertical creates, holds, releases, waives and reads the deadline tasks of
//! its agreements, without depending on the `wl-ticketing` crate.
//!
//! A deadline task is something due by a date (a VAT return, a corporate tax
//! filing): it is generated `upcoming`, worked, maybe sent for review, and
//! completed; once its due date has passed it is `overdue` (and can still be
//! completed, flagged *late*). It is not a ticket: staff tasks other modules
//! open are tickets in the `task` category ([`crate::tickets::TaskCreator`]).
//!
//! The vertical names its agreement (`accounting.engagement:12`) as the
//! task's source and calls the [`DeadlineTasks`] of [`deadline_tasks`] with
//! it. Without the `ticketing` module the calls do nothing
//! ([`NoDeadlineTasks`]):
//!
//! ```ignore
//! use wl_kernel::deadlines::{self, NewDeadlineTask, TaskRecurrence};
//!
//! let tasks = deadlines::deadline_tasks(&state);
//! // The compliance calendar of an engagement:
//! tasks.create(&state, NewDeadlineTask::new(engagement.subject(), "vat_return", "VAT Q3", due)
//!     .party(client).lead_days(21).owner_user(accountant).evidence_required()
//!     .recurrence(TaskRecurrence::Quarterly)).await?;
//! // 04 engagement `suspend` and `resume`:
//! tasks.hold(&state, &engagement.subject(), today).await?;
//! tasks.release(&state, &engagement.subject(), today).await?;
//! // 04 engagement `end`:
//! tasks.end_source(&state, &engagement.subject(), today).await?;
//! // 04 return `cancel` (the client files itself):
//! tasks.hand_over(&state, task_id, HandOverTask::new()).await?;
//! ```
//!
//! `wl-ticketing` emits [`TaskOverdue`] and [`TaskCompleted`] after the
//! commit of the transition.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::chrono::{Months, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::subject::Subject;
use crate::tickets::TicketRef;

/// The subject type of a deadline task, for [`Subject`] references.
pub const DEADLINE_TASK_SUBJECT_TYPE: &str = "ticketing.task";

/// The reason recorded when the source agreement ended and its tasks are
/// waived ([`DeadlineTasks::end_source`]).
pub const SOURCE_ENDED_REASON: &str = "source agreement ended";

/// The evidence recorded when a task is handed over to the client
/// ([`DeadlineTasks::hand_over`]): the firm no longer files it, the client
/// does, and the task stays visible as the client's deadline.
pub const HANDED_OVER_EVIDENCE: &str = "handed over to client";

/// The id of a deadline task.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DeadlineTaskId(pub i64);

impl DeadlineTaskId {
    /// The task as a [`Subject`]: `ticketing.task:42`.
    pub fn subject(self) -> Subject {
        Subject::new(DEADLINE_TASK_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for DeadlineTaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for DeadlineTaskId {
    fn from(id: i64) -> Self {
        DeadlineTaskId(id)
    }
}

/// The lifecycle state of a deadline task (wl-ticketing §4.4); the state of
/// the `wl-ticketing` machine.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeadlineTaskStatus {
    /// Generated; start date not reached or not started.
    #[default]
    Upcoming,
    /// Being worked on.
    InProgress,
    /// Blocked on someone else (client, authority, approval). 04 shows it as
    /// *waiting on client*.
    Waiting,
    /// Prepared; with the reviewer.
    InReview,
    /// Due date passed, not completed; can still be completed (*late*).
    Overdue,
    /// Done, with evidence (terminal).
    Completed,
    /// Closed as not done (terminal).
    Missed,
    /// Not applicable this period (terminal).
    NotRequired,
}

impl DeadlineTaskStatus {
    /// Whether no transition leaves it: `completed`, `missed` or
    /// `not_required`.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            DeadlineTaskStatus::Completed
                | DeadlineTaskStatus::Missed
                | DeadlineTaskStatus::NotRequired
        )
    }

    /// Whether the work has started: `in_progress` or `waiting` (the
    /// *started task* of wl-ticketing §4.4 `waive`).
    pub fn is_started(self) -> bool {
        matches!(
            self,
            DeadlineTaskStatus::InProgress | DeadlineTaskStatus::Waiting
        )
    }
}

/// How often a deadline task comes back: the next occurrence is generated
/// when one is completed, closed as missed or waived.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskRecurrence {
    /// A one-off task: no next occurrence.
    #[default]
    Once,
    /// Every month.
    Monthly,
    /// Every three months.
    Quarterly,
    /// Every six months.
    HalfYearly,
    /// Every year.
    Yearly,
}

impl TaskRecurrence {
    /// The same day one recurrence later (clamped to the month's last day);
    /// `None` for a one-off task.
    pub fn next(self, date: NaiveDate) -> Option<NaiveDate> {
        let months = match self {
            TaskRecurrence::Once => return None,
            TaskRecurrence::Monthly => 1,
            TaskRecurrence::Quarterly => 3,
            TaskRecurrence::HalfYearly => 6,
            TaskRecurrence::Yearly => 12,
        };
        date.checked_add_months(Months::new(months))
    }
}

/// A deadline task to create ([`DeadlineTasks::create`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewDeadlineTask {
    /// The agreement the task belongs to (its source), e.g.
    /// `accounting.engagement:12`. Holding, releasing and ending act on it.
    pub source: Subject,
    /// The task type, a key of the vertical (`vat_return`, `ct_return`).
    pub task_type: String,
    /// The title, e.g. "VAT Q3".
    pub title: String,
    /// A longer description.
    pub description: String,
    /// The due date (Asia/Dubai).
    pub due_date: NaiveDate,
    /// The days before the due date the work should start; the start date is
    /// the due date minus these.
    pub lead_days: i64,
    /// The client (the counterpart told when the task waits on them).
    pub party: Option<PartyId>,
    /// The staff user who owns (works) it.
    pub owner_user: Option<i64>,
    /// The role that owns it, when no user is named.
    pub owner_role: Option<String>,
    /// The role that reviews it; without one `complete` needs no review.
    pub reviewer_role: Option<String>,
    /// Whether `complete` needs evidence (e.g. the FTA acknowledgement).
    pub evidence_required: bool,
    /// Whether the task needs client input (an open request or query is
    /// linked): the client is reminded too, unless the task is held.
    pub needs_client_input: bool,
    /// How often it comes back.
    pub recurrence: TaskRecurrence,
    /// The last day of the period it covers, if any; moved with the next
    /// occurrence.
    pub period_end: Option<NaiveDate>,
}

impl NewDeadlineTask {
    /// A one-off task of `task_type` for `source`, titled `title`, due on
    /// `due_date`, starting that day (no lead time), with no owner.
    pub fn new(source: Subject, task_type: &str, title: &str, due_date: NaiveDate) -> Self {
        NewDeadlineTask {
            source,
            task_type: task_type.to_owned(),
            title: title.to_owned(),
            description: String::new(),
            due_date,
            lead_days: 0,
            party: None,
            owner_user: None,
            owner_role: None,
            reviewer_role: None,
            evidence_required: false,
            needs_client_input: false,
            recurrence: TaskRecurrence::Once,
            period_end: None,
        }
    }

    /// Sets the description.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// Sets the lead time, in days.
    pub fn lead_days(mut self, days: i64) -> Self {
        self.lead_days = days;
        self
    }

    /// Sets the client.
    pub fn party(mut self, party: PartyId) -> Self {
        self.party = Some(party);
        self
    }

    /// Sets the owning staff user.
    pub fn owner_user(mut self, user: i64) -> Self {
        self.owner_user = Some(user);
        self
    }

    /// Sets the owning role.
    pub fn owner_role(mut self, role: &str) -> Self {
        self.owner_role = Some(role.to_owned());
        self
    }

    /// Sets the reviewing role: the task goes through `in_review`.
    pub fn reviewer_role(mut self, role: &str) -> Self {
        self.reviewer_role = Some(role.to_owned());
        self
    }

    /// `complete` needs evidence.
    pub fn evidence_required(mut self) -> Self {
        self.evidence_required = true;
        self
    }

    /// The task needs client input (an open request or query is linked).
    pub fn needs_client_input(mut self) -> Self {
        self.needs_client_input = true;
        self
    }

    /// Sets the recurrence.
    pub fn recurrence(mut self, recurrence: TaskRecurrence) -> Self {
        self.recurrence = recurrence;
        self
    }

    /// Sets the last day of the period covered.
    pub fn period_end(mut self, period_end: NaiveDate) -> Self {
        self.period_end = Some(period_end);
        self
    }
}

/// A waive run by the system ([`DeadlineTasks::waive`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WaiveTask {
    /// The reason recorded (required).
    pub reason: String,
    /// Whether the next occurrence is generated (the default). Off for a
    /// regeneration that creates the new tasks itself (04 `ACC.RUL.10`).
    /// Never generated once the source agreement ended.
    pub next_occurrence: bool,
}

impl WaiveTask {
    /// A waive with `reason`, generating the next occurrence.
    pub fn new(reason: &str) -> Self {
        WaiveTask {
            reason: reason.to_owned(),
            next_occurrence: true,
        }
    }

    /// No next occurrence is generated.
    pub fn without_next(mut self) -> Self {
        self.next_occurrence = false;
        self
    }
}

/// A hand-over run by the system ([`DeadlineTasks::hand_over`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct HandOverTask {
    /// Whether the next occurrence is generated (the default). Off when the
    /// source agreement is ending.
    pub next_occurrence: bool,
}

impl HandOverTask {
    /// A hand-over that generates the next occurrence.
    pub fn new() -> Self {
        HandOverTask {
            next_occurrence: true,
        }
    }

    /// No next occurrence is generated.
    pub fn without_next(mut self) -> Self {
        self.next_occurrence = false;
        self
    }
}

impl Default for HandOverTask {
    fn default() -> Self {
        HandOverTask::new()
    }
}

/// What other modules see of a deadline task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadlineTaskSummary {
    /// The id.
    pub id: DeadlineTaskId,
    /// The agreement it belongs to.
    pub source: Subject,
    /// The task type.
    pub task_type: String,
    /// The title.
    pub title: String,
    /// The due date.
    pub due_date: NaiveDate,
    /// The start date (due date minus lead time).
    pub start_date: NaiveDate,
    /// The lifecycle state.
    pub status: DeadlineTaskStatus,
    /// Whether it went `overdue` at some point (a completed task is *late*).
    pub late: bool,
    /// Whether it is held (its source agreement is paused): client reminders
    /// and new request lists stop.
    pub held: bool,
    /// The day it went `overdue`, if it did.
    pub overdue_on: Option<NaiveDate>,
    /// The client.
    pub party: Option<PartyId>,
    /// The owning staff user.
    pub owner_user: Option<i64>,
    /// The reason it was waived or closed as missed.
    pub closed_reason: Option<String>,
    /// The next occurrence, once generated.
    pub next: Option<DeadlineTaskId>,
}

/// What [`DeadlineTasks::release`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ReleasedTasks {
    /// The tasks whose held flag was cleared.
    pub released: Vec<DeadlineTaskId>,
    /// The tasks that went overdue while held (listed for the manager).
    pub overdue: Vec<DeadlineTaskSummary>,
    /// The manager's task listing them, when there were any.
    pub manager_task: Option<TicketRef>,
}

/// What [`DeadlineTasks::end_source`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct EndedTasks {
    /// The tasks waived with [`SOURCE_ENDED_REASON`].
    pub waived: Vec<DeadlineTaskId>,
    /// The open tasks left as they are and listed for the manager (started
    /// tasks while *waive a started task* is off, and tasks in review or
    /// overdue).
    pub listed: Vec<DeadlineTaskId>,
    /// The manager's task listing them, when there were any.
    pub manager_task: Option<TicketRef>,
}

/// The future a [`DeadlineTasks`] method returns.
pub type DeadlineFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Creates, holds, releases, waives and reads deadline tasks for the
/// verticals. `wl-ticketing` provides it; without that module,
/// [`deadline_tasks`] gives [`NoDeadlineTasks`], whose every method does
/// nothing.
pub trait DeadlineTasks: Send + Sync + 'static {
    /// Creates an `upcoming` task and returns its id; `None` when ticketing
    /// is off. A task created while its source is held is created held.
    /// Refused (422) for a blank title or type, or a negative lead time.
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        _task: NewDeadlineTask,
    ) -> DeadlineFuture<'a, Option<DeadlineTaskId>> {
        Box::pin(async { Ok(None) })
    }

    /// The task with this id; the default knows none.
    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: DeadlineTaskId,
    ) -> DeadlineFuture<'a, Option<DeadlineTaskSummary>> {
        Box::pin(async { Ok(None) })
    }

    /// The tasks of `source`, by due date then id, closed ones included; the
    /// default has none.
    fn of_subject<'a>(
        &'a self,
        _db: &'a Db,
        _source: &'a Subject,
    ) -> DeadlineFuture<'a, Vec<DeadlineTaskSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Holds the tasks of `source` from `on` (the agreement is paused): every
    /// open task gets the held flag and keeps its state and due date; tasks
    /// created later are created held. Holding a held source does nothing.
    /// Answers the tasks afterwards; the default has none.
    fn hold<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
        _on: NaiveDate,
    ) -> DeadlineFuture<'a, Vec<DeadlineTaskSummary>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Releases the tasks of `source` on `on` (the agreement resumed):
    /// open tasks whose due date has passed are made `overdue`, the held
    /// flag clears, and the tasks that went overdue while held are listed
    /// once for the manager. Releasing a source that is not held does
    /// nothing; the default does nothing.
    fn release<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
        _on: NaiveDate,
    ) -> DeadlineFuture<'a, ReleasedTasks> {
        Box::pin(async { Ok(ReleasedTasks::default()) })
    }

    /// Waives task `id` as the system with `waive.reason`. A started task
    /// is waived only when the client's *waive a started task* setting is
    /// on. Answers whether it was waived (`false` when the task is not in a
    /// state the waive leaves); the default waives nothing.
    fn waive<'a>(
        &'a self,
        _state: &'a AppState,
        _id: DeadlineTaskId,
        _waive: WaiveTask,
    ) -> DeadlineFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Hands task `id` over to the client as the system: the firm no longer
    /// files it, so the task is completed with the evidence
    /// [`HANDED_OVER_EVIDENCE`] (also kept as its closed reason, so the
    /// portal can show the deadline as the client's), from any open state.
    /// No completion is reported (`TaskCompleted` is not emitted) and the
    /// next occurrence follows unless `hand_over.next_occurrence` is off.
    /// Answers whether it was handed over (`false` when the task is already
    /// closed); the default hands over nothing.
    fn hand_over<'a>(
        &'a self,
        _state: &'a AppState,
        _id: DeadlineTaskId,
        _hand_over: HandOverTask,
    ) -> DeadlineFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// The source agreement ended on `on`: no next occurrence is generated
    /// for its tasks from now on, `upcoming` tasks are waived with
    /// [`SOURCE_ENDED_REASON`] (started ones too when *waive a started
    /// task* is on), the held flag clears, and the open tasks left are listed
    /// for the manager on one task. The default does nothing.
    fn end_source<'a>(
        &'a self,
        _state: &'a AppState,
        _source: &'a Subject,
        _on: NaiveDate,
    ) -> DeadlineFuture<'a, EndedTasks> {
        Box::pin(async { Ok(EndedTasks::default()) })
    }
}

/// The deadline tasks when the `ticketing` module is off: nothing is
/// created or found, and hold, release, waive and end do nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDeadlineTasks;

impl DeadlineTasks for NoDeadlineTasks {}

/// Makes `tasks` the app's [`DeadlineTasks`]; `wl-ticketing` calls it in
/// `register`.
pub fn provide_deadline_tasks(app: &mut Registry, tasks: impl DeadlineTasks) {
    let tasks: Arc<dyn DeadlineTasks> = Arc::new(tasks);
    app.provide(tasks);
}

/// The app's [`DeadlineTasks`], or [`NoDeadlineTasks`] when none is
/// provided.
pub fn deadline_tasks(state: &AppState) -> Arc<dyn DeadlineTasks> {
    match state.provided::<Arc<dyn DeadlineTasks>>() {
        Some(tasks) => Arc::clone(&tasks),
        None => Arc::new(NoDeadlineTasks),
    }
}

/// A deadline task went overdue (`mark_overdue`, `WRK.RUL.05`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskOverdue {
    /// The task.
    pub task: DeadlineTaskId,
    /// The agreement it belongs to.
    pub source: Subject,
    /// The owning staff user.
    pub owner: Option<i64>,
    /// The due date that passed.
    pub due_date: NaiveDate,
    /// Whether the task is held.
    pub held: bool,
}

impl Event for TaskOverdue {}

/// A deadline task was completed (`complete`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCompleted {
    /// The task.
    pub task: DeadlineTaskId,
    /// The agreement it belongs to.
    pub source: Subject,
    /// The last day of the period it covered, if any.
    pub period_end: Option<NaiveDate>,
    /// Whether it was completed after going overdue.
    pub late: bool,
}

impl Event for TaskCompleted {}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn states_are_stored_in_snake_case() {
        let states: Vec<_> = DeadlineTaskStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            states,
            [
                "upcoming",
                "in_progress",
                "waiting",
                "in_review",
                "overdue",
                "completed",
                "missed",
                "not_required"
            ]
        );
        assert!(DeadlineTaskStatus::NotRequired.is_terminal());
        assert!(!DeadlineTaskStatus::Overdue.is_terminal());
        assert!(DeadlineTaskStatus::Waiting.is_started());
        assert!(!DeadlineTaskStatus::InReview.is_started());
    }

    #[test]
    fn the_next_occurrence_keeps_the_day_where_it_can() {
        assert_eq!(TaskRecurrence::Once.next(day(2026, 11, 28)), None);
        assert_eq!(
            TaskRecurrence::Quarterly.next(day(2026, 11, 28)),
            Some(day(2027, 2, 28))
        );
        assert_eq!(
            TaskRecurrence::Monthly.next(day(2027, 1, 31)),
            Some(day(2027, 2, 28))
        );
        assert_eq!(
            TaskRecurrence::Yearly.next(day(2026, 3, 31)),
            Some(day(2027, 3, 31))
        );
    }

    #[test]
    fn the_builders_keep_their_defaults() {
        let source = Subject::new("accounting.engagement", 12);
        let task = NewDeadlineTask::new(source.clone(), "vat_return", "VAT Q3", day(2026, 11, 28))
            .lead_days(21)
            .evidence_required();
        assert_eq!(task.lead_days, 21);
        assert!(task.evidence_required);
        assert!(!task.needs_client_input);
        assert_eq!(task.recurrence, TaskRecurrence::Once);
        assert!(WaiveTask::new("x").next_occurrence);
        assert!(!WaiveTask::new("x").without_next().next_occurrence);
        assert_eq!(DeadlineTaskId(4).subject().to_string(), "ticketing.task:4");
    }
}
