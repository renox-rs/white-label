//! The deployment services: one function per action, each gathering what the
//! machine's guards need and running the transition (09 Manpower §7.1
//! *Deployment*; Base 09).
//!
//! `activate` also takes a position of the request it fills, in the same
//! transaction (`fill_partly` or `fill`, §7.1 *Manpower request*). After the
//! commit the client is told (`notices`) and the event is emitted.

use renox::chrono::NaiveDate;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine};
use wl_kernel::block_guard::ActionKind;
use wl_kernel::documents::today;
use wl_kernel::parties::directory;
use wl_kernel::workforce::{EmployeeId, workforce};
use wl_kernel::{PartyId, Subject};

use crate::blocked;
use crate::contract::{ClientContract, ContractStatus};
use crate::db::{answer, lock_contract, refused};
use crate::deployment::{
    ACTIVATE, AUTO_CANCEL, AUTO_END, CANCEL, Deployment, DeploymentInput, DeploymentKind,
    DeploymentStatus, END, EXTEND,
};
use crate::events::{DeploymentActivated, DeploymentCancelled, DeploymentEnded};
use crate::hold;
use crate::notices;
use crate::permissions;
use crate::request::{FILL, FILL_PARTLY, ManpowerRequest};
use crate::site::Site;
use crate::texts::english;

/// What a new deployment is made of.
#[derive(Debug, Clone)]
pub struct NewDeployment {
    /// The wl-hr employee.
    pub employee: EmployeeId,
    /// The client contract.
    pub contract_id: i64,
    /// The site.
    pub site_id: i64,
    /// The request it fills, if any.
    pub request_id: Option<i64>,
    /// The role.
    pub role: String,
    /// The shift type the rate card prices.
    pub shift_type: String,
    /// The shift start as `HH:MM`.
    pub shift_start: Option<String>,
    /// The shift end as `HH:MM`.
    pub shift_end: Option<String>,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day; none while open-ended.
    pub end_date: Option<NaiveDate>,
}

/// The deployment `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<Deployment> {
    Deployment::find_or_404(&state.db, id).await
}

