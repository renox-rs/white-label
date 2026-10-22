//! The client contract services: one function per action, each gathering what
//! the machine's guards need and running the transition (09 Manpower §7.1
//! *Client contract*; Base 09).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::parties::directory;
use wl_kernel::rate_cards::rate_cards;

use crate::contract::{
    ACTIVATE, BillingCycle, CANCEL, ClientContract, ContractInput, ContractStatus,
};
use crate::db::{answer, refused, set_contract};
use crate::events::ContractActivated;
use crate::permissions;
use crate::texts::english;

/// What a new contract is made of.
#[derive(Debug, Clone)]
pub struct NewContract {
    /// The reference the agency gives it.
    pub reference: String,
    /// The client company.
    pub client: PartyId,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day.
    pub end_date: NaiveDate,
    /// How often the client is invoiced.
    pub billing_cycle: BillingCycle,
    /// The days the client has to pay.
    pub payment_terms_days: i64,
    /// Whether the agency must cover leave, suspension and rejection.
    pub cover_required: bool,
    /// The deemed-approval days of a timesheet; none by default.
    pub deemed_approval_days: Option<i64>,
    /// The client contact the timesheet escalation goes to.
    pub escalation_contact: Option<PartyId>,
    /// The calendar days of notice an early termination needs.
    pub notice_period_days: i64,
    /// The account manager; the creator when empty.
    pub account_manager_user_id: Option<i64>,
}

impl NewContract {
    /// A monthly contract of `client` from `start_date` to `end_date`, payable
    /// in 30 days with 30 days of notice, no cover obligation and no
    /// deemed approval.
    pub fn new(
        reference: &str,
        client: PartyId,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Self {
        NewContract {
            reference: reference.to_owned(),
            client,
            start_date,
            end_date,
            billing_cycle: BillingCycle::Monthly,
            payment_terms_days: 30,
            cover_required: false,
            deemed_approval_days: None,
            escalation_contact: None,
            notice_period_days: 30,
            account_manager_user_id: None,
        }
    }
}

/// The contract `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<ClientContract> {
    ClientContract::find_or_404(&state.db, id).await
}

/// `Forbidden` unless `by` manages contracts of the client company.
fn require_manage(by: &AuthUser, contract: &ClientContract) -> Result {
    if by.has_permission_in(permissions::CONTRACTS_MANAGE, &contract.scope()) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Creates a `draft` contract.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 for a blank
/// reference, an end date that is not after the start date, a payment term,
/// notice period or deemed-approval days that is not positive, or a client
/// that is not a known party.
pub async fn create(state: &AppState, by: &AuthUser, new: NewContract) -> Result<ClientContract> {
    if !by.has_permission_in(
        permissions::CONTRACTS_MANAGE,
        &permissions::client_scope(new.client.0),
    ) {
        return Err(Error::Forbidden);
    }
    if new.reference.trim().is_empty() {
        return Err(refused(english("manpower.problems.reference", &[])));
    }
    if new.end_date <= new.start_date {
        return Err(refused(english("manpower.problems.dates", &[])));
    }
    if new.payment_terms_days < 1
        || new.notice_period_days < 1
        || new.deemed_approval_days.is_some_and(|days| days < 1)
    {
        return Err(refused(english("manpower.problems.payment_terms", &[])));
    }
    if directory(state)
        .find(&state.db, new.client)
        .await?
        .is_none()
    {
        return Err(refused(english("manpower.problems.client_unknown", &[])));
    }
    ClientContract::create(
        &state.db,
        ClientContract {
            reference: new.reference.trim().to_owned(),
            client_party_id: new.client.0,
            start_date: new.start_date,
            end_date: new.end_date,
            billing_cycle: new.billing_cycle,
            payment_terms_days: new.payment_terms_days,
            cover_required: new.cover_required,
            deemed_approval_days: new.deemed_approval_days,
            escalation_contact_party_id: new.escalation_contact.map(|party| party.0),
            notice_period_days: new.notice_period_days,
            account_manager_user_id: new.account_manager_user_id.or(Some(by.id)),
            created_by: Some(by.id),
            ..ClientContract::default()
        },
    )
    .await
}

/// `by` records that the client signed the contract on `on` (`DOC.SIG.01`),
/// with the signed `document`.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 unless the
/// contract is a `draft`, for a day that has not come or a blank document.
pub async fn record_signed(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    on: NaiveDate,
    document: &str,
) -> Result<ClientContract> {
    let contract = load(state, id).await?;
    require_manage(by, &contract)?;
    if contract.status != ContractStatus::Draft {
        return Err(refused(english("manpower.problems.not_draft", &[])));
    }
    if on > today() {
        return Err(refused(english("manpower.problems.signed_in_future", &[])));
    }
    if document.trim().is_empty() {
        return Err(refused(english("manpower.problems.signed_document", &[])));
    }
    let mut tx = state.db.begin().await?;
    set_contract(
        &mut tx,
        id,
        &[
            ("signed_on", &Some(on)),
            ("signed_document", &Some(document.trim().to_owned())),
        ],
    )
    .await?;
    tx.commit().await?;
    load(state, id).await
}

/// The checks that fail for `contract` on `on`, one English line each: the
/// contract signed, a rate card version accepted by the client and in force
/// on the start date, the end date not past.
async fn readiness(
    state: &AppState,
    contract: &ClientContract,
    on: NaiveDate,
) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    if contract.signed_on.is_none() {
        problems.push(english("manpower.contract.not_signed", &[]));
    }
    let accepted = rate_cards(state)
        .accepted_version(state, &contract.subject(), contract.start_date)
        .await?;
    if accepted.is_none() {
        problems.push(english("manpower.contract.no_rate_card", &[]));
    }
    if contract.end_date < on {
        problems.push(english("manpower.contract.over", &[]));
    }
    Ok(problems)
}

