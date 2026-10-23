//! The client approver's pages in the portal (BL-MPW-28, `MPW.ACL.01`,
//! `MPW.ACL.02`, `PTY.ACL.02`, `docs/UX.md` §2.10): *Requests*, *Workers on
//! site* and *Timesheets* of the companies the signed-in person acts for.
//!
//! | Page | Shows | The approver |
//! |---|---|---|
//! | `/portal/manpower/requests` | the company's contracts (a contract on credit hold reads *On hold*, no reason), its requests with filled and missing, and *New request* | *New request*, *Submit*, *Cancel request* |
//! | `/portal/manpower/deployments` | worker name, role, site, shift, dates, status; no photo (the agency keeps none), salary, document or medical datum | nothing to do |
//! | `/portal/manpower/timesheets` | the timesheets sent to the client, hours, status in words; the ones waiting for him first | opens `/timesheets/{id}` to *Approve* or *Reject* (the page of BL-MPW-27) |
//!
//! Decisions:
//!
//! - **Who.** A person who acts for no client company with a contract gets a
//!   403 (staff use the back-office pages). One who acts for a company sees only
//!   its rows; a request of another company answers 404, never 403 with data.
//!   Approver is the portal role `owner` or `approver`; a viewer sees the same
//!   pages and no button, and a direct POST is the service's 403.
//! - **A blocked client** sees no *New request* and the one neutral text of
//!   the portal ([`PortalUsers::neutral_text`](wl_kernel::portal::PortalUsers::neutral_text)),
//!   the same whatever the reason; a POST is refused with it.
//! - **The actions** call the services of [`crate::requests`] and land back on
//!   the portal page, not on the staff record page.
//! - **Words.** Status in words, the neutral on-hold text for a contract on
//!   hold, no raw key and no `None`; the same data answer JSON.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::block_guard::ActionKind;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use super::request_page::{self, NewRequestForm};
use super::{Act, Texts, done, party_text, period_text, role_text};
use crate::blocked;
use crate::contract::{ClientContract, ContractStatus};
use crate::db::refused;
use crate::deployments::worker_name;
use crate::pages::{field, new_request_from};
use crate::portal::{self, Acting};
use crate::request::{ManpowerRequest, RequestStatus};
use crate::requests;
use crate::site::Site;
use crate::texts::{screen_texts, t, tf, word};

/// The portal page of the requests.
pub(crate) const REQUESTS_URL: &str = "/portal/manpower/requests";

/// A contract of the client, as the client sees it.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ContractLine {
    reference: String,
    client: String,
    period: String,
    status: String,
    status_kind: &'static str,
}

/// A request of the client.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct RequestLine {
    id: i64,
    title: String,
    client: String,
    positions: String,
    period: String,
    status: String,
    status_kind: &'static str,
    submit: Act,
    cancel: Act,
}

/// What the requests page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/portal_requests.html")]
pub(crate) struct RequestsPage {
    tx: Texts,
    contracts: Vec<ContractLine>,
    requests: Vec<RequestLine>,
    can_create: bool,
    /// The neutral text of a blocked client; empty otherwise.
    neutral: String,
    form: NewRequestForm,
}

/// A worker on site.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct WorkerLine {
    worker: String,
    client: String,
    role: String,
    site: String,
    shift: String,
    period: String,
    status: String,
    status_kind: &'static str,
}

/// What the workers page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/portal_deployments.html")]
pub(crate) struct DeploymentsPage {
    tx: Texts,
    workers: Vec<WorkerLine>,
}

/// A timesheet of the client.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct SheetLine {
    url: String,
    worker: String,
    client: String,
    work: String,
    month: String,
    hours: String,
    status: String,
    status_kind: &'static str,
    waiting: bool,
    adjustment: bool,
}

/// What the timesheets page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/portal_timesheets.html")]
pub(crate) struct TimesheetsPage {
    tx: Texts,
    /// `2 timesheets waiting for you`; empty when none.
    waiting: String,
    sheets: Vec<SheetLine>,
}

