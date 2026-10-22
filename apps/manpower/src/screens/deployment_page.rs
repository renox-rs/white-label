//! The record page of a deployment (`GET /manpower/deployments/{id}`,
//! BL-MPW-24, `docs/UX.md` §2.2) and the options its sheets need.
//!
//! The page shows the worker, the client, the site, the role, the dates, the
//! status in words, the cover, the days suspended, the documents the worker
//! must hold with their validity, the checks that stop *Activate* now, the
//! timesheets of the deployment and its activity, and the actions *Activate*,
//! *Extend*, *Suspend*, *Resume*, *End*, *Cancel* and *Ask for cover* for the
//! people who may run them. The site supervisor sees the page read-only.
//!
//! Decisions where the spec is silent:
//!
//! - An action is offered only in the states the machine allows it from;
//!   *Activate* is never drawn disabled, because pressing it lists every
//!   failing check, and the page already lists them on a planned deployment.
//! - *Client non-payment* is offered to the MD (and the company admin who
//!   stands in for the role), like the service decides.
//! - A deployment ended *entered in error* reads *Voided* with the reason;
//!   its request count falls back because the request page counts only the
//!   planned and active deployments.
//! - **A refusal is the service's** (one line per failing check, the guard's
//!   own words for the documents, which existing tests pin); the page card
//!   *Why it cannot start yet* says the same checks with the worker and the
//!   date in words and links *Request override* where one is possible.
//! - Each timesheet links to its page (`/timesheets/{id}`, BL-MPW-27) for the
//!   people who may see attendance.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::Subject;
use wl_kernel::documents::{BlockMode, guard, today};
use wl_kernel::timesheets::timesheets;
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, workforce};

use super::{
    Act, Texts, date_or, date_text, kind_label, options, party_text, period_text, role_text,
    transition_text, user_text,
};
use crate::contract::ClientContract;
use crate::deployment::{Deployment, DeploymentKind, DeploymentStatus, reasons};
use crate::deployments::{self, worker_name};
use crate::documents::ACTIVATE_GUARDED;
use crate::documents::DeploymentDocumentFlag;
use crate::guard as document_guard;
use crate::permissions;
use crate::request::{ManpowerRequest, RequestKind};
use crate::screens::request_page;
use crate::site::Site;
use crate::suspension::DeploymentSuspension;
use crate::texts::{screen_texts, t, tf, word};

/// The address of a deployment's page.
pub(crate) fn url(id: i64) -> String {
    format!("/manpower/deployments/{id}")
}

/// The badge tone of a deployment status.
pub(crate) fn status_kind(status: DeploymentStatus) -> &'static str {
    match status {
        DeploymentStatus::Active => "success",
        DeploymentStatus::Suspended => "warning",
        DeploymentStatus::Planned | DeploymentStatus::Ended | DeploymentStatus::Cancelled => "info",
    }
}

/// The status as the page says it: an ended deployment entered in error is
/// *Voided*.
pub(crate) fn status_text(lang: &Lang, deployment: &Deployment) -> String {
    if deployment.status == DeploymentStatus::Ended
        && deployment.end_reason.as_deref() == Some(reasons::ENTERED_IN_ERROR)
    {
        t(lang, "manpower.screen.status.voided")
    } else {
        word(lang, "status", deployment.status.as_str())
    }
}

/// *1 day suspended*, *3 days suspended*.
pub(crate) fn suspended_days_text(lang: &Lang, days: i64) -> String {
    let key = if days == 1 {
        "manpower.screen.deployment.days_suspended_one"
    } else {
        "manpower.screen.deployment.days_suspended"
    };
    tf(lang, key, &[("days", &days)])
}

/// A fact of the overview.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Fact {
    label: String,
    value: String,
}

fn fact(label: String, value: String) -> Fact {
    Fact { label, value }
}

/// A check that stops *Activate* now, or a document that fails.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CheckLine {
    pub(crate) text: String,
    /// The page that asks for an override; empty when none is possible.
    override_url: String,
}

/// A document the worker must hold, with its validity.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DocumentLine {
    kind: String,
    validity: String,
    status: String,
    status_kind: &'static str,
}

/// A timesheet of the deployment.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SheetLine {
    /// The page of the timesheet, empty when the person may not open it.
    url: String,
    period: String,
    kind: String,
    status: String,
}