/// `Forbidden` unless `by` manages deployments of the client company.
fn require_manage(by: &AuthUser, client: PartyId) -> Result {
    if by.has_permission_in(
        permissions::DEPLOYMENTS_MANAGE,
        &permissions::client_scope(client.0),
    ) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Plans a deployment: `planned`, for an employee HR knows, at a site of the
/// client, under a contract that is not a draft, over or cancelled (a hold
/// does not stop a plan, `MPW.ACL.14`), within the contract's dates. When it
/// fills a request, the request takes workers, is the same contract's and
/// asks for the same role and site; a request for cover makes the deployment a
/// cover for the deployment it replaces.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 for any of
/// the above.
pub async fn plan(state: &AppState, by: &AuthUser, new: NewDeployment) -> Result<Deployment> {
    let contract = ClientContract::find(&state.db, new.contract_id)
        .await?
        .ok_or_else(|| refused(english("manpower.deployment.contract_unknown", &[])))?;
    require_manage(by, contract.client())?;
    if matches!(
        contract.status,
        ContractStatus::Draft | ContractStatus::Ended | ContractStatus::Cancelled
    ) {
        return Err(refused(english("manpower.deployment.contract_closed", &[])));
    }
    let site_ok = Site::find(&state.db, new.site_id)
        .await?
        .is_some_and(|site| site.client_party_id == contract.client_party_id && site.active);
    if !site_ok {
        return Err(refused(english("manpower.request.site", &[])));
    }
    let known = workforce(state)
        .staff(state, new.employee)
        .await?
        .is_some_and(|profile| profile.listed);
    if !known {
        return Err(refused(english("manpower.deployment.employee", &[])));
    }
    if new.role.trim().is_empty() || new.shift_type.trim().is_empty() {
        return Err(refused(english("manpower.request.fields", &[])));
    }
    if new.start_date < contract.start_date
        || new.start_date > contract.end_date
        || new
            .end_date
            .is_some_and(|end| end < new.start_date || end > contract.end_date)
    {
        return Err(refused(english("manpower.deployment.dates", &[])));
    }
    let mut kind = DeploymentKind::Standard;
    let mut covers = None;
    let mut end_date = new.end_date;
    if let Some(request_id) = new.request_id {
        let request = ManpowerRequest::find(&state.db, request_id)
            .await?
            .filter(|request| request.client_contract_id == contract.id)
            .ok_or_else(|| refused(english("manpower.deployment.request", &[])))?;
        if !request.status.takes_workers()
            || !request.role.eq_ignore_ascii_case(new.role.trim())
            || request.site_id != new.site_id
        {
            return Err(refused(english("manpower.deployment.request", &[])));
        }
        if request.kind == crate::request::RequestKind::Replacement {
            kind = DeploymentKind::Cover;
            covers = request.replaces_deployment_id;
            // A cover runs to the end of the cover period unless told
            // otherwise (§7.1 `resume`).
            end_date = end_date.or(request.end_date);
        }
    }
    Deployment::create(
        &state.db,
        Deployment {
            employee_id: new.employee.0,
            client_contract_id: contract.id,
            client_party_id: contract.client_party_id,
            site_id: new.site_id,
            request_id: new.request_id,
            role: new.role.trim().to_lowercase(),
            shift_type: new.shift_type.trim().to_lowercase(),
            shift_start: new.shift_start.filter(|time| !time.trim().is_empty()),
            shift_end: new.shift_end.filter(|time| !time.trim().is_empty()),
            start_date: new.start_date,
            end_date,
            kind,
            covers_deployment_id: covers,
            created_by: Some(by.id),
            ..Deployment::default()
        },
    )
    .await
}

/// The name of the worker as the client sees it, else `#id`.
pub(crate) async fn worker_name(state: &AppState, lang: &str, employee: EmployeeId) -> String {
    let party = workforce(state)
        .staff(state, employee)
        .await
        .ok()
        .flatten()
        .map(|profile| profile.party);
    if let Some(party) = party
        && let Ok(Some(name)) = directory(state).display_name(&state.db, party, lang).await
    {
        return name;
    }
    format!("#{}", employee.0)
}

/// Why `contract`, as `tx` reads it, takes no deployment now, if it does not
/// (`MPW.ACL.14`): *Contract on credit hold since <date>: <reason>* while it is
/// `on_hold`, else that it takes none. A deployment is for staff, so the
/// reason is shown.
async fn contract_problem(
    tx: &mut Transaction,
    contract: &ClientContract,
) -> Result<Option<String>> {
    if contract.takes_deployments() {
        return Ok(None);
    }
    Ok(Some(if contract.status == ContractStatus::OnHold {
        hold::refusal_in(tx, contract).await?
    } else {
        english("manpower.deployment.contract_not_active", &[])
    }))
}

/// The checks that fail for activating `deployment` on `on`, one English line
/// each: the employee `active` in HR, the end date not past, the request still
/// taking workers. The contract (`active` or `terminating`, `MPW.ACL.14`) is
/// read in the transaction of the run ([`contract_problem`]).
async fn activation_problems(
    state: &AppState,
    deployment: &Deployment,
    on: NaiveDate,
) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    if !workforce(state)
        .is_active(state, deployment.employee())
        .await?
    {
        problems.push(english("manpower.deployment.employee_not_active", &[]));
    }
    if deployment.end_date.is_some_and(|end| end < on) {
        problems.push(english("manpower.deployment.over", &[]));
    }
    if let Some(request_id) = deployment.request_id {
        let takes = ManpowerRequest::find(&state.db, request_id)
            .await?
            .is_some_and(|request| request.status.takes_workers());
        if !takes {
            problems.push(english("manpower.deployment.request_closed", &[]));
        }
    }
    Ok(problems)
}

/// Activates deployment `id` today for `by`: the worker is on site, the
/// request it fills takes a position in the same transaction, the client is
/// told and [`DeploymentActivated`] is emitted.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 listing
/// every failing check, or unless the deployment is `planned`.
pub async fn activate(state: &AppState, by: &AuthUser, id: i64, lang: &str) -> Result<Deployment> {
    let deployment = load(state, id).await?;
    require_manage(by, deployment.client())?;
    let on = today();
    let mut problems = activation_problems(state, &deployment, on).await?;
    let mut tx = state.db.begin().await?;
    // The contract is read in the transaction of the transition, so a `hold`
    // committed first wins (`MPW.ACL.14`).
    let contract = lock_contract(&mut tx, deployment.client_contract_id).await?;
    if let Some(problem) = contract_problem(&mut tx, &contract).await? {
        problems.insert(0, problem);
    }
    let input = DeploymentInput {
        problems,
        on: Some(on),
        new_end_date: None,
    };
    // A blocked client starts nothing new (BL-MPW-15).
    blocked::check(
        &mut tx,
        state,
        Some(by),
        deployment.client(),
        ActionKind::Start,
    )
    .await?;
    let activated = deployment
        .transition(ACTIVATE)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(answer)?;
    let filled = match deployment.request_id {
        Some(request_id) => Some(fill_in(&mut tx, request_id).await?),
        None => None,
    };
    tx.commit().await?;
    activated.emit(state).await?;
    if let Some(filled) = filled {
        filled.emit(state).await?;
    }
    let active = load(state, id).await?;
    state
        .emit(DeploymentActivated {
            deployment: active.subject(),
            employee: active.employee(),
            client: active.client(),
            site: Subject::new(crate::site::SUBJECT_TYPE, active.site_id),
            request: active
                .request_id
                .map(|request| Subject::new(crate::request::SUBJECT_TYPE, request)),
            role: active.role.clone(),
        })
        .await?;
    let worker = worker_name(state, lang, active.employee()).await;
    tell_client(state, &active, "manpower.deployment.activated", &worker).await?;
    Ok(active)
}

