//! The manpower request services: one function per action, each gathering
//! what the machine's guards need and running the transition (09 Manpower
//! §7.1 *Manpower request*; Base 09).
//!
//! Who may write a request: staff with `requests.create` (`requests.manage`
//! to cancel) for the client company, and the client approver of that company
//! through the portal (`POR.SLF.01`, `MPW.ACL.01`): the user acts for the
//! company in the kernel's [`PortalUsers`](wl_kernel::portal::PortalUsers)
//! with the portal role `owner` or `approver`. A client viewer only sees.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::block_guard::ActionKind;
use wl_kernel::documents::today;
use wl_kernel::rate_cards::rate_cards;

use crate::access::may_write;
use crate::blocked;
use crate::contract::{ClientContract, ContractStatus};
use crate::db::{answer, lock_contract, refused};
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments;
use crate::hold;
use crate::notices;
use crate::permissions;
use crate::request::{CANCEL, ManpowerRequest, RequestInput, SUBMIT};
use crate::site::Site;
use crate::texts::english;

/// What a new request is made of.
#[derive(Debug, Clone)]
pub struct NewRequest {
    /// The client contract it is under.
    pub contract_id: i64,
    /// The site the workers go to.
    pub site_id: i64,
    /// The role asked for.
    pub role: String,
    /// How many workers.
    pub headcount: i64,
    /// The shift type the rate card prices.
    pub shift_type: String,
    /// The shift start as `HH:MM`.
    pub shift_start: Option<String>,
    /// The shift end as `HH:MM`.
    pub shift_end: Option<String>,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day; none for an open-ended order.
    pub end_date: Option<NaiveDate>,
}

/// The request `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<ManpowerRequest> {
    ManpowerRequest::find_or_404(&state.db, id).await
}

/// Creates a `draft` request.
///
/// # Errors
///
/// `Forbidden` unless `by` is staff with `requests.create` for the client or
/// the client's approver; a 422 for a contract that is over or unknown, a site
/// that is not the client's or not in use, a blank role or shift, a headcount
/// below 1 or an end date before the start.
pub async fn create(state: &AppState, by: &AuthUser, new: NewRequest) -> Result<ManpowerRequest> {
    let contract = ClientContract::find(&state.db, new.contract_id)
        .await?
        .ok_or_else(|| refused(english("manpower.request.contract_unknown", &[])))?;
    if !may_write(
        state,
        by,
        contract.client(),
        permissions::REQUESTS_CREATE,
        true,
    )
    .await?
    {
        return Err(Error::Forbidden);
    }
    if matches!(
        contract.status,
        ContractStatus::Ended | ContractStatus::Cancelled
    ) {
        return Err(refused(english("manpower.request.contract_over", &[])));
    }
    let site = Site::find(&state.db, new.site_id).await?;
    if !site
        .as_ref()
        .is_some_and(|site| site.client_party_id == contract.client_party_id && site.active)
    {
        return Err(refused(english("manpower.request.site", &[])));
    }
    if new.role.trim().is_empty() || new.shift_type.trim().is_empty() || new.headcount < 1 {
        return Err(refused(english("manpower.request.fields", &[])));
    }
    if new.end_date.is_some_and(|end| end < new.start_date) {
        return Err(refused(english("manpower.request.dates", &[])));
    }
    ManpowerRequest::create(
        &state.db,
        ManpowerRequest {
            client_contract_id: contract.id,
            client_party_id: contract.client_party_id,
            site_id: new.site_id,
            role: new.role.trim().to_lowercase(),
            headcount: new.headcount,
            shift_type: new.shift_type.trim().to_lowercase(),
            shift_start: new.shift_start.filter(|time| !time.trim().is_empty()),
            shift_end: new.shift_end.filter(|time| !time.trim().is_empty()),
            start_date: new.start_date,
            end_date: new.end_date,
            created_by: Some(by.id),
            ..ManpowerRequest::default()
        },
    )
    .await
}

/// The checks that fail for submitting `request` under `contract`, one
/// English line each: the dates within the contract, and a rate card that
/// covers the role and the shift. The state of the contract is read in the
/// transaction of the run ([`status_problem`]).
async fn readiness(
    state: &AppState,
    request: &ManpowerRequest,
    contract: &ClientContract,
) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    if request.start_date > contract.end_date
        || request.end_date.is_some_and(|end| end > contract.end_date)
    {
        problems.push(english("manpower.request.outside_contract", &[]));
    }
    let on = request.start_date.max(today());
    let covered = rate_cards(state)
        .covers(
            state,
            &contract.subject(),
            &request.role,
            &request.shift_type,
            on,
        )
        .await?;
    if !covered {
        problems.push(english(
            "manpower.request.not_on_rate_card",
            &[("role", &request.role), ("shift", &request.shift_type)],
        ));
    }
    Ok(problems)
}

