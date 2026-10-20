//! The IPA, the interim payment application (02 Construction §6, §7.1
//! *IPA*), as far as the termination and the final account need it: the
//! model, the machine's states and the final account IPA of a terminated
//! project. `submit`, `certify`, `invoice` and the calculation of an IPA come
//! with the issues that build them; a fixture is inserted in the state it
//! needs.
//!
//! The **final account IPA** is the one of kind `final_account`, valuing the
//! work to the effective date of the termination, the materials on site, the
//! approved variations, the open claims and the termination costs. At a
//! termination a `draft` IPA of the project becomes it (its kind and its
//! valuation date change); an IPA still `submitted` stays and is certified
//! normally; without a draft a new one is made ([`ensure_final_account_in`]).
//! A decision of a court or an arbitrator is recorded as its certificate,
//! with the judgment attached ([`Ipa::decision_reference`]).

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of an IPA.
pub const SUBJECT_TYPE: &str = "construction.ipa";

/// What kind of IPA it is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IpaKind {
    /// A monthly progress payment.
    #[default]
    Interim,
    /// The final account of a terminated project.
    FinalAccount,
}

/// Where an IPA stands (§7.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IpaStatus {
    /// Progress being measured.
    #[default]
    Draft,
    /// Sent; locked (`CON.ACL.05`).
    Submitted,
    /// Certificate received; certified amount and deductions recorded.
    Certified,
    /// Tax invoice issued for the certified amount (terminal).
    Invoiced,
    /// Replaced by a new revision, or by a later IPA (terminal).
    Superseded,
}

/// An interim payment application.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "construction_ipas", state_machine, index(project_id))]
pub struct Ipa {
    /// The primary key.
    pub id: i64,
    /// The project.
    pub project_id: i64,
    /// Interim or final account.
    pub kind: IpaKind,
    /// The revision, from 1.
    pub revision: i64,
    /// The day the work is valued to.
    pub valuation_on: Option<NaiveDate>,
    /// What was claimed, in fils.
    pub claimed_amount: i64,
    /// The certified work, before deductions, in fils.
    pub certified_amount: Option<i64>,
    /// The reference of the court or arbitration decision recorded as the
    /// certificate (the judgment is attached).
    pub decision_reference: Option<String>,
    /// The amount the court or the arbitrator decided, in fils.
    pub decided_amount: Option<i64>,
    /// The tax invoice in wl-invoicing, once invoiced.
    pub invoice_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: IpaStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Ipa {
    /// A draft IPA of `project_id`.
    pub fn new(project_id: i64) -> Self {
        Ipa {
            project_id,
            revision: 1,
            ..Ipa::default()
        }
    }

    /// The IPA as a [`Subject`]: `construction.ipa:3`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// Whether the client's certificate (or a recorded decision that stands
    /// for it) is in: `certified` or `invoiced`.
    pub fn is_certified(&self) -> bool {
        matches!(self.status, IpaStatus::Certified | IpaStatus::Invoiced)
    }
}

impl StateMachine for Ipa {
    type State = IpaStatus;
    const COLUMN: renox::db::Col<Ipa, IpaStatus> = Ipa::STATUS;

    fn machine() -> Machine<Self> {
        use IpaStatus::{Certified, Draft, Invoiced, Submitted, Superseded};
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Submitted)
            .tone("info")
            .state(Certified)
            .tone("success")
            .state(Invoiced)
            .tone("success")
            .terminal()
            .state(Superseded)
            .tone("neutral")
            .terminal()
    }
}

/// The final account IPA of `project_id`, if there is one.
///
/// # Errors
///
/// A database error.
pub async fn final_account_of(db: &Db, project_id: i64) -> Result<Option<Ipa>> {
    Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::KIND.eq(IpaKind::FinalAccount))
        .first(db)
        .await
}

/// The number of IPAs of `project_id`, whatever their state.
///
/// # Errors
///
/// A database error.
pub async fn count_of(db: &Db, project_id: i64) -> Result<usize> {
    Ok(Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .get(db)
        .await?
        .len())
}

/// Makes sure the project has a final account IPA valued to `valuation_on`,
/// inside `tx`: the one it has, else its latest `draft` IPA turned into it,
/// else a new `draft`. An IPA that is `submitted` or later is never touched.
///
/// # Errors
///
/// A database error.
pub async fn ensure_final_account_in(
    tx: &mut Transaction,
    project_id: i64,
    valuation_on: NaiveDate,
) -> Result<Ipa> {
    let existing = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::KIND.eq(IpaKind::FinalAccount))
        .first(&mut *tx)
        .await?;
    if let Some(found) = existing {
        return Ok(found);
    }
    let drafts = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.eq(IpaStatus::Draft))
        .order_by(Ipa::ID)
        .get(&mut *tx)
        .await?;
    if let Some(draft) = drafts.last() {
        let kind = IpaKind::FinalAccount.as_str().to_owned();
        let valued = Some(valuation_on);
        let now = renox::db::now();
        Ipa::query()
            .where_(Ipa::ID.eq(draft.id))
            .update(
                &mut *tx,
                &[
                    ("kind", &kind),
                    ("valuation_on", &valued),
                    ("updated_at", &now),
                ],
            )
            .await?;
        return Ipa::find_or_404(&mut *tx, draft.id).await;
    }
    let mut made = Ipa {
        kind: IpaKind::FinalAccount,
        valuation_on: Some(valuation_on),
        ..Ipa::new(project_id)
    };
    made.insert(&mut *tx).await?;
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ipa_starts_a_draft_and_two_states_are_terminal() {
        let machine = Ipa::machine();
        assert_eq!(machine.initial_states(), ["draft"]);
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|state| state.terminal)
            .map(|state| state.name.as_str())
            .collect();
        assert_eq!(terminal, ["invoiced", "superseded"]);
    }

    #[test]
    fn a_certified_or_invoiced_ipa_has_its_certificate() {
        let mut ipa = Ipa::new(1);
        assert!(!ipa.is_certified());
        ipa.status = IpaStatus::Submitted;
        assert!(!ipa.is_certified());
        ipa.status = IpaStatus::Certified;
        assert!(ipa.is_certified());
        ipa.status = IpaStatus::Invoiced;
        assert!(ipa.is_certified());
    }
}
