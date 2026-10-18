//! The "fix and resubmit" task (#26, `BL-KRN-02`): when an approver returns a
//! request for changes, its requester gets one open task, created in the
//! same transaction as the `return`. The task is a kernel record; it is
//! closed when the requester resubmits or says they will not.

use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

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
