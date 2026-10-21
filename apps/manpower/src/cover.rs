//! Replacements and cover (09 Manpower §4.1, §4.6, §7.1 *Manpower request*;
//! `MPW.DEP.02`, `MPW.RUL.06`).
//!
//! A replacement is a manpower request of the kind
//! [`RequestKind::Replacement`]: one worker, the role, site and shift of the
//! deployment covered, for the days the worker is away (open-ended for a
//! suspension). It opens at once (it is the agency's own order, so it is not
//! `submit`ted) and goes through the request machine like any other: a
//! deployment planned for it is a [`DeploymentKind::Cover`] of the deployment
//! it replaces, `activate` fills it, `cancel` withdraws it. A cover
//! deployment "runs to its own end date" (§7.1 `resume`): the daily end-date
//! run ends it.
//!
//! - [`open_cover`] is the one function that makes the request. It does
//!   nothing unless the client contract requires cover (`cover_required`) or
//!   an account manager asks by hand, and it never makes a second request for
//!   the same cause (`cause_ref`), so a repeated event changes nothing. The
//!   account manager is told. **BL-MPW-11 (suspension) and BL-MPW-14 (client
//!   rejection, [`crate::ends`]) call it; the notice of cover not found in time, `MPW.RUL.13`,
//!   is BL-MPW-16.**
//! - `MPW.RUL.06`: when a leave request is approved (the kernel's
//!   [`LeaveApproved`]) every `active` deployment of the employee that
//!   overlaps the leave gets a cover request for the days of the overlap.
//!   Decision: only a deployment on site needs cover; a `planned` one is
//!   covered when it starts, by the account manager's own request.
//! - The account manager may open a cover request by hand
//!   ([`open_by_hand`], `requests.manage`), with or without the contract's
//!   cover obligation (§4.1: "the account manager may create a replacement").
//!   A contract on credit hold (`MPW.ACL.14`, BL-MPW-02) takes no cover request
//!   by hand: it is refused with the hold's reason. The cover the contract
//!   requires still opens, since the worker on site is the agency's to
//!   replace; the cover worker's `activate` is refused until the release.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::LeaveApproved;
use wl_kernel::documents::today;

use crate::access::may_write;
use crate::contract::{ClientContract, ContractStatus};
use crate::db::refused;
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments::worker_name;
use crate::events::CoverRequested;
use crate::hold;
use crate::notices;
use crate::permissions;
use crate::request::{ManpowerRequest, RequestKind, RequestStatus};
use crate::site::Site;
use crate::texts::english;

/// The causes of a cover request.
pub mod causes {
    /// A leave request was approved (`MPW.RUL.06`).
    pub const LEAVE: &str = "leave";
    /// The deployment was suspended (BL-MPW-11).
    pub const SUSPENSION: &str = "suspension";
    /// The client rejected the worker (BL-MPW-14).
    pub const REJECTION: &str = "rejection";
    /// The account manager asked for it.
    pub const MANUAL: &str = "manual";
    /// Every cause.
    pub const ALL: &[&str] = &[LEAVE, SUSPENSION, REJECTION, MANUAL];
}

/// What a cover request is for.
#[derive(Debug, Clone)]
pub struct CoverAsk<'a> {
    /// The deployment covered.
    pub deployment: &'a Deployment,
    /// The first day of cover.
    pub from: NaiveDate,
    /// The last day; none for an open-ended cover.
    pub to: Option<NaiveDate>,
    /// Why, one of [`causes`].
    pub cause: &'a str,
    /// What asked for it (`hr.leave_request:5:12`): one request per cause. None
    /// for a request made by hand.
    pub cause_ref: Option<String>,
}

/// What [`open_cover`] did.
#[derive(Debug, Clone)]
pub enum Cover {
    /// A cover request was opened.
    Opened(ManpowerRequest),
    /// The cause had its request already; nothing changed.
    Existing(ManpowerRequest),
    /// The contract does not require cover, or no longer takes requests.
    NotRequired,
}