/// The companies `user` acts for, or 403 when none.
async fn companies(state: &AppState, user: &AuthUser) -> Result<Vec<Acting>> {
    let acting = portal::acting(state, user.id).await?;
    if acting.is_empty() {
        return Err(Error::Forbidden);
    }
    Ok(acting)
}

/// 404 for a record of a company the person does not act for (403 for one who
/// acts for none); the company's [`Acting`] otherwise.
async fn own(state: &AppState, user: &AuthUser, client: PartyId) -> Result<Acting> {
    let acting = companies(state, user).await?;
    acting
        .into_iter()
        .find(|a| a.party == client)
        .ok_or(Error::NotFound)
}

/// Like [`own`], and the person must be the company's approver: a viewer only
/// sees (403).
async fn own_approver(state: &AppState, user: &AuthUser, client: PartyId) -> Result {
    if own(state, user, client).await?.approver {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Whether the client is blocked (the portal never says why).
async fn is_blocked(state: &AppState, client: PartyId) -> Result<bool> {
    match blocked::ensure(state, None, client, ActionKind::Start).await {
        Ok(()) => Ok(false),
        Err(Error::Status(status, _)) if status == StatusCode::UNPROCESSABLE_ENTITY => Ok(true),
        Err(other) => Err(other),
    }
}

/// The neutral text of a blocked client, for the person's language.
fn neutral(state: &AppState, lang: &Lang) -> String {
    portal_users(state).neutral_text(lang)
}

/// The page as a view for a browser, the same data as JSON otherwise.
fn answer<P: Serialize + renox::view::ViewData>(headers: &HeaderMap, page: P) -> Result<Response> {
    if wl_kernel::negotiate::wants_html(headers) {
        return Ok(View::typed(page).into_response());
    }
    let mut data = renox::serde_json::to_value(&page)?;
    if let Some(map) = data.as_object_mut() {
        map.remove("tx");
    }
    Ok(Json(data).into_response())
}

/// The badge tone of a contract status as the client sees it.
fn contract_kind(status: ContractStatus) -> &'static str {
    match status {
        ContractStatus::Active => "success",
        ContractStatus::OnHold | ContractStatus::Terminating => "warning",
        _ => "info",
    }
}

/// The options of the *New request* sheet: the contracts and sites of the
/// companies the person may order for and that are not blocked.
async fn form_for(state: &AppState, lang: &Lang, ordering: &[PartyId]) -> Result<NewRequestForm> {
    let mut form = NewRequestForm {
        today: today().to_string(),
        ..NewRequestForm::default()
    };
    for client in ordering {
        let name = party_text(state, lang, Some(client.0), "").await;
        for contract in ClientContract::query()
            .where_(ClientContract::CLIENT_PARTY_ID.eq(client.0))
            .order_by(ClientContract::ID)
            .get(&state.db)
            .await?
        {
            if matches!(
                contract.status,
                ContractStatus::Ended | ContractStatus::Cancelled | ContractStatus::Draft
            ) {
                continue;
            }
            form.contracts.push((
                contract.id.to_string(),
                tf(
                    lang,
                    "manpower.screen.requests.contract_option",
                    &[("reference", &contract.reference), ("client", &name)],
                ),
            ));
        }
        for site in Site::query()
            .where_(Site::CLIENT_PARTY_ID.eq(client.0))
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
        .map(|c| c.0.clone())
        .unwrap_or_default();
    form.site_selected = form.sites.first().map(|c| c.0.clone()).unwrap_or_default();
    form.roles = request_page::ROLES
        .iter()
        .map(|role| (role_text(role), role_text(role)))
        .collect();
    form.role_selected = role_text(request_page::ROLES[1]);
    form.shifts = super::options(lang, "shift", &request_page::SHIFTS);
    form.shift_selected = request_page::SHIFTS[0].to_owned();
    Ok(form)
}

/// `GET /portal/manpower/requests`.
pub(crate) async fn requests(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut contracts = Vec::new();
    let mut lines = Vec::new();
    let mut ordering = Vec::new();
    let mut blocked_text = String::new();
    for acting in companies(&state, &user).await? {
        let client = party_text(&state, &lang, Some(acting.party.0), "").await;
        let blocked_now = is_blocked(&state, acting.party).await?;
        if blocked_now {
            blocked_text = neutral(&state, &lang);
        }
        if acting.approver && !blocked_now {
            ordering.push(acting.party);
        }
        let mut on_hold = false;
        for contract in ClientContract::query()
            .where_(ClientContract::CLIENT_PARTY_ID.eq(acting.party.0))
            .order_by(ClientContract::ID)
            .get(&state.db)
            .await?
        {
            on_hold |= contract.status == ContractStatus::OnHold;
            contracts.push(ContractLine {
                reference: contract.reference.clone(),
                client: client.clone(),
                period: period_text(&lang, contract.start_date, Some(contract.end_date)),
                status: word(&lang, "status", contract.status.as_str()),
                status_kind: contract_kind(contract.status),
            });
        }
        for request in ManpowerRequest::query()
            .where_(ManpowerRequest::CLIENT_PARTY_ID.eq(acting.party.0))
            .order_by(ManpowerRequest::ID)
            .get(&state.db)
            .await?
        {
            let site = Site::find(&state.db, request.site_id)
                .await?
                .map(|site| site.name)
                .unwrap_or_default();
            let filled = request.filled_count.min(request.headcount);
            let contract = ClientContract::find(&state.db, request.client_contract_id).await?;
            let held = contract
                .as_ref()
                .is_some_and(|c| c.status != ContractStatus::Active);
            let submit = if !acting.approver || request.status != RequestStatus::Draft {
                Act::hidden()
            } else if blocked_now {
                Act::blocked(neutral(&state, &lang))
            } else if held || on_hold {
                Act::blocked(t(&lang, "manpower.screen.portal.requests.on_hold"))
            } else {
                Act::ready()
            };
            let cancel = Act::when(
                acting.approver
                    && matches!(
                        request.status,
                        RequestStatus::Draft | RequestStatus::Open | RequestStatus::PartiallyFilled
                    ),
            );
            lines.push(RequestLine {
                id: request.id,
                title: format!("{} - {site}", role_text(&request.role)),
                client: client.clone(),
                positions: tf(
                    &lang,
                    "manpower.screen.requests.positions",
                    &[
                        ("filled", &filled),
                        ("headcount", &request.headcount),
                        ("missing", &(request.headcount - filled).max(0)),
                    ],
                ),
                period: period_text(&lang, request.start_date, request.end_date),
                status: request_page::status_text(&lang, request.status),
                status_kind: request_page::status_kind(request.status),
                submit,
                cancel,
            });
        }
    }
    let form = form_for(&state, &lang, &ordering).await?;
    answer(
        &headers,
        RequestsPage {
            tx: screen_texts(&lang),
            contracts,
            requests: lines,
            can_create: !ordering.is_empty() && !form.contracts.is_empty(),
            neutral: blocked_text,
            form,
        },
    )
}

/// The client of the contract a form names, for the 404/403 rule.
async fn client_of_contract(
    state: &AppState,
    form: &BTreeMap<String, String>,
) -> Result<Option<PartyId>> {
    let Ok(id) = field(form, "client_contract_id").parse::<i64>() else {
        return Ok(None);
    };
    Ok(ClientContract::find(&state.db, id)
        .await?
        .map(|contract| contract.client()))
}

/// Refuses with the neutral text while the client is blocked.
async fn ensure_open(state: &AppState, lang: &Lang, client: PartyId) -> Result {
    if is_blocked(state, client).await? {
        return Err(refused(neutral(state, lang)));
    }
    Ok(())
}

/// `POST /portal/manpower/requests`: the same fields as the back office's
/// sheet.
pub(crate) async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    if let Some(client) = client_of_contract(&state, &form).await? {
        own_approver(&state, &user, client).await?;
        ensure_open(&state, &lang, client).await?;
    } else {
        companies(&state, &user).await?;
    }
    requests::create(&state, &user, new_request_from(&lang, &form)?).await?;
    Ok(done(
        &lang,
        REQUESTS_URL,
        "manpower.screen.done.request_created",
    ))
}

/// `POST /portal/manpower/requests/{id}/submit`.
pub(crate) async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let request = ManpowerRequest::find_or_404(&state.db, id).await?;
    own_approver(&state, &user, request.client()).await?;
    ensure_open(&state, &lang, request.client()).await?;
    requests::submit(&state, &user, id).await?;
    Ok(done(
        &lang,
        REQUESTS_URL,
        "manpower.screen.done.request_submitted",
    ))
}