/// A line of the activity.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ActivityLine {
    at: String,
    what: String,
    by: String,
    reason: String,
}

/// A cover request or deployment tied to this one.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CoverLine {
    text: String,
    url: String,
}

/// How the deployment ended.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct EndView {
    text: String,
    note: String,
    waiver: String,
    /// The replacement request opened by a client rejection.
    replacement_url: String,
}

/// What the person may do with the deployment now.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeploymentActions {
    activate: Act,
    extend: Act,
    suspend: Act,
    resume: Act,
    end: Act,
    cancel: Act,
    cover: Act,
}

/// What the deployment page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/deployment.html")]
pub(crate) struct DeploymentPage {
    tx: Texts,
    id: i64,
    title: String,
    subtitle: String,
    status: String,
    status_key: String,
    status_kind: &'static str,
    contract_url: String,
    contract: String,
    facts: Vec<Fact>,
    alerts: Vec<String>,
    end: Option<EndView>,
    covers: Vec<CoverLine>,
    checks: Vec<CheckLine>,
    documents: Vec<DocumentLine>,
    timesheets: Vec<SheetLine>,
    activity: Vec<ActivityLine>,
    actions: DeploymentActions,
    end_reasons: Vec<(String, String)>,
    end_selected: String,
    suspend_selected: String,
    today: String,
    new_end: String,
}

/// The reasons the person may end a deployment with.
fn end_reasons(lang: &Lang, md: bool) -> Vec<(String, String)> {
    let mut values = vec![
        reasons::END_DATE,
        reasons::REPLACED,
        reasons::CLIENT_REJECTED,
    ];
    if md {
        values.push(reasons::CLIENT_NON_PAYMENT);
    }
    values.push(reasons::ENTERED_IN_ERROR);
    options(lang, "end_reason", &values)
}

/// Whether `user` is the MD: the role, else the company admin.
pub(crate) fn is_md(user: &AuthUser) -> bool {
    user.has_role(permissions::MD_ROLE) || user.has_role(wl_kernel::COMPANY_ADMIN_ROLE)
}

/// `GET /manpower/deployments/{id}`: the page for a browser, the same data as
/// JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let deployment = Deployment::find_or_404(&state.db, id).await?;
    let may_see = [
        permissions::DEPLOYMENTS_VIEW,
        permissions::DEPLOYMENTS_MANAGE,
    ]
    .iter()
    .any(|permission| {
        user.has_permission_in(permission, &deployment.scope())
            || user.has_permission_in(permission, &deployment.site_scope())
    });
    if !may_see {
        return Err(Error::Forbidden);
    }
    let page = build(&state, &user, &lang, &deployment).await?;
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

