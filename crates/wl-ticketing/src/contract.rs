//! Service contracts (wl-ticketing §3, §4.6, §5.6, issue #149): an agreement
//! with a vendor to maintain covered subjects.
//!
//! This is the part of the contract lifecycle the negative flows need: a
//! contract is created as a `draft` with the subjects (and categories) it
//! covers, `activate`d, and ends by `expire` or `terminate`; what happens
//! then is [`crate::contract_end`] (`WRK.RUL.18`). Creating and editing
//! contracts on a page, `renew` and `discard`, the contract document and the
//! visit schedule come with the contract issue of the roadmap; so does the
//! creation of planned visits by the plans (`WRK.PPM.01`, a visit is opened
//! with [`OpenTicket::under_contract`](wl_kernel::tickets::OpenTicket::under_contract)).
//!
//! A contract **covers** a subject for a category while it is `active` or
//! `expiring`, and, once `terminated`, until its termination date: on that
//! date `WRK.RUL.18` runs. [`ServiceContracts::coverage`] answers which
//! contract covers a subject and category on a date; a ticket opened on a
//! covered subject records the contract and takes its SLA policy, and
//! `assign` proposes the contract's vendor ([`ServiceContracts::proposed_vendor`]).

use renox::auth::permissions::Scope;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::parties::{PartyStatus, directory};
use wl_kernel::tickets::{
    SERVICE_CONTRACT_SERIES, SERVICE_CONTRACT_SUBJECT_TYPE, ServiceContractActivated,
    ServiceContractId,
};
use wl_kernel::{PartyId, Subject};

use crate::config::Category;
use crate::contract_machine::{ACTIVATE, EXPIRE, FLAG_EXPIRING, TERMINATE, TERMINATE_BY_DECISION};
use crate::flags::{self, REPLACEMENT_ACTIVE};
use crate::permissions;
use crate::plans::PlanSeed;
use crate::ticket::{Ticket, TicketStatus};

/// Where a service contract is in its life (wl-ticketing §4.6). Stored as
/// `draft`, `active`, `expiring`, `renewed`, `expired` or `terminated`.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContractStatus {
    /// Being prepared; coverage and schedule editable.
    #[default]
    Draft,
    /// In force: generates planned visits and sets the SLA for covered
    /// tickets.
    Active,
    /// Inside the renewal notice window; still in force.
    Expiring,
    /// Replaced by a new contract version, which is `active`.
    Renewed,
    /// End date passed without renewal.
    Expired,
    /// Ended early.
    Terminated,
}

