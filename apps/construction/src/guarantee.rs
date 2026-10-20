//! The Guarantee (02 Construction §6, §7.1 *Guarantee*): a bank guarantee or
//! a bond of a project. Only the model and the machine's states are built;
//! `issue`, `reduce`, `extend`, `call`, `settle_call`, `release` and `expire`
//! come with the issues that build them, and a fixture is inserted in the
//! state it needs. A termination gives the accountant one follow-up task per
//! `active` guarantee ([`crate::termination`]).

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a guarantee.
pub const SUBJECT_TYPE: &str = "construction.guarantee";

/// What a guarantee secures.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GuaranteeKind {
    /// An advance payment guarantee.
    #[default]
    Advance,
    /// A performance bond.
    Performance,
    /// A bond that replaces a retention part.
    RetentionBond,
}

/// Where a guarantee stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GuaranteeStatus {
    /// Requested from the bank.
    #[default]
    Draft,
    /// Issued and held by the beneficiary.
    Active,
    /// The beneficiary demanded payment.
    Called,
    /// Returned, cancelled by the bank, or ended after a call was paid
    /// (terminal).
    Released,
    /// Expiry date passed without extension (terminal).
    Expired,
}

impl GuaranteeStatus {
    /// The states in which the guarantee is not finished.
    pub const OPEN: [GuaranteeStatus; 2] = [GuaranteeStatus::Active, GuaranteeStatus::Called];
}

/// A bank guarantee or a bond.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_guarantees", state_machine, index(project_id))]
pub struct Guarantee {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// What it secures.
    pub kind: GuaranteeKind,
    /// The bank.
    pub bank: String,
    /// The amount, in fils.
    pub amount: i64,
    /// The expiry date.
    pub expires_on: Option<renox::chrono::NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: GuaranteeStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Guarantee {
    /// A draft guarantee of `kind` for `project_id`.
    pub fn new(project_id: i64, kind: GuaranteeKind, amount: i64) -> Self {
        Guarantee {
            project_id,
            kind,
            amount,
            ..Guarantee::default()
        }
    }

    /// The guarantee as a [`Subject`]: `construction.guarantee:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }
}

impl StateMachine for Guarantee {
    type State = GuaranteeStatus;
    const COLUMN: renox::db::Col<Guarantee, GuaranteeStatus> = Guarantee::STATUS;

    fn machine() -> Machine<Self> {
        use GuaranteeStatus::{Active, Called, Draft, Expired, Released};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Active)
            .tone("success")
            .state(Called)
            .tone("error")
            .state(Released)
            .tone("success")
            .terminal()
            .state(Expired)
            .tone("neutral")
            .terminal()
    }
}

/// The guarantees of `project_id` that are `active`, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn active_of(db: &Db, project_id: i64) -> Result<Vec<Guarantee>> {
    Guarantee::query()
        .where_(Guarantee::PROJECT_ID.eq(project_id))
        .where_(Guarantee::STATUS.eq(GuaranteeStatus::Active))
        .order_by(Guarantee::ID)
        .get(db)
        .await
}

/// The guarantees of `project_id` that are not finished.
///
/// # Errors
///
/// A database error.
pub async fn open_of(db: &Db, project_id: i64) -> Result<Vec<Guarantee>> {
    Guarantee::query()
        .where_(Guarantee::PROJECT_ID.eq(project_id))
        .where_(Guarantee::STATUS.is_in(GuaranteeStatus::OPEN))
        .order_by(Guarantee::ID)
        .get(db)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guarantee_starts_a_draft() {
        let machine = Guarantee::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        assert_eq!(
            Guarantee::new(1, GuaranteeKind::Performance, 5_000).status,
            GuaranteeStatus::Draft
        );
        assert_eq!(GuaranteeKind::RetentionBond.as_str(), "retention_bond");
    }
}
