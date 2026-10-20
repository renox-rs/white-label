//! The Retention (02 Construction §6, §7.1 *Retention*): the amount withheld
//! on certified IPAs, one record per release part (the first and the final).
//! Only `make_due` is built, because the termination needs it; `deduct`,
//! `invoice` and `settle` come with the issues that build them, and a fixture
//! is inserted in the state it needs.
//!
//! ```text
//! held --make_due--> due
//! ```
//!
//! `make_due` is run by the system: for both parts when the project is
//! terminated, unless the project says the retention does not become due on
//! termination (`retention_due_on_termination`, `⚠ open: lawyer (L-37)`).

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a retention part.
pub const SUBJECT_TYPE: &str = "construction.retention";

/// The `make_due` transition's name.
pub const MAKE_DUE: &str = "make_due";

/// Which release part it is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RetentionPart {
    /// Released at practical completion.
    #[default]
    First,
    /// Released at the end of the defects liability period.
    Final,
}

/// Where a retention part stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RetentionStatus {
    /// Withheld on certified IPAs; no tax document yet.
    #[default]
    Held,
    /// Release condition met; to be invoiced.
    Due,
    /// Retention tax invoice issued.
    Invoiced,
    /// Retention invoice paid, credited or written off (terminal).
    Settled,
    /// Kept in full by the client before it was invoiced (terminal).
    Kept,
}

impl RetentionStatus {
    /// The states in which the part is not finished.
    pub const OPEN: [RetentionStatus; 3] = [
        RetentionStatus::Held,
        RetentionStatus::Due,
        RetentionStatus::Invoiced,
    ];
}

/// One release part of the retention of a project.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "construction_retentions",
    state_machine,
    unique(project_id, part)
)]
pub struct Retention {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// First or final.
    pub part: RetentionPart,
    /// The amount held, in fils.
    pub amount: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: RetentionStatus,
    /// When it became due.
    pub due_since: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Retention {
    /// A held part of `project_id` of `amount` fils.
    pub fn new(project_id: i64, part: RetentionPart, amount: i64) -> Self {
        Retention {
            project_id,
            part,
            amount,
            ..Retention::default()
        }
    }

    /// The part as a [`Subject`]: `construction.retention:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }
}

impl StateMachine for Retention {
    type State = RetentionStatus;
    const COLUMN: renox::db::Col<Retention, RetentionStatus> = Retention::STATUS;

    fn machine() -> Machine<Self> {
        use RetentionStatus::{Due, Held, Invoiced, Kept, Settled};
        Machine::new()
            .initial([Held])
            .state(Held)
            .tone("info")
            .state(Due)
            .tone("warning")
            .state(Invoiced)
            .tone("info")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Kept)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(MAKE_DUE)
                    .label("Make due")
                    .system()
                    .from([Held])
                    .to(Due)
                    .effect(|part: &Retention, _ctx, tx| {
                        let id = part.id;
                        Box::pin(async move {
                            let since = Some(renox::db::now());
                            Retention::query()
                                .where_(Retention::ID.eq(id))
                                .update(&mut *tx, &[("due_since", &since)])
                                .await?;
                            Ok(())
                        })
                    }),
            )
    }
}

/// The parts of `project_id`, first then final, read in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn parts_in(tx: &mut Transaction, project_id: i64) -> Result<Vec<Retention>> {
    Retention::query()
        .where_(Retention::PROJECT_ID.eq(project_id))
        .order_by(Retention::ID)
        .get(&mut *tx)
        .await
}

/// The parts of `project_id` that are not finished.
///
/// # Errors
///
/// A database error.
pub async fn open_of(db: &Db, project_id: i64) -> Result<Vec<Retention>> {
    Retention::query()
        .where_(Retention::PROJECT_ID.eq(project_id))
        .where_(Retention::STATUS.is_in(RetentionStatus::OPEN))
        .order_by(Retention::ID)
        .get(db)
        .await
}

/// Makes every `held` part of `project_id` due, as the system, inside `tx`.
/// Returns how many it made due.
///
/// # Errors
///
/// A database error.
pub async fn make_all_due_in(tx: &mut Transaction, project_id: i64) -> Result<usize> {
    let mut made = 0;
    for part in parts_in(tx, project_id).await? {
        if part.status != RetentionStatus::Held {
            continue;
        }
        // Nothing listens to a retention part yet: the outcome is not emitted.
        let _outcome = part
            .transition(MAKE_DUE)
            .by_system()
            .run_in(&mut *tx)
            .await
            .map_err(TransitionError::into_error)?;
        made += 1;
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_held_part_becomes_due() {
        let machine = Retention::machine();
        assert_eq!(machine.initial_states(), ["held"]);
        let make_due = machine.find(MAKE_DUE).expect("transition");
        assert!(make_due.is_system());
        assert_eq!(make_due.from_states(), ["held"]);
        assert_eq!(make_due.to_state(), Some("due"));
    }
}
