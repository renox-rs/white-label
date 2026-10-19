//! The legal case flag of a lease (01 Property §7.1 *Lease* "Legal case
//! flag", §4.3 rows "The tenant does not pay after the formal demand" and
//! "The tenant disputes the deductions"; BL-PRP-13).
//!
//! When a tenant does not pay after the formal demand, or a bounced cheque or a
//! deposit dispute goes to the Rental Dispute Centre or a court, the lease
//! carries a legal case flag. It is not a state: a lease may carry several, the
//! effects apply while any is open, and each is cleared separately with an
//! outcome (`paid`, `settled`, `judgment`, `eviction_executed`). The flag
//! belongs to the lease, all its versions: it is stored with the id of version
//! 1.
//!
//! While a flag is open:
//!
//! - **reminders** on the invoices of the lease are paused, as for an invoice
//!   dispute. The vertical registers a [`ChaseHolds`] provider for
//!   `property.lease` ([`LeaseHolds`]): `wl-invoicing` asks it before each
//!   reminder and `wl-payments` before the reminder of a bounced cheque. Invoices
//!   still become `overdue`. When the last flag is cleared, chasing resumes
//!   from the next scheduled step and the steps reached meanwhile are not sent
//!   ([`ChaseHold::Resumed`]);
//! - **renewal and amendment versions** are refused at `submit` and `activate`
//!   with "legal case open" ([`crate::lease_checks`]); a version in `draft`
//!   stays;
//! - **the tenant is sent no automatic message** about the case (the renewal
//!   window notice is not sent); staff may still write by hand;
//! - **`PRP.RUL.10`** (BL-PRP-14) skips the lease: it asks [`has_open`].
//!
//! Every flag has its ticket of the category [`LEGAL_CASE_CATEGORY`]: setting
//! the flag creates it (assigned to the property manager, subject the lease)
//! unless the property manager picks an open *legal case* ticket of the same
//! lease that has no flag yet ([`TicketChoice::Existing`]); a bounced cheque
//! referred to legal creates the flag and its ticket the same way
//! ([`on_cheque_referred`]). Clearing the flag resolves its ticket if still
//! open; resolving the ticket by hand does not clear the flag. Where
//! ticketing is off the flag has no ticket. The category is seeded by the
//! vertical's starter data; where ticketing does not know it yet the ticket is
//! a staff task.
//!
//! A flag of type *deposit dispute* keeps the deductions dispute flag of the
//! deposit in settlement until the case is cleared
//! ([`wl_kernel::deposits::Deposits::set_legal_case`]).
//!
//! The reference may be empty while the case is prepared, but is required to
//! clear with outcome *judgment*. A flag can stay open on a terminated lease
//! (arrears after move-out); it cannot be set on a lease that never started.
//! Clearing with *eviction executed* is done by `force_terminate` (BL-PRP-12),
//! which calls [`clear_case`] for the flags of type *eviction*.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::chase_holds::{ChaseHold, ChaseHolds, HoldFuture, register_chase_holds};
use wl_kernel::cheques::ChequeReferredToLegal;
use wl_kernel::deposits::deposits;
use wl_kernel::documents::today;
use wl_kernel::role_tasks::RoleTask;
use wl_kernel::tickets::{TicketId, task_creator};

use crate::building::{clean, refused};
use crate::lease::{LEASE_SUBJECT_TYPE, Lease, LeaseStatus};
use crate::permissions::{self, PROPERTY_MANAGER_ROLE};
use crate::texts::english;

/// The ticket category of a legal case.
pub const LEGAL_CASE_CATEGORY: &str = "legal_case";

/// The key the ticket of the flag `id` is found by.
pub fn ticket_key(id: i64) -> String {
    format!("property.legal_case:{id}")
}

/// What the case is about.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaseType {
    /// Rent that was not paid after the formal demand.
    #[default]
    Arrears,
    /// An eviction.
    Eviction,
    /// A dispute over the deposit deductions at the Rental Dispute Centre.
    DepositDispute,
    /// Anything else.
    Other,
}

/// Where a flag stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaseStatus {
    /// Set; its effects apply.
    #[default]
    Open,
    /// Cleared with an outcome.
    Cleared,
}

/// How a flag was cleared.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CaseOutcome {
    /// The tenant paid.
    #[default]
    Paid,
    /// The parties settled.
    Settled,
    /// The court or the Rental Dispute Centre gave judgment.
    Judgment,
    /// The eviction was executed (`force_terminate`).
    EvictionExecuted,
}