async fn build(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
    deployment: &Deployment,
) -> Result<DeploymentPage> {
    let not_set = t(lang, "manpower.screen.not_set");
    let status = deployment.status;
    let on = today();
    let contract = ClientContract::find_or_404(&state.db, deployment.client_contract_id).await?;
    let site = Site::find(&state.db, deployment.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_else(|| not_set.clone());
    let client = party_text(state, lang, Some(deployment.client_party_id), &not_set).await;
    let worker = worker_name(state, &lang.locale, deployment.employee()).await;
    let role = role_text(&deployment.role);
    let manage = user.has_permission_in(permissions::DEPLOYMENTS_MANAGE, &deployment.scope());

    // The facts.
    let mut facts = vec![
        fact(t(lang, "manpower.screen.deployment.worker"), worker.clone()),
        fact(t(lang, "manpower.screen.deployment.client"), client.clone()),
        fact(t(lang, "manpower.screen.deployments.site"), site.clone()),
        fact(t(lang, "manpower.screen.deployments.role"), role.clone()),
        fact(
            t(lang, "manpower.screen.deployments.shift"),
            shift_text(lang, deployment),
        ),
        fact(
            t(lang, "manpower.screen.deployments.period"),
            period_text(lang, deployment.start_date, deployment.end_date),
        ),
    ];
    if deployment.kind == DeploymentKind::Cover {
        facts.push(fact(
            t(lang, "manpower.screen.deployments.cover"),
            t(lang, "manpower.screen.deployment.is_cover"),
        ));
    }
    if let Some(on) = deployment.activated_on {
        facts.push(fact(
            t(lang, "manpower.screen.deployment.activated"),
            date_text(on),
        ));
    }
    let days = deployment.days_suspended(on);
    if days > 0 {
        facts.push(fact(
            t(lang, "manpower.screen.deployment.suspension"),
            suspended_days_text(lang, days),
        ));
    }
    if let Some(reason) = deployment
        .cancel_reason
        .as_ref()
        .filter(|reason| !reason.trim().is_empty())
    {
        facts.push(fact(
            t(lang, "manpower.screen.deployment.cancel_reason"),
            reason.clone(),
        ));
    }

    // What to know at a glance.
    let mut alerts = Vec::new();
    if status == DeploymentStatus::Suspended {
        let reasons: Vec<String> = DeploymentSuspension::open_for(&state.db, deployment.id)
            .await?
            .iter()
            .map(DeploymentSuspension::text)
            .collect();
        alerts.push(tf(
            lang,
            "manpower.screen.deployment.suspended_because",
            &[
                ("reasons", &reasons.join(", ")),
                ("days", &suspended_days_text(lang, days)),
            ],
        ));
    }
    for flag in DeploymentDocumentFlag::open_for(&state.db, deployment.id).await? {
        alerts.push(flag.text());
    }
    if deployment.is_on_site()
        && deployment.planned_end_reason.as_deref() == Some(reasons::CLIENT_NON_PAYMENT)
    {
        alerts.push(tf(
            lang,
            "manpower.screen.deployment.withdrawal_planned",
            &[("date", &date_or(deployment.end_date, &not_set))],
        ));
    }

    // How it ended.
    let end = match (status, deployment.end_reason.as_deref()) {
        (DeploymentStatus::Ended, Some(reason)) => {
            let replacement = ManpowerRequest::query()
                .where_(ManpowerRequest::REPLACES_DEPLOYMENT_ID.eq(deployment.id))
                .where_(ManpowerRequest::KIND.eq(RequestKind::Replacement))
                .order_by(ManpowerRequest::ID)
                .first(&state.db)
                .await?;
            Some(EndView {
                text: tf(
                    lang,
                    "manpower.screen.deployment.ended_because",
                    &[
                        ("date", &date_or(deployment.ended_on, &not_set)),
                        ("reason", &word(lang, "end_reason", reason)),
                    ],
                ),
                note: deployment
                    .end_note
                    .clone()
                    .filter(|note| !note.trim().is_empty())
                    .map(|note| {
                        tf(
                            lang,
                            "manpower.screen.deployment.client_note",
                            &[("note", &note)],
                        )
                    })
                    .unwrap_or_default(),
                waiver: deployment
                    .end_waiver
                    .clone()
                    .filter(|waiver| !waiver.trim().is_empty())
                    .map(|waiver| {
                        tf(
                            lang,
                            "manpower.screen.deployment.waiver_note",
                            &[("waiver", &waiver)],
                        )
                    })
                    .unwrap_or_default(),
                replacement_url: replacement
                    .map(|request| request_page::url(request.id))
                    .unwrap_or_default(),
            })
        }
        _ => None,
    };

    // The cover: the deployment it stands in for, and the requests and
    // deployments that stand in for this one.
    let mut covers = Vec::new();
    if let Some(covered) = deployment.covers_deployment_id
        && let Some(other) = Deployment::find(&state.db, covered).await?
    {
        covers.push(CoverLine {
            text: tf(
                lang,
                "manpower.screen.deployment.covers",
                &[(
                    "worker",
                    &worker_name(state, &lang.locale, other.employee()).await,
                )],
            ),
            url: url(other.id),
        });
    }
    for request in ManpowerRequest::query()
        .where_(ManpowerRequest::REPLACES_DEPLOYMENT_ID.eq(deployment.id))
        .order_by(ManpowerRequest::ID)
        .get(&state.db)
        .await?
    {
        covers.push(CoverLine {
            text: tf(
                lang,
                "manpower.screen.deployment.cover_request",
                &[
                    (
                        "period",
                        &period_text(lang, request.start_date, request.end_date),
                    ),
                    ("status", &request_page::status_text(lang, request.status)),
                ],
            ),
            url: request_page::url(request.id),
        });
    }

    // The documents the worker must hold, judged on the day *Activate* would.
    let check_on = document_guard::check_date(on, deployment.end_date);
    let mut documents = Vec::new();
    let entries = guard(state).blocks_for(&state.db, ACTIVATE_GUARDED).await?;
    for entry in entries
        .iter()
        .filter(|entry| entry.mode == BlockMode::Block && entry.applies_to(Some(&deployment.role)))
    {
        let subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, deployment.employee_id);
        let validity = guard(state)
            .is_valid(&state.db, &subject, &entry.kind, check_on)
            .await?;
        let (text, label, kind) = match validity {
            wl_kernel::documents::Validity::Valid { expires_on, .. } => (
                expires_on.map_or_else(
                    || t(lang, "manpower.screen.deployment.doc_no_expiry"),
                    |date| {
                        tf(
                            lang,
                            "manpower.screen.deployment.doc_valid_until",
                            &[("date", &date_text(date))],
                        )
                    },
                ),
                word(lang, "doc_state", "valid"),
                "success",
            ),
            wl_kernel::documents::Validity::Invalid {
                reason, expired_on, ..
            } => (
                match expired_on {
                    Some(date) => tf(
                        lang,
                        "manpower.screen.deployment.doc_expired_on",
                        &[("date", &date_text(date))],
                    ),
                    None => word(lang, "doc_state", reason_key(reason)),
                },
                word(lang, "doc_state", reason_key(reason)),
                "error",
            ),
        };
        documents.push(DocumentLine {
            kind: kind_label(&entry.kind),
            validity: text,
            status: label,
            status_kind: kind,
        });
    }

    // The checks that stop *Activate* now, every one.
    let mut checks = Vec::new();
    if status == DeploymentStatus::Planned && manage {
        checks = activation_lines(state, user, deployment, &worker).await?;
    }

    // The timesheets of the deployment.
    let mut sheets = Vec::new();
    for sheet in timesheets(state)
        .of_context(state, &deployment.subject())
        .await?
    {
        sheets.push(SheetLine {
            url: if user.has_permission_anywhere("attendance.view") {
                format!("/timesheets/{}", sheet.subject.subject_id)
            } else {
                String::new()
            },
            period: format!("{} - {}", date_text(sheet.from), date_text(sheet.to)),
            kind: word(lang, "sheet_kind", &sheet.kind),
            status: word(lang, "sheet_status", &sheet.status),
        });
    }

    // The activity.
    let mut activity = Vec::new();
    for row in renox::state_machine::history(&state.db, Deployment::TABLE, deployment.id).await? {
        activity.push(ActivityLine {
            at: row
                .at
                .with_timezone(&wl_kernel::BusinessCalendar::offset())
                .format("%d %b %Y %H:%M")
                .to_string(),
            what: transition_text(lang, &row.transition),
            by: match row.actor_id {
                Some(id) => user_text(state, Some(id), &not_set).await,
                None => t(lang, "manpower.screen.activity.system"),
            },
            reason: row.reason.unwrap_or_default(),
        });
    }

    // The actions.
    let may_cover = user.has_permission_in(permissions::REQUESTS_MANAGE, &deployment.scope());
    let on_site = deployment.is_on_site();
    let actions = DeploymentActions {
        activate: Act::when(manage && status == DeploymentStatus::Planned),
        extend: Act::when(manage && on_site && deployment.end_date.is_some()),
        suspend: Act::when(manage && status == DeploymentStatus::Active),
        resume: Act::when(manage && status == DeploymentStatus::Suspended),
        end: Act::when(manage && on_site),
        cancel: Act::when(manage && status == DeploymentStatus::Planned),
        cover: Act::when(may_cover && on_site),
    };
    let can_end_for_payment = is_md(user);

    Ok(DeploymentPage {
        tx: screen_texts(lang),
        id: deployment.id,
        title: worker.clone(),
        subtitle: format!("{role}, {site}, {client}"),
        status: status_text(lang, deployment),
        status_key: status.as_str().to_owned(),
        status_kind: status_kind(status),
        contract_url: super::contract_page::url(contract.id),
        contract: contract.reference.clone(),
        facts,
        alerts,
        end,
        covers,
        checks,
        documents,
        timesheets: sheets,
        activity,
        actions,
        end_reasons: end_reasons(lang, can_end_for_payment),
        end_selected: reasons::END_DATE.to_owned(),
        suspend_selected: String::new(),
        today: on.to_string(),
        new_end: deployment
            .end_date
            .map(|end| end.to_string())
            .unwrap_or_default(),
    })
}