/// Opens the cover request of `ask` unless the contract does not require it.
///
/// # Errors
///
/// The database's errors.
pub async fn open_cover(state: &AppState, ask: CoverAsk<'_>) -> Result<Cover> {
    let deployment = ask.deployment;
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let by_hand = ask.cause == causes::MANUAL;
    let open_to_requests = !matches!(
        contract.status,
        ContractStatus::Draft | ContractStatus::Ended | ContractStatus::Cancelled
    );
    if !open_to_requests || (!by_hand && !contract.cover_required) {
        return Ok(Cover::NotRequired);
    }
    if let Some(cause_ref) = &ask.cause_ref
        && let Some(existing) = ManpowerRequest::query()
            .where_(ManpowerRequest::CAUSE_REF.eq(cause_ref.clone()))
            .first(&state.db)
            .await?
    {
        return Ok(Cover::Existing(existing));
    }
    let mut request = ManpowerRequest {
        client_contract_id: contract.id,
        client_party_id: contract.client_party_id,
        site_id: deployment.site_id,
        role: deployment.role.clone(),
        headcount: 1,
        shift_type: deployment.shift_type.clone(),
        shift_start: deployment.shift_start.clone(),
        shift_end: deployment.shift_end.clone(),
        start_date: ask.from,
        end_date: ask.to,
        kind: RequestKind::Replacement,
        cause: Some(ask.cause.to_owned()),
        cause_ref: ask.cause_ref.clone(),
        replaces_deployment_id: Some(deployment.id),
        status: RequestStatus::Open,
        ..ManpowerRequest::default()
    };
    let mut tx = state.db.begin().await?;
    request
        .insert_in_state(
            &mut tx,
            RequestStatus::Open,
            &english("manpower.cover.history", &[("cause", &ask.cause)]),
        )
        .await?;
    tx.commit().await?;
    state
        .emit(CoverRequested {
            request: request.subject(),
            deployment: deployment.subject(),
            client: request.client(),
            from: request.start_date,
            to: request.end_date,
            cause: ask.cause.to_owned(),
        })
        .await?;
    tell_account_manager(state, &contract, deployment, &request).await?;
    Ok(Cover::Opened(request))
}

/// Tells the account manager that a cover request is open.
async fn tell_account_manager(
    state: &AppState,
    contract: &ClientContract,
    deployment: &Deployment,
    request: &ManpowerRequest,
) -> Result {
    let managers =
        notices::account_managers(state, contract.client(), contract.account_manager_user_id)
            .await?;
    let worker = worker_name(state, "en", deployment.employee()).await;
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let until = request
        .end_date
        .map_or_else(|| "-".to_owned(), |end| end.to_string());
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("worker", &worker),
        ("role", &request.role),
        ("site", &site),
        ("from", &request.start_date),
        ("until", &until),
    ];
    notices::tell(
        state,
        request.subject(),
        &contract.reference,
        &managers,
        &english("manpower.cover.opened_title", &params),
        &english("manpower.cover.opened_body", &params),
    )
    .await?;
    Ok(())
}

/// `by` opens a cover request for deployment `deployment_id` from `from` to
/// `to` by hand (§4.1: "the account manager may create a replacement
/// request"), whether or not the contract requires cover.
///
/// # Errors
///
/// `Forbidden` without `requests.manage` for the client; a 422 unless the
/// deployment is `active` or `suspended`, the dates are in order and the
/// contract takes requests.
pub async fn open_by_hand(
    state: &AppState,
    by: &AuthUser,
    deployment_id: i64,
    from: NaiveDate,
    to: Option<NaiveDate>,
) -> Result<ManpowerRequest> {
    let deployment = Deployment::find_or_404(&state.db, deployment_id).await?;
    if !may_write(
        state,
        by,
        deployment.client(),
        permissions::REQUESTS_MANAGE,
        false,
    )
    .await?
    {
        return Err(Error::Forbidden);
    }
    if !deployment.is_on_site() || to.is_some_and(|to| to < from) {
        return Err(refused(english("manpower.cover.not_covered", &[])));
    }
    // A cover request by hand asks for a new worker, which a contract on
    // credit hold takes no more of (`MPW.ACL.14`). The cover the contract
    // requires (`MPW.RUL.06`, suspension, rejection) still opens: the worker
    // on site is the agency's to replace, and the cover worker's `activate`
    // is refused until the hold is released.
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    if contract.status == ContractStatus::OnHold {
        return Err(refused(hold::refusal(state, &contract).await?));
    }
    match open_cover(
        state,
        CoverAsk {
            deployment: &deployment,
            from,
            to,
            cause: causes::MANUAL,
            cause_ref: None,
        },
    )
    .await?
    {
        Cover::Opened(request) | Cover::Existing(request) => Ok(request),
        Cover::NotRequired => Err(refused(english("manpower.cover.contract_closed", &[]))),
    }
}

/// `MPW.RUL.06`: the cover requests a leave approved for `leave` asks for:
/// one for every `active` deployment of the employee that overlaps the
/// leave, for the days of the overlap. Returns what [`open_cover`] did for
/// each; the same leave a second time changes nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn for_leave(state: &AppState, leave: &LeaveApproved) -> Result<Vec<Cover>> {
    let deployments = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(leave.employee.0))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Active))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let mut done = Vec::new();
    for deployment in &deployments {
        let from = leave.from.max(deployment.start_date).max(today());
        let to = deployment
            .end_date
            .map_or(leave.to, |end| end.min(leave.to));
        if to < from {
            continue;
        }
        done.push(
            open_cover(
                state,
                CoverAsk {
                    deployment,
                    from,
                    to: Some(to),
                    cause: causes::LEAVE,
                    cause_ref: Some(format!("{}:{}", leave.leave, deployment.id)),
                },
            )
            .await?,
        );
    }
    Ok(done)
}

/// Listens for the approval of a leave request (`MPW.RUL.06`).
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: LeaveApproved, state: AppState| async move {
        for_leave(&state, &event).await?;
        Ok(())
    });
}