/// A service contract. Dates are calendar days (Asia/Dubai). Its status
/// moves only through the machine in [`crate::contract_machine`].
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_service_contracts",
    state_machine,
    unique(number),
    index(vendor_party_id),
    index(status)
)]
pub struct ServiceContract {
    /// The primary key.
    pub id: i64,
    /// The number from the `service_contract` series, e.g. `AMC-2026-0007`.
    pub number: String,
    /// The vendor party.
    pub vendor_party_id: i64,
    /// The title.
    pub title: String,
    /// The first day it holds.
    pub start_date: NaiveDate,
    /// The last day it holds.
    pub end_date: NaiveDate,
    /// How many days before the end date it becomes `expiring`.
    pub renewal_notice_days: i64,
    /// The SLA policy that replaces the category's for the tickets it covers.
    #[model(references = "wl_ticketing_sla_policies")]
    pub sla_policy_id: Option<i64>,
    /// Whether a covered category is compliance-critical (set at `activate`).
    pub compliance_contract: bool,
    /// The contract this one renews.
    pub previous_version_id: Option<i64>,
    /// The reason entered with `terminate`.
    pub termination_reason: Option<String>,
    /// The termination date entered with `terminate`.
    pub termination_date: Option<NaiveDate>,
    /// When `WRK.RUL.18` handled the end: once per contract end.
    pub ended_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: ContractStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ServiceContract {
    /// The id as the kernel's [`ServiceContractId`].
    pub fn contract_id(&self) -> ServiceContractId {
        ServiceContractId(self.id)
    }

    /// The contract as a [`Subject`].
    pub fn subject(&self) -> Subject {
        self.contract_id().subject()
    }

    /// The vendor party.
    pub fn vendor(&self) -> PartyId {
        PartyId(self.vendor_party_id)
    }

    /// Whether it covers its subjects on `date`: while `active` or
    /// `expiring`, and once `terminated` until its termination date (the
    /// day `WRK.RUL.18` runs).
    pub fn covers_on(&self, date: NaiveDate) -> bool {
        match self.status {
            ContractStatus::Active | ContractStatus::Expiring => true,
            ContractStatus::Terminated => {
                self.ended_at.is_none() && self.termination_date.is_some_and(|end| date < end)
            }
            _ => false,
        }
    }

    /// The date it ends on for `WRK.RUL.18`: the termination date of a
    /// terminated contract, else the end date.
    pub fn ending_on(&self) -> NaiveDate {
        match (self.status, self.termination_date) {
            (ContractStatus::Terminated, Some(date)) => date,
            _ => self.end_date,
        }
    }

    /// Whether it is over and `WRK.RUL.18` has not handled it yet: `expired`,
    /// or `terminated` with a termination date reached on `today`.
    pub fn end_is_due(&self, today: NaiveDate) -> bool {
        self.ended_at.is_none()
            && match self.status {
                ContractStatus::Expired => true,
                ContractStatus::Terminated => {
                    self.termination_date.is_some_and(|date| date <= today)
                }
                _ => false,
            }
    }

    /// The contracts of `vendor` that are `active` or `expiring`, oldest
    /// first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn in_force_of(db: &Db, vendor: PartyId) -> Result<Vec<ServiceContract>> {
        ServiceContract::query()
            .where_(ServiceContract::VENDOR_PARTY_ID.eq(vendor.0))
            .where_(
                ServiceContract::STATUS.is_in([ContractStatus::Active, ContractStatus::Expiring]),
            )
            .order_by(ServiceContract::ID)
            .get(db)
            .await
    }

    /// The planned visits handled under the contract (tickets with a planned
    /// due date), oldest planned date first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn visits(&self, db: &Db) -> Result<Vec<Ticket>> {
        let mut visits = Ticket::query()
            .where_(Ticket::CONTRACT_ID.eq(self.id))
            .where_(Ticket::PLANNED_DUE_DATE.is_not_null())
            .order_by(Ticket::PLANNED_DUE_DATE)
            .order_by(Ticket::ID)
            .get(db)
            .await?;
        visits.sort_by_key(|v| (v.planned_due_date, v.id));
        Ok(visits)
    }

    /// The covered subjects with their categories, in the order added.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn coverage(&self, db: &Db) -> Result<Vec<Coverage>> {
        Coverage::query()
            .where_(Coverage::CONTRACT_ID.eq(self.id))
            .order_by(Coverage::ID)
            .get(db)
            .await
    }
}

/// Whether a visit is open for `WRK.RUL.18`: `open`, `assigned` or
/// `on_hold` (not started). One `in_progress` is left to finish.
pub fn is_open_visit(visit: &Ticket) -> bool {
    visit.planned_due_date.is_some()
        && matches!(
            visit.status,
            TicketStatus::Open | TicketStatus::Assigned | TicketStatus::OnHold
        )
}

/// One subject covered for one category by one contract.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_ticketing_contract_coverage",
    unique(contract_id, subject_type, subject_id, category_id),
    index(subject_type, subject_id)
)]
pub struct Coverage {
    /// The primary key.
    pub id: i64,
    /// The contract.
    #[model(references = "wl_ticketing_service_contracts")]
    pub contract_id: i64,
    /// The type of the covered subject, e.g. `property.building_system`.
    pub subject_type: String,
    /// The id of the covered subject.
    pub subject_id: i64,
    /// The category covered.
    #[model(references = "wl_ticketing_categories")]
    pub category_id: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Coverage {
    /// The covered subject.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }
}

/// What a new draft contract is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContract {
    /// The vendor party.
    pub vendor: PartyId,
    /// The title.
    pub title: String,
    /// The first day it holds.
    pub start_date: NaiveDate,
    /// The last day it holds.
    pub end_date: NaiveDate,
    /// Days before the end date it becomes `expiring` (60 is usual).
    pub renewal_notice_days: i64,
    /// The SLA policy of the tickets it covers, if it has its own.
    pub sla_policy_id: Option<i64>,
}