/// One legal case flag of a lease.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_legal_cases", index(root_id, status))]
pub struct LegalCase {
    /// The primary key.
    pub id: i64,
    /// The id of version 1: the lease all versions belong to.
    pub root_id: i64,
    /// The version the flag was set on.
    #[model(references = "property_leases")]
    pub lease_id: i64,
    /// What it is about.
    pub case_type: CaseType,
    /// The Rental Dispute Centre or court reference.
    pub reference: Option<String>,
    /// Open or cleared.
    pub status: CaseStatus,
    /// How it was cleared.
    pub outcome: Option<CaseOutcome>,
    /// A note on the outcome.
    pub note: Option<String>,
    /// The linked ticket, when ticketing is on.
    pub ticket_id: Option<i64>,
    /// The number of the linked ticket.
    pub ticket_number: Option<String>,
    /// The cheque that was referred to legal, when the flag came from it.
    pub cheque_id: Option<i64>,
    /// The user who set it.
    pub opened_by: Option<i64>,
    /// The day it was set.
    pub opened_on: NaiveDate,
    /// The user who cleared it.
    pub cleared_by: Option<i64>,
    /// The day it was cleared.
    pub cleared_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LegalCase {
    /// The flags of the lease `root_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_root(db: &Db, root_id: i64) -> Result<Vec<LegalCase>> {
        LegalCase::query()
            .where_(LegalCase::ROOT_ID.eq(root_id))
            .order_by(LegalCase::ID)
            .get(db)
            .await
    }

    /// Whether the flag is open.
    pub fn is_open(&self) -> bool {
        self.status == CaseStatus::Open
    }
}

/// Whether the lease `root_id` carries an open legal case flag. The rules that
/// pause for it (`PRP.RUL.10`) and the checks of `submit` and `activate` ask
/// this.
///
/// # Errors
///
/// The database's errors.
pub async fn has_open<'c, E: Executor<'c>>(db: E, root_id: i64) -> Result<bool> {
    LegalCase::query()
        .where_(LegalCase::ROOT_ID.eq(root_id))
        .where_(LegalCase::STATUS.eq(CaseStatus::Open))
        .exists(db)
        .await
}

/// Says whether chasing for the invoices and cheques of a lease is held
/// (registered for `property.lease`).
pub struct LeaseHolds;

impl ChaseHolds for LeaseHolds {
    fn hold<'a>(&'a self, db: &'a Db, source: &'a Subject) -> HoldFuture<'a, ChaseHold> {
        Box::pin(async move {
            let Some(lease) = Lease::find(db, source.subject_id).await? else {
                return Ok(ChaseHold::Free);
            };
            // A lease handed over with its management agreement (BL-PRP-22):
            // its issued invoices stay with the owner and the client stops
            // chasing them.
            if lease.handed_over_on.is_some() {
                return Ok(ChaseHold::Held);
            }
            let cases = LegalCase::of_root(db, lease.root_id).await?;
            if cases.iter().any(LegalCase::is_open) {
                return Ok(ChaseHold::Held);
            }
            Ok(cases
                .iter()
                .filter_map(|case| case.cleared_on)
                .max()
                .map_or(ChaseHold::Free, |on| ChaseHold::Resumed { on }))
        })
    }
}

/// The ticket a flag is linked to when it is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketChoice {
    /// A new *legal case* ticket is created.
    New,
    /// This open *legal case* ticket of the lease, which has no flag yet.
    Existing(i64),
}

/// What a user enters to set a flag.
#[derive(Debug, Clone)]
pub struct NewCase {
    /// What it is about.
    pub case_type: CaseType,
    /// The Rental Dispute Centre or court reference; may be empty while the
    /// case is prepared.
    pub reference: Option<String>,
    /// The ticket.
    pub ticket: TicketChoice,
}

impl NewCase {
    /// A flag of `case_type` with a new ticket and no reference yet.
    pub fn new(case_type: CaseType) -> Self {
        NewCase {
            case_type,
            reference: None,
            ticket: TicketChoice::New,
        }
    }

    /// With the case reference.
    pub fn reference(mut self, reference: &str) -> Self {
        self.reference = Some(reference.to_owned());
        self
    }

    /// Linked to the open ticket `id` instead of a new one.
    pub fn existing_ticket(mut self, id: i64) -> Self {
        self.ticket = TicketChoice::Existing(id);
        self
    }
}

