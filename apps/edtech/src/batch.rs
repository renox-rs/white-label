//! The batch: one run of a course (10 §6 *Batch*, §7.1 *Batch*, `EDU.BAT.01`).
//!
//! A batch has a capacity, start and end dates, a weekly pattern (the
//! weekdays, one start time, one duration), a tutor and a room. The tutor and
//! the room are wl-scheduling resources, named by their id; `tutor_employee_id`
//! is the wl-hr employee behind the tutor, which the qualification guard of
//! `EDU.ACL.14` asks about (BL-EDU-08). The status moves only through the
//! machine in [`crate::batch_machine`]; [`crate::batches`] holds the services
//! and [`crate::seats`] the seat count.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::exit::ExitOutcome;

/// The subject type of a batch.
pub const BATCH_SUBJECT_TYPE: &str = "edtech.batch";

/// Where a batch stands (10 §7.1 *Batch*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BatchStatus {
    /// Being scheduled; not visible to students.
    #[default]
    Planned,
    /// Open for enrolment.
    Open,
    /// Sessions in progress.
    Running,
    /// The last session was held (terminal).
    Completed,
    /// Did not run (terminal).
    Cancelled,
    /// Stopped after it started (terminal).
    Terminated,
}

impl BatchStatus {
    /// The stored name, as the machine and the texts use it.
    pub fn as_str(self) -> &'static str {
        match self {
            BatchStatus::Planned => "planned",
            BatchStatus::Open => "open",
            BatchStatus::Running => "running",
            BatchStatus::Completed => "completed",
            BatchStatus::Cancelled => "cancelled",
            BatchStatus::Terminated => "terminated",
        }
    }

    /// Whether the batch can still change: it is not completed, cancelled or
    /// terminated.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            BatchStatus::Planned | BatchStatus::Open | BatchStatus::Running
        )
    }
}

/// A batch.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "edtech_batches", state_machine, index(course_id), index(status))]
pub struct Batch {
    /// The primary key.
    pub id: i64,
    /// The course it runs.
    #[model(references = "edtech_courses")]
    pub course_id: i64,
    /// The type of the campus (a wl-scheduling location), when it has one.
    pub campus_type: Option<String>,
    /// The id of the campus.
    pub campus_id: Option<i64>,
    /// The seats of the batch (`EDU.ACL.17`).
    pub capacity: i64,
    /// The seats taken (see [`crate::seats`]).
    pub seats_taken: i64,
    /// The first day the pattern may give a session.
    pub start_date: NaiveDate,
    /// The last day the pattern may give a session.
    pub end_date: NaiveDate,
    /// The weekdays of the pattern, 1 = Monday ... 7 = Sunday.
    pub weekdays: Json<Vec<i64>>,
    /// The start time of every session, `HH:MM`, Asia/Dubai.
    pub start_time: String,
    /// The length of every session in minutes.
    pub duration_minutes: i64,
    /// The tutor, a wl-scheduling resource.
    pub tutor_resource_id: Option<i64>,
    /// The employee behind the tutor (wl-hr).
    pub tutor_employee_id: Option<i64>,
    /// The room, a wl-scheduling resource; `None` for an online batch.
    pub room_resource_id: Option<i64>,
    /// The persons the room holds: the hard limit the centre manager may
    /// reach (`EDU.ACL.17`); `None` when the room states none.
    pub room_limit: Option<i64>,
    /// Why the batch was cancelled.
    pub cancel_reason: Option<String>,
    /// Why the batch was terminated.
    pub termination_reason: Option<String>,
    /// What the `cancel` or `terminate` did with each enrolment, written by its
    /// effect and read once after the commit ([`crate::exit`]).
    pub exit_log: Json<Vec<ExitOutcome>>,
    /// The user who asked for the `cancel` or `terminate`: told when a transfer
    /// fell back to a withdrawal.
    pub exit_requested_by: Option<i64>,
    /// The termination is for a regulatory reason (approval expired or
    /// revoked): the centre manager and the owner are told.
    pub exit_regulatory: bool,
    /// When the people were told and the money documents made; the claim that
    /// keeps the after-commit step to one run.
    pub exit_finished_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: BatchStatus,
    /// The user who planned it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Batch {
    /// The number people read: `BAT-0007`.
    pub fn number(&self) -> String {
        format!("BAT-{:04}", self.id)
    }

    /// The batch as a [`Subject`]: `edtech.batch:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(BATCH_SUBJECT_TYPE, self.id)
    }

    /// The campus as a [`Subject`], when the batch has one.
    pub fn campus(&self) -> Option<Subject> {
        match (&self.campus_type, self.campus_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The weekdays of the pattern, in order.
    pub fn weekday_list(&self) -> Vec<i64> {
        let mut days: Vec<i64> = self
            .weekdays
            .0
            .iter()
            .copied()
            .filter(|day| (1..=7).contains(day))
            .collect();
        days.sort_unstable();
        days.dedup();
        days
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_batch_is_numbered_and_is_a_subject() {
        let batch = Batch {
            id: 7,
            ..Batch::default()
        };
        assert_eq!(batch.number(), "BAT-0007");
        assert_eq!(batch.subject().to_string(), "edtech.batch:7");
    }

    #[test]
    fn the_weekdays_are_sorted_and_only_the_real_ones() {
        let batch = Batch {
            weekdays: Json(vec![3, 1, 3, 9, 0]),
            ..Batch::default()
        };
        assert_eq!(batch.weekday_list(), [1, 3]);
    }

    #[test]
    fn only_the_three_first_states_are_live() {
        assert!(BatchStatus::Planned.is_live());
        assert!(BatchStatus::Open.is_live());
        assert!(BatchStatus::Running.is_live());
        assert!(!BatchStatus::Completed.is_live());
        assert!(!BatchStatus::Cancelled.is_live());
        assert!(!BatchStatus::Terminated.is_live());
    }
}
