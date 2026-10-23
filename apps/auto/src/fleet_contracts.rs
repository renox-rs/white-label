//! The fleet contract services (08 §4.5, §7.1 *Fleet contract*,
//! `AUT.FLT.01`): agreeing a contract with a fleet, putting vehicles under it,
//! activating, suspending, renewing and ending it.
//!
//! Every status change goes through the machine
//! ([`crate::fleet_contract_machine`]); the services check the permission,
//! work out what a guard needs from the database and pass it in the run's
//! input, and do after the commit what a transition's effect may not (the
//! service schedules of an activated contract, [`crate::fleet_services`]).
//!
//! Decisions: a vehicle is under one contract at a time (its
//! `fleet_contract_id`); the contract's terms are edited while it is a draft,
//! and only the contact, inclusions, price tier, notice period, fee and
//! renewal window after, because a rate change is a renewal; a vehicle with an
//! open job card under the contract is not taken off it.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::PartyId;
use wl_kernel::approvals::submit::REQUEST_KEY;
use wl_kernel::approvals::{ApprovalPolicy, announce};
use wl_kernel::documents::today;
use wl_kernel::parties::directory;

use crate::db::refused;
use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::fleet_contract_machine::{
    ACTIVATE, END, FLAG_RENEWAL, FleetInput, RENEW, RESUME, SUSPEND, TERMINATE, TERMINATE_ACTION,
    TERMINATE_SYSTEM, schedule_request,
};
use crate::fleet_job::FleetJob;
use crate::fleet_service::FleetService;
use crate::fleet_services;
use crate::job_card::JobCard;
use crate::permissions;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The name of the history row of a contract that came to be.
pub const CREATE: &str = "create";

/// A contract to agree or to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewContract {
    /// The fleet (a party).
    pub fleet_party_id: i64,
    /// The fleet contact (a party).
    pub contact_party_id: Option<i64>,
    /// What the contract is called.
    pub title: String,
    /// The first day of the term.
    pub start_date: NaiveDate,
    /// The last day of the term.
    pub end_date: NaiveDate,
    /// The hourly labour rate in fils.
    pub labour_rate: i64,
    /// The part price tier.
    pub price_tier: Option<String>,
    /// What is included.
    pub inclusions: Option<String>,
    /// The notice period in days.
    pub notice_period_days: i64,
    /// The early termination fee in fils.
    pub early_termination_fee: i64,
    /// Days before the end at which the renewal is due.
    pub renewal_window_days: i64,
}

