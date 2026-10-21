//! Resuming a suspended deployment (09 Manpower §4.5 row 1, §7.1 *Deployment*
//! `resume`, §7.2 `MPW.RUL.12`, BL-MPW-12).
//!
//! - **By hand** ([`resume`], `deployments.manage` for the client): the only
//!   way for a manual reason (*site access withdrawn*, *investigation*). The
//!   documents and licences must pass again (`MPW.ACL.07`, judged on today plus
//!   30 days or the end date, [`crate::guard`]; an override of a kind that
//!   allows one covers it), the end date must not have passed and the contract
//!   must not have `ended`. **A contract on credit hold (`MPW.ACL.14`) and a
//!   blocked client (BL-MPW-15) do not stop a `resume`**: it restarts no new
//!   work, the worker was on site before, so neither check is made.
//! - **By the system** ([`listen`], `MPW.RUL.12`): when a renewed document is
//!   accepted (`DocumentAccepted`) the open reason of its kind is cleared (the
//!   new expiry date must be later than the reason's); the deployment resumes
//!   (`auto_resume`) only when no reason is left and none is manual, and the
//!   checks pass. Accepting a kind that is not the expired one changes nothing
//!   while another is still expired. When the checks still fail with no reason
//!   left, the deployment stays `suspended` and the account manager is told
//!   which kind still fails (the next acceptance tries again); a deployment
//!   whose end date has passed or whose contract has ended does not resume and
//!   is ended by the daily run. The run is one compliance-critical `run_rule`
//!   per deployment and document, retried by the 02:00 run
//!   ([`crate::suspension::retry`]).
//! - **Effects.** Attendance is accepted again (the deployment is `active`),
//!   `resumed_on` is the day and the calendar days suspended are added to
//!   `suspended_days` (not billed), every reason is cleared, [`DeploymentResumed`]
//!   is emitted and the client is told. The cover request of the suspension, if
//!   still `open` or `partially_filled`, is cancelled with the reason *worker
//!   returned* and its `planned` cover deployments with it; an `active` cover
//!   deployment runs to its own end date, and a `filled` request is left as it
//!   is. Nothing ends the cover deployment early.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::automation::{RuleRunSpec, run_rule};
use wl_kernel::documents::{DocumentAccepted, today};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_kernel::{PartyId, Subject};

use crate::contract::{ClientContract, ContractStatus};
use crate::db::answer;
use crate::deployment::{
    AUTO_RESUME, Deployment, DeploymentInput, DeploymentStatus, RESUME, SUBJECT_TYPE,
};
use crate::deployments::{cancel_planned_of_request, worker_name};
use crate::documents::RESUME_GUARDED;
use crate::events::DeploymentResumed;
use crate::guard;
use crate::notices;
use crate::permissions;
use crate::request::{self, ManpowerRequest, RequestStatus};
use crate::site::Site;
use crate::suspension::{DeploymentSuspension, RULE, SuspensionKind};
use crate::texts::english;

/// The reason a cover request is cancelled with when the worker returns.
pub const WORKER_RETURNED: &str = "worker returned";

/// The checks of a `resume` that are not the documents': the end date has not
/// passed and the contract has not ended, one English line each.
async fn problems(state: &AppState, deployment: &Deployment, on: NaiveDate) -> Result<Vec<String>> {
    let mut found = Vec::new();
    if deployment.end_date.is_some_and(|end| end < on) {
        found.push(english("manpower.deployment.over", &[]));
    }
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    if matches!(
        contract.status,
        ContractStatus::Ended | ContractStatus::Cancelled
    ) {
        found.push(english("manpower.deployment.resume_contract_ended", &[]));
    }
    Ok(found)
}

/// `by` resumes suspended deployment `id` today (`lang` words the client's
/// notice).
///
/// # Errors
///
/// `Forbidden` without `deployments.manage` for the client; a 422 listing the
/// documents and licences that still fail, an end date that has passed, a
/// contract that has ended, or unless the deployment is `suspended`.
pub async fn resume(state: &AppState, by: &AuthUser, id: i64, lang: &str) -> Result<Deployment> {
    let deployment = Deployment::find_or_404(&state.db, id).await?;
    if !by.has_permission_in(permissions::DEPLOYMENTS_MANAGE, &deployment.scope()) {
        return Err(Error::Forbidden);
    }
    let on = today();
    let check_on = guard::check_date(on, deployment.end_date);
    guard::guarded(state, by, RESUME_GUARDED, id, check_on, || {
        run(state, Some(by), &deployment, on, lang)
    })
    .await
}

