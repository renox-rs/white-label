//! The deadline Task (wl-ticketing §3 *Task*, §4.4): something due by a date
//! for the agreement it belongs to, and the agreements whose tasks are held
//! or ended ([`TaskSource`]).
//!
//! Its status moves only through the machine in
//! [`crate::deadline_machine`]; the services are in [`crate::deadlines`].
//! Other modules reach it through the kernel's
//! [`DeadlineTasks`](wl_kernel::deadlines::DeadlineTasks).

use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::deadlines::{
    DEADLINE_TASK_SUBJECT_TYPE, DeadlineTaskId, DeadlineTaskStatus, DeadlineTaskSummary,
    TaskRecurrence,
};
use wl_kernel::{PartyId, Subject};

/// One deadline task: one occurrence of a recurring deadline, or a one-off.
///
/// Decision: the task's subject is its source agreement (04: the
/// engagement); the client is `party_id`. Holding, releasing and ending act
/// on the source.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_deadline_tasks",
    state_machine,
    index(status),
    index(subject_type, subject_id),
    index(due_date)
)]
pub struct DeadlineTask {
    /// The primary key.
    pub id: i64,
    /// The task type, a key of the vertical (`vat_return`).
    pub task_type: String,
    /// The title ("VAT Q3").
    pub title: String,
    /// A longer description.
    pub description: String,
    /// The type of the agreement it belongs to (`accounting.engagement`).
    pub subject_type: String,
    /// The id of that agreement.
    pub subject_id: i64,
    /// The client, told when the task waits on them.
    pub party_id: Option<i64>,
    /// The due date (Asia/Dubai).
    pub due_date: NaiveDate,
    /// The lead time in days.
    pub lead_days: i64,
    /// The start date: the due date minus the lead time.
    pub start_date: NaiveDate,
    /// The owning staff user.
    pub owner_user_id: Option<i64>,
    /// The owning role, when no user is named.
    pub owner_role: Option<String>,
    /// The reviewing role; without one, `complete` needs no review.
    pub reviewer_role: Option<String>,
    /// Whether `complete` needs evidence.
    pub evidence_required: bool,
    /// The evidence given at `complete` (a document reference or a note).
    pub evidence: Option<String>,
    /// Whether the task needs client input (an open request is linked).
    pub needs_client_input: bool,
    /// How often it comes back.
    pub recurrence: TaskRecurrence,
    /// The last day of the period it covers.
    pub period_end: Option<NaiveDate>,
    /// Whether it is held (its source agreement is paused). Not a state.
    pub held: bool,
    /// Whether it went `overdue` (a completed task is *late*).
    pub late: bool,
    /// Where the work stood when it went `overdue` (`upcoming`,
    /// `in_progress`, `waiting` or `in_review`).
    pub stage: Option<String>,
    /// When work started first.
    pub started_at: Option<DateTime>,
    /// The reason it is waiting.
    pub waiting_reason: Option<String>,
    /// The day it went `overdue`.
    pub overdue_on: Option<NaiveDate>,
    /// When it was completed.
    pub completed_at: Option<DateTime>,
    /// The reason it was waived or closed as missed.
    pub closed_reason: Option<String>,
    /// The day the owner was told the start date is reached (`WRK.RUL.06`).
    pub start_notified_on: Option<NaiveDate>,
    /// The occurrence this one follows.
    pub previous_task_id: Option<i64>,
    /// The next occurrence, once generated.
    pub next_task_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: DeadlineTaskStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DeadlineTask {
    /// The task's id in the kernel contract.
    pub fn task_id(&self) -> DeadlineTaskId {
        DeadlineTaskId(self.id)
    }

    /// The task as a [`Subject`]: `ticketing.task:42`.
    pub fn subject(&self) -> Subject {
        Subject::new(DEADLINE_TASK_SUBJECT_TYPE, self.id)
    }

    /// The agreement it belongs to.
    pub fn source(&self) -> Subject {
        Subject {
            subject_type: self.subject_type.clone(),
            subject_id: self.subject_id,
        }
    }

    /// The client.
    pub fn party(&self) -> Option<PartyId> {
        self.party_id.map(PartyId)
    }

    /// The tasks of `source`, by due date then id.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_source(db: &Db, source: &Subject) -> Result<Vec<DeadlineTask>> {
        DeadlineTask::query()
            .where_(DeadlineTask::SUBJECT_TYPE.eq(source.subject_type.as_str()))
            .where_(DeadlineTask::SUBJECT_ID.eq(source.subject_id))
            .order_by(DeadlineTask::DUE_DATE)
            .order_by(DeadlineTask::ID)
            .get(db)
            .await
    }

