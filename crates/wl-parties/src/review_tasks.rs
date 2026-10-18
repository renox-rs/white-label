//! Review tasks for a blocked party (wl-parties §5.6, issue #36).
//!
//! When a party is blocked, an agreement kind whose block effect is `review`
//! gets one task per (party, kind, scope). The task lists the party's open
//! agreements as items; the decider chooses pause, continue or end for each.
//! The task's status moves only through the machine: `open` → `closed`, run
//! by the system once every item has a decision ([`record_decision`]).
//!
//! The task keeps only the display reason (`on_hold` for a restricted code),
//! so nothing on it can tip the party off (`PTY.ACL.02`).

use renox::auth::permissions::Scope;
use renox::db::Executor;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Decision;

/// The system transition that closes a task.
pub const CLOSE: &str = "close";

/// Where a review task stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReviewTaskStatus {
    /// Waiting for decisions.
    #[default]
    Open,
    /// Every item is decided.
    Closed,
}

/// The decision on one item.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ItemDecision {
    /// Not decided yet.
    #[default]
    Pending,
    /// Stop service and billing until unblock.
    Pause,
    /// Leave the agreement running.
    Continue,
    /// End the agreement through its own transition.
    End,
}

impl From<Decision> for ItemDecision {
    fn from(decision: Decision) -> Self {
        match decision {
            Decision::Pause => ItemDecision::Pause,
            Decision::Continue => ItemDecision::Continue,
            Decision::End => ItemDecision::End,
        }
    }
}

