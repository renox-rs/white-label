//! The seats of a batch (10 `EDU.ACL.17`, BL-EDU-02).
//!
//! A seat is taken in one atomic step: [`take`] runs inside the caller's
//! transaction, touches the batch row first (the write lock of SQLite and the
//! row lock of PostgreSQL, so a second taker waits for the first to commit),
//! then counts the enrolments of the batch that hold a seat (`pending_payment`,
//! `active` and `frozen`, [`EnrolmentStatus::takes_a_seat`]) and refuses when
//! they reach the limit. The caller inserts its enrolment in the same
//! transaction before it commits, so the second taker, once it runs, sees the
//! full count. Callers open the transaction with `Db::begin_immediate` so a
//! concurrent taker waits at the start instead of failing at its first write.
//!
//! The enrolments are the truth. The `seats_taken` column of the batch is a
//! copy for the lists ([`taken`], [`free`]): [`take`] sets it to the count
//! including the seat just taken, [`release`] sets it to the count as it is.
//! A seat is taken by creating an enrolment, by `transfer` into the batch and
//! by re-enrolment; `unfreeze` takes none (a frozen enrolment keeps its seat);
//! `lapsed`, `withdrawn`, `transferred` and `completed` enrolments count for
//! nothing.
//!
//! A taker who is refused is offered the other `open` batches of the same
//! course (or of the same programme and level) with free seats, and the
//! waiting list of the batch ([`full_message`], [`crate::waiting`]).

use renox::db::{Executor, Transaction, now};
use renox::prelude::*;

use crate::batch::{Batch, BatchStatus};
use crate::course::Course;
use crate::db::refused;
use crate::enrolment::{Enrolment, EnrolmentStatus};
use crate::texts::english;

/// How far a taker may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// The capacity of the batch: a counsellor or a guardian in the portal.
    Capacity,
    /// The room's limit when the room states one, else the capacity: the
    /// centre manager only (`EDU.ACL.17`).
    Room,
}

/// The statuses that hold a seat.
const SEAT_STATUSES: [EnrolmentStatus; 3] = [
    EnrolmentStatus::PendingPayment,
    EnrolmentStatus::Active,
    EnrolmentStatus::Frozen,
];

/// The seats taken in `batch`, as the batch row last counted them.
pub fn taken(batch: &Batch) -> i64 {
    batch.seats_taken
}

/// The seats free in `batch`; 0 when it is full or over capacity.
pub fn free(batch: &Batch) -> i64 {
    (batch.capacity - batch.seats_taken).max(0)
}

/// The enrolments of batch `batch_id` that hold a seat now, counted.
///
/// # Errors
///
/// The database's errors.
pub async fn count_in<'c, E: Executor<'c>>(db: E, batch_id: i64) -> Result<i64> {
    let held = Enrolment::query()
        .where_(Enrolment::BATCH_ID.eq(batch_id))
        .where_(Enrolment::STATUS.is_in(SEAT_STATUSES))
        .count(db)
        .await?;
    Ok(i64::try_from(held).unwrap_or(i64::MAX))
}

/// The seats free in batch `batch_id`, by the count of its enrolments.
///
/// # Errors
///
/// A 404 for an unknown batch; the database's errors.
pub async fn available(db: &Db, batch_id: i64) -> Result<i64> {
    let batch = Batch::find_or_404(db, batch_id).await?;
    Ok((batch.capacity - count_in(db, batch_id).await?).max(0))
}

/// The number of seats the batch may hold for `limit`.
pub fn ceiling(batch: &Batch, limit: Limit) -> i64 {
    match limit {
        Limit::Capacity => batch.capacity,
        Limit::Room => batch
            .room_limit
            .map_or(batch.capacity, |room| room.max(batch.capacity)),
    }
}

/// Sets the copy of the count on the batch row to `seats`.
async fn store(tx: &mut Transaction, batch_id: i64, seats: i64) -> Result {
    renox::db::sql(format!(
        "UPDATE {} SET seats_taken = ?, updated_at = ? WHERE id = ?",
        Batch::TABLE
    ))
    .bind(seats)
    .bind(now())
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Takes `count` seats of batch `batch_id` in `tx`: locks the batch row,
/// counts the enrolments that hold a seat and refuses when they and `count`
/// more would pass the ceiling of `limit`. The caller inserts its enrolment
/// before it commits.
///
/// # Errors
///
/// A 404 for an unknown batch; a 422 when the batch cannot hold `count` more
/// seats within `limit` ([`is_full`]); the database's errors.
pub async fn take(tx: &mut Transaction, batch_id: i64, count: i64, limit: Limit) -> Result {
    // The first statement is a write: the lock is taken before the count is
    // read, so nobody counts a stale number.
    let touched = renox::db::sql(format!(
        "UPDATE {} SET updated_at = ? WHERE id = ?",
        Batch::TABLE
    ))
    .bind(now())
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    if touched == 0 {
        return Err(Error::NotFound);
    }
    let batch = Batch::find_or_404(&mut *tx, batch_id).await?;
    let held = count_in(&mut *tx, batch_id).await?;
    if held + count > ceiling(&batch, limit) {
        return Err(refused(english("edtech.seats.problem.full", &[])));
    }
    store(tx, batch_id, held + count).await
}

/// Gives the seats of batch `batch_id` back: the copy on the batch row is set
/// to the count of the enrolments that hold a seat now (the caller has moved
/// its enrolment out of the seat statuses). `count` is kept for the callers
/// that name how many seats they free.
///
/// # Errors
///
/// The database's errors.
pub async fn release(tx: &mut Transaction, batch_id: i64, _count: i64) -> Result {
    let held = count_in(&mut *tx, batch_id).await?;
    store(tx, batch_id, held).await
}

/// Whether `error` is the refusal of [`take`] for a full batch.
pub fn is_full(error: &Error) -> bool {
    matches!(
        error,
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
            if *message == english("edtech.seats.problem.full", &[])
    )
}

/// Another `open` batch with free seats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alternative {
    /// The batch.
    pub batch_id: i64,
    /// `BAT-0007`.
    pub number: String,
    /// The seats free in it.
    pub free: i64,
}