/// The transition of a `resume` (`by`: the user; none: the system), the
/// cleared reasons, the cancelled cover request, then the event and the
/// client's notice after the commit.
async fn run(
    state: &AppState,
    by: Option<&AuthUser>,
    deployment: &Deployment,
    on: NaiveDate,
    lang: &str,
) -> Result<Deployment> {
    let id = deployment.id;
    let input = DeploymentInput {
        problems: problems(state, deployment, on).await?,
        on: Some(on),
        new_end_date: None,
        ..DeploymentInput::default()
    };
    let mut tx = state.db.begin().await?;
    let locked = Deployment::query()
        .where_(Deployment::ID.eq(id))
        .lock_for_update()
        .first(&mut tx)
        .await?
        .ok_or(Error::NotFound)?;
    let outcome = match by {
        Some(user) => locked.transition(RESUME).by(user),
        None => locked.transition(AUTO_RESUME).by_system(),
    }
    .input(&input)
    .run_in(&mut tx)
    .await
    .map_err(answer)?;
    DeploymentSuspension::query()
        .where_(DeploymentSuspension::DEPLOYMENT_ID.eq(id))
        .where_(DeploymentSuspension::CLEARED_ON.is_null())
        .update(&mut tx, &[("cleared_on", &Some(on))])
        .await?;
    let cancelled = cancel_cover(&mut tx, &locked).await?;
    tx.commit().await?;
    outcome.emit(state).await?;
    for (request, planned) in cancelled {
        request.emit(state).await?;
        for deployment in planned {
            deployment.emit(state).await?;
        }
    }
    let back = Deployment::find_or_404(&state.db, id).await?;
    state
        .emit(DeploymentResumed {
            deployment: back.subject(),
            employee: back.employee(),
            client: back.client(),
            on,
        })
        .await?;
    tell_client(state, &back, on, lang).await?;
    Ok(back)
}

/// The outcomes of cancelling a cover request and its planned deployments.
type Cancelled = (
    renox::state_machine::Outcome<ManpowerRequest>,
    Vec<renox::state_machine::Outcome<Deployment>>,
);

/// Cancels the cover request of the suspension of `deployment` while it is
/// still `open` or `partially_filled`, with the planned deployments filling it.
async fn cancel_cover(
    tx: &mut renox::db::Transaction,
    deployment: &Deployment,
) -> Result<Vec<Cancelled>> {
    let Some(since) = deployment.suspended_on else {
        return Ok(Vec::new());
    };
    let cause_ref = format!("{SUBJECT_TYPE}:{}:suspension:{since}", deployment.id);
    let requests = ManpowerRequest::query()
        .where_(ManpowerRequest::CAUSE_REF.eq(cause_ref))
        .where_(
            ManpowerRequest::STATUS.is_in([RequestStatus::Open, RequestStatus::PartiallyFilled]),
        )
        .order_by(ManpowerRequest::ID)
        .get(&mut *tx)
        .await?;
    let mut done = Vec::new();
    for request in requests {
        let first = request
            .transition(request::AUTO_CANCEL)
            .by_system()
            .reason(WORKER_RETURNED)
            .run_in(&mut *tx)
            .await
            .map_err(answer)?;
        let (planned, _) = cancel_planned_of_request(&mut *tx, request.id, WORKER_RETURNED).await?;
        done.push((first, planned));
    }
    Ok(done)
}

/// Tells the client the worker is back.
async fn tell_client(
    state: &AppState,
    deployment: &Deployment,
    on: NaiveDate,
    lang: &str,
) -> Result {
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let worker = worker_name(state, lang, deployment.employee()).await;
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default();
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] =
        [("worker", &worker), ("site", &site), ("on", &on)];
    notices::tell_client(
        state,
        deployment.subject(),
        &contract.reference,
        deployment.client(),
        &english("manpower.deployment.resumed_title", &params),
        &english("manpower.deployment.resumed_body", &params),
    )
    .await
}