impl NewContract {
    /// A contract with fleet `fleet_party_id` called `title` for the term.
    pub fn new(
        fleet_party_id: i64,
        title: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Self {
        NewContract {
            fleet_party_id,
            contact_party_id: None,
            title: title.to_owned(),
            start_date,
            end_date,
            labour_rate: 0,
            price_tier: None,
            inclusions: None,
            notice_period_days: 30,
            early_termination_fee: 0,
            renewal_window_days: 60,
        }
    }

    /// With this fleet contact.
    #[must_use]
    pub fn contact(mut self, party: i64) -> Self {
        self.contact_party_id = Some(party);
        self
    }

    /// At this hourly labour rate in fils.
    #[must_use]
    pub fn labour_rate(mut self, fils: i64) -> Self {
        self.labour_rate = fils;
        self
    }

    /// On this part price tier.
    #[must_use]
    pub fn tier(mut self, tier: &str) -> Self {
        self.price_tier = Some(tier.to_owned());
        self
    }

    /// Including this.
    #[must_use]
    pub fn inclusions(mut self, text: &str) -> Self {
        self.inclusions = Some(text.to_owned());
        self
    }

    /// With this notice period in days.
    #[must_use]
    pub fn notice_days(mut self, days: i64) -> Self {
        self.notice_period_days = days;
        self
    }

    /// With this early termination fee in fils.
    #[must_use]
    pub fn fee(mut self, fils: i64) -> Self {
        self.early_termination_fee = fils;
        self
    }

    /// With this renewal window in days.
    #[must_use]
    pub fn renewal_window(mut self, days: i64) -> Self {
        self.renewal_window_days = days;
        self
    }
}

/// The text of a trimmed field, `None` when blank.
fn optional(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// Checks the fields of `new`.
fn clean(new: &NewContract) -> Result<String> {
    let title = new.title.trim().to_owned();
    if title.is_empty() {
        return Err(refused(english("auto.fleet.problem.title", &[])));
    }
    if new.end_date <= new.start_date {
        return Err(refused(english("auto.fleet.problem.term", &[])));
    }
    if new.labour_rate < 0
        || new.early_termination_fee < 0
        || new.notice_period_days < 0
        || new.renewal_window_days < 0
    {
        return Err(refused(english("auto.fleet.problem.negative", &[])));
    }
    Ok(title)
}

/// Refuses a party that is not in the directory.
async fn ensure_party(state: &AppState, party: i64) -> Result {
    let found = directory(state)
        .display_name(&state.db, PartyId(party), "en")
        .await?;
    if found.is_none() {
        return Err(refused(english("auto.fleet.problem.party", &[])));
    }
    Ok(())
}

/// Agrees a contract (`AUT.FLT.01`); it starts as a draft.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 for a fleet or contact that
/// is not a party, a blank title, an end date not after the start, or a
/// negative amount.
pub async fn create(state: &AppState, by: &AuthUser, new: NewContract) -> Result<FleetContract> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let title = clean(&new)?;
    ensure_party(state, new.fleet_party_id).await?;
    if let Some(contact) = new.contact_party_id {
        ensure_party(state, contact).await?;
    }
    let mut contract = FleetContract {
        fleet_party_id: new.fleet_party_id,
        contact_party_id: new.contact_party_id,
        title,
        start_date: new.start_date,
        end_date: new.end_date,
        labour_rate: new.labour_rate,
        price_tier: optional(&new.price_tier),
        inclusions: optional(&new.inclusions),
        notice_period_days: new.notice_period_days,
        early_termination_fee: new.early_termination_fee,
        renewal_window_days: new.renewal_window_days,
        created_by: Some(by.id),
        ..FleetContract::default()
    };
    let mut tx = state.db.begin().await?;
    contract
        .insert_in_state(&mut tx, FleetStatus::Draft, CREATE)
        .await?;
    tx.commit().await?;
    FleetContract::find_or_404(&state.db, contract.id).await
}

/// Changes the terms of contract `id`. The fleet, the term and the labour rate
/// change only while it is a draft (after, a rate change is a renewal).
///
/// # Errors
///
/// As for [`create`]; a 422 for a contract that ended, or a change of what a
/// contract in force does not change.
pub async fn update(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    new: NewContract,
) -> Result<FleetContract> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut contract = FleetContract::find_or_404(&state.db, id).await?;
    if contract.status == FleetStatus::Ended {
        return Err(refused(english("auto.fleet.problem.ended", &[])));
    }
    let title = clean(&new)?;
    if let Some(contact) = new.contact_party_id {
        ensure_party(state, contact).await?;
    }
    if contract.status == FleetStatus::Draft {
        ensure_party(state, new.fleet_party_id).await?;
        contract.fleet_party_id = new.fleet_party_id;
        contract.start_date = new.start_date;
        contract.end_date = new.end_date;
        contract.labour_rate = new.labour_rate;
    } else if new.fleet_party_id != contract.fleet_party_id
        || new.start_date != contract.start_date
        || new.end_date != contract.end_date
        || new.labour_rate != contract.labour_rate
    {
        return Err(refused(english("auto.fleet.problem.locked", &[])));
    }
    contract.title = title;
    contract.contact_party_id = new.contact_party_id;
    contract.price_tier = optional(&new.price_tier);
    contract.inclusions = optional(&new.inclusions);
    contract.notice_period_days = new.notice_period_days;
    contract.early_termination_fee = new.early_termination_fee;
    contract.renewal_window_days = new.renewal_window_days;
    contract.save(&state.db).await?;
    FleetContract::find_or_404(&state.db, id).await
}