/// The contract that covers a subject and a category on a date.
#[derive(Debug, Clone)]
pub struct Covering {
    /// The contract.
    pub contract: ServiceContract,
}

impl Covering {
    /// The contract's vendor.
    pub fn vendor(&self) -> PartyId {
        self.contract.vendor()
    }

    /// The contract's SLA policy, when it has one.
    pub fn sla_policy_id(&self) -> Option<i64> {
        self.contract.sla_policy_id
    }
}

/// What a lifecycle run carries to its guards and effects, worked out by the
/// services in this module (an effect gets no `AppState`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ContractInput {
    /// The Asia/Dubai date of the run.
    #[serde(default)]
    pub today: NaiveDate,
    /// The termination date entered with `terminate`.
    #[serde(default)]
    pub termination_date: Option<NaiveDate>,
    /// How many subjects the contract covers (`activate`).
    #[serde(default)]
    pub covered: i64,
    /// Whether a covered category is compliance-critical (`activate`).
    #[serde(default)]
    pub compliance: bool,
    /// The plans to create, one per covered subject and category
    /// (`activate`).
    #[serde(default)]
    pub plans: Vec<PlanSeed>,
}

/// The service for contracts, as the spec's `ServiceContracts` names it.
#[derive(Debug, Clone, Copy, Default)]
pub struct ServiceContracts;