/// The other `open` batches of the same course, or of the same programme and
/// level, that have a free seat (what a refused taker is offered).
///
/// # Errors
///
/// The database's errors.
pub async fn alternatives(db: &Db, batch_id: i64) -> Result<Vec<Alternative>> {
    let batch = Batch::find_or_404(db, batch_id).await?;
    let course = Course::find_or_404(db, batch.course_id).await?;
    let mut courses = vec![course.id];
    if let Some(level) = course.level.clone() {
        let same = Course::query()
            .where_(Course::PROGRAMME.eq(course.programme.clone()))
            .where_(Course::LEVEL.eq(level))
            .get(db)
            .await?;
        courses.extend(same.iter().map(|c| c.id).filter(|id| *id != course.id));
    }
    let open = Batch::query()
        .where_(Batch::STATUS.eq(BatchStatus::Open))
        .where_(Batch::COURSE_ID.is_in(courses))
        .order_by(Batch::START_DATE)
        .order_by(Batch::ID)
        .get(db)
        .await?;
    let mut found = Vec::new();
    for other in open.into_iter().filter(|b| b.id != batch.id) {
        let room = (other.capacity - count_in(db, other.id).await?).max(0);
        if room > 0 {
            found.push(Alternative {
                batch_id: other.id,
                number: other.number(),
                free: room,
            });
        }
    }
    Ok(found)
}

/// The words of the refusal of a full batch: the other open batches with a
/// free seat, and the waiting list.
///
/// # Errors
///
/// The database's errors.
pub async fn full_message(db: &Db, batch_id: i64) -> Result<String> {
    let others = alternatives(db, batch_id).await?;
    if others.is_empty() {
        return Ok(english("edtech.seats.problem.full_no_alternative", &[]));
    }
    let list: Vec<String> = others
        .iter()
        .map(|a| {
            english(
                "edtech.seats.alternative",
                &[("batch", &a.number), ("free", &a.free)],
            )
        })
        .collect();
    let list = list.join(", ");
    Ok(english(
        "edtech.seats.problem.full_with_alternatives",
        &[("batches", &list)],
    ))
}

/// Turns the refusal of [`take`] into the refusal with the alternatives and
/// the waiting list; any other error is returned as it is.
///
/// # Errors
///
/// The refusal, or the database's errors.
pub async fn offer(db: &Db, batch_id: i64, error: Error) -> Error {
    if !is_full(&error) {
        return error;
    }
    match full_message(db, batch_id).await {
        Ok(message) => refused(message),
        Err(other) => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(capacity: i64, taken: i64, room: Option<i64>) -> Batch {
        Batch {
            capacity,
            seats_taken: taken,
            room_limit: room,
            ..Batch::default()
        }
    }

    #[test]
    fn the_free_seats_are_the_capacity_less_the_taken_never_below_none() {
        assert_eq!(free(&batch(12, 11, None)), 1);
        assert_eq!(free(&batch(12, 12, None)), 0);
        assert_eq!(free(&batch(12, 13, Some(14))), 0);
        assert_eq!(taken(&batch(12, 5, None)), 5);
    }

    #[test]
    fn only_the_room_limit_goes_over_the_capacity_and_a_room_without_one_does_not() {
        assert_eq!(ceiling(&batch(12, 0, Some(14)), Limit::Capacity), 12);
        assert_eq!(ceiling(&batch(12, 0, Some(14)), Limit::Room), 14);
        assert_eq!(ceiling(&batch(12, 0, None), Limit::Room), 12);
        // A room smaller than the capacity never lowers it.
        assert_eq!(ceiling(&batch(12, 0, Some(10)), Limit::Room), 12);
    }

    #[test]
    fn only_the_three_open_statuses_hold_a_seat() {
        for status in SEAT_STATUSES {
            assert!(status.takes_a_seat());
        }
        for status in [
            EnrolmentStatus::Lapsed,
            EnrolmentStatus::Withdrawn,
            EnrolmentStatus::Transferred,
            EnrolmentStatus::Completed,
        ] {
            assert!(!status.takes_a_seat());
        }
    }

    #[test]
    fn the_refusal_of_a_full_batch_is_recognised() {
        let full = refused(english("edtech.seats.problem.full", &[]));
        assert!(is_full(&full));
        assert!(!is_full(&refused("another".to_owned())));
        assert!(!is_full(&Error::Forbidden));
    }
}