/// `by` activates contract `id` today (Asia/Dubai): every check of
/// [`readiness`] must pass. After the commit [`ContractActivated`] is emitted.
///
/// # Errors
///
/// A 422 listing every failing check; `Forbidden` without
/// `contracts.manage` for the client.
pub async fn activate(state: &AppState, by: &AuthUser, id: i64) -> Result<ClientContract> {
    let contract = load(state, id).await?;
    let on = today();
    let input = ContractInput {
        problems: readiness(state, &contract, on).await?,
        on: Some(on),
    };
    let _outcome = contract
        .transition(ACTIVATE)
        .by(by)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    let active = load(state, id).await?;
    state
        .emit(ContractActivated {
            contract: active.subject(),
            client: active.client(),
            end_date: active.end_date,
        })
        .await?;
    Ok(active)
}

/// `by` cancels draft contract `id` with `reason`.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 unless the
/// contract is a `draft` or without a reason.
pub async fn cancel(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<ClientContract> {
    let contract = load(state, id).await?;
    let _outcome = contract
        .transition(CANCEL)
        .by(by)
        .reason(reason)
        .run(state)
        .await
        .map_err(answer)?;
    load(state, id).await
}

/// `by` puts contract `id` on credit hold with `reason` (`overdue`,
/// `bounced_cheque` or `other`) and, for `other`, the `text`; a contract
/// already on hold gets the reason added. The account manager and the MD are
/// told once.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 without a
/// reason, for `other` without its text, or unless the contract is `active`,
/// `terminating` or `on_hold`.
pub async fn hold(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    text: Option<&str>,
) -> Result<ClientContract> {
    crate::hold::hold_by_hand(state, by, id, reason, text).await
}

/// `by` asks to release the credit hold of contract `id`: at once when no
/// hold condition remains, else the MD is asked and the contract stays
/// `on_hold` until approved.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 unless the
/// contract is `on_hold`.
pub async fn release(state: &AppState, by: &AuthUser, id: i64) -> Result<crate::release::Released> {
    crate::release::release(state, by, id).await
}

/// `by` requests the early termination of contract `id`; the MD approves it.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 listing every
/// failing check, or unless the contract is `active` or `on_hold`.
pub async fn terminate(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: crate::termination::TerminateAsk,
) -> Result<crate::termination::Requested> {
    crate::termination::request(state, by, id, ask).await
}

/// `by` renews contract `id` with a later end date, the signed renewal and,
/// for changed rates, a new rate card version.
///
/// # Errors
///
/// `Forbidden` without `contracts.manage` for the client; a 422 listing every
/// failing check, for a blocked client, or unless the contract is `active` or
/// `on_hold`.
pub async fn renew(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: crate::renewal::RenewAsk,
) -> Result<ClientContract> {
    crate::renewal::renew(state, by, id, ask).await
}