/// One review task of a blocked party.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_parties_review_tasks",
    state_machine,
    index(party_id),
    index(status)
)]
pub struct ReviewTask {
    /// The primary key.
    pub id: i64,
    /// The blocked party.
    pub party_id: i64,
    /// The agreement kind key, e.g. `booking`.
    pub agreement_kind: String,
    /// The kind of scope the agreements live in; `None` when global.
    pub scope_kind: Option<String>,
    /// The id of that scope.
    pub scope_id: Option<i64>,
    /// The role that decides; `None` when no decider is configured.
    pub decider_role: Option<String>,
    /// The reason code others may see: the code, or `on_hold`.
    pub display_reason: String,
    /// Where it stands; changed only by a transition.
    pub status: ReviewTaskStatus,
    /// The user who took the task; `None` means the decider role has it.
    pub assigned_user: Option<i64>,
    /// When it was closed.
    pub closed_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One agreement of a task and the decision on it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_review_task_items", index(task_id))]
pub struct ReviewTaskItem {
    /// The primary key.
    pub id: i64,
    /// The task.
    pub task_id: i64,
    /// The agreement's subject type, e.g. `scheduling.booking`.
    pub subject_type: String,
    /// The agreement's id.
    pub subject_id: i64,
    /// A one-line label.
    pub label: String,
    /// The decision; `pending` until the decider chooses.
    pub decision: ItemDecision,
    /// The user who decided.
    pub decided_by: Option<i64>,
    /// When it was decided.
    pub decided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// A note on a task.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_review_task_notes", index(task_id))]
pub struct ReviewTaskNote {
    /// The primary key.
    pub id: i64,
    /// The task.
    pub task_id: i64,
    /// The text.
    pub body: String,
    /// The author; `None` for the system.
    pub by_user: Option<i64>,
    /// When it was written.
    pub noted_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for ReviewTask {
    type State = ReviewTaskStatus;
    const COLUMN: renox::db::Col<ReviewTask, ReviewTaskStatus> = ReviewTask::STATUS;

    fn machine() -> Machine<Self> {
        use ReviewTaskStatus::{Closed, Open};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("warning")
            .state(Closed)
            .tone("success")
            .terminal()
            .transition(
                Transition::new(CLOSE)
                    .label("Close")
                    .tone("success")
                    .from([Open])
                    .to(Closed)
                    .system()
                    .effect(|task: &ReviewTask, _ctx, tx| {
                        let id = task.id;
                        Box::pin(async move {
                            // Not the status column, so outside the machine's guard.
                            renox::db::sql(format!(
                                "UPDATE {} SET closed_at = ?, updated_at = ? WHERE id = ?",
                                ReviewTask::TABLE
                            ))
                            .bind(renox::db::now())
                            .bind(renox::db::now())
                            .bind(id)
                            .execute(&mut *tx)
                            .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

impl ReviewTask {
    /// A new open task for `party_id`, not yet stored. `display_reason` must
    /// already be a display code, never a restricted one.
    pub fn new(
        party_id: i64,
        agreement_kind: &str,
        scope: Option<(&str, i64)>,
        decider_role: Option<&str>,
        display_reason: &str,
    ) -> ReviewTask {
        ReviewTask {
            party_id,
            agreement_kind: agreement_kind.to_owned(),
            scope_kind: scope.map(|(kind, _)| kind.to_owned()),
            scope_id: scope.map(|(_, id)| id),
            decider_role: decider_role.map(str::to_owned),
            display_reason: display_reason.to_owned(),
            ..ReviewTask::default()
        }
    }

    /// The party's open tasks, oldest first.
    pub async fn open_for_party<'c>(
        db: impl Executor<'c>,
        party_id: i64,
    ) -> Result<Vec<ReviewTask>> {
        ReviewTask::query()
            .where_(ReviewTask::PARTY_ID.eq(party_id))
            .where_(ReviewTask::STATUS.eq(ReviewTaskStatus::Open))
            .order_by(ReviewTask::ID)
            .get(db)
            .await
    }

    /// The scope the task's agreements live in; global when it has none.
    pub fn scope(&self) -> Scope {
        match (&self.scope_kind, self.scope_id) {
            (Some(kind), Some(id)) => Scope::new(kind, id),
            _ => Scope::global(),
        }
    }

    /// Whether `user` may decide on this task: holds the decider role in its
    /// scope (or globally). A task with no configured decider role is open
    /// to holders of `parties.block`.
    pub fn can_decide(&self, user: &AuthUser) -> bool {
        match self.decider_role.as_deref() {
            Some(role) => user.has_role_in(role, &self.scope()),
            None => user.has_permission(crate::permissions::BLOCK),
        }
    }

    /// The open tasks `user` may decide on, oldest first.
    pub async fn open_for_user<'c>(
        db: impl Executor<'c>,
        user: &AuthUser,
    ) -> Result<Vec<ReviewTask>> {
        let open = ReviewTask::query()
            .where_(ReviewTask::STATUS.eq(ReviewTaskStatus::Open))
            .order_by(ReviewTask::ID)
            .get(db)
            .await?;
        Ok(open.into_iter().filter(|t| t.can_decide(user)).collect())
    }

    /// The task's items, in the order they were added.
    pub async fn items<'c>(&self, db: impl Executor<'c>) -> Result<Vec<ReviewTaskItem>> {
        ReviewTaskItem::query()
            .where_(ReviewTaskItem::TASK_ID.eq(self.id))
            .order_by(ReviewTaskItem::ID)
            .get(db)
            .await
    }

    /// The task's notes, oldest first.
    pub async fn notes<'c>(&self, db: impl Executor<'c>) -> Result<Vec<ReviewTaskNote>> {
        ReviewTaskNote::query()
            .where_(ReviewTaskNote::TASK_ID.eq(self.id))
            .order_by(ReviewTaskNote::ID)
            .get(db)
            .await
    }

    /// Adds a note by `by` (`None` for the system) now and returns it.
    pub async fn add_note<'c>(
        &self,
        db: impl Executor<'c>,
        by: Option<i64>,
        body: &str,
    ) -> Result<ReviewTaskNote> {
        let mut note = ReviewTaskNote {
            task_id: self.id,
            body: body.trim().to_owned(),
            by_user: by,
            noted_at: renox::db::now(),
            ..ReviewTaskNote::default()
        };
        note.insert(db).await?;
        Ok(note)
    }

    /// Puts the task back to its decider role: clears the assigned user.
    pub async fn assign_to_decider<'c>(&mut self, db: impl Executor<'c>) -> Result<()> {
        self.assigned_user = None;
        self.save(db).await
    }
}

impl ReviewTaskItem {
    /// A new pending item of `task_id` for the agreement `subject_type:subject_id`.
    pub fn pending(task_id: i64, subject_type: &str, subject_id: i64, label: &str) -> Self {
        ReviewTaskItem {
            task_id,
            subject_type: subject_type.to_owned(),
            subject_id,
            label: label.to_owned(),
            ..ReviewTaskItem::default()
        }
    }

    /// Whether no decision has been made.
    pub fn is_pending(&self) -> bool {
        self.decision == ItemDecision::Pending
    }
}

/// Records `decision` by `user` on item `item_id` of `task`, now. When no
/// item is pending any more, the system runs `close` on the task.
///
/// The caller has already checked that `user` holds the decider role and has
/// carried the decision out through the agreement's source; this only stores
/// it. Returns the decided item, or `None` when the task is not open or the
/// item was already decided (nothing is written then).
///
/// # Errors
///
/// `NotFound` when the task has no such item.
pub async fn record_decision(
    state: &AppState,
    task: &ReviewTask,
    item_id: i64,
    user: i64,
    decision: Decision,
) -> Result<Option<ReviewTaskItem>> {
    let mut tx = state.db.begin_immediate().await?;
    let open = ReviewTask::query()
        .where_(ReviewTask::ID.eq(task.id))
        .where_(ReviewTask::STATUS.eq(ReviewTaskStatus::Open))
        .first(&mut tx)
        .await?
        .is_some();
    let mut item = ReviewTaskItem::query()
        .where_(ReviewTaskItem::ID.eq(item_id))
        .where_(ReviewTaskItem::TASK_ID.eq(task.id))
        .first(&mut tx)
        .await?
        .ok_or(renox::Error::NotFound)?;
    if !open || !item.is_pending() {
        return Ok(None);
    }
    item.decision = decision.into();
    item.decided_by = Some(user);
    item.decided_at = Some(renox::db::now());
    item.save(&mut tx).await?;
    let all_decided = task.items(&mut tx).await?.iter().all(|i| !i.is_pending());
    tx.commit().await?;
    if all_decided {
        let _outcome = task.transition(CLOSE).by_system().run(state).await;
    }
    Ok(Some(item))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_machine_closes_an_open_task_by_the_system() {
        let machine = ReviewTask::machine();
        let close = machine.find(CLOSE).expect("transition");
        assert_eq!(close.from_states(), ["open"]);
        assert_eq!(close.to_state(), Some("closed"));
        assert!(close.is_system());
    }

    #[test]
    fn a_new_task_is_open_and_unassigned() {
        let task = ReviewTask::new(
            7,
            "booking",
            Some(("clinics", 3)),
            Some("clinic_manager"),
            "on_hold",
        );
        assert_eq!(task.status, ReviewTaskStatus::Open);
        assert_eq!(task.scope_kind.as_deref(), Some("clinics"));
        assert_eq!(task.scope_id, Some(3));
        assert_eq!(task.assigned_user, None);
        let global = ReviewTask::new(7, "lease", None, None, "on_hold");
        assert_eq!(
            (global.scope_kind, global.scope_id, global.decider_role),
            (None, None, None)
        );
    }

    #[test]
    fn decisions_map_to_items() {
        assert_eq!(ItemDecision::from(Decision::End), ItemDecision::End);
        assert!(ReviewTaskItem::pending(1, "x.y", 2, "l").is_pending());
    }
}