/// Why `contract`, as the transaction `tx` reads it, takes no new request, if
/// it does not: it must be `active` (`MPW.ACL.14`: not `on_hold`, nor
/// `terminating`). `staff` is whether the person holds `requests.create` for
/// the client: the refusal for a contract on hold names the hold and its
/// reason for staff, and is neutral for the client's own user.
async fn status_problem(
    tx: &mut Transaction,
    contract: &ClientContract,
    staff: bool,
) -> Result<Option<String>> {
    if contract.status == ContractStatus::Active {
        return Ok(None);
    }
    Ok(Some(if !staff {
        english("manpower.request.portal_refused", &[])
    } else if contract.status == ContractStatus::OnHold {
        hold::refusal_in(tx, contract).await?
    } else {
        english("manpower.request.contract_not_active", &[])
    }))
}

/// `by` submits draft request `id`: it becomes `open` and the account manager
/// is told.
///
/// # Errors
///
/// `Forbidden` unless `by` may write requests of the client; a 422 listing
/// every failing check, or unless the request is a `draft`.
pub async fn submit(state: &AppState, by: &AuthUser, id: i64) -> Result<ManpowerRequest> {
    let request = load(state, id).await?;
    if !may_write(
        state,
        by,
        request.client(),
        permissions::REQUESTS_CREATE,
        true,
    )
    .await?
    {
        return Err(Error::Forbidden);
    }
    // The dates and the rate card are read before the transaction opens (the
    // rate cards are another module's); the state of the contract is read in
    // the transaction of the transition, so a `hold` committed first wins
    // (`MPW.ACL.14`).
    let first = ClientContract::find_or_404(&state.db, request.client_contract_id).await?;
    let mut problems = readiness(state, &request, &first).await?;
    let mut tx = state.db.begin().await?;
    // A blocked client orders nothing new (BL-MPW-15).
    blocked::check(
        &mut tx,
        state,
        Some(by),
        request.client(),
        ActionKind::Start,
    )
    .await?;
    let contract = lock_contract(&mut tx, request.client_contract_id).await?;
    let staff = by.has_permission_in(permissions::REQUESTS_CREATE, &request.scope());
    if let Some(problem) = status_problem(&mut tx, &contract, staff).await? {
        problems.insert(0, problem);
    }
    let input = RequestInput {
        problems,
        by: Some(by.id),
    };
    let outcome = request
        .transition(SUBMIT)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(answer)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let open = load(state, id).await?;
    let managers =
        notices::account_managers(state, open.client(), contract.account_manager_user_id).await?;
    notices::tell(
        state,
        open.subject(),
        &contract.reference,
        &managers,
        &english("manpower.request.submitted_title", &[("role", &open.role)]),
        &english(
            "manpower.request.submitted_body",
            &[
                ("count", &open.headcount),
                ("role", &open.role),
                ("from", &open.start_date),
            ],
        ),
    )
    .await?;
    Ok(open)
}

/// `by` cancels request `id` with `reason`. The planned deployments of the
/// request are cancelled with it.
///
/// # Errors
///
/// `Forbidden` unless `by` holds `requests.manage` for the client or is the
/// client's approver; a 422 without a reason or unless the request is a
/// `draft`, `open` or `partially_filled`, and a 422 *N deployments are still
/// active* while a deployment of the request is `active` (BL-MPW-23): the
/// worker on site is ended first; the planned ones are cancelled with it.
pub async fn cancel(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
) -> Result<ManpowerRequest> {
    let request = load(state, id).await?;
    if !may_write(
        state,
        by,
        request.client(),
        permissions::REQUESTS_MANAGE,
        true,
    )
    .await?
    {
        return Err(Error::Forbidden);
    }
    let active = Deployment::query()
        .where_(Deployment::REQUEST_ID.eq(id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Active))
        .get(&state.db)
        .await?
        .len();
    if active > 0 {
        let key = if active == 1 {
            "manpower.screen.requests.active_one"
        } else {
            "manpower.screen.requests.active_many"
        };
        return Err(refused(english(key, &[("count", &active)])));
    }
    let reason_for_deployments = english("manpower.request.cancelled_deployments", &[]);
    let mut tx = state.db.begin().await?;
    let first = request
        .transition(CANCEL)
        .by(by)
        .reason(reason)
        .input(&RequestInput {
            problems: Vec::new(),
            by: Some(by.id),
        })
        .run_in(&mut tx)
        .await
        .map_err(answer)?;
    let (outcomes, planned) =
        deployments::cancel_planned_of_request(&mut tx, id, &reason_for_deployments).await?;
    tx.commit().await?;
    first.emit(state).await?;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    deployments::announce_all_cancelled(state, &planned, &reason_for_deployments).await?;
    load(state, id).await
}