/// Tells the account manager that the deployment stays suspended because
/// `problems` still fail.
async fn tell_still_failing(state: &AppState, deployment: &Deployment, problems: &str) -> Result {
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let worker = worker_name(state, "en", deployment.employee()).await;
    let managers = notices::account_managers(
        state,
        PartyId(deployment.client_party_id),
        contract.account_manager_user_id,
    )
    .await?;
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 2] =
        [("worker", &worker), ("problems", &problems)];
    notices::tell(
        state,
        deployment.subject(),
        &contract.reference,
        &managers,
        &english("manpower.deployment.resume_failing_title", &params),
        &english("manpower.deployment.resume_failing_body", &params),
    )
    .await?;
    Ok(())
}

/// One `MPW.RUL.12` run for suspended deployment `id` and the accepted
/// document `document` of `kind` (valid to `expires_on`, none: no expiry).
pub(crate) async fn run_accepted(
    state: &AppState,
    id: i64,
    kind: &str,
    document: i64,
    expires_on: Option<NaiveDate>,
) -> Result<()> {
    let expiry = expires_on.map_or_else(|| "-".to_owned(), |date| date.to_string());
    let spec = RuleRunSpec::new(
        RULE,
        Subject::new(SUBJECT_TYPE, id),
        format!("{RULE}:resume:{kind}:{document}:{expiry}"),
    )
    .compliance_critical();
    let kind = kind.to_owned();
    run_rule(state, spec, move |state: AppState| async move {
        accepted(&state, id, &kind, expires_on).await
    })
    .await?;
    Ok(())
}

/// The action of a run: clear the reason of the accepted kind, then resume
/// when nothing else holds the deployment.
async fn accepted(state: &AppState, id: i64, kind: &str, expires_on: Option<NaiveDate>) -> Result {
    let Some(deployment) = Deployment::find(&state.db, id).await? else {
        return Ok(());
    };
    if deployment.status != DeploymentStatus::Suspended {
        return Ok(());
    }
    let on = today();
    // The reason of the renewed kind goes when the new document runs longer.
    for reason in DeploymentSuspension::open_for(&state.db, id).await? {
        let renewed = reason.kind == SuspensionKind::Document
            && reason.reason == kind
            && match (reason.expired_on, expires_on) {
                (_, None) => true,
                (Some(old), Some(new)) => new > old,
                (None, Some(_)) => true,
            };
        if renewed {
            let mut cleared = reason;
            cleared.cleared_on = Some(on);
            cleared.save_only(&state.db, &["cleared_on"]).await?;
        }
    }
    // Another reason (a kind still expired, a manual one) keeps it suspended.
    if !DeploymentSuspension::open_for(&state.db, id)
        .await?
        .is_empty()
    {
        return Ok(());
    }
    if !problems(state, &deployment, on).await?.is_empty() {
        // The end date has passed or the contract has ended: the daily run
        // ends it.
        return Ok(());
    }
    let check_on = guard::check_date(on, deployment.end_date);
    match wl_kernel::documents::guard(state)
        .check(state, RESUME_GUARDED, id, check_on)
        .await?
    {
        Ok(_checked) => {
            run(state, None, &deployment, on, "en").await?;
            Ok(())
        }
        Err(refusal) => tell_still_failing(state, &deployment, &refusal.message()).await,
    }
}

/// `DocumentAccepted` for a worker: tries every suspended deployment of the
/// worker.
async fn on_accepted(state: &AppState, event: &DocumentAccepted) -> Result {
    if event.subject.subject_type != EMPLOYEE_SUBJECT_TYPE {
        return Ok(());
    }
    let suspended = Deployment::query()
        .where_(Deployment::EMPLOYEE_ID.eq(event.subject.subject_id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Suspended))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let mut failure = None;
    for deployment in suspended {
        if let Err(error) = run_accepted(
            state,
            deployment.id,
            &event.kind,
            event.document.0,
            event.expires_on,
        )
        .await
        {
            eprintln!("{RULE}: deployment {}: {error:?}", deployment.id);
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

/// Listens for the acceptance of a renewed document.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        on_accepted(&state, &event).await
    });
}