    /// What other modules see of it.
    pub fn summary(&self) -> DeadlineTaskSummary {
        DeadlineTaskSummary {
            id: self.task_id(),
            source: self.source(),
            task_type: self.task_type.clone(),
            title: self.title.clone(),
            due_date: self.due_date,
            start_date: self.start_date,
            status: self.status,
            late: self.late,
            held: self.held,
            overdue_on: self.overdue_on,
            party: self.party(),
            owner_user: self.owner_user_id,
            closed_reason: self.closed_reason.clone(),
            next: self.next_task_id.map(DeadlineTaskId),
        }
    }
}

/// The start date of a task due on `due` with `lead_days` of lead time.
pub fn start_date(due: NaiveDate, lead_days: i64) -> NaiveDate {
    due - Duration::days(lead_days.max(0))
}

/// One agreement whose tasks are held (its `held_on` is set) or that ended
/// (its `ended_on` is set). A source with no row is neither.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_ticketing_task_sources", unique(subject_type, subject_id))]
pub struct TaskSource {
    /// The primary key.
    pub id: i64,
    /// The type of the agreement.
    pub subject_type: String,
    /// The id of the agreement.
    pub subject_id: i64,
    /// The first day of the current hold; empty when not held.
    pub held_on: Option<NaiveDate>,
    /// The day the agreement ended; no next occurrence is generated after.
    pub ended_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TaskSource {
    /// The row of `source`, if it has one.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of(db: &Db, source: &Subject) -> Result<Option<TaskSource>> {
        TaskSource::query()
            .where_(TaskSource::SUBJECT_TYPE.eq(source.subject_type.as_str()))
            .where_(TaskSource::SUBJECT_ID.eq(source.subject_id))
            .first(db)
            .await
    }

    /// The row of `source`, created when it has none.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_or_new(db: &Db, source: &Subject) -> Result<TaskSource> {
        if let Some(found) = TaskSource::of(db, source).await? {
            return Ok(found);
        }
        let mut row = TaskSource {
            subject_type: source.subject_type.clone(),
            subject_id: source.subject_id,
            ..TaskSource::default()
        };
        row.insert(db).await?;
        Ok(row)
    }

    /// Whether the tasks of `source` are held now.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn is_held(db: &Db, source: &Subject) -> Result<bool> {
        Ok(TaskSource::of(db, source)
            .await?
            .is_some_and(|row| row.held_on.is_some() && row.ended_on.is_none()))
    }

    /// Whether `source` ended.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn has_ended(db: &Db, source: &Subject) -> Result<bool> {
        Ok(TaskSource::of(db, source)
            .await?
            .is_some_and(|row| row.ended_on.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_start_date_is_the_due_date_less_the_lead_time() {
        let due = NaiveDate::from_ymd_opt(2026, 11, 28).unwrap();
        assert_eq!(
            start_date(due, 23),
            NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
        );
        assert_eq!(start_date(due, -3), due);
    }

    #[test]
    fn a_task_belongs_to_its_source() {
        let task = DeadlineTask {
            id: 3,
            subject_type: "accounting.engagement".into(),
            subject_id: 12,
            party_id: Some(9),
            ..DeadlineTask::default()
        };
        assert_eq!(task.source(), Subject::new("accounting.engagement", 12));
        assert_eq!(task.subject().to_string(), "ticketing.task:3");
        assert_eq!(task.party(), Some(PartyId(9)));
        assert_eq!(task.summary().id, DeadlineTaskId(3));
    }
}
