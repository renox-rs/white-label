//! The tutor assignment: who teaches a batch, or one of its sessions, and from
//! when to when (10 §7.1 *Batch* `change_tutor`, *Session* `change_tutor`,
//! `EDU.ACL.14`).
//!
//! The batch and its sessions keep the tutor they have now in their own
//! columns (the timetable reads them); this is the history behind them and
//! what the tutor-eligibility guard and the look-ahead of BL-EDU-08 and
//! BL-EDU-10 walk. A row with a `session_id` is that session's own tutor; a
//! row without one is the batch's tutor from `valid_from` until `valid_to`
//! (open-ended while it holds).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a tutor assignment.
pub const TUTOR_ASSIGNMENT_SUBJECT_TYPE: &str = "edtech.tutor_assignment";

/// A tutor assignment.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_tutor_assignments",
    index(batch_id),
    index(session_id),
    index(tutor_resource_id)
)]
pub struct TutorAssignment {
    /// The primary key.
    pub id: i64,
    /// The batch.
    pub batch_id: i64,
    /// The session, for an assignment of one session.
    pub session_id: Option<i64>,
    /// The tutor, a wl-scheduling resource.
    pub tutor_resource_id: i64,
    /// The employee behind the tutor (wl-hr).
    pub tutor_employee_id: Option<i64>,
    /// The first day the tutor teaches.
    pub valid_from: NaiveDate,
    /// The last day the tutor teaches; `None` while it holds.
    pub valid_to: Option<NaiveDate>,
    /// The user who assigned the tutor.
    pub assigned_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TutorAssignment {
    /// The assignment as a [`Subject`]: `edtech.tutor_assignment:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(TUTOR_ASSIGNMENT_SUBJECT_TYPE, self.id)
    }

    /// Whether the tutor teaches on `day` under this assignment.
    pub fn covers(&self, day: NaiveDate) -> bool {
        self.valid_from <= day && self.valid_to.is_none_or(|to| day <= to)
    }

    /// The assignments of batch `batch_id` and of its sessions, by start.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_batch(db: &Db, batch_id: i64) -> Result<Vec<TutorAssignment>> {
        TutorAssignment::query()
            .where_(TutorAssignment::BATCH_ID.eq(batch_id))
            .order_by(TutorAssignment::VALID_FROM)
            .order_by(TutorAssignment::ID)
            .get(db)
            .await
    }

    /// The assignments of the tutor `resource_id`, by start.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_tutor(db: &Db, resource_id: i64) -> Result<Vec<TutorAssignment>> {
        TutorAssignment::query()
            .where_(TutorAssignment::TUTOR_RESOURCE_ID.eq(resource_id))
            .order_by(TutorAssignment::VALID_FROM)
            .order_by(TutorAssignment::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn an_assignment_covers_the_days_from_its_start_to_its_end() {
        let closed = TutorAssignment {
            valid_from: day(5),
            valid_to: Some(day(9)),
            ..TutorAssignment::default()
        };
        assert!(!closed.covers(day(4)));
        assert!(closed.covers(day(5)));
        assert!(closed.covers(day(9)));
        assert!(!closed.covers(day(10)));
        let open = TutorAssignment {
            valid_from: day(5),
            ..TutorAssignment::default()
        };
        assert!(open.covers(day(30)));
    }
}
