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
use wl_kernel::documents::{PlannedWarning, today};
use wl_kernel::parties::directory;
use wl_kernel::timesheets::{TimesheetClosure, timesheet_closing};
use wl_kernel::workforce::{EmployeeId, workforce};
use wl_kernel::{PartyId, Subject};

use crate::blocked;
use crate::contract::{ClientContract, ContractStatus};
use crate::db::{answer, lock_contract, refused};
use crate::deployment::{
    ACTIVATE, AUTO_CANCEL, AUTO_END, CANCEL, Deployment, DeploymentInput, DeploymentKind,
    DeploymentStatus, EXTEND, EXTEND_SUSPENDED,
};
use crate::documents::{ACTIVATE_GUARDED, EXTEND_GUARDED};
use crate::events::{DeploymentActivated, DeploymentCancelled, DeploymentEnded};
use crate::guard;
use crate::hold;
use crate::notices;
use crate::overlap;
use crate::permissions;
use crate::request::{FILL, FILL_PARTLY, ManpowerRequest};
use crate::site::Site;
use crate::texts::{english, refusal};

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
    /// The weekdays of the shift (`mon,tue,fri`); none: every day.
    pub work_days: Option<String>,
    /// The first day.
    pub start_date: NaiveDate,
    /// The last day; none while open-ended.
    pub end_date: Option<NaiveDate>,
}

/// The deployment `id`, or 404.
pub(crate) async fn load(state: &AppState, id: i64) -> Result<Deployment> {
    Deployment::find_or_404(&state.db, id).await
}

/// `Forbidden` unless `by` manages deployments of the client company.
pub(crate) fn require_manage(by: &AuthUser, client: PartyId) -> Result {
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
    Ok(plan_with_warning(state, by, new).await?.0)
}

/// [`plan`], and the document guard's warning for the first day of the new
/// deployment when a document or licence it needs will not be valid then
/// (`MPW.ACL.07`: planning only warns; `activate` is refused later).
///
/// # Errors
///
/// As [`plan`].
pub async fn plan_with_warning(
    state: &AppState,
    by: &AuthUser,
    new: NewDeployment,
) -> Result<(Deployment, Option<PlannedWarning>)> {
    let contract = ClientContract::find(&state.db, new.contract_id)
        .await?
        .ok_or_else(|| refused(refusal("manpower.deployment.contract_unknown", &[])))?;
    require_manage(by, contract.client())?;
    if matches!(
        contract.status,
        ContractStatus::Draft | ContractStatus::Ended | ContractStatus::Cancelled
    ) {
        return Err(refused(refusal("manpower.deployment.contract_closed", &[])));
    }
    let site_ok = Site::find(&state.db, new.site_id)
        .await?
        .is_some_and(|site| site.client_party_id == contract.client_party_id && site.active);
    if !site_ok {
        return Err(refused(refusal("manpower.request.site", &[])));
    }
    let known = workforce(state)
        .staff(state, new.employee)
        .await?
        .is_some_and(|profile| profile.listed);
    if !known {
        return Err(refused(refusal("manpower.deployment.employee", &[])));
    }
    if new.role.trim().is_empty() || new.shift_type.trim().is_empty() {
        return Err(refused(refusal("manpower.request.fields", &[])));
    }
    let work_days = new
        .work_days
        .as_deref()
        .map(str::trim)
        .filter(|days| !days.is_empty())
        .map(str::to_lowercase);
    let shift_ok = [&new.shift_start, &new.shift_end]
        .into_iter()
        .flatten()
        .filter(|time| !time.trim().is_empty())
        .all(|time| overlap::parse_time(time).is_some());
    if !shift_ok
        || work_days
            .as_deref()
            .is_some_and(|d| overlap::parse_days(d).is_none())
    {
        return Err(refused(refusal("manpower.deployment.shift", &[])));
    }
    if new.start_date < contract.start_date
        || new.start_date > contract.end_date
        || new
            .end_date
            .is_some_and(|end| end < new.start_date || end > contract.end_date)
    {
        return Err(refused(refusal("manpower.deployment.dates", &[])));
    }
    let mut kind = DeploymentKind::Standard;
    let mut covers = None;
    let mut end_date = new.end_date;
    if let Some(request_id) = new.request_id {
        let request = ManpowerRequest::find(&state.db, request_id)
            .await?
            .filter(|request| request.client_contract_id == contract.id)
            .ok_or_else(|| refused(refusal("manpower.deployment.request", &[])))?;
        if !request.status.takes_workers()
            || !request.role.eq_ignore_ascii_case(new.role.trim())
            || request.site_id != new.site_id
        {
            return Err(refused(refusal("manpower.deployment.request", &[])));
        }
        if request.kind == crate::request::RequestKind::Replacement {
            kind = DeploymentKind::Cover;
            covers = request.replaces_deployment_id;
            // A cover runs to the end of the cover period unless told
            // otherwise (§7.1 `resume`).
            end_date = end_date.or(request.end_date);
        }
    }
    let planned = Deployment::create(
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
            work_days,
            start_date: new.start_date,
            end_date,
            kind,
            covers_deployment_id: covers,
            created_by: Some(by.id),
            ..Deployment::default()
        },
    )
    .await?;
    crate::search::index(&state.db, &planned.subject()).await?;
    let warning = guard::warn_planned(state, &planned).await?;
    Ok((planned, warning))
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
        refusal("manpower.deployment.contract_not_active", &[])
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
        problems.push(refusal("manpower.deployment.employee_not_active", &[]));
    }
    if deployment.end_date.is_some_and(|end| end < on) {
        problems.push(refusal("manpower.deployment.over", &[]));
    }
    if let Some(request_id) = deployment.request_id {
        let takes = ManpowerRequest::find(&state.db, request_id)
            .await?
            .is_some_and(|request| request.status.takes_workers());
        if !takes {
            problems.push(refusal("manpower.deployment.request_closed", &[]));
        }
    }
    Ok(problems)
}