/// Takes a position of request `request_id` in `tx`: `fill` for the last,
/// `fill_partly` otherwise, from the request as it is in the transaction.
async fn fill_in(tx: &mut Transaction, request_id: i64) -> Result<Outcome<ManpowerRequest>> {
    let request = ManpowerRequest::query()
        .where_(ManpowerRequest::ID.eq(request_id))
        .lock_for_update()
        .first(&mut *tx)
        .await?
        .ok_or(Error::NotFound)?;
    let name = if request.filled_count + 1 >= request.headcount {
        FILL
    } else {
        FILL_PARTLY
    };
    request
        .transition(name)
        .by_system()
        .run_in(&mut *tx)
        .await
        .map_err(answer)
}

/// Tells the client company about `deployment` with the texts of `key`
/// (`<key>_title`, `<key>_body`).
async fn tell_client(state: &AppState, deployment: &Deployment, key: &str, worker: &str) -> Result {
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let ends = deployment
        .end_date
        .map_or_else(|| "-".to_owned(), |end| end.to_string());
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 5] = [
        ("worker", &worker),
        ("role", &deployment.role),
        ("site", &site),
        ("from", &deployment.start_date),
        ("until", &ends),
    ];
    notices::tell_client(
        state,
        deployment.subject(),
        &contract.reference,
        deployment.client(),
        &english(&format!("{key}_title"), &params),
        &english(&format!("{key}_body"), &params),
    )
    .await
}

/// Extends active deployment `id` to `new_end` for `by`: later than its end
/// date, within the contract, while the contract takes deployments. The
/// client is told.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 unless the
/// deployment is `active` and every check passes.
pub async fn extend(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    new_end: NaiveDate,
    lang: &str,
) -> Result<Deployment> {
    let deployment = load(state, id).await?;
    require_manage(by, deployment.client())?;
    let mut tx = state.db.begin().await?;
    // A blocked client extends nothing (BL-MPW-15).
    blocked::check(
        &mut tx,
        state,
        Some(by),
        deployment.client(),
        ActionKind::Extend,
    )
    .await?;
    // The contract is read in the transaction of the transition, so a `hold`
    // committed first wins (`MPW.ACL.14`).
    let contract = lock_contract(&mut tx, deployment.client_contract_id).await?;
    let mut problems = Vec::new();
    if let Some(problem) = contract_problem(&mut tx, &contract).await? {
        problems.push(problem);
    }
    match deployment.end_date {
        None => problems.push(english("manpower.deployment.open_ended", &[])),
        Some(end) if new_end <= end => {
            problems.push(english("manpower.deployment.extend_later", &[]));
        }
        Some(_) => {}
    }
    if new_end > contract.end_date {
        problems.push(english("manpower.deployment.beyond_contract", &[]));
    }
    let input = DeploymentInput {
        problems,
        on: Some(today()),
        new_end_date: Some(new_end),
    };
    let outcome = deployment
        .transition(EXTEND)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(answer)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    let extended = load(state, id).await?;
    let worker = worker_name(state, lang, extended.employee()).await;
    tell_client(state, &extended, "manpower.deployment.extended", &worker).await?;
    Ok(extended)
}

/// Ends deployment `id` for `by` with `reason` (one of
/// [`crate::deployment::reasons::USER`]). *End date* needs the end date to
/// have come. The client is told and [`DeploymentEnded`] is emitted.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 for a
/// reason the user may not give yet, an end date still to come or unless the
/// deployment is `active` or `suspended`.
pub async fn end(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    lang: &str,
) -> Result<Deployment> {
    let deployment = load(state, id).await?;
    require_manage(by, deployment.client())?;
    let mut problems = Vec::new();
    if reason.trim() == crate::deployment::reasons::END_DATE
        && !deployment.end_date.is_some_and(|end| end <= today())
    {
        problems.push(english("manpower.deployment.end_date_not_come", &[]));
    }
    let input = DeploymentInput {
        problems,
        on: Some(today()),
        new_end_date: None,
    };
    let _outcome = deployment
        .transition(END)
        .by(by)
        .reason(reason)
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    after_end(state, id, lang).await
}

