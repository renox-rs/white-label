//! The session: one class meeting of a batch (10 §6 *Session*, §7.1
//! *Session*, `EDU.BAT.01`).
//!
//! Sessions are generated from the batch's weekly pattern when the batch
//! opens ([`crate::sessions::insert_planned`]) and copy the batch's tutor and
//! room, so that one session can move to another time or tutor without moving
//! the batch. The tutor and the room are wl-scheduling resources, named by
//! their id; [`crate::sessions::SessionBlocks`] tells wl-scheduling when they
//! are taken. The status moves only through the machine in
//! [`crate::session_machine`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a session.
pub const SESSION_SUBJECT_TYPE: &str = "edtech.session";

/// Where a session stands (10 §7.1 *Session*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SessionStatus {
    /// Planned with tutor, room and time.
    #[default]
    Scheduled,
    /// Took place; attendance submitted; can be corrected.
    Held,
    /// Did not take place; a make-up may be added (terminal).
    Cancelled,
}

impl SessionStatus {}

/// A session.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_sessions",
    state_machine,
    unique(batch_id, number),
    index(session_date),
    index(status)
)]
pub struct Session {
    /// The primary key.
    pub id: i64,
    /// The batch it belongs to.
    #[model(references = "edtech_batches")]
    pub batch_id: i64,
    /// 1, 2, 3 ... in the batch; a make-up continues the count.
    pub number: i64,
    /// The day (Asia/Dubai).
    pub session_date: NaiveDate,
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
    /// The tutor, a wl-scheduling resource.
    pub tutor_resource_id: Option<i64>,
    /// The employee behind the tutor (wl-hr).
    pub tutor_employee_id: Option<i64>,
    /// The room, a wl-scheduling resource.
    pub room_resource_id: Option<i64>,
    /// The online class link, pasted by staff (`EDU.ONL.01` is Later).
    pub online_link: Option<String>,
    /// Added in place of a cancelled session.
    pub make_up: bool,
    /// Why it was cancelled.
    pub cancel_reason: Option<String>,
    /// When the attendance was submitted.
    pub held_at: Option<DateTime>,
    /// The user who submitted the attendance.
    pub attendance_by: Option<i64>,
    /// Why the tutor is not available or not eligible for it (`EDU.RUL.12`);
    /// `None` while the session is not flagged ([`crate::flags`]).
    pub flag_reason: Option<String>,
    /// When the session was flagged.
    pub flagged_at: Option<DateTime>,
    /// When the centre manager was told that it was still flagged 24 hours
    /// before it starts.
    pub flag_escalated_at: Option<DateTime>,
    /// The alert (tutor and trigger) that flagged it.
    pub flag_alert_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: SessionStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Session {
    /// The session as a [`Subject`]: `edtech.session:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(SESSION_SUBJECT_TYPE, self.id)
    }

    /// The reference wl-scheduling keeps with the block this session makes.
    pub fn block_reference(&self) -> String {
        self.subject().to_string()
    }

    /// Whether the session shows "tutor not available or not eligible".
    pub fn is_flagged(&self) -> bool {
        self.flag_reason.is_some()
    }

    /// Whether the session still holds its tutor and room.
    pub fn holds_resources(&self) -> bool {
        self.status == SessionStatus::Scheduled
    }

    /// The sessions of batch `batch_id`, in date order.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_batch(db: &Db, batch_id: i64) -> Result<Vec<Session>> {
        Session::query()
            .where_(Session::BATCH_ID.eq(batch_id))
            .order_by(Session::STARTS_AT)
            .order_by(Session::NUMBER)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_is_a_subject_and_its_block_reference() {
        let session = Session {
            id: 9,
            ..Session::default()
        };
        assert_eq!(session.subject().to_string(), "edtech.session:9");
        assert_eq!(session.block_reference(), "edtech.session:9");
    }

    #[test]
    fn only_a_scheduled_session_holds_its_resources() {
        let mut session = Session::default();
        assert!(session.holds_resources());
        session.status = SessionStatus::Held;
        assert!(!session.holds_resources());
        session.status = SessionStatus::Cancelled;
        assert!(!session.holds_resources());
    }
}