/// Every check of `activate` that is not a document, in `tx`: the contract's
/// hold, the employee, the end date, the request, the overlap and a blocked
/// client. It never stops at the first (BL-MPW-24).
async fn gather_problems(
    state: &AppState,
    by: &AuthUser,
    deployment: &Deployment,
    on: NaiveDate,
    early: Vec<String>,
    tx: &mut Transaction,
) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let contract = lock_contract(&mut *tx, deployment.client_contract_id).await?;
    if let Some(text) = contract_problem(&mut *tx, &contract).await? {
        // The screen says it first; the service's line follows (it is pinned).
        found.push(if contract.status == ContractStatus::OnHold {
            english("manpower.check.hold", &[("detail", &text)])
        } else {
            text
        });
    }
    found.extend(early);
    if let Some(text) = overlap::problem(
        &mut *tx,
        deployment,
        on.max(deployment.start_date),
        deployment.end_date,
    )
    .await?
    {
        found.push(text);
    }
    // A blocked client starts nothing new (BL-MPW-15).
    if let Err(Error::Status(_, text)) = blocked::check(
        &mut *tx,
        state,
        Some(by),
        deployment.client(),
        ActionKind::Start,
    )
    .await
    {
        found.push(text);
    }
    Ok(found)
}

/// The checks of `activate` that are not documents, nothing written.
pub(crate) async fn other_problems(
    state: &AppState,
    by: &AuthUser,
    deployment: &Deployment,
    on: NaiveDate,
) -> Result<Vec<String>> {
    let early = activation_problems(state, deployment, on).await?;
    let mut tx = state.db.begin().await?;
    let found = gather_problems(state, by, deployment, on, early, &mut tx).await;
    drop(tx);
    found
}

/// A failing document, in words, as the page and the sheet show it.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct DocumentCheck {
    /// *Residence visa of J. Khan expired on 03 Nov 2026 (cannot be overridden)*.
    pub text: String,
    /// The page that asks for an override; none when it cannot be overridden.
    pub override_url: Option<String>,
}

/// The documents that fail for transition `guarded` of deployment `record_id`
/// judged on `check_on`, one line each in words; the user's approved
/// overrides count. The service's own refusal is the guard's raw message.
pub(crate) async fn document_checks(
    state: &AppState,
    by: &AuthUser,
    guarded: &str,
    record_id: i64,
    check_on: NaiveDate,
    worker: &str,
) -> Result<Vec<DocumentCheck>> {
    let checked = wl_kernel::documents::guard(state)
        .check_as(state, guarded, record_id, check_on, Some(by.id))
        .await?;
    let Err(refusal) = checked else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    for failure in &refusal.failures {
        let kind = document_label(&failure.message, &failure.entry.kind);
        let (key, date) = match &failure.validity {
            wl_kernel::documents::Validity::Invalid {
                reason, expired_on, ..
            } => (
                match reason {
                    wl_kernel::documents::InvalidReason::Expired => "manpower.check.expired",
                    wl_kernel::documents::InvalidReason::Missing => "manpower.check.missing",
                    wl_kernel::documents::InvalidReason::Rejected => "manpower.check.rejected",
                    wl_kernel::documents::InvalidReason::PendingReview => "manpower.check.pending",
                },
                expired_on
                    .map(crate::screens::date_text)
                    .unwrap_or_default(),
            ),
            wl_kernel::documents::Validity::Valid { .. } => continue,
        };
        let allowed = failure.override_allowed();
        let ending = english(
            if allowed {
                "manpower.check.override_allowed"
            } else {
                "manpower.check.override_never"
            },
            &[],
        );
        let line = english(
            key,
            &[("kind", &kind), ("worker", &worker), ("date", &date)],
        );
        let single = wl_kernel::documents::BlockRefusal {
            guarded: refusal.guarded.clone(),
            record_id: refusal.record_id,
            failures: vec![failure.clone()],
        };
        lines.push(DocumentCheck {
            text: format!("{line} {ending}"),
            override_url: single.override_links().into_iter().next(),
        });
    }
    Ok(lines)
}