fn reason_key(reason: wl_kernel::documents::InvalidReason) -> &'static str {
    match reason {
        wl_kernel::documents::InvalidReason::Expired => "expired",
        wl_kernel::documents::InvalidReason::Missing => "missing",
        wl_kernel::documents::InvalidReason::Rejected => "rejected",
        wl_kernel::documents::InvalidReason::PendingReview => "pending",
    }
}

/// Every check that stops *Activate* of `deployment` now, one line each in
/// words: the documents (with *Request override* where one is possible), the
/// hold, the overlap, the employee, the end date, the request and a blocked
/// client.
pub(crate) async fn activation_lines(
    state: &AppState,
    user: &AuthUser,
    deployment: &Deployment,
    worker: &str,
) -> Result<Vec<CheckLine>> {
    let on = today();
    let check_on = document_guard::check_date(on, deployment.end_date);
    let mut lines = Vec::new();
    for check in deployments::document_checks(
        state,
        user,
        ACTIVATE_GUARDED,
        deployment.id,
        check_on,
        worker,
    )
    .await?
    {
        lines.push(CheckLine {
            text: check.text,
            override_url: check.override_url.unwrap_or_default(),
        });
    }
    for problem in deployments::other_problems(state, user, deployment, on).await? {
        lines.push(CheckLine {
            text: problem,
            override_url: String::new(),
        });
    }
    Ok(lines)
}