/// Attaches the signed contract (a document reference).
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 for a blank reference or a
/// contract that ended.
pub async fn attach_document(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    document: &str,
) -> Result<FleetContract> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut contract = FleetContract::find_or_404(&state.db, id).await?;
    if contract.status == FleetStatus::Ended {
        return Err(refused(english("auto.fleet.problem.ended", &[])));
    }
    let document = document.trim();
    if document.is_empty() {
        return Err(refused(english("auto.fleet.problem.document", &[])));
    }
    contract.signed_document = Some(document.to_owned());
    contract.save(&state.db).await?;
    FleetContract::find_or_404(&state.db, id).await
}

/// The vehicles under contract `id`.
///
/// # Errors
///
/// The database's errors.
pub async fn vehicles_of(db: &Db, id: i64) -> Result<Vec<Vehicle>> {
    Vehicle::query()
        .where_(Vehicle::FLEET_CONTRACT_ID.eq(Some(id)))
        .order_by(Vehicle::ID)
        .get(db)
        .await
}

/// Puts vehicle `vehicle_id` under contract `id`.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 for a contract that ended, a
/// vehicle that is not serviced here, or one that is under another contract.
pub async fn add_vehicle(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    vehicle_id: i64,
) -> Result<Vehicle> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let contract = FleetContract::find_or_404(&state.db, id).await?;
    if contract.status == FleetStatus::Ended {
        return Err(refused(english("auto.fleet.problem.ended", &[])));
    }
    let vehicle = Vehicle::find(&state.db, vehicle_id)
        .await?
        .filter(|vehicle| vehicle.active)
        .ok_or_else(|| refused(english("auto.fleet.problem.vehicle", &[])))?;
    // A vehicle of a contract that ended is free to join another.
    if let Some(other) = vehicle.fleet_contract_id.filter(|other| *other != id) {
        let holds = FleetContract::find(&state.db, other)
            .await?
            .is_some_and(|contract| contract.status != FleetStatus::Ended);
        if holds {
            return Err(refused(english("auto.fleet.problem.other_contract", &[])));
        }
    }
    Vehicle::query()
        .where_(Vehicle::ID.eq(vehicle.id))
        .update(
            &state.db,
            &[("fleet_contract_id", &Some(id)), ("updated_at", &now())],
        )
        .await?;
    Vehicle::find_or_404(&state.db, vehicle_id).await
}

/// Takes vehicle `vehicle_id` off contract `id` and stops its service plans.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 for a vehicle that is not
/// under the contract or has a job card open under it.
pub async fn remove_vehicle(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    vehicle_id: i64,
) -> Result<Vehicle> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let vehicle = Vehicle::find_or_404(&state.db, vehicle_id).await?;
    if vehicle.fleet_contract_id != Some(id) {
        return Err(refused(english("auto.fleet.problem.not_on_contract", &[])));
    }
    for job in FleetJob::of_contract(&state.db, id).await? {
        if job.vehicle_id != vehicle_id {
            continue;
        }
        let open = JobCard::find(&state.db, job.job_card_id)
            .await?
            .is_some_and(|card| card.status.is_open());
        if open {
            return Err(refused(english("auto.fleet.problem.job_open", &[])));
        }
    }
    let none: Option<i64> = None;
    Vehicle::query()
        .where_(Vehicle::ID.eq(vehicle_id))
        .update(
            &state.db,
            &[("fleet_contract_id", &none), ("updated_at", &now())],
        )
        .await?;
    FleetService::query()
        .where_(FleetService::CONTRACT_ID.eq(id))
        .where_(FleetService::VEHICLE_ID.eq(vehicle_id))
        .update(&state.db, &[("active", &false), ("updated_at", &now())])
        .await?;
    Vehicle::find_or_404(&state.db, vehicle_id).await
}

