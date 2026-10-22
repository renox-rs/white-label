//! The record page of a manpower request (`GET /manpower/requests/{id}`,
//! BL-MPW-23, `docs/UX.md` §2.2) and what the requests list needs for its
//! *New request* sheet.
//!
//! The page shows the request with its status, how many positions are filled
//! and how many are missing, the deployments against it, the cause of a
//! replacement or cover request in words, and the actions *Submit*, *Cancel*
//! and *Plan a deployment* for the people who may run them: a button blocked by
//! a guard is drawn disabled with the guard in words.
//!
//! Decisions where the spec is silent:
//!
//! - **Filled** counts the `planned` and `active` deployments of the request
//!   (the Terms of the issue); a `suspended` one counts as missing and is
//!   said (*1 suspended*). The list keeps the request's own `filled_count`.
//! - *Plan a deployment* is a sheet of this page that posts to the existing
//!   `POST /manpower/deployments` with the request, the contract, the site, the
//!   role and the shift set; the full deployment screens come with BL-MPW-24.
//! - The status `open` is worded *Submitted* on the page, as the issue says.

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::documents::today;
use wl_kernel::workforce::workforce;

use super::{Act, Texts, date_text, party_text, period_text, role_text};
use crate::access;
use crate::contract::{ClientContract, ContractStatus};
use crate::deployment::{Deployment, DeploymentStatus};
use crate::deployments::worker_name;
use crate::permissions;
use crate::request::{ManpowerRequest, RequestKind, RequestStatus};
use crate::site::Site;
use crate::texts::{screen_texts, t, tf, word};

/// The address of a request's page.
pub(crate) fn url(id: i64) -> String {
    format!("/manpower/requests/{id}")
}

/// The badge tone of a request status.
pub(crate) fn status_kind(status: RequestStatus) -> &'static str {
    match status {
        RequestStatus::Filled => "success",
        RequestStatus::Open | RequestStatus::PartiallyFilled => "warning",
        RequestStatus::Draft | RequestStatus::Cancelled => "info",
    }
}

/// The word of a request status: `open` is *Submitted*.
pub(crate) fn status_text(lang: &Lang, status: RequestStatus) -> String {
    word(lang, "request_status", status.as_str())
}

/// A deployment against the request.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct DeploymentLine {
    url: String,
    worker: String,
    period: String,
    status: String,
    status_kind: &'static str,
}

/// What the person may do with the request now.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct RequestActions {
    submit: Act,
    cancel: Act,
    plan: Act,
}

/// A fact of the overview.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct Fact {
    label: String,
    value: String,
}

/// The replacement or cover request's cause in words.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct CauseView {
    text: String,
    note: String,
    /// The page of the deployment it stands in for; empty when unknown.
    url: String,
}

/// What the request page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/request.html")]
pub(crate) struct RequestPage {
    tx: Texts,
    id: i64,
    title: String,
    subtitle: String,
    status: String,
    status_key: String,
    status_kind: &'static str,
    contract_url: String,
    contract: String,
    contract_id: i64,
    site_id: i64,
    role: String,
    shift_type: String,
    shift_start: String,
    shift_end: String,
    positions: String,
    missing: i64,
    facts: Vec<Fact>,
    cause: Option<CauseView>,
    deployments: Vec<DeploymentLine>,
    actions: RequestActions,
    /// The sheet that plans a deployment against the request.
    workers: Vec<(String, String)>,
    worker_selected: String,
    plan_start: String,
    plan_end: String,
}

/// What the requests list needs to offer *New request*.
#[derive(Debug, Serialize, Clone, Default)]
pub(crate) struct NewRequestForm {
    pub(crate) contracts: Vec<(String, String)>,
    pub(crate) contract_selected: String,
    pub(crate) sites: Vec<(String, String)>,
    pub(crate) site_selected: String,
    pub(crate) roles: Vec<(String, String)>,
    pub(crate) role_selected: String,
    pub(crate) shifts: Vec<(String, String)>,
    pub(crate) shift_selected: String,
    pub(crate) today: String,
}