/// The open *legal case* tickets of the lease (every version) that have no
/// flag yet: `(id, number)`.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub async fn free_tickets(state: &AppState, lease: &Lease) -> Result<Vec<(i64, String)>> {
    let taken: Vec<i64> = LegalCase::of_root(&state.db, lease.root_id)
        .await?
        .iter()
        .filter_map(|case| case.ticket_id)
        .collect();
    let creator = task_creator(state);
    let mut found = Vec::new();
    for version in Lease::versions_of(&state.db, lease.root_id).await? {
        let subject = version.subject();
        for ticket in creator
            .open_of(state, &subject, LEGAL_CASE_CATEGORY)
            .await?
        {
            if !taken.contains(&ticket.id.0) {
                found.push((ticket.id.0, ticket.number));
            }
        }
    }
    Ok(found)
}

/// Sets a flag on the lease `lease` for `by` (`None` is the system), with its
/// ticket, and keeps the deductions dispute flag of the deposit when the case
/// is a deposit dispute.
async fn create_case(
    state: &AppState,
    lease: &Lease,
    by: Option<i64>,
    new: NewCase,
    cheque: Option<i64>,
) -> Result<LegalCase> {
    let picked = match new.ticket {
        TicketChoice::New => None,
        TicketChoice::Existing(id) => {
            let free = free_tickets(state, lease).await?;
            match free.into_iter().find(|(free_id, _)| *free_id == id) {
                Some(found) => Some(found),
                None => return Err(refused(english("property.legal.ticket_unavailable", &[]))),
            }
        }
    };
    let mut case = LegalCase {
        root_id: lease.root_id,
        lease_id: lease.id,
        case_type: new.case_type,
        reference: clean(new.reference),
        status: CaseStatus::Open,
        ticket_id: picked.as_ref().map(|(id, _)| *id),
        ticket_number: picked.map(|(_, number)| number),
        cheque_id: cheque,
        opened_by: by,
        opened_on: today(),
        ..Default::default()
    };
    case.insert(&state.db).await?;

    if case.ticket_id.is_none() {
        let kind = english(
            &format!("property.legal.type_{}", case.case_type.as_str()),
            &[],
        );
        let title = english(
            "property.legal.ticket_title",
            &[("type", &kind), ("lease", &lease.number())],
        );
        let description = english(
            "property.legal.ticket_description",
            &[("lease", &lease.number_with_version())],
        );
        let task = RoleTask::new(
            lease.subject(),
            PROPERTY_MANAGER_ROLE,
            &title,
            &ticket_key(case.id),
        )
        .description(&description);
        match crate::tasks::open_in(state, &lease.scope(), task, LEGAL_CASE_CATEGORY).await {
            Ok(Some(ticket)) => {
                case.ticket_id = Some(ticket.id.0);
                case.ticket_number = Some(ticket.number);
                case.save(&state.db).await?;
            }
            // Ticketing is off: the flag has no ticket.
            Ok(None) => {}
            // Never a flag without its ticket.
            Err(error) => {
                case.delete(&state.db).await?;
                return Err(error);
            }
        }
    }

    if case.case_type == CaseType::DepositDispute {
        deposits(state)
            .set_legal_case(state, &lease.agreement(), true)
            .await?;
    }
    LegalCase::find(&state.db, case.id)
        .await?
        .ok_or(Error::NotFound)
}

/// `user` sets a legal case flag on lease `id`.
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 for a lease that
/// never started, or an existing ticket that is not an open *legal case*
/// ticket of the lease without a flag.
pub async fn set_case(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: NewCase,
) -> Result<LegalCase> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    if matches!(
        lease.status,
        LeaseStatus::Draft | LeaseStatus::PendingSignature | LeaseStatus::Cancelled
    ) {
        return Err(refused(english("property.legal.not_started", &[])));
    }
    create_case(state, &lease, Some(user.id), new, None).await
}

