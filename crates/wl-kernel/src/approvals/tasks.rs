//! The "fix and resubmit" task (#26, `BL-KRN-02`): when an approver returns a
//! request for changes, its requester gets one open task, created in the
//! same transaction as the `return`. The task is a kernel record; it is
//! closed when the requester resubmits or says they will not.

use renox::db::{Executor, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::texts;

/// Whether a task still waits for the requester.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskStatus {
    /// Waiting for the requester.
    #[default]
    Open,
    /// Done: see the closed reason.
    Closed,
}

/// Why a task was closed.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskClosed {
    /// The requester submitted the action again.
    #[default]
    Resubmitted,
    /// The requester decided to leave it.
    NotResubmitting,
}

/// The requester's task to fix a returned request and resubmit it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_approval_resubmit_tasks", index(requester_id, status))]
pub struct ResubmitTask {
    /// The primary key.
    pub id: i64,
    /// The returned request; one task per request.
    pub request_id: i64,
    /// The user who must fix and resubmit: the request's requester.
    pub requester_id: i64,
    /// `open` until closed.
    pub status: TaskStatus,
    /// Why it was closed; `None` while open.
    pub closed_reason: Option<TaskClosed>,
    /// The new request, when the task was closed by a resubmission.
    pub resubmission_id: Option<i64>,
    /// Who closed it.
    pub closed_by: Option<i64>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ResubmitTask {
    /// Whether the task still waits for the requester.
    pub fn is_open(&self) -> bool {
        self.status == TaskStatus::Open
    }

    /// The open task for `request_id`, if it has one.
    pub async fn open_for<'c>(
        db: impl Executor<'c>,
        request_id: i64,
    ) -> Result<Option<ResubmitTask>> {
        ResubmitTask::query()
            .where_(ResubmitTask::REQUEST_ID.eq(request_id))
            .where_(ResubmitTask::STATUS.eq(TaskStatus::Open))
            .first(db)
            .await
    }
}

/// The 422 for a task that is no longer open.
fn already_closed() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        texts::en("wl_kernel.approvals.task_closed"),
    )
}

/// Closes the open task of `request_id` with `resubmitted`, linking the new
/// request, inside `tx`. A conditional update: a task that is not open (or
/// does not exist) is left alone. Returns whether it closed one.
pub(super) async fn close_resubmitted_in(
    tx: &mut Transaction,
    request_id: i64,
    resubmission_id: i64,
    requester_id: i64,
) -> Result<bool> {
    let closed = ResubmitTask::where_eq("request_id", request_id)
        .where_eq("status", TaskStatus::Open)
        .update(
            &mut *tx,
            &[
                ("status", &TaskStatus::Closed),
                ("closed_reason", &Some(TaskClosed::Resubmitted)),
                ("resubmission_id", &Some(resubmission_id)),
                ("closed_by", &Some(requester_id)),
                ("closed_at", &Some(renox::db::now())),
            ],
        )
        .await?;
    Ok(closed > 0)
}

/// The open tasks of `user`, newest first: what the user must act on.
pub async fn needs_action(db: &Db, user: &User) -> Result<Vec<ResubmitTask>> {
    ResubmitTask::query()
        .where_(ResubmitTask::REQUESTER_ID.eq(user.id))
        .where_(ResubmitTask::STATUS.eq(TaskStatus::Open))
        .order_by_desc(ResubmitTask::ID)
        .get(db)
        .await
}

/// The requester says they will not resubmit request `request_id`: its task
/// closes with `not_resubmitting`. 403 for anyone but the task's requester,
/// 404 when the request has no task, 422 when the task is already closed.
pub async fn close_not_resubmitting(db: &Db, request_id: i64, user: &User) -> Result<ResubmitTask> {
    let task = ResubmitTask::query()
        .where_(ResubmitTask::REQUEST_ID.eq(request_id))
        .first(db)
        .await?
        .ok_or(Error::NotFound)?;
    if task.requester_id != user.id {
        return Err(Error::Forbidden);
    }
    if !task.is_open() {
        return Err(already_closed());
    }
    let closed = ResubmitTask::where_eq("id", task.id)
        .where_eq("status", TaskStatus::Open)
        .update(
            db,
            &[
                ("status", &TaskStatus::Closed),
                ("closed_reason", &Some(TaskClosed::NotResubmitting)),
                ("closed_by", &Some(user.id)),
                ("closed_at", &Some(renox::db::now())),
            ],
        )
        .await?;
    if closed == 0 {
        return Err(already_closed());
    }
    ResubmitTask::find(db, task.id)
        .await?
        .ok_or(Error::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_task_is_open_and_has_no_closing() {
        let task = ResubmitTask::default();
        assert!(task.is_open());
        assert_eq!(task.closed_reason, None);
        assert_eq!(task.resubmission_id, None);
    }

    #[test]
    fn a_closed_task_is_not_open() {
        let task = ResubmitTask {
            status: TaskStatus::Closed,
            closed_reason: Some(TaskClosed::NotResubmitting),
            ..Default::default()
        };
        assert!(!task.is_open());
    }
}