/// `POST /portal/manpower/requests/{id}/cancel`: `reason`. A request with a
/// worker on site is not cancelled by the client: it says how many.
pub(crate) async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let request = ManpowerRequest::find_or_404(&state.db, id).await?;
    own_approver(&state, &user, request.client()).await?;
    requests::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        REQUESTS_URL,
        "manpower.screen.done.request_cancelled",
    ))
}

/// `GET /portal/manpower/deployments`.
pub(crate) async fn deployments(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut workers = Vec::new();
    for acting in companies(&state, &user).await? {
        let client = party_text(&state, &lang, Some(acting.party.0), "").await;
        for deployment in crate::deployment::Deployment::query()
            .where_(crate::deployment::Deployment::CLIENT_PARTY_ID.eq(acting.party.0))
            .order_by(crate::deployment::Deployment::ID)
            .get(&state.db)
            .await?
        {
            if !deployment.shown_to_client() {
                continue;
            }
            let site = Site::find(&state.db, deployment.site_id)
                .await?
                .map(|site| site.name)
                .unwrap_or_default();
            workers.push(WorkerLine {
                worker: worker_name(&state, &lang.locale, deployment.employee()).await,
                client: client.clone(),
                role: role_text(&deployment.role),
                site,
                shift: word(&lang, "shift", &deployment.shift_type),
                period: period_text(&lang, deployment.start_date, deployment.end_date),
                status: word(&lang, "status", deployment.status.as_str()),
                status_kind: match deployment.status {
                    crate::deployment::DeploymentStatus::Active => "success",
                    crate::deployment::DeploymentStatus::Suspended => "warning",
                    _ => "info",
                },
            });
        }
    }
    answer(
        &headers,
        DeploymentsPage {
            tx: screen_texts(&lang),
            workers,
        },
    )
}