/// `user` clears the flag `case_id` with `outcome`: its ticket is resolved if
/// still open, and the deductions dispute flag of the deposit is released when
/// no other deposit dispute case is open. `user` is `None` for the system
/// (`force_terminate`).
///
/// # Errors
///
/// 403 without `leases.manage` in the lease's building; 422 for a flag that is
/// cleared already, or the outcome *judgment* with no reference.
pub async fn clear_case(
    state: &AppState,
    user: Option<&AuthUser>,
    case_id: i64,
    outcome: CaseOutcome,
    note: Option<String>,
) -> Result<LegalCase> {
    let case = LegalCase::find(&state.db, case_id)
        .await?
        .ok_or(Error::NotFound)?;
    let lease = Lease::find(&state.db, case.lease_id)
        .await?
        .ok_or(Error::NotFound)?;
    if let Some(user) = user
        && !user.has_permission_in(permissions::LEASES_MANAGE, &lease.scope())
    {
        return Err(Error::Forbidden);
    }
    if !case.is_open() {
        return Err(refused(english("property.legal.cleared_already", &[])));
    }
    if outcome == CaseOutcome::Judgment && clean(case.reference.clone()).is_none() {
        return Err(refused(english("property.legal.reference_required", &[])));
    }
    let note = clean(note);
    if let Some(ticket) = case.ticket_id {
        let resolution = english(
            "property.legal.ticket_resolved",
            &[("outcome", &outcome.as_str())],
        );
        task_creator(state)
            .resolve_from_source(state, TicketId(ticket), &resolution)
            .await?;
    }
    LegalCase::query()
        .where_(LegalCase::ID.eq(case.id))
        .where_(LegalCase::STATUS.eq(CaseStatus::Open))
        .update(
            &state.db,
            &[
                ("status", &CaseStatus::Cleared),
                ("outcome", &Some(outcome)),
                ("note", &note),
                ("cleared_by", &user.map(|u| u.id)),
                ("cleared_on", &Some(today())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    if case.case_type == CaseType::DepositDispute {
        let others = LegalCase::query()
            .where_(LegalCase::ROOT_ID.eq(case.root_id))
            .where_(LegalCase::CASE_TYPE.eq(CaseType::DepositDispute))
            .where_(LegalCase::STATUS.eq(CaseStatus::Open))
            .exists(&state.db)
            .await?;
        if !others {
            deposits(state)
                .set_legal_case(state, &lease.agreement(), false)
                .await?;
        }
    }
    LegalCase::find(&state.db, case.id)
        .await?
        .ok_or(Error::NotFound)
}

/// A bounced cheque was referred to legal: when it was received with a lease,
/// the lease gets a flag of type *arrears* linked to the cheque, with its
/// ticket; the property manager completes the reference. Once per cheque.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub async fn on_cheque_referred(state: &AppState, event: &ChequeReferredToLegal) -> Result {
    let Some(agreement) = event.agreement.as_ref() else {
        return Ok(());
    };
    if agreement.subject_type != LEASE_SUBJECT_TYPE {
        return Ok(());
    }
    let Some(root) = Lease::find(&state.db, agreement.subject_id).await? else {
        return Ok(());
    };
    let known = LegalCase::query()
        .where_(LegalCase::CHEQUE_ID.eq(event.cheque.0))
        .exists(&state.db)
        .await?;
    if known {
        return Ok(());
    }
    // The flag sits on the newest version, which is the one in force.
    let lease = Lease::versions_of(&state.db, root.root_id)
        .await?
        .pop()
        .unwrap_or(root);
    create_case(
        state,
        &lease,
        Some(event.case_owner),
        NewCase::new(CaseType::Arrears),
        Some(event.cheque.0),
    )
    .await
    .map(drop)
}

/// Registers the hold on chasing and the listener of the cheque referral.
pub(crate) fn register(app: &mut Registry) {
    register_chase_holds(LEASE_SUBJECT_TYPE, LeaseHolds);
    app.listen(|event: ChequeReferredToLegal, state: AppState| async move {
        on_cheque_referred(&state, &event).await
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ticket_of_a_flag_is_found_by_its_key() {
        assert_eq!(ticket_key(4), "property.legal_case:4");
    }

    #[test]
    fn types_statuses_and_outcomes_are_stored_in_snake_case() {
        assert_eq!(CaseType::DepositDispute.as_str(), "deposit_dispute");
        assert_eq!(CaseStatus::Cleared.as_str(), "cleared");
        assert_eq!(CaseOutcome::EvictionExecuted.as_str(), "eviction_executed");
        assert_eq!(CaseStatus::default(), CaseStatus::Open);
    }

    #[test]
    fn a_new_case_has_a_new_ticket_and_no_reference_until_given() {
        let case = NewCase::new(CaseType::Arrears);
        assert_eq!(case.ticket, TicketChoice::New);
        assert_eq!(case.reference, None);
        let picked = NewCase::new(CaseType::Other)
            .reference("RDC-2027-1234")
            .existing_ticket(8);
        assert_eq!(picked.ticket, TicketChoice::Existing(8));
        assert_eq!(picked.reference.as_deref(), Some("RDC-2027-1234"));
    }

    #[test]
    fn only_an_open_flag_is_open() {
        let mut case = LegalCase::default();
        assert!(case.is_open());
        case.status = CaseStatus::Cleared;
        assert!(!case.is_open());
    }
}
