//! The make-up right (10 §4.2 row *A student is absent*, `EDU.MKP.01`; BL-EDU-15,
//! BL-EDU-14): an excused absence gives its student **one** right to take a
//! make-up session in another batch, valid until the enrolment ends. A course fee
//! has no credits, a membership consumes nothing, and the excused absence still
//! gives the right. Booking the make-up session itself is `EDU.MKP.01`; here the
//! right is granted when the attendance is submitted, taken away again when a
//! correction says the absence was not excused (if it was not used), and counted.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::attendance::Attendance;
use crate::enrolment::{Enrolment, EnrolmentStatus};
use crate::session::Session;

/// The right to one make-up session, earned by one excused absence.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_make_up_rights",
    unique(attendance_id),
    index(enrolment_id)
)]
pub struct MakeUpRight {
    /// The primary key.
    pub id: i64,
    /// The enrolment that has the right.
    #[model(references = "edtech_enrolments")]
    pub enrolment_id: i64,
    /// The excused absence that gave it.
    #[model(references = "edtech_attendance")]
    pub attendance_id: i64,
    /// The session that was missed.
    pub session_id: i64,
    /// The session where it was taken; `None` while it is unused.
    pub used_session_id: Option<i64>,
    /// When a correction took it away.
    pub voided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl MakeUpRight {
    /// Whether the right is there to take: not used and not taken away.
    pub fn is_open(&self) -> bool {
        self.used_session_id.is_none() && self.voided_at.is_none()
    }

    /// The rights of `enrolment_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_enrolment(db: &Db, enrolment_id: i64) -> Result<Vec<MakeUpRight>> {
        MakeUpRight::query()
            .where_(MakeUpRight::ENROLMENT_ID.eq(enrolment_id))
            .order_by(MakeUpRight::ID)
            .get(db)
            .await
    }
}

/// Gives the right the excused absence `row` earns, once.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn grant(tx: &mut Transaction, row: &Attendance, session: &Session) -> Result {
    let found = MakeUpRight::query()
        .where_(MakeUpRight::ATTENDANCE_ID.eq(row.id))
        .first(&mut *tx)
        .await?;
    match found {
        Some(right) if right.voided_at.is_some() => {
            // The absence is excused again: the right comes back.
            MakeUpRight::query()
                .where_(MakeUpRight::ID.eq(right.id))
                .update(
                    &mut *tx,
                    &[("voided_at", &None::<DateTime>), ("updated_at", &now())],
                )
                .await?;
        }
        Some(_) => {}
        None => {
            let mut right = MakeUpRight {
                enrolment_id: row.enrolment_id,
                attendance_id: row.id,
                session_id: session.id,
                ..MakeUpRight::default()
            };
            right.insert(&mut *tx).await?;
        }
    }
    Ok(())
}

/// Takes away the right of the absence `attendance_id` if it was not used.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn void(tx: &mut Transaction, attendance_id: i64) -> Result {
    MakeUpRight::query()
        .where_(MakeUpRight::ATTENDANCE_ID.eq(attendance_id))
        .where_(MakeUpRight::USED_SESSION_ID.is_null())
        .where_(MakeUpRight::VOIDED_AT.is_null())
        .update(
            &mut *tx,
            &[("voided_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// The rights of `enrolment` that can still be taken: open, and the enrolment
/// has not ended.
///
/// # Errors
///
/// The database's errors.
pub async fn open_rights(db: &Db, enrolment: &Enrolment) -> Result<Vec<MakeUpRight>> {
    if enrolment.status.is_terminal() && enrolment.status != EnrolmentStatus::Completed {
        return Ok(Vec::new());
    }
    Ok(MakeUpRight::of_enrolment(db, enrolment.id)
        .await?
        .into_iter()
        .filter(MakeUpRight::is_open)
        .collect())
}

/// The day a right was earned, for the page: the date of the session missed.
///
/// # Errors
///
/// The database's errors.
pub async fn missed_on(db: &Db, right: &MakeUpRight) -> Result<Option<NaiveDate>> {
    Ok(Session::find(db, right.session_id)
        .await?
        .map(|session| session.session_date))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_right_is_open_until_used_or_taken_away() {
        let mut right = MakeUpRight::default();
        assert!(right.is_open());
        right.used_session_id = Some(4);
        assert!(!right.is_open());
        right.used_session_id = None;
        right.voided_at = Some(now());
        assert!(!right.is_open());
    }
}