fn unprocessable(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

impl ServiceContracts {
    /// Creates a `draft` contract, numbered from the `service_contract`
    /// series.
    ///
    /// # Errors
    ///
    /// 422 for a blank title, an end date before the start date, or a vendor
    /// the party directory does not know.
    pub async fn create_draft(state: &AppState, new: NewContract) -> Result<ServiceContract> {
        let title = new.title.trim();
        if title.is_empty() {
            return Err(unprocessable("A contract needs a title."));
        }
        if new.end_date < new.start_date {
            return Err(unprocessable("The end date is before the start date."));
        }
        if directory(state).status(&state.db, new.vendor).await?.is_none() {
            return Err(unprocessable("The vendor is not a known party."));
        }
        let number = SERVICE_CONTRACT_SERIES
            .next_allowing_gaps(&state.db, None, None, today())
            .await?;
        let mut contract = ServiceContract {
            number,
            vendor_party_id: new.vendor.0,
            title: title.to_owned(),
            start_date: new.start_date,
            end_date: new.end_date,
            renewal_notice_days: new.renewal_notice_days.max(0),
            sla_policy_id: new.sla_policy_id,
            status: ContractStatus::Draft,
            ..ServiceContract::default()
        };
        contract.insert(&state.db).await?;
        Ok(contract)
    }

    /// Adds `subject` for the category `category_key` to the coverage of the
    /// `draft` `contract`. Adding the same pair again changes nothing.
    ///
    /// # Errors
    ///
    /// 422 unless the contract is a `draft`, or for an unknown or inactive
    /// category.
    pub async fn add_coverage(
        db: &Db,
        contract: &ServiceContract,
        subject: Subject,
        category_key: &str,
    ) -> Result<Coverage> {
        if contract.status != ContractStatus::Draft {
            return Err(unprocessable(
                "The coverage of a contract is edited while it is a draft.",
            ));
        }
        let category = match Category::by_key(db, category_key.trim()).await? {
            Some(category) if category.active => category,
            _ => {
                return Err(unprocessable(format!(
                    "`{category_key}` is not a ticket category."
                )));
            }
        };
        let existing = Coverage::query()
            .where_(Coverage::CONTRACT_ID.eq(contract.id))
            .where_(Coverage::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(Coverage::SUBJECT_ID.eq(subject.subject_id))
            .where_(Coverage::CATEGORY_ID.eq(category.id))
            .first(db)
            .await?;
        if let Some(existing) = existing {
            return Ok(existing);
        }
        Coverage::create(
            db,
            Coverage {
                contract_id: contract.id,
                subject_type: subject.subject_type,
                subject_id: subject.subject_id,
                category_id: category.id,
                ..Coverage::default()
            },
        )
        .await
    }

    /// The contract covering `subject` for the category `category_id` on
    /// `on` (`ServiceContracts::coverage` of wl-ticketing §6): the newest
    /// one when several do.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn coverage(
        db: &Db,
        subject: &Subject,
        category_id: i64,
        on: NaiveDate,
    ) -> Result<Option<Covering>> {
        let rows = Coverage::query()
            .where_(Coverage::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(Coverage::SUBJECT_ID.eq(subject.subject_id))
            .where_(Coverage::CATEGORY_ID.eq(category_id))
            .get(db)
            .await?;
        let mut found: Vec<ServiceContract> = Vec::new();
        for row in rows {
            if let Some(contract) = ServiceContract::find(db, row.contract_id).await?
                && contract.covers_on(on)
            {
                found.push(contract);
            }
        }
        found.sort_by_key(|c| c.id);
        Ok(found.pop().map(|contract| Covering { contract }))
    }

    /// Whether another `active` contract than `except` covers `subject` for
    /// the category `category_id`: a replacement.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn has_replacement(
        db: &Db,
        subject: &Subject,
        category_id: i64,
        except: i64,
    ) -> Result<bool> {
        let rows = Coverage::query()
            .where_(Coverage::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(Coverage::SUBJECT_ID.eq(subject.subject_id))
            .where_(Coverage::CATEGORY_ID.eq(category_id))
            .where_(Coverage::CONTRACT_ID.ne(except))
            .get(db)
            .await?;
        for row in rows {
            if let Some(contract) = ServiceContract::find(db, row.contract_id).await?
                && contract.status == ContractStatus::Active
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The vendor `assign` proposes for `ticket`: the vendor of the contract
    /// it is handled under, while that contract still covers its subject and
    /// the vendor can be given new work, on the date `on`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn proposed_vendor(
        state: &AppState,
        ticket: &Ticket,
        on: NaiveDate,
    ) -> Result<Option<PartyId>> {
        let Some(contract_id) = ticket.contract_id else {
            return Ok(None);
        };
        let Some(contract) = ServiceContract::find(&state.db, contract_id).await? else {
            return Ok(None);
        };
        if !contract.covers_on(on) {
            return Ok(None);
        }
        let status = directory(state).status(&state.db, contract.vendor()).await?;
        Ok((status == Some(PartyStatus::Active)).then(|| contract.vendor()))
    }

    /// Activates the `draft` `contract` as `user`: one maintenance plan per
    /// covered subject and category, the `no_contract` and
    /// `vendor_document_expired` flags of the covered subjects and
    /// categories cleared, `ServiceContractActivated` emitted.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `service_contracts.manage`; `NotFrom` unless it is
    /// a `draft`; `Guard` (422) without a covered subject; `Failed` (422)
    /// while the vendor's document of a guarded category is not valid on the
    /// start date ([`crate::vendor_guard`]).
    pub async fn activate(
        state: &AppState,
        user: &AuthUser,
        contract: &ServiceContract,
    ) -> std::result::Result<(), TransitionError> {
        if !user.has_permission(permissions::SERVICE_CONTRACTS_MANAGE) {
            return Err(TransitionError::Forbidden);
        }
        let coverage = contract.coverage(&state.db).await?;
        let mut compliance = false;
        let mut plans = Vec::new();
        for row in &coverage {
            if let Some(category) = Category::find(&state.db, row.category_id).await? {
                compliance |= category.compliance_critical;
            }
            plans.push(PlanSeed {
                subject_type: row.subject_type.clone(),
                subject_id: row.subject_id,
                category_id: row.category_id,
            });
        }
        let input = ContractInput {
            today: today(),
            covered: i64::try_from(coverage.len()).unwrap_or(i64::MAX),
            compliance,
            plans,
            ..ContractInput::default()
        };
        // `WRK.ACL.11`: the vendor's document of a guarded category must be
        // valid on the start date.
        crate::vendor_guard::run(
            state,
            crate::vendor_guard::ACTIVATE,
            contract.id,
            contract.start_date,
            Some(user.id),
            || async {
                contract
                    .transition(ACTIVATE)
                    .by(user)
                    .input(&input)
                    .run(state)
                    .await
                    .map(|_| ())
            },
        )
        .await?;
        let at = renox::db::now();
        let mut subjects: Vec<Subject> = Vec::new();
        for row in &coverage {
            flags::clear_covered(state, &row.subject(), row.category_id, REPLACEMENT_ACTIVE, at)
                .await?;
            if !subjects.contains(&row.subject()) {
                subjects.push(row.subject());
            }
        }
        state
            .emit(ServiceContractActivated {
                contract: contract.contract_id(),
                vendor: contract.vendor(),
                subjects,
                start: contract.start_date,
                end: contract.end_date,
            })
            .await?;
        Ok(())
    }

    /// The system moves the `active` `contract` into its renewal notice
    /// window (`WRK.RUL.12`) when the end date minus the notice days is
    /// reached on `today`.
    ///
    /// # Errors
    ///
    /// `NotFrom` unless it is `active`; `Guard` (422) before the window.
    pub async fn flag_expiring(
        state: &AppState,
        contract: &ServiceContract,
        today: NaiveDate,
    ) -> std::result::Result<(), TransitionError> {
        let input = ContractInput {
            today,
            ..ContractInput::default()
        };
        let _outcome = contract
            .transition(FLAG_EXPIRING)
            .by_system()
            .input(&input)
            .run(state)
            .await?;
        Ok(())
    }

    /// The system expires the `expiring` `contract` (`WRK.RUL.13`) on the
    /// Asia/Dubai date `today`, when its end date has passed, and then handles
    /// its open visits and compliance (`WRK.RUL.18`, [`crate::contract_end`]).
    ///
    /// # Errors
    ///
    /// `NotFrom` unless it is `expiring`; `Guard` (422) before the end date
    /// has passed.
    pub async fn expire(
        state: &AppState,
        contract: &ServiceContract,
        today: NaiveDate,
    ) -> std::result::Result<(), TransitionError> {
        let input = ContractInput {
            today,
            ..ContractInput::default()
        };
        let _outcome = contract
            .transition(EXPIRE)
            .by_system()
            .input(&input)
            .run(state)
            .await?;
        crate::contract_end::handle(state, contract.id, today).await?;
        Ok(())
    }

    /// `user` terminates the `active` or `expiring` `contract` for `reason`,
    /// ending on `termination_date`. The contract is `terminated` at once but
    /// covers its subjects until that date; `WRK.RUL.18` runs on the date, at
    /// once when it is today.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `service_contracts.manage`; `NotFrom` unless it is
    /// `active` or `expiring`; `ReasonRequired` without a reason; `Guard`
    /// (422) for a termination date before the start date.
    pub async fn terminate(
        state: &AppState,
        user: &AuthUser,
        contract: &ServiceContract,
        reason: &str,
        termination_date: NaiveDate,
    ) -> std::result::Result<(), TransitionError> {
        let input = ContractInput {
            today: today(),
            termination_date: Some(termination_date),
            ..ContractInput::default()
        };
        let _outcome = contract
            .transition(TERMINATE)
            .by(user)
            .reason(reason)
            .input(&input)
            .run(state)
            .await?;
        if termination_date <= input.today {
            crate::contract_end::handle(state, contract.id, input.today).await?;
        }
        Ok(())
    }

    /// The decider of a blocked vendor's review ends the contract today
    /// (`terminate`, run by the system for the user `decided_by`; the history
    /// names who decided).
    ///
    /// # Errors
    ///
    /// As [`terminate`](Self::terminate), without the permission check.
    pub async fn terminate_by_decision(
        state: &AppState,
        contract: &ServiceContract,
        decided_by: i64,
        reason: &str,
    ) -> std::result::Result<(), TransitionError> {
        let input = ContractInput {
            today: today(),
            termination_date: Some(today()),
            ..ContractInput::default()
        };
        let _outcome = contract
            .transition(TERMINATE_BY_DECISION)
            .by_system()
            .reason(reason)
            .input(&input)
            .meta(renox::serde_json::json!({ "decided_by": decided_by }))
            .run(state)
            .await?;
        crate::contract_end::handle(state, contract.id, input.today).await?;
        Ok(())
    }
}

/// Registers how to find the scope of a contract: the scope of its first
/// covered subject, or global when it covers nothing.
pub(crate) fn register_scope() {
    wl_kernel::scopes::register(SERVICE_CONTRACT_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            let first = Coverage::query()
                .where_(Coverage::CONTRACT_ID.eq(id))
                .order_by(Coverage::ID)
                .first(db)
                .await?;
            match first {
                Some(row) => wl_kernel::scopes::scope_of(db, &row.subject()).await,
                None => Ok(Scope::global()),
            }
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    fn contract(status: ContractStatus) -> ServiceContract {
        ServiceContract {
            start_date: day(1),
            end_date: day(30),
            status,
            ..ServiceContract::default()
        }
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        let keys: Vec<_> = ContractStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            ["draft", "active", "expiring", "renewed", "expired", "terminated"]
        );
        assert_eq!(ContractStatus::default(), ContractStatus::Draft);
    }

    #[test]
    fn an_active_or_expiring_contract_covers_and_a_draft_or_expired_one_does_not() {
        for covering in [ContractStatus::Active, ContractStatus::Expiring] {
            assert!(contract(covering).covers_on(day(15)));
        }
        for idle in [
            ContractStatus::Draft,
            ContractStatus::Renewed,
            ContractStatus::Expired,
        ] {
            assert!(!contract(idle).covers_on(day(15)));
        }
    }

    #[test]
    fn a_terminated_contract_covers_until_its_termination_date() {
        let mut terminated = contract(ContractStatus::Terminated);
        terminated.termination_date = Some(day(30));
        assert!(terminated.covers_on(day(20)));
        assert!(terminated.covers_on(day(29)));
        assert!(!terminated.covers_on(day(30)), "WRK.RUL.18 runs that day");
        terminated.termination_date = None;
        assert!(!terminated.covers_on(day(20)));
    }

    #[test]
    fn the_end_is_due_once_for_an_expired_or_a_terminated_contract_whose_date_is_reached() {
        let mut expired = contract(ContractStatus::Expired);
        assert!(expired.end_is_due(day(1)));
        expired.ended_at = Some(renox::db::now());
        assert!(!expired.end_is_due(day(1)), "handled once");

        let mut terminated = contract(ContractStatus::Terminated);
        terminated.termination_date = Some(day(30));
        assert!(!terminated.end_is_due(day(29)));
        assert!(terminated.end_is_due(day(30)));
        assert_eq!(terminated.ending_on(), day(30));
        assert_eq!(contract(ContractStatus::Expired).ending_on(), day(30));
        assert!(!contract(ContractStatus::Active).end_is_due(day(30)));
    }

    #[test]
    fn a_planned_visit_that_is_not_started_is_open() {
        let mut visit = Ticket {
            planned_due_date: Some(day(5)),
            status: TicketStatus::Assigned,
            ..Ticket::default()
        };
        assert!(is_open_visit(&visit));
        visit.status = TicketStatus::InProgress;
        assert!(!is_open_visit(&visit));
        visit.status = TicketStatus::OnHold;
        assert!(is_open_visit(&visit));
        visit.planned_due_date = None;
        assert!(!is_open_visit(&visit), "a reactive ticket is not a visit");
    }

    #[test]
    fn a_contract_is_a_subject() {
        let contract = ServiceContract {
            id: 7,
            vendor_party_id: 5,
            ..ServiceContract::default()
        };
        assert_eq!(contract.subject().to_string(), "ticketing.service_contract:7");
        assert_eq!(contract.vendor(), PartyId(5));
    }
}