/// What follows the commit of an `end`: the event and the client's notice.
async fn after_end(state: &AppState, id: i64, lang: &str) -> Result<Deployment> {
    let ended = load(state, id).await?;
    let last_day = ended.ended_on.unwrap_or_else(today);
    state
        .emit(DeploymentEnded {
            deployment: ended.subject(),
            employee: ended.employee(),
            client: ended.client(),
            reason: ended.end_reason.clone().unwrap_or_default(),
            last_day,
        })
        .await?;
    let worker = worker_name(state, lang, ended.employee()).await;
    tell_client(state, &ended, "manpower.deployment.ended", &worker).await?;
    Ok(ended)
}

/// Cancels planned deployment `id` for `by` with `reason`.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 without a
/// reason or unless the deployment is `planned`.
pub async fn cancel(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<Deployment> {
    let deployment = load(state, id).await?;
    require_manage(by, deployment.client())?;
    let _outcome = deployment
        .transition(CANCEL)
        .by(by)
        .reason(reason)
        .input(&DeploymentInput::default())
        .run(state)
        .await
        .map_err(answer)?;
    let cancelled = load(state, id).await?;
    announce_cancelled(state, &cancelled, reason).await?;
    Ok(cancelled)
}

/// Emits [`DeploymentCancelled`] for `deployment`.
async fn announce_cancelled(state: &AppState, deployment: &Deployment, reason: &str) -> Result {
    state
        .emit(DeploymentCancelled {
            deployment: deployment.subject(),
            employee: deployment.employee(),
            client: deployment.client(),
            reason: reason.trim().to_owned(),
        })
        .await
}

/// The system cancels the planned deployments of request `request_id` in
/// `tx` with `reason` (the request was cancelled); returns the outcomes to
/// emit after the commit and the deployments cancelled.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn cancel_planned_of_request(
    tx: &mut Transaction,
    request_id: i64,
    reason: &str,
) -> Result<(Vec<Outcome<Deployment>>, Vec<Deployment>)> {
    let planned = Deployment::query()
        .where_(Deployment::REQUEST_ID.eq(request_id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Planned))
        .order_by(Deployment::ID)
        .get(&mut *tx)
        .await?;
    let mut outcomes = Vec::new();
    for deployment in &planned {
        outcomes.push(
            deployment
                .transition(AUTO_CANCEL)
                .by_system()
                .reason(reason)
                .run_in(&mut *tx)
                .await
                .map_err(answer)?,
        );
    }
    Ok((outcomes, planned))
}

/// Announces `deployments` cancelled with `reason` after the commit.
pub(crate) async fn announce_all_cancelled(
    state: &AppState,
    deployments: &[Deployment],
    reason: &str,
) -> Result {
    for deployment in deployments {
        announce_cancelled(state, deployment, reason).await?;
    }
    Ok(())
}

/// The daily end-date run: on `on`, every `active` or `suspended` deployment
/// whose end date is before `on` is ended by the system with the reason *end
/// date*. A deployment ended is not ended again, so a second run changes
/// nothing. Returns the deployments ended.
///
/// # Errors
///
/// The database's errors.
pub async fn end_due(state: &AppState, on: NaiveDate) -> Result<Vec<Deployment>> {
    let due = Deployment::query()
        .where_(Deployment::STATUS.is_in([DeploymentStatus::Active, DeploymentStatus::Suspended]))
        .where_(Deployment::END_DATE.lt(on))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let input = DeploymentInput {
        problems: Vec::new(),
        on: Some(on),
        new_end_date: None,
    };
    let mut ended = Vec::new();
    for deployment in due {
        let done = deployment
            .transition(AUTO_END)
            .by_system()
            .reason(crate::deployment::reasons::END_DATE)
            .input(&input)
            .run(state)
            .await;
        match done {
            Ok(_outcome) => {
                ended.push(after_end(state, deployment.id, "en").await?);
            }
            // Another run ended it first.
            Err(renox::state_machine::TransitionError::NotFrom(_))
            | Err(renox::state_machine::TransitionError::Conflict) => {}
            Err(other) => return Err(answer(other)),
        }
    }
    Ok(ended)
}

/// The daily run at 02:00 Asia/Dubai.
///
/// # Errors
///
/// As [`end_due`].
pub async fn daily(state: &AppState) -> Result {
    end_due(state, today()).await?;
    Ok(())
}
