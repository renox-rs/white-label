//! The seats of a batch (10 `EDU.ACL.17`).
//!
//! The boundary BL-EDU-02 builds on: [`available`] says how many seats are
//! free and [`take`] takes one inside the caller's transaction, refusing it
//! when the batch is full. Every enrolment, `transfer` and re-enrolment goes
//! through [`take`]; [`release`] gives a seat back (`lapse`, `withdraw`,
//! `transfer` out). An `unfreeze` takes none: a frozen enrolment keeps its
//! seat.
//!
//! Decision: this is the plain version. The seats taken are a counter on the
//! batch row, moved in one `UPDATE ... WHERE seats_taken < limit`, so two
//! takers of the last seat cannot both win even before enrolments exist. When
//! the enrolments arrive, BL-EDU-02 replaces [`taken`] with the count of the
//! `active`, `frozen` and `pending_payment` enrolments of the batch under a
//! lock on the batch row, and keeps the signatures of [`available`], [`take`]
//! and [`release`].

use renox::db::{Transaction, now};
use renox::prelude::*;

use crate::batch::Batch;
use crate::db::refused;
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

/// The seats taken in `batch`.
pub fn taken(batch: &Batch) -> i64 {
    batch.seats_taken
}

/// The seats free in `batch`; 0 when it is full or over capacity.
pub fn free(batch: &Batch) -> i64 {
    (batch.capacity - batch.seats_taken).max(0)
}

/// The seats free in batch `batch_id`.
///
/// # Errors
///
/// A 404 for an unknown batch; the database's errors.
pub async fn available(db: &Db, batch_id: i64) -> Result<i64> {
    Ok(free(&Batch::find_or_404(db, batch_id).await?))
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

/// Takes `count` seats of batch `batch_id` in `tx`.
///
/// # Errors
///
/// A 404 for an unknown batch; a 422 when the batch cannot hold `count` more
/// seats within `limit`; the database's errors.
pub async fn take(tx: &mut Transaction, batch_id: i64, count: i64, limit: Limit) -> Result {
    let batch = Batch::find_or_404(&mut *tx, batch_id).await?;
    let top = ceiling(&batch, limit);
    let changed = renox::db::sql(format!(
        "UPDATE {} SET seats_taken = seats_taken + ?, updated_at = ? \
         WHERE id = ? AND seats_taken + ? <= ?",
        Batch::TABLE
    ))
    .bind(count)
    .bind(now())
    .bind(batch_id)
    .bind(count)
    .bind(top)
    .execute(&mut *tx)
    .await?;
    if changed == 0 {
        return Err(refused(english("edtech.seats.problem.full", &[])));
    }
    Ok(())
}

/// Gives `count` seats of batch `batch_id` back, never below none.
///
/// # Errors
///
/// The database's errors.
pub async fn release(tx: &mut Transaction, batch_id: i64, count: i64) -> Result {
    renox::db::sql(format!(
        "UPDATE {} SET seats_taken = CASE WHEN seats_taken > ? THEN seats_taken - ? ELSE 0 END, \
         updated_at = ? WHERE id = ?",
        Batch::TABLE
    ))
    .bind(count)
    .bind(count)
    .bind(now())
    .bind(batch_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
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
}