/// `GET /portal/manpower/timesheets`.
pub(crate) async fn timesheets(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut sheets = Vec::new();
    for acting in companies(&state, &user).await? {
        let client = party_text(&state, &lang, Some(acting.party.0), "").await;
        for sheet in portal::sheets(&state, &acting, &lang.locale).await? {
            sheets.push(SheetLine {
                url: format!("/timesheets/{}", sheet.id),
                worker: sheet.worker,
                client: client.clone(),
                work: sheet.work,
                month: sheet.to.format("%b %Y").to_string(),
                hours: portal::hours_text(sheet.minutes),
                status: t(
                    &lang,
                    &format!("manpower.screen.portal.sheet.{}", sheet.standing),
                ),
                status_kind: match sheet.standing {
                    "escalated" => "warning",
                    "approved" | "deemed" => "success",
                    _ => "info",
                },
                waiting: sheet.waiting,
                adjustment: sheet.adjustment,
            });
        }
    }
    let waiting = sheets.iter().filter(|sheet| sheet.waiting).count();
    let waiting = match waiting {
        0 => String::new(),
        1 => t(&lang, "manpower.screen.portal.timesheets.waiting_one"),
        count => tf(
            &lang,
            "manpower.screen.portal.timesheets.waiting_many",
            &[("count", &count)],
        ),
    };
    answer(
        &headers,
        TimesheetsPage {
            tx: screen_texts(&lang),
            waiting,
            sheets,
        },
    )
}
