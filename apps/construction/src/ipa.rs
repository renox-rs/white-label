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
//!
//! ```text
//! draft --submit--> submitted --certify--> certified
//! submitted --supersede--> superseded   (a later IPA submits as its replacement)
//! ```
//!
//! - [`crate::submission`][]: `submit` (the project manager, `ipas.submit`)
//!   and `supersede` (the system, at the replacing `submit`);
//! - [`crate::certificate`][]: `certify` (`ipas.certify`), the late certificate
//!   of a superseded IPA and "previous certified";
//! - [`crate::cutoff`][]: the monthly draft (`CON.RUL.03`);
//! - [`crate::deduction`][]: the typed deductions on a certificate;
//! - [`crate::invoice`][]: `invoice` (`invoices.issue`).
//!
//! `revise` comes with the issue that builds it.

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::certificate::{certify_effects, certify_input, certify_problem};
use crate::invoice::{invoice_effects, invoice_problem};
use crate::permissions;
use crate::submission::{progress_problem, submit_effects, submit_input, submit_problem};

/// The subject type of an IPA.
pub const SUBJECT_TYPE: &str = "construction.ipa";

/// The `submit` transition's name.
pub const SUBMIT: &str = "submit";
/// The `supersede` transition's name.
pub const SUPERSEDE: &str = "supersede";
/// The `certify` transition's name.
pub const CERTIFY: &str = "certify";
/// The `invoice` transition's name.
pub const INVOICE: &str = "invoice";

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
    /// The certified work, before deductions, in fils (cumulative).
    pub certified_amount: Option<i64>,
    /// The sequence number within the project, from 1.
    pub number: i64,
    /// A draft made while an earlier IPA still waits for its certificate.
    pub previous_pending: bool,
    /// The cumulative certified amount of the latest `certified` or
    /// `invoiced` IPA of the project, in fils.
    pub previous_certified: i64,
    /// The pending IPA this one replaces.
    pub replaces_id: Option<i64>,
    /// The cut-off day a monthly draft was made for.
    pub cutoff_on: Option<NaiveDate>,
    /// The day it was submitted.
    pub submitted_on: Option<NaiveDate>,
    /// The day the client certified it.
    pub certified_on: Option<NaiveDate>,
    /// The client's certificate, a document reference.
    pub certificate_ref: Option<String>,
    /// What was claimed less the certified work, recorded at `certify`.
    pub claim_gap: Option<i64>,
    /// A certificate of a superseded IPA that arrived when this one was
    /// certified already: kept for reference, no amount changes.
    pub late_certificate_ref: Option<String>,
    /// The reference of the court or arbitration decision recorded as the
    /// certificate (the judgment is attached).
    pub decision_reference: Option<String>,
    /// The amount the court or the arbitrator decided, in fils.
    pub decided_amount: Option<i64>,
    /// The tax invoice in wl-invoicing, once made.
    pub invoice_id: Option<i64>,
    /// The credit note for the accepted liquidated damages and price
    /// reductions.
    pub credit_note_id: Option<i64>,
    /// The dispute of the contested part of the invoice.
    pub dispute_id: Option<i64>,
    /// The supplier bill from the client for the accepted back- and
    /// contra-charges.
    pub supplier_bill_id: Option<i64>,
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

/// The scope roles are assigned in: the project of the IPA.
fn project_scope(ipa: &Ipa) -> Scope {
    Scope::new(crate::project::SUBJECT_TYPE, ipa.project_id)
}

/// A user transition of the IPA: `permission` in the project's scope.
fn user(name: &str, label: &str, permission: &'static str) -> Transition<Ipa> {
    Transition::new(name)
        .label(label)
        .permission_in(permission, project_scope)
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
            .transition(
                user(SUBMIT, "Submit", permissions::IPAS_SUBMIT)
                    .tone("primary")
                    .from([Draft])
                    .to(Submitted)
                    .guard("progress_recorded", |ipa: &Ipa, _ctx| {
                        let problem = progress_problem(ipa);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .guard("replacement", |_ipa: &Ipa, ctx| {
                        let problem = submit_problem(&submit_input(ctx));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|ipa: &Ipa, ctx, tx| {
                        let id = ipa.id;
                        let input = submit_input(ctx);
                        Box::pin(async move { submit_effects(&mut *tx, id, &input).await })
                    }),
            )
            .transition(
                Transition::new(SUPERSEDE)
                    .label("Supersede")
                    .system()
                    .from([Submitted])
                    .to(Superseded),
            )
            .transition(
                user(CERTIFY, "Certify", permissions::IPAS_CERTIFY)
                    .tone("primary")
                    .from([Submitted])
                    .to(Certified)
                    .guard("certificate", |_ipa: &Ipa, ctx| {
                        let problem = certify_problem(&certify_input(ctx));
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|ipa: &Ipa, ctx, tx| {
                        let ipa = ipa.clone();
                        let input = certify_input(ctx);
                        Box::pin(async move { certify_effects(&mut *tx, &ipa, &input).await })
                    }),
            )
            .transition(
                user(INVOICE, "Invoice", permissions::INVOICES_ISSUE)
                    .tone("primary")
                    .from([Certified])
                    .to(Invoiced)
                    .guard("documents_issued", |ipa: &Ipa, _ctx| {
                        let problem = invoice_problem(ipa);
                        Box::pin(async move { problem.map_or(Ok(()), Err) })
                    })
                    .effect(|ipa: &Ipa, _ctx, tx| {
                        let ipa = ipa.clone();
                        Box::pin(async move { invoice_effects(&mut *tx, &ipa).await })
                    }),
            )
    }
}

/// The IPAs of `project_id` in `status`, oldest number first, inside `tx`.
///
/// # Errors
///
/// A database error.
pub async fn in_status_in(
    tx: &mut Transaction,
    project_id: i64,
    status: IpaStatus,
) -> Result<Vec<Ipa>> {
    Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .where_(Ipa::STATUS.eq(status))
        .order_by(Ipa::NUMBER)
        .get(&mut *tx)
        .await
}

/// The number the next IPA of `project_id` takes, inside `tx`.
///
/// # Errors
///
/// A database error.
pub async fn next_number_in(tx: &mut Transaction, project_id: i64) -> Result<i64> {
    let all = Ipa::query()
        .where_(Ipa::PROJECT_ID.eq(project_id))
        .get(&mut *tx)
        .await?;
    Ok(all.iter().map(|ipa| ipa.number).max().unwrap_or(0) + 1)
}

/// The IPA that replaced `id`, if one did.
///
/// # Errors
///
/// A database error.
pub async fn replacement_of(db: &Db, id: i64) -> Result<Option<Ipa>> {
    Ipa::query().where_(Ipa::REPLACES_ID.eq(id)).first(db).await
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
    let number = next_number_in(&mut *tx, project_id).await?;
    let mut made = Ipa {
        kind: IpaKind::FinalAccount,
        number,
        valuation_on: Some(valuation_on),
        ..Ipa::new(project_id)
    };
    made.insert(&mut *tx).await?;
    wl_kernel::search::index_in(&mut *tx, &Subject::new(SUBJECT_TYPE, made.id)).await?;
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