/// The label of a document kind as the guard words it: the start of its
/// message (*Residence visa expired on ...* is *Residence visa*); the key in
/// words when the message has another shape.
fn document_label(message: &str, kind: &str) -> String {
    for marker in [" expired on", " missing", " rejected", " not accepted yet"] {
        if let Some(at) = message.find(marker) {
            return message[..at].to_owned();
        }
    }
    crate::screens::kind_label(kind)
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
    // The documents and licences are judged on the day of the transition plus
    // the look-ahead, or the end date if earlier (`MPW.ACL.07`).
    let check_on = guard::check_date(on, deployment.end_date);
    let started = std::sync::atomic::AtomicBool::new(false);
    let done = guard::guarded(state, by, ACTIVATE_GUARDED, id, check_on, || {
        started.store(true, std::sync::atomic::Ordering::SeqCst);
        run_activate(state, by, &deployment, on)
    })
    .await;
    let active = match done {
        // The documents refused before the other checks ran: list those too,
        // before the line that names the day the documents were judged on.
        Err(Error::Status(code, message)) if !started.load(std::sync::atomic::Ordering::SeqCst) => {
            let others: Vec<String> = other_problems(state, by, &deployment, on)
                .await?
                .into_iter()
                .collect();
            let message = match (others.is_empty(), message.rsplit_once('\n')) {
                (true, _) => message,
                (false, Some((documents, day))) => {
                    format!("{documents}\n{}\n{day}", others.join("\n"))
                }
                (false, None) => format!("{message}\n{}", others.join("\n")),
            };
            return Err(Error::Status(code, message));
        }
        other => other?,
    };
    let worker = worker_name(state, lang, active.employee()).await;
    tell_client(state, &active, "manpower.deployment.activated", &worker).await?;
    Ok(active)
}