/// The roles a request can ask for.
const ROLES: [&str; 6] = [
    "worker",
    "security_guard",
    "security_supervisor",
    "cleaner",
    "driver",
    "labourer",
];

/// The key of a job role written as words: `Security guard` is
/// `security_guard`; a key stays as it is.
pub(crate) fn role_key(role: &str) -> String {
    role.trim().to_lowercase().replace([' ', '-'], "_")
}

/// The shifts the rate cards price.
const SHIFTS: [&str; 2] = ["day", "night"];

/// The options of the *New request* sheet for `user`: the contracts and sites
/// of the client companies the person writes requests for.
pub(crate) async fn new_request_form(
    state: &AppState,
    user: &AuthUser,
    lang: &Lang,
) -> Result<NewRequestForm> {
    let mut form = NewRequestForm {
        today: today().to_string(),
        ..NewRequestForm::default()
    };
    let mut clients = Vec::new();
    for contract in ClientContract::query()
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?
    {
        if matches!(
            contract.status,
            ContractStatus::Ended | ContractStatus::Cancelled
        ) || !user.has_permission_in(permissions::REQUESTS_CREATE, &contract.scope())
        {
            continue;
        }
        let client = party_text(state, lang, Some(contract.client_party_id), "").await;
        form.contracts.push((
            contract.id.to_string(),
            tf(
                lang,
                "manpower.screen.requests.contract_option",
                &[("reference", &contract.reference), ("client", &client)],
            ),
        ));
        if !clients.contains(&contract.client_party_id) {
            clients.push(contract.client_party_id);
        }
    }
    for client in clients {
        let name = party_text(state, lang, Some(client), "").await;
        for site in Site::query()
            .where_(Site::CLIENT_PARTY_ID.eq(client))
            .order_by(Site::ID)
            .get(&state.db)
            .await?
        {
            if site.active {
                form.sites
                    .push((site.id.to_string(), format!("{}, {name}", site.name)));
            }
        }
    }
    form.contract_selected = form
        .contracts
        .first()
        .map(|(value, _)| value.clone())
        .unwrap_or_default();
    form.site_selected = form
        .sites
        .first()
        .map(|(value, _)| value.clone())
        .unwrap_or_default();
    // The option sends the words (`Security guard`); the route turns them into
    // the key ([`role_key`]).
    form.roles = ROLES
        .iter()
        .map(|role| (role_text(role), role_text(role)))
        .collect();
    form.role_selected = role_text(ROLES[1]);
    form.shifts = super::options(lang, "shift", &SHIFTS);
    form.shift_selected = SHIFTS[0].to_owned();
    Ok(form)
}

/// How many deployments of request `id` are `active`: workers on site.
pub(crate) async fn active_count(state: &AppState, id: i64) -> Result<usize> {
    Ok(Deployment::query()
        .where_(Deployment::REQUEST_ID.eq(id))
        .where_(Deployment::STATUS.eq(DeploymentStatus::Active))
        .get(&state.db)
        .await?
        .len())
}

/// `1 deployment is still active`, `2 deployments are still active`.
pub(crate) fn active_text(lang: &Lang, active: usize) -> String {
    let key = if active == 1 {
        "manpower.screen.requests.active_one"
    } else {
        "manpower.screen.requests.active_many"
    };
    tf(lang, key, &[("count", &active)])
}