/// Runs the transition `name` of the user `by` on `contract`.
async fn run(
    state: &AppState,
    by: &AuthUser,
    contract: &FleetContract,
    name: &str,
    reason: Option<&str>,
    input: FleetInput,
) -> Result<FleetContract> {
    let transition = contract.transition(name).by(by).input(&input);
    let _outcome = match reason {
        Some(reason) => transition.reason(reason).run(state).await,
        None => transition.run(state).await,
    }
    .map_err(TransitionError::into_error)?;
    FleetContract::find_or_404(&state.db, contract.id).await
}

/// Runs the system transition `name` on `contract`.
pub(crate) async fn run_system(
    state: &AppState,
    contract: &FleetContract,
    name: &str,
    reason: Option<&str>,
    input: FleetInput,
) -> Result<FleetContract> {
    let transition = contract.transition(name).by_system().input(&input);
    let _outcome = match reason {
        Some(reason) => transition.reason(reason).run(state).await,
        None => transition.run(state).await,
    }
    .map_err(TransitionError::into_error)?;
    FleetContract::find_or_404(&state.db, contract.id).await
}

/// The input every run starts from.
fn input_for(by: &AuthUser) -> FleetInput {
    FleetInput {
        by: Some(by.id),
        on: Some(today()),
        ..FleetInput::default()
    }
}

