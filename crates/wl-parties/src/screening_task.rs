//! The compliance review task of a possible match (wl-parties §4, `PTY.SCR.01`,
//! issue #38).
//!
//! A `flag` opens one [`ScreeningTask`] per party, due 24 elapsed hours after
//! the flag. While it is open, a later flagged run is linked to it instead of
//! opening a second one and does not move the due time. The task is for the
//! compliance role only (`parties.block_reason.view`); the routes that show
//! it come later. [`ScreeningTask::close_for_party`] closes it for the
//! decisions that follow.

use renox::chrono::Duration;
use renox::db::{DateTime, Executor, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::screening::{Screening, ScreeningCandidate};

/// Hours between the flag and the task's due time (elapsed, not working).
pub const DUE_HOURS: i64 = 24;

/// A time as the back office and the alerts show it: Asia/Dubai, to the
/// minute (stored UTC, wl-parties §4).
pub fn dubai_label(at: DateTime) -> String {
    at.with_timezone(&wl_kernel::BusinessCalendar::offset())
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// Where a task stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ScreeningTaskStatus {
    /// Waiting for the compliance decision.
    #[default]
    Open,
    /// Decided.
    Closed,
}

/// The review task of a flagged party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_screening_tasks", index(party_id), index(status))]
pub struct ScreeningTask {
    /// The primary key.
    pub id: i64,
    /// The flagged party.
    pub party_id: i64,
    /// Open until a decision closes it.
    pub status: ScreeningTaskStatus,
    /// When the review is due: the flag time plus 24 hours.
    pub due_at: DateTime,
    /// When the first flag happened.
    pub flagged_at: DateTime,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ScreeningTask {
    /// The party's open task, if any.
    pub async fn open_for_party<'c>(
        db: impl Executor<'c>,
        party_id: i64,
    ) -> Result<Option<ScreeningTask>> {
        ScreeningTask::query()
            .where_(ScreeningTask::PARTY_ID.eq(party_id))
            .where_(ScreeningTask::STATUS.eq(ScreeningTaskStatus::Open))
            .order_by(ScreeningTask::ID)
            .first(db)
            .await
    }

    /// The task for a flag of `party_id` at `flagged_at`, in the caller's
    /// transaction: the open one when there is one (its due time stays),
    /// else a new one due [`DUE_HOURS`] later. The flag bool is `true` when
    /// the task was opened now.
    pub async fn for_flag(
        tx: &mut Transaction,
        party_id: i64,
        flagged_at: DateTime,
    ) -> Result<(ScreeningTask, bool)> {
        if let Some(open) = ScreeningTask::open_for_party(&mut *tx, party_id).await? {
            return Ok((open, false));
        }
        let mut task = ScreeningTask {
            party_id,
            status: ScreeningTaskStatus::Open,
            due_at: flagged_at + Duration::hours(DUE_HOURS),
            flagged_at,
            ..ScreeningTask::default()
        };
        task.insert(&mut *tx).await?;
        Ok((task, true))
    }

    /// Closes the party's open task, now, in the caller's transaction.
    /// Idempotent: returns `true` when a task was closed and `false` when
    /// none was open. The next flag then opens a new task.
    pub async fn close_for_party(tx: &mut Transaction, party_id: i64) -> Result<bool> {
        let Some(mut task) = ScreeningTask::open_for_party(&mut *tx, party_id).await? else {
            return Ok(false);
        };
        task.status = ScreeningTaskStatus::Closed;
        task.closed_at = Some(renox::db::now());
        task.save(&mut *tx).await?;
        Ok(true)
    }

    /// The open tasks, the earliest due first.
    pub async fn open_tasks<'c>(db: impl Executor<'c>) -> Result<Vec<ScreeningTask>> {
        ScreeningTask::query()
            .where_(ScreeningTask::STATUS.eq(ScreeningTaskStatus::Open))
            .order_by(ScreeningTask::DUE_AT)
            .order_by(ScreeningTask::ID)
            .get(db)
            .await
    }

    /// The screening runs listed on the task, oldest first.
    pub async fn screenings<'c>(&self, db: impl Executor<'c>) -> Result<Vec<Screening>> {
        Screening::query()
            .where_(Screening::TASK_ID.eq(self.id))
            .order_by(Screening::ID)
            .get(db)
            .await
    }

    /// The candidates of every run listed on the task, best score first.
    pub async fn candidates(&self, db: &renox::db::Db) -> Result<Vec<ScreeningCandidate>> {
        let ids: Vec<i64> = self.screenings(db).await?.iter().map(|s| s.id).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        ScreeningCandidate::query()
            .where_in("screening_id", ids)
            .order_by_desc(ScreeningCandidate::SCORE)
            .order_by(ScreeningCandidate::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_task_is_open() {
        assert_eq!(ScreeningTask::default().status, ScreeningTaskStatus::Open);
        assert_eq!(DUE_HOURS, 24);
    }
}