/// The transition of [`activate`] with the request's position, and the events
/// after the commit.
async fn run_activate(
    state: &AppState,
    by: &AuthUser,
    deployment: &Deployment,
    on: NaiveDate,
) -> Result<Deployment> {
    let id = deployment.id;
    // One activation or extension of the worker at a time (`MPW.ACL.15`).
    let _worker = overlap::take(deployment.employee_id).await;
    let early = activation_problems(state, deployment, on).await?;
    let mut tx = state.db.begin().await?;
    // The contract is read in the transaction of the transition, so a `hold`
    // committed first wins (`MPW.ACL.14`).
    let problems: Vec<String> = gather_problems(state, by, deployment, on, early, &mut tx)
        .await?
        .into_iter()
        .collect();
    let input = DeploymentInput {
        problems,
        on: Some(on),
        new_end_date: None,
        ..DeploymentInput::default()
    };
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
pub(crate) async fn tell_client(
    state: &AppState,
    deployment: &Deployment,
    key: &str,
    worker: &str,
) -> Result {
    tell_client_as(state, notices::CLIENT_NOTICE, deployment, key, worker).await
}

/// [`tell_client`] with the notification type `type_key`.
pub(crate) async fn tell_client_as(
    state: &AppState,
    type_key: &str,
    deployment: &Deployment,
    key: &str,
    worker: &str,
) -> Result {
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
    notices::tell_client_as(
        state,
        type_key,
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
    // The documents and licences are judged on today plus the look-ahead, or
    // the new end date if earlier (`MPW.ACL.07`).
    let check_on = guard::check_date(today(), Some(new_end));
    let extended = guard::guarded(state, by, EXTEND_GUARDED, id, check_on, || {
        run_extend(state, by, &deployment, new_end)
    })
    .await?;
    let worker = worker_name(state, lang, extended.employee()).await;
    tell_client(state, &extended, "manpower.deployment.extended", &worker).await?;
    Ok(extended)
}

/// The transition of [`extend`].
async fn run_extend(
    state: &AppState,
    by: &AuthUser,
    deployment: &Deployment,
    new_end: NaiveDate,
) -> Result<Deployment> {
    // One activation or extension of the worker at a time (`MPW.ACL.15`).
    let _worker = overlap::take(deployment.employee_id).await;
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
        None => problems.push(refusal("manpower.deployment.open_ended", &[])),
        Some(end) if new_end <= end => {
            problems.push(refusal("manpower.deployment.extend_later", &[]));
        }
        Some(_) => {}
    }
    if new_end > contract.end_date {
        problems.push(refusal("manpower.deployment.beyond_contract", &[]));
    }
    if let Some(problem) = overlap::problem(
        &mut tx,
        deployment,
        today().max(deployment.start_date),
        Some(new_end),
    )
    .await?
    {
        problems.push(problem);
    }
    let input = DeploymentInput {
        problems,
        on: Some(today()),
        new_end_date: Some(new_end),
        ..DeploymentInput::default()
    };
    // A suspended deployment stays suspended (BL-MPW-11).
    let name = if deployment.status == DeploymentStatus::Suspended {
        EXTEND_SUSPENDED
    } else {
        EXTEND
    };
    let outcome = deployment
        .transition(name)
        .by(by)
        .input(&input)
        .run_in(&mut tx)
        .await
        .map_err(answer)?;
    tx.commit().await?;
    outcome.emit(state).await?;
    load(state, deployment.id).await
}

/// Ends deployment `id` for `by` with `reason` (one of
/// [`crate::deployment::reasons::USER`]), without the details the dearer
/// reasons ask for ([`crate::ends::end`] takes them). *End date* needs the end
/// date to have come. The client is told and [`DeploymentEnded`] is emitted.
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 for a
/// reason the user may not give, an end date still to come, a reason that
/// needs more than this gives (a note for *client rejected*) or unless the
/// deployment is `active` or `suspended`.
pub async fn end(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reason: &str,
    lang: &str,
) -> Result<Deployment> {
    crate::ends::end(
        state,
        by,
        id,
        reason,
        crate::ends::EndDetails::default(),
        lang,
    )
    .await
}

/// What follows the commit of an `end`: the open timesheet of the worker is
/// closed at the last day and submitted at once (the kernel's
/// `TimesheetClosing`, wl-attendance), the event and the client's notice.
/// Answers the deployment and the timesheets closed.
pub(crate) async fn finish_end(
    state: &AppState,
    id: i64,
    lang: &str,
) -> Result<(Deployment, Vec<TimesheetClosure>)> {
    finish_end_as(state, id, lang, "manpower.deployment.ended").await
}

/// [`finish_end`] with the client told by the texts of `client_key`
/// (`<key>_title`, `<key>_body`): an exit of the worker tells the client only
/// that the worker is no longer available (`MPW.RUL.12`).
pub(crate) async fn finish_end_as(
    state: &AppState,
    id: i64,
    lang: &str,
    client_key: &str,
) -> Result<(Deployment, Vec<TimesheetClosure>)> {
    let ended = load(state, id).await?;
    let last_day = ended.ended_on.unwrap_or_else(today);
    let closed = timesheet_closing(state)
        .close_open(
            state,
            &ended.subject(),
            ended.employee(),
            ended.end_date.map_or(last_day, |end| end.min(last_day)),
        )
        .await?;
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
    tell_client(state, &ended, client_key, &worker).await?;
    Ok((ended, closed))
}

/// [`finish_end`] for the callers that need the deployment only.
pub(crate) async fn after_end(state: &AppState, id: i64, lang: &str) -> Result<Deployment> {
    Ok(finish_end(state, id, lang).await?.0)
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
pub(crate) async fn announce_cancelled(
    state: &AppState,
    deployment: &Deployment,
    reason: &str,
) -> Result {
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
        ..DeploymentInput::default()
    };
    let mut ended = Vec::new();
    for deployment in due {
        let done = deployment
            .transition(AUTO_END)
            .by_system()
            .reason(
                deployment
                    .planned_end_reason
                    .as_deref()
                    .unwrap_or(crate::deployment::reasons::END_DATE),
            )
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
    // The suspensions (`MPW.RUL.12`) that failed are tried again.
    crate::suspension::retry(state).await
}