/// The shift in words: *Night shift, 19:00 to 07:00*.
fn shift_text(lang: &Lang, deployment: &Deployment) -> String {
    let name = word(lang, "shift", &deployment.shift_type);
    match (&deployment.shift_start, &deployment.shift_end) {
        (Some(start), Some(end)) if !start.is_empty() && !end.is_empty() => tf(
            lang,
            "manpower.screen.requests.shift_hours",
            &[("shift", &name), ("start", start), ("end", end)],
        ),
        _ => name,
    }
}

/// The options of the *Plan a deployment* sheet of the list: the requests that
/// take workers and the staff.
#[derive(Debug, Serialize, Clone, Default)]
pub(crate) struct PlanForm {
    pub(crate) requests: Vec<(String, String)>,
    pub(crate) request_selected: String,
    pub(crate) workers: Vec<(String, String)>,
    pub(crate) worker_selected: String,
    pub(crate) today: String,
}

/// The plan sheet's options for `user`: requests of the clients they manage
/// deployments for that still take workers, and every listed worker.
pub(crate) async fn plan_form(state: &AppState, user: &AuthUser, lang: &Lang) -> Result<PlanForm> {
    let mut form = PlanForm {
        today: today().to_string(),
        ..PlanForm::default()
    };
    for request in ManpowerRequest::query()
        .order_by(ManpowerRequest::ID)
        .get(&state.db)
        .await?
    {
        if !request.status.takes_workers()
            || !user.has_permission_in(permissions::DEPLOYMENTS_MANAGE, &request.scope())
        {
            continue;
        }
        let client = party_text(state, lang, Some(request.client_party_id), "").await;
        let site = Site::find(&state.db, request.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        form.requests.push((
            request.id.to_string(),
            tf(
                lang,
                "manpower.screen.deployments.request_option",
                &[
                    ("role", &role_text(&request.role)),
                    ("site", &site),
                    ("client", &client),
                    (
                        "period",
                        &period_text(lang, request.start_date, request.end_date),
                    ),
                ],
            ),
        ));
    }
    if form.requests.is_empty() {
        return Ok(form);
    }
    let staff = workforce(state);
    for employee in staff.all_staff(&state.db).await? {
        form.workers.push((
            employee.0.to_string(),
            worker_name(state, &lang.locale, employee).await,
        ));
    }
    form.request_selected = form
        .requests
        .first()
        .map(|(value, _)| value.clone())
        .unwrap_or_default();
    form.worker_selected = form
        .workers
        .first()
        .map(|(value, _)| value.clone())
        .unwrap_or_default();
    Ok(form)
}