/// The contract `id` for a user who manages fleet contracts.
async fn load(state: &AppState, by: &AuthUser, id: i64) -> Result<FleetContract> {
    if !by.has_permission(permissions::FLEET_CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    FleetContract::find_or_404(&state.db, id).await
}

/// How many vehicles are under the contract.
async fn vehicle_count(db: &Db, id: i64) -> Result<i64> {
    Ok(i64::try_from(
        Vehicle::query()
            .where_(Vehicle::FLEET_CONTRACT_ID.eq(Some(id)))
            .count(db)
            .await?,
    )
    .unwrap_or(i64::MAX))
}

/// `activate`: the contract is in force and its service schedules are created
/// (`WRK.PPM.01`).
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 without a vehicle, a labour
/// rate, a service plan or the signed contract, or in another state.
pub async fn activate(state: &AppState, by: &AuthUser, id: i64) -> Result<FleetContract> {
    let contract = load(state, by, id).await?;
    let mut input = input_for(by);
    input.vehicles = Some(vehicle_count(&state.db, id).await?);
    input.schedules = Some(fleet_services::count_active(&state.db, id).await?);
    let active = run(state, by, &contract, ACTIVATE, None, input).await?;
    fleet_services::schedule_all(state, &active).await?;
    Ok(active)
}

/// `suspend`: new bookings are blocked (`AUT.RUL.04` requests none); the
/// bookings already made are kept and listed for the advisor
/// ([`fleet_services::bookings_to_confirm`]); open job cards finish.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 without a reason or in
/// another state.
pub async fn suspend(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<FleetContract> {
    let contract = load(state, by, id).await?;
    run(state, by, &contract, SUSPEND, Some(reason), input_for(by)).await
}

/// `resume`: the contract is in force again.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 in another state.
pub async fn resume(state: &AppState, by: &AuthUser, id: i64) -> Result<FleetContract> {
    let contract = load(state, by, id).await?;
    run(state, by, &contract, RESUME, None, input_for(by)).await
}

/// `renew`: a new last day of the term, and a new labour rate when it changes.
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 for an end date not after the
/// current one, a rate of zero, or another state.
pub async fn renew(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    new_end_date: NaiveDate,
    labour_rate: Option<i64>,
) -> Result<FleetContract> {
    let contract = load(state, by, id).await?;
    let mut input = input_for(by);
    input.new_end_date = Some(new_end_date);
    input.labour_rate = labour_rate;
    run(state, by, &contract, RENEW, None, input).await
}

/// What a request to terminate a contract came to.
#[derive(Debug, Clone)]
pub struct Termination {
    /// The contract: as it was while the owner decides; `ended` when the
    /// request was approved already and its effective date had passed.
    pub contract: FleetContract,
    /// The approval request that waits for the owner.
    pub approval_request: Option<i64>,
}

/// `terminate`: the fleet account manager asks to end the contract early, with
/// a reason, a notice date and an effective date (the last day of service),
/// and whether the fleet asks for the early termination fee to be waived. The
/// kernel defers the run as a request that a holder of
/// `fleet_contracts.approve` other than the requester decides (`PLT.APR.01`,
/// action `auto.fleet_terminate`); nothing happens until then and a rejected
/// request leaves the contract as it is. At the approval an effective date
/// before the day of the request ends the contract at once; any other date
/// records the termination and marks the contract *termination scheduled*, and
/// the daily job ends it on the first run after the effective date
/// ([`crate::fleet_daily`]); the effects follow the end ([`crate::fleet_end`]).
///
/// # Errors
///
/// A 403 without `fleet_contracts.manage`; a 422 without a reason or the two
/// dates, for an effective date before the notice date or inside an invoiced
/// period, a termination already scheduled, a contract that is not in force, or
/// no approval policy in force.
pub async fn terminate(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    notice_date: NaiveDate,
    effective_date: NaiveDate,
    waive_fee: bool,
) -> Result<Termination> {
    let contract = load(state, by, id).await?;
    if !contract.status.is_in_force() {
        return Err(refused(english("auto.fleet.problem.terminate_status", &[])));
    }
    // A date before the day of the request ends the contract at the approval;
    // any other keeps its state until the first run after the effective date.
    let name = if effective_date < today() {
        TERMINATE
    } else {
        schedule_request(contract.status)
            .ok_or_else(|| refused(english("auto.fleet.problem.terminate_status", &[])))?
    };
    let mut input = input_for(by);
    input.notice_date = Some(notice_date);
    input.effective_date = Some(effective_date);
    input.reason = Some(reason.trim().to_owned());
    input.waive_fee = waive_fee;
    // A policy taken away must not let a termination through unapproved.
    let value = renox::serde_json::to_value(&input).unwrap_or(renox::serde_json::Value::Null);
    input.approval_ready = ApprovalPolicy::for_action(&state.db, TERMINATE_ACTION, &value)
        .await?
        .is_some();
    let outcome = contract
        .transition(name)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    announce(state, &outcome).await?;
    let approval_request = match &outcome {
        Outcome::Pending(pending) => pending
            .data
            .get(REQUEST_KEY)
            .and_then(renox::serde_json::Value::as_i64),
        Outcome::Done(_) => None,
    };
    Ok(Termination {
        contract: FleetContract::find_or_404(&state.db, id).await?,
        approval_request,
    })
}

/// The daily run's part for the contracts in force (`end`, `flag_renewal`,
/// `terminate_system`): which transition the contract is due for on `day`,
/// with its input; `None` when it is due for none.
pub fn due_transition(
    contract: &FleetContract,
    day: NaiveDate,
) -> Option<(&'static str, FleetInput)> {
    if !contract.status.is_in_force() {
        return None;
    }
    let base = FleetInput {
        on: Some(day),
        ..FleetInput::default()
    };
    if contract.termination_scheduled
        && contract
            .effective_date
            .is_some_and(|effective| effective < day)
    {
        return Some((
            TERMINATE_SYSTEM,
            FleetInput {
                notice_date: contract.notice_date,
                effective_date: contract.effective_date,
                ..base
            },
        ));
    }
    if contract.end_date < day {
        return Some((END, base));
    }
    if contract.status == FleetStatus::Active {
        let days = (contract.end_date - day).num_days();
        if days <= contract.renewal_window_days {
            return Some((
                FLAG_RENEWAL,
                FleetInput {
                    days_to_end: Some(days),
                    ..base
                },
            ));
        }
    }
    None
}

/// The contracts matching `text` (the number, the title) and `status` when
/// given, newest first.
///
/// # Errors
///
/// The database's errors.
pub async fn search(
    db: &Db,
    text: &str,
    status: Option<FleetStatus>,
) -> Result<Vec<FleetContract>> {
    let mut query = FleetContract::query().order_by_desc(FleetContract::ID);
    if let Some(status) = status {
        query = query.where_(FleetContract::STATUS.eq(status));
    }
    let all = query.get(db).await?;
    let wanted = text.trim().to_lowercase();
    if wanted.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|contract| {
            let haystack = format!("{} {}", contract.number(), contract.title).to_lowercase();
            wanted
                .split_whitespace()
                .all(|word| haystack.contains(word))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap()
    }

    fn contract(status: FleetStatus) -> FleetContract {
        FleetContract {
            id: 3,
            start_date: day(1, 1),
            end_date: day(12, 31),
            renewal_window_days: 60,
            status,
            ..FleetContract::default()
        }
    }

    #[test]
    fn the_title_the_term_and_the_amounts_are_checked() {
        let base = NewContract::new(1, "Fleet 2026", day(1, 1), day(12, 31));
        assert_eq!(clean(&base).unwrap(), "Fleet 2026");
        assert!(clean(&NewContract::new(1, " ", day(1, 1), day(12, 31))).is_err());
        assert!(clean(&NewContract::new(1, "x", day(1, 1), day(1, 1))).is_err());
        assert!(clean(&base.clone().labour_rate(-1)).is_err());
        assert!(clean(&base.fee(-1)).is_err());
    }

    #[test]
    fn a_contract_is_due_for_nothing_before_its_renewal_window() {
        assert_eq!(
            due_transition(&contract(FleetStatus::Active), day(6, 1)),
            None
        );
    }

    #[test]
    fn a_contract_inside_its_renewal_window_is_flagged_once() {
        let (name, input) = due_transition(&contract(FleetStatus::Active), day(11, 15)).unwrap();
        assert_eq!(name, FLAG_RENEWAL);
        assert_eq!(input.days_to_end, Some(46));
        assert_eq!(
            due_transition(&contract(FleetStatus::RenewalDue), day(11, 15)),
            None
        );
        assert_eq!(
            due_transition(&contract(FleetStatus::Suspended), day(11, 15)),
            None
        );
    }

    #[test]
    fn a_contract_ends_on_the_first_day_after_its_end_date() {
        let last = contract(FleetStatus::Active);
        assert_ne!(
            due_transition(&last, day(12, 31)).map(|(name, _)| name),
            Some(END),
            "valid through its end date"
        );
        for status in [
            FleetStatus::Active,
            FleetStatus::Suspended,
            FleetStatus::RenewalDue,
        ] {
            let (name, _) =
                due_transition(&contract(status), day(12, 31).succ_opt().unwrap()).unwrap();
            assert_eq!(name, END, "{status:?}");
        }
        assert_eq!(
            due_transition(
                &contract(FleetStatus::Ended),
                day(12, 31).succ_opt().unwrap()
            ),
            None
        );
        assert_eq!(
            due_transition(
                &contract(FleetStatus::Draft),
                day(12, 31).succ_opt().unwrap()
            ),
            None
        );
    }

    #[test]
    fn a_scheduled_termination_runs_on_the_first_day_after_its_effective_date() {
        let mut terminating = contract(FleetStatus::Active);
        terminating.termination_scheduled = true;
        terminating.effective_date = Some(day(11, 20));
        terminating.notice_date = Some(day(11, 1));
        assert_ne!(
            due_transition(&terminating, day(11, 20)).map(|(name, _)| name),
            Some(TERMINATE_SYSTEM),
            "the effective date is still served"
        );
        let (name, input) = due_transition(&terminating, day(11, 21)).unwrap();
        assert_eq!(name, TERMINATE_SYSTEM);
        assert_eq!(input.effective_date, Some(day(11, 20)));
        assert_eq!(input.notice_date, Some(day(11, 1)));
    }
}
