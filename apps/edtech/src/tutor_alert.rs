//! A tutor alert: one trigger that stops a tutor from teaching from a date (10
//! §7.2 `EDU.RUL.12`, BL-EDU-10). The rule is in [`crate::tutor_alerts`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a tutor alert.
pub const TUTOR_ALERT_SUBJECT_TYPE: &str = "edtech.tutor_alert";

/// A document of the tutor expires or expired.
pub const KIND_DOCUMENT: &str = "document";
/// Notice was given to the tutor, or by them.
pub const KIND_NOTICE: &str = "notice";
/// A manual availability block on the tutor (wl-scheduling).
pub const KIND_BLOCK: &str = "block";
/// Approved leave (stays inactive until tutor leave is enabled, `HRM.LEV.01`).
pub const KIND_LEAVE: &str = "leave";

/// A tutor and a trigger: the sessions it flags carry its id.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_tutor_alerts",
    unique(employee_id, trigger_key),
    index(employee_id)
)]
pub struct TutorAlert {
    /// The primary key.
    pub id: i64,
    /// The tutor's employee (wl-hr).
    pub employee_id: i64,
    /// What triggered it, with its date: `document:police_clearance:2026-12-15`.
    pub trigger_key: String,
    /// `document`, `notice`, `block` or `leave`.
    pub kind: String,
    /// The document kind, for a document.
    pub document_kind: Option<String>,
    /// The first day the tutor cannot teach.
    pub from_date: NaiveDate,
    /// The last day, for a block or a leave.
    pub until_date: Option<NaiveDate>,
    /// The exact start of a block.
    pub starts_at: Option<DateTime>,
    /// The exact end of a block.
    pub ends_at: Option<DateTime>,
    /// What the coordinator reads: "police clearance expires on 2026-12-15".
    pub detail: Option<String>,
    /// The key the coordinator's one task is found by.
    pub task_key: String,
    /// When it was resolved: the document renewed, the block over, the notice
    /// withdrawn or nothing left to change.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TutorAlert {
    /// The alert as a [`Subject`].
    pub fn subject(&self) -> Subject {
        Subject::new(TUTOR_ALERT_SUBJECT_TYPE, self.id)
    }

    /// Whether it is still open.
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }

    /// The alerts still open, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn open_ones(db: &Db) -> Result<Vec<TutorAlert>> {
        TutorAlert::query()
            .where_(TutorAlert::CLOSED_AT.is_null())
            .order_by(TutorAlert::ID)
            .get(db)
            .await
    }

    /// The alerts of the tutor `employee_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_employee(db: &Db, employee_id: i64) -> Result<Vec<TutorAlert>> {
        TutorAlert::query()
            .where_(TutorAlert::EMPLOYEE_ID.eq(employee_id))
            .order_by(TutorAlert::ID)
            .get(db)
            .await
    }
}