/// `GET /manpower/requests/{id}`: the page for a browser, the same data as
/// JSON otherwise.
pub(crate) async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response> {
    let request = ManpowerRequest::find_or_404(&state.db, id).await?;
    let see = access::may_see(
        &state,
        &user,
        request.client(),
        &[
            permissions::REQUESTS_VIEW,
            permissions::REQUESTS_CREATE,
            permissions::REQUESTS_MANAGE,
        ],
    )
    .await?;
    if !see {
        return Err(Error::Forbidden);
    }
    let page = build(&state, &user, &lang, &request).await?;
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
    request: &ManpowerRequest,
) -> Result<RequestPage> {
    let not_set = t(lang, "manpower.screen.not_set");
    let contract = ClientContract::find_or_404(&state.db, request.client_contract_id).await?;
    let site = Site::find(&state.db, request.site_id)
        .await?
        .map(|site| site.name)
        .unwrap_or_else(|| not_set.clone());
    let client = party_text(state, lang, Some(request.client_party_id), &not_set).await;
    let status = request.status;

    // The deployments against the request, and the positions they fill.
    let against = Deployment::query()
        .where_(Deployment::REQUEST_ID.eq(request.id))
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?;
    let count = |wanted: &[DeploymentStatus]| {
        against
            .iter()
            .filter(|deployment| wanted.contains(&deployment.status))
            .count()
    };
    let active = count(&[DeploymentStatus::Active]);
    let suspended = count(&[DeploymentStatus::Suspended]);
    let filled = i64::try_from(count(&[
        DeploymentStatus::Planned,
        DeploymentStatus::Active,
    ]))
    .unwrap_or(0)
    .min(request.headcount);
    let missing = (request.headcount - filled).max(0);
    let mut positions = tf(
        lang,
        "manpower.screen.requests.positions",
        &[
            ("filled", &filled),
            ("headcount", &request.headcount),
            ("missing", &missing),
        ],
    );
    if suspended > 0 {
        positions = format!(
            "{positions} ({})",
            tf(
                lang,
                "manpower.screen.requests.suspended",
                &[("count", &suspended)]
            )
        );
    }

    let mut lines = Vec::new();
    for deployment in &against {
        lines.push(DeploymentLine {
            url: format!("/manpower/deployments/{}", deployment.id),
            worker: worker_name(state, &lang.locale, deployment.employee()).await,
            period: period_text(lang, deployment.start_date, deployment.end_date),
            status: word(lang, "status", deployment.status.as_str()),
            status_kind: match deployment.status {
                DeploymentStatus::Active => "success",
                DeploymentStatus::Suspended => "warning",
                _ => "info",
            },
        });
    }

    // The replacement or cover request's cause.
    let cause = match request.replaces_deployment_id {
        Some(replaced) if request.kind == RequestKind::Replacement => {
            let former = Deployment::find(&state.db, replaced).await?;
            let worker = match &former {
                Some(deployment) => worker_name(state, &lang.locale, deployment.employee()).await,
                None => t(lang, "manpower.screen.requests.an_absent_worker"),
            };
            let cause = request.cause.clone().unwrap_or_default();
            let ended = former
                .as_ref()
                .and_then(|deployment| deployment.ended_on)
                .map(date_text)
                .unwrap_or_else(|| not_set.clone());
            let text = match cause.as_str() {
                "rejection" => tf(
                    lang,
                    "manpower.screen.requests.cause_rejection",
                    &[("worker", &worker), ("date", &ended)],
                ),
                "leave" | "suspension" | "absence" => tf(
                    lang,
                    &format!("manpower.screen.requests.cause_{cause}"),
                    &[("worker", &worker)],
                ),
                _ => tf(
                    lang,
                    "manpower.screen.requests.cause_manual",
                    &[("worker", &worker)],
                ),
            };
            Some(CauseView {
                text,
                note: former
                    .as_ref()
                    .and_then(|deployment| deployment.end_note.clone())
                    .filter(|note| !note.trim().is_empty())
                    .map(|note| {
                        tf(
                            lang,
                            "manpower.screen.requests.client_note",
                            &[("note", &note)],
                        )
                    })
                    .unwrap_or_default(),
                url: former
                    .map(|deployment| format!("/manpower/deployments/{}", deployment.id))
                    .unwrap_or_default(),
            })
        }
        _ => None,
    };

    // The facts.
    let mut facts = vec![
        Fact {
            label: t(lang, "manpower.screen.requests.site"),
            value: site.clone(),
        },
        Fact {
            label: t(lang, "manpower.screen.requests.shift"),
            value: shift_text(lang, request),
        },
        Fact {
            label: t(lang, "manpower.screen.requests.period"),
            value: period_text(lang, request.start_date, request.end_date),
        },
        Fact {
            label: t(lang, "manpower.screen.requests.workers"),
            value: positions.clone(),
        },
    ];
    if let Some(at) = request.submitted_at {
        facts.push(Fact {
            label: t(lang, "manpower.screen.requests.submitted"),
            value: date_text(wl_kernel::documents::dubai_date(at)),
        });
    }
    if let Some(reason) = request
        .cancel_reason
        .as_ref()
        .filter(|reason| !reason.trim().is_empty())
    {
        facts.push(Fact {
            label: t(lang, "manpower.screen.requests.cancel_reason"),
            value: reason.clone(),
        });
    }

    // The actions.
    let scope = request.scope();
    let may_create = user.has_permission_in(permissions::REQUESTS_CREATE, &scope);
    let may_manage = user.has_permission_in(permissions::REQUESTS_MANAGE, &scope);
    let may_plan = user.has_permission_in(permissions::DEPLOYMENTS_MANAGE, &scope);
    let submit = if may_create && status == RequestStatus::Draft {
        match contract.status {
            ContractStatus::Active => Act::ready(),
            ContractStatus::OnHold => {
                Act::blocked(t(lang, "manpower.screen.requests.on_hold_blocked"))
            }
            _ => Act::blocked(t(lang, "manpower.screen.requests.contract_blocked")),
        }
    } else {
        Act::hidden()
    };
    let cancelable = matches!(
        status,
        RequestStatus::Draft | RequestStatus::Open | RequestStatus::PartiallyFilled
    );
    let cancel = if may_manage && cancelable {
        Act::unless((active > 0).then(|| active_text(lang, active)))
    } else {
        Act::hidden()
    };
    let plan = if may_plan && !matches!(status, RequestStatus::Filled | RequestStatus::Cancelled) {
        if status == RequestStatus::Draft {
            Act::blocked(t(lang, "manpower.screen.requests.plan_blocked"))
        } else {
            Act::ready()
        }
    } else {
        Act::hidden()
    };

    // The workers the plan sheet offers: the staff of the request's role.
    let mut workers = Vec::new();
    if plan.shown && plan.blocked.is_empty() {
        let staff = workforce(state);
        for employee in staff.all_staff(&state.db).await? {
            let Some(profile) = staff.staff(state, employee).await? else {
                continue;
            };
            if profile.role.eq_ignore_ascii_case(&request.role) {
                workers.push((
                    employee.0.to_string(),
                    worker_name(state, &lang.locale, employee).await,
                ));
            }
        }
    }
    let worker_selected = workers
        .first()
        .map(|(value, _)| value.clone())
        .unwrap_or_default();

    Ok(RequestPage {
        tx: screen_texts(lang),
        id: request.id,
        title: format!("{}, {site}", role_text(&request.role)),
        subtitle: format!("{client}, {}", contract.reference),
        status: status_text(lang, status),
        status_key: status.as_str().to_owned(),
        status_kind: status_kind(status),
        contract_url: super::contract_page::url(contract.id),
        contract: contract.reference.clone(),
        contract_id: contract.id,
        site_id: request.site_id,
        role: request.role.clone(),
        shift_type: request.shift_type.clone(),
        shift_start: request.shift_start.clone().unwrap_or_default(),
        shift_end: request.shift_end.clone().unwrap_or_default(),
        positions,
        missing,
        facts,
        cause,
        deployments: lines,
        actions: RequestActions {
            submit,
            cancel,
            plan,
        },
        workers,
        worker_selected,
        plan_start: request.start_date.max(today()).to_string(),
        plan_end: request
            .end_date
            .map(|end| end.to_string())
            .unwrap_or_default(),
    })
}

/// The shift in words: *Night shift, 19:00 to 07:00*.
fn shift_text(lang: &Lang, request: &ManpowerRequest) -> String {
    let name = word(lang, "shift", &request.shift_type);
    match (&request.shift_start, &request.shift_end) {
        (Some(start), Some(end)) if !start.is_empty() && !end.is_empty() => tf(
            lang,
            "manpower.screen.requests.shift_hours",
            &[("shift", &name), ("start", start), ("end", end)],
        ),
        _ => name,
    }
}
