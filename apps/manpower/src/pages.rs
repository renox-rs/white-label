//! The back-office actions of the vertical (09 Manpower §7.1):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /manpower/contracts` | `manpower.contracts` | `contracts.view` or `contracts.manage` for the client |
//! | `POST /manpower/contracts` | `manpower.contract_create` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/signed` | `manpower.contract_signed` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/activate` | `manpower.contract_activate` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/cancel` | `manpower.contract_cancel` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/hold` | `manpower.contract_hold` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/terminate` | `manpower.contract_terminate` | `contracts.manage` for the client (the MD approves) |
//! | `POST /manpower/contracts/{id}/renew` | `manpower.contract_renew` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/release` | `manpower.contract_release` | `contracts.manage` for the client (the MD approves while a hold condition remains) |
//! | `POST /manpower/sites` | `manpower.site_create` | `sites.manage` for the client |
//! | `GET /manpower/requests` | `manpower.requests` | `requests.view`, `requests.create` or `requests.manage` for the client, or the client's portal access |
//! | `POST /manpower/requests` | `manpower.request_create` | `requests.create` for the client, or the client's approver |
//! | `POST /manpower/requests/{id}/submit` | `manpower.request_submit` | `requests.create` for the client, or the client's approver |
//! | `POST /manpower/requests/{id}/cancel` | `manpower.request_cancel` | `requests.manage` for the client, or the client's approver |
//! | `POST /manpower/deployments/{id}/cover` | `manpower.deployment_cover` | `requests.manage` for the client |
//! | `GET /manpower/deployments` | `manpower.deployments` | `deployments.view` or `deployments.manage` for the client or the site, or the client's portal access (only the workers on site, name, role, site, shift and dates) |
//! | `POST /manpower/deployments` | `manpower.deployment_plan` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/activate` | `manpower.deployment_activate` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/extend` | `manpower.deployment_extend` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/suspend` | `manpower.deployment_suspend` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/resume` | `manpower.deployment_resume` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/end` | `manpower.deployment_end` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/cancel` | `manpower.deployment_cancel` | `deployments.manage` for the client |
//!
//! The list answers a page when the request names `text/html` and JSON
//! otherwise ([`wl_kernel::negotiate`]); the actions answer 303 back to the
//! list for a form post and a refusal as its status.

use std::collections::BTreeMap;

use renox::axum::http::HeaderMap;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::Serialize;
use wl_kernel::PartyId;
use wl_kernel::workforce::EmployeeId;

use crate::absence;
use crate::approvers;
use crate::contract::{BillingCycle, ClientContract, ContractStatus};
use crate::contracts::{self, NewContract};
use crate::db::refused;
use crate::deployment::{Deployment, DeploymentKind, DeploymentStatus};
use crate::deployments::{self, NewDeployment};
use crate::documents::DeploymentDocumentFlag;
use crate::ends;
use crate::hold::ContractHold;
use crate::permissions;
use crate::renewal::RenewAsk;
use crate::request::{ManpowerRequest, RequestKind, RequestStatus};
use crate::requests::{self, NewRequest};
use crate::resumption;
use crate::site::{self, Site};
use crate::suspension::{self, DeploymentSuspension};
use crate::termination::TerminateAsk;
use crate::texts::english;
use crate::texts::{screen_texts, t, word};
use wl_kernel::documents::today;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/manpower/contracts", contracts_index)
        .name("manpower.contracts")
        .post("/manpower/contracts", create_contract)
        .name("manpower.contract_create")
        .post("/manpower/contracts/{id}/signed", sign_contract)
        .name("manpower.contract_signed")
        .post("/manpower/contracts/{id}/activate", activate_contract)
        .name("manpower.contract_activate")
        .post("/manpower/contracts/{id}/cancel", cancel_contract)
        .name("manpower.contract_cancel")
        .post("/manpower/contracts/{id}/hold", hold_contract)
        .name("manpower.contract_hold")
        .post("/manpower/contracts/{id}/renew", renew_contract)
        .name("manpower.contract_renew")
        .post("/manpower/contracts/{id}/timesheet-terms", timesheet_terms)
        .name("manpower.contract_terms")
        .post("/manpower/contracts/{id}/terminate", terminate_contract)
        .name("manpower.contract_terminate")
        .post("/manpower/contracts/{id}/release", release_contract)
        .name("manpower.contract_release")
        .post("/manpower/sites", create_site)
        .name("manpower.site_create")
        .get("/manpower/requests", requests_index)
        .name("manpower.requests")
        .post("/manpower/requests", create_request)
        .name("manpower.request_create")
        .post("/manpower/requests/{id}/submit", submit_request)
        .name("manpower.request_submit")
        .post("/manpower/requests/{id}/cancel", cancel_request)
        .name("manpower.request_cancel")
        .get("/manpower/deployments", deployments_index)
        .name("manpower.deployments")
        .post("/manpower/deployments", plan_deployment)
        .name("manpower.deployment_plan")
        .post("/manpower/deployments/{id}/activate", activate_deployment)
        .name("manpower.deployment_activate")
        .post("/manpower/deployments/{id}/extend", extend_deployment)
        .name("manpower.deployment_extend")
        .post("/manpower/deployments/{id}/suspend", suspend_deployment)
        .name("manpower.deployment_suspend")
        .post("/manpower/deployments/{id}/resume", resume_deployment)
        .name("manpower.deployment_resume")
        .post("/manpower/deployments/{id}/end", end_deployment)
        .name("manpower.deployment_end")
        .post("/manpower/deployments/{id}/cancel", cancel_deployment)
        .name("manpower.deployment_cancel")
        .post("/manpower/deployments/{id}/cover", cover_deployment)
        .name("manpower.deployment_cover")
        .get("/manpower/absence", absence_index)
        .name("manpower.absence")
        .post("/manpower/absence/{id}/report", absence_report)
        .name("manpower.absence_report")
        .post("/manpower/absence/{id}/close", absence_close)
        .name("manpower.absence_close")
        .require_auth()
}

/// The back to the list a form post answers.
pub(crate) fn back(path: &str) -> Response {
    renox::axum::response::Redirect::to(path).into_response()
}

/// The text of a form field, empty when absent.
pub(crate) fn field(form: &BTreeMap<String, String>, name: &str) -> String {
    form.get(name)
        .map(|v| v.trim().to_owned())
        .unwrap_or_default()
}

/// A whole number field; `None` when empty, a 422 when it is not a number.
pub(crate) fn number(
    lang: &Lang,
    form: &BTreeMap<String, String>,
    name: &str,
) -> Result<Option<i64>> {
    match field(form, name).as_str() {
        "" => Ok(None),
        value => value
            .parse::<i64>()
            .map(Some)
            .map_err(|_| refused(t(lang, "manpower.bad_input"))),
    }
}

/// A date field (`2027-05-31`); `None` when empty, a 422 when it is not a
/// date.
pub(crate) fn date(
    lang: &Lang,
    form: &BTreeMap<String, String>,
    name: &str,
) -> Result<Option<NaiveDate>> {
    match field(form, name).as_str() {
        "" => Ok(None),
        value => value
            .parse::<NaiveDate>()
            .map(Some)
            .map_err(|_| refused(t(lang, "manpower.bad_input"))),
    }
}

/// A flag field: `1`, `true`, `on` or `yes`.
pub(crate) fn flag(form: &BTreeMap<String, String>, name: &str) -> bool {
    matches!(
        field(form, name).to_lowercase().as_str(),
        "1" | "true" | "on" | "yes"
    )
}

/// A contract as the list shows it.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ContractRow {
    id: i64,
    reference: String,
    client_party_id: i64,
    client: String,
    status: String,
    status_kind: &'static str,
    start_date: NaiveDate,
    end_date: NaiveDate,
    billing_cycle: String,
    cover_required: bool,
    /// On hold: since when, the state before, who held it and the reasons.
    hold: Option<HoldRow>,
}

/// The credit hold of a contract as the list shows it (staff only: the client
/// is never told).
#[derive(Debug, Serialize, Clone)]
pub(crate) struct HoldRow {
    held_at: Option<NaiveDate>,
    status_before: Option<String>,
    held_by: Option<i64>,
    reasons: Vec<String>,
}

/// What the contracts page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/contracts.html")]
pub(crate) struct ContractsPage {
    tx: BTreeMap<&'static str, String>,
    contracts: Vec<ContractRow>,
}

/// The badge colour of a contract.
fn contract_kind(status: ContractStatus) -> &'static str {
    match status {
        ContractStatus::Active => "success",
        ContractStatus::OnHold | ContractStatus::Terminating => "warning",
        ContractStatus::Draft | ContractStatus::Ended | ContractStatus::Cancelled => "info",
    }
}

/// The name of a party, else `#id`.
pub(crate) async fn party_name(state: &AppState, lang: &Lang, id: i64) -> String {
    wl_kernel::parties::directory(state)
        .display_name(&state.db, PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// `GET /manpower/contracts`.
async fn contracts_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut rows = Vec::new();
    for contract in ClientContract::query()
        .order_by(ClientContract::ID)
        .get(&state.db)
        .await?
    {
        let scope = contract.scope();
        let may = user.has_permission_in(permissions::CONTRACTS_VIEW, &scope)
            || user.has_permission_in(permissions::CONTRACTS_MANAGE, &scope);
        if !may {
            continue;
        }
        let hold = if contract.status == ContractStatus::OnHold {
            let open = ContractHold::open_of(&state.db, contract.id).await?;
            Some(HoldRow {
                held_at: contract.held_at.map(wl_kernel::documents::dubai_date),
                status_before: contract
                    .status_before_hold
                    .map(|status| word(&lang, "status", status.as_str())),
                held_by: contract.held_by,
                reasons: crate::hold::reason_lines(&open),
            })
        } else {
            None
        };
        rows.push(ContractRow {
            id: contract.id,
            reference: contract.reference.clone(),
            client_party_id: contract.client_party_id,
            client: party_name(&state, &lang, contract.client_party_id).await,
            status: word(&lang, "status", contract.status.as_str()),
            status_kind: contract_kind(contract.status),
            start_date: contract.start_date,
            end_date: contract.end_date,
            billing_cycle: word(&lang, "billing", contract.billing_cycle.as_str()),
            cover_required: contract.cover_required,
            hold,
        });
    }
    if rows.is_empty()
        && !user.has_permission_anywhere(permissions::CONTRACTS_VIEW)
        && !user.has_permission_anywhere(permissions::CONTRACTS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(ContractsPage {
            tx: screen_texts(&lang),
            contracts: rows,
        })
        .into_response());
    }
    Ok(Json(rows).into_response())
}

/// `POST /manpower/contracts`: `reference`, `client_party_id`, `start_date`,
/// `end_date` and, optionally, `billing_cycle`, `payment_terms_days`,
/// `cover_required`, `deemed_approval_days`, `escalation_contact_party_id`,
/// `notice_period_days` and `account_manager_user_id`.
async fn create_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let client = number(&lang, &form, "client_party_id")?
        .ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    let start =
        date(&lang, &form, "start_date")?.ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    let end =
        date(&lang, &form, "end_date")?.ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    let mut new = NewContract::new(&field(&form, "reference"), PartyId(client), start, end);
    new.billing_cycle = match field(&form, "billing_cycle").as_str() {
        "" | "monthly" => BillingCycle::Monthly,
        "weekly" => BillingCycle::Weekly,
        "fortnightly" => BillingCycle::Fortnightly,
        _ => return Err(refused(t(&lang, "manpower.bad_input"))),
    };
    if let Some(days) = number(&lang, &form, "payment_terms_days")? {
        new.payment_terms_days = days;
    }
    if let Some(days) = number(&lang, &form, "notice_period_days")? {
        new.notice_period_days = days;
    }
    new.cover_required = flag(&form, "cover_required");
    new.deemed_approval_days = number(&lang, &form, "deemed_approval_days")?;
    new.escalation_contact = number(&lang, &form, "escalation_contact_party_id")?.map(PartyId);
    new.account_manager_user_id = number(&lang, &form, "account_manager_user_id")?;
    contracts::create(&state, &user, new).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/signed`: `signed_on` (`2027-01-05`) and
/// `document`.
async fn sign_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let on =
        date(&lang, &form, "signed_on")?.ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    contracts::record_signed(&state, &user, id, on, &field(&form, "document")).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/activate`.
async fn activate_contract(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    contracts::activate(&state, &user, id).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/cancel`: `reason`.
async fn cancel_contract(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    contracts::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/hold`: `reason` (`overdue`,
/// `bounced_cheque` or `other`) and, for `other`, `text`.
async fn hold_contract(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let text = field(&form, "text");
    contracts::hold(
        &state,
        &user,
        id,
        &field(&form, "reason"),
        Some(text.as_str()).filter(|text| !text.is_empty()),
    )
    .await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/timesheet-terms`: `deemed_approval_days`
/// (whole days; empty: no clause) and `escalation_contact_party_id` (empty:
/// none).
async fn timesheet_terms(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let days = number(&lang, &form, "deemed_approval_days")?;
    let contact = number(&lang, &form, "escalation_contact_party_id")?.map(PartyId);
    approvers::set_terms(&state, &user, id, days, contact).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/terminate`: `initiated_by` (`client` or
/// `agency`), `reason`, `notice_on` (`2026-11-01`), `end_date` and, when the
/// notice is short, the `waiver` document.
async fn terminate_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let bad = || refused(t(&lang, "manpower.bad_input"));
    let ask = TerminateAsk {
        initiated_by: field(&form, "initiated_by"),
        reason: field(&form, "reason"),
        notice_on: date(&lang, &form, "notice_on")?.ok_or_else(bad)?,
        end_date: date(&lang, &form, "end_date")?.ok_or_else(bad)?,
        waiver: Some(field(&form, "waiver")).filter(|waiver| !waiver.is_empty()),
    };
    contracts::terminate(&state, &user, id, ask).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/renew`: the new `end_date`, the day the
/// client signed (`signed_on`), the signed renewal (`document`) and, for
/// changed rates, the effective date of the new rate card version
/// (`rates_from`).
async fn renew_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let bad = || refused(t(&lang, "manpower.bad_input"));
    let ask = RenewAsk {
        end_date: date(&lang, &form, "end_date")?.ok_or_else(bad)?,
        signed_on: date(&lang, &form, "signed_on")?.ok_or_else(bad)?,
        document: field(&form, "document"),
        rates_from: date(&lang, &form, "rates_from")?,
    };
    contracts::renew(&state, &user, id, ask).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/contracts/{id}/release`.
async fn release_contract(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    contracts::release(&state, &user, id).await?;
    Ok(back("/manpower/contracts"))
}

/// `POST /manpower/sites`: `client_party_id`, `name` and `address`.
async fn create_site(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let client = number(&lang, &form, "client_party_id")?
        .ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    let address = field(&form, "address");
    site::create(
        &state,
        &user,
        PartyId(client),
        &field(&form, "name"),
        Some(&address),
    )
    .await?;
    Ok(back("/manpower/contracts"))
}

/// A request as the list shows it.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct RequestRow {
    id: i64,
    client_contract_id: i64,
    client_party_id: i64,
    client: String,
    site_id: i64,
    site: String,
    role: String,
    headcount: i64,
    filled_count: i64,
    shift_type: String,
    status: String,
    status_kind: &'static str,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    replacement: bool,
    cause: Option<String>,
    replaces_deployment_id: Option<i64>,
}

/// What the requests page needs.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/requests.html")]
pub(crate) struct RequestsPage {
    tx: BTreeMap<&'static str, String>,
    requests: Vec<RequestRow>,
}

/// The badge colour of a request.
fn request_kind(status: RequestStatus) -> &'static str {
    match status {
        RequestStatus::Filled => "success",
        RequestStatus::Open | RequestStatus::PartiallyFilled => "warning",
        RequestStatus::Draft | RequestStatus::Cancelled => "info",
    }
}

/// `GET /manpower/requests`: the requests of the client companies the person
/// works for, or the one the portal grants them (`MPW.ACL.01`).
async fn requests_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let mut rows = Vec::new();
    let mut any = false;
    for request in ManpowerRequest::query()
        .order_by(ManpowerRequest::ID)
        .get(&state.db)
        .await?
    {
        let see = crate::access::may_see(
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
            continue;
        }
        any = true;
        let site = Site::find(&state.db, request.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        rows.push(RequestRow {
            id: request.id,
            client_contract_id: request.client_contract_id,
            client_party_id: request.client_party_id,
            client: party_name(&state, &lang, request.client_party_id).await,
            site_id: request.site_id,
            site,
            role: request.role.clone(),
            headcount: request.headcount,
            filled_count: request.filled_count,
            shift_type: request.shift_type.clone(),
            status: word(&lang, "status", request.status.as_str()),
            status_kind: request_kind(request.status),
            start_date: request.start_date,
            end_date: request.end_date,
            replacement: request.kind == RequestKind::Replacement,
            cause: request.cause.clone(),
            replaces_deployment_id: request.replaces_deployment_id,
        });
    }
    if !any
        && !user.has_permission_anywhere(permissions::REQUESTS_VIEW)
        && !user.has_permission_anywhere(permissions::REQUESTS_CREATE)
        && !user.has_permission_anywhere(permissions::REQUESTS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    if wl_kernel::negotiate::wants_html(&headers) {
        return Ok(View::typed(RequestsPage {
            tx: screen_texts(&lang),
            requests: rows,
        })
        .into_response());
    }
    Ok(Json(rows).into_response())
}

/// `POST /manpower/requests`: `client_contract_id`, `site_id`, `role`,
/// `headcount`, `shift_type`, `start_date` and, optionally, `shift_start`,
/// `shift_end` (`07:00`) and `end_date`.
async fn create_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let bad = || refused(t(&lang, "manpower.bad_input"));
    let new = NewRequest {
        contract_id: number(&lang, &form, "client_contract_id")?.ok_or_else(bad)?,
        site_id: number(&lang, &form, "site_id")?.ok_or_else(bad)?,
        role: field(&form, "role"),
        headcount: number(&lang, &form, "headcount")?.ok_or_else(bad)?,
        shift_type: field(&form, "shift_type"),
        shift_start: Some(field(&form, "shift_start")),
        shift_end: Some(field(&form, "shift_end")),
        start_date: date(&lang, &form, "start_date")?.ok_or_else(bad)?,
        end_date: date(&lang, &form, "end_date")?,
    };
    requests::create(&state, &user, new).await?;
    Ok(back("/manpower/requests"))
}

/// `POST /manpower/requests/{id}/submit`.
async fn submit_request(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Response> {
    requests::submit(&state, &user, id).await?;
    Ok(back("/manpower/requests"))
}

/// `POST /manpower/requests/{id}/cancel`: `reason`.
async fn cancel_request(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    requests::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(back("/manpower/requests"))
}

/// A deployment as staff see it.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct DeploymentRow {
    id: i64,
    employee_id: i64,
    worker: String,
    client_contract_id: i64,
    client_party_id: i64,
    client: String,
    site_id: i64,
    site: String,
    request_id: Option<i64>,
    role: String,
    shift_type: String,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    cover: bool,
    status: String,
    status_kind: &'static str,
    end_reason: Option<String>,
    /// *document expiring <kind> <date>*, until the renewal is accepted
    /// (BL-MPW-09).
    document_flags: Vec<String>,
    /// The calendar days suspended so far (BL-MPW-11); suspended days are not
    /// billed.
    days_suspended: i64,
    /// While `suspended`: why, and for how long.
    suspension: Vec<String>,
}

/// A deployment as a client user sees it (`MPW.ACL.01`, `MPW.ACL.02`): the
/// name, the role, the site, the shift and the dates; no ids of the
/// agency's records, no reasons, nothing of the worker's pay, documents or
/// medical data, which a deployment does not hold.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct ClientDeploymentRow {
    worker: String,
    role: String,
    site: String,
    shift_type: String,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    status: String,
    status_kind: &'static str,
}

/// A row of the deployments page: a [`ClientDeploymentRow`] and, for staff, what
/// they are warned of (BL-MPW-09); empty for a client user, whose JSON rows
/// stay exactly the client's keys.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct PageDeploymentRow {
    #[serde(flatten)]
    row: ClientDeploymentRow,
    flags: Vec<String>,
}

/// What the deployments page needs; both kinds of row have the keys the
/// template reads.
#[derive(Serialize, ViewData)]
#[view(template = "manpower/deployments.html")]
pub(crate) struct DeploymentsPage {
    tx: BTreeMap<&'static str, String>,
    deployments: Vec<PageDeploymentRow>,
}

/// The line staff see on a `suspended` deployment: the reasons and the days
/// suspended; none otherwise.
async fn suspension_note(state: &AppState, deployment: &Deployment) -> Result<Vec<String>> {
    if deployment.status != DeploymentStatus::Suspended {
        return Ok(Vec::new());
    }
    let reasons: Vec<String> = DeploymentSuspension::open_for(&state.db, deployment.id)
        .await?
        .iter()
        .map(DeploymentSuspension::text)
        .collect();
    let reasons = reasons.join(", ");
    let days = deployment.days_suspended(today());
    let key = if days == 1 {
        "manpower.deployment.suspended_note_one"
    } else {
        "manpower.deployment.suspended_note"
    };
    Ok(vec![english(
        key,
        &[("reasons", &reasons), ("days", &days)],
    )])
}

/// The badge colour of a deployment.
fn deployment_kind(status: DeploymentStatus) -> &'static str {
    match status {
        DeploymentStatus::Active => "success",
        DeploymentStatus::Suspended => "warning",
        DeploymentStatus::Planned | DeploymentStatus::Ended | DeploymentStatus::Cancelled => "info",
    }
}

/// `GET /manpower/deployments`: staff see the deployments of the client
/// companies and sites they work for; a client user sees only the workers on
/// site of their own company, as a [`ClientDeploymentRow`].
async fn deployments_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
) -> Result<Response> {
    let wants_page = wl_kernel::negotiate::wants_html(&headers);
    let mut staff = Vec::new();
    let mut clients = Vec::new();
    for deployment in Deployment::query()
        .order_by(Deployment::ID)
        .get(&state.db)
        .await?
    {
        let by_staff = [
            permissions::DEPLOYMENTS_VIEW,
            permissions::DEPLOYMENTS_MANAGE,
        ]
        .iter()
        .any(|permission| {
            user.has_permission_in(permission, &deployment.scope())
                || user.has_permission_in(permission, &deployment.site_scope())
        });
        let by_client = !by_staff
            && deployment.shown_to_client()
            && crate::access::may_see(&state, &user, deployment.client(), &[]).await?;
        if !by_staff && !by_client {
            continue;
        }
        let site = Site::find(&state.db, deployment.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        let worker = deployments::worker_name(&state, &lang.locale, deployment.employee()).await;
        let status = word(&lang, "status", deployment.status.as_str());
        let kind = deployment_kind(deployment.status);
        if by_staff {
            staff.push(DeploymentRow {
                id: deployment.id,
                employee_id: deployment.employee_id,
                worker: worker.clone(),
                client_contract_id: deployment.client_contract_id,
                client_party_id: deployment.client_party_id,
                client: party_name(&state, &lang, deployment.client_party_id).await,
                site_id: deployment.site_id,
                site: site.clone(),
                request_id: deployment.request_id,
                role: deployment.role.clone(),
                shift_type: deployment.shift_type.clone(),
                start_date: deployment.start_date,
                end_date: deployment.end_date,
                cover: deployment.kind == DeploymentKind::Cover,
                status: status.clone(),
                status_kind: kind,
                end_reason: deployment.end_reason.clone(),
                document_flags: DeploymentDocumentFlag::open_for(&state.db, deployment.id)
                    .await?
                    .iter()
                    .map(DeploymentDocumentFlag::text)
                    .collect(),
                days_suspended: deployment.days_suspended(today()),
                suspension: suspension_note(&state, &deployment).await?,
            });
        } else {
            clients.push(ClientDeploymentRow {
                worker,
                role: deployment.role.clone(),
                site,
                shift_type: deployment.shift_type.clone(),
                start_date: deployment.start_date,
                end_date: deployment.end_date,
                status,
                status_kind: kind,
            });
        }
    }
    if staff.is_empty()
        && clients.is_empty()
        && !user.has_permission_anywhere(permissions::DEPLOYMENTS_VIEW)
        && !user.has_permission_anywhere(permissions::DEPLOYMENTS_MANAGE)
    {
        // A client user with no worker on site yet gets an empty list; only
        // someone with no access at all is refused.
        let may = crate::access::may_see_any(&state, &user).await?;
        if !may {
            return Err(Error::Forbidden);
        }
    }
    if wants_page {
        // The page reads the same keys for both kinds of row.
        let rows: Vec<PageDeploymentRow> = if staff.is_empty() {
            clients
                .into_iter()
                .map(|row| PageDeploymentRow {
                    row,
                    flags: Vec::new(),
                })
                .collect()
        } else {
            staff
                .iter()
                .map(|row| PageDeploymentRow {
                    row: ClientDeploymentRow {
                        worker: row.worker.clone(),
                        role: row.role.clone(),
                        site: row.site.clone(),
                        shift_type: row.shift_type.clone(),
                        start_date: row.start_date,
                        end_date: row.end_date,
                        status: row.status.clone(),
                        status_kind: row.status_kind,
                    },
                    flags: row
                        .document_flags
                        .iter()
                        .chain(&row.suspension)
                        .cloned()
                        .collect(),
                })
                .collect()
        };
        return Ok(View::typed(DeploymentsPage {
            tx: screen_texts(&lang),
            deployments: rows,
        })
        .into_response());
    }
    if staff.is_empty() {
        Ok(Json(clients).into_response())
    } else {
        Ok(Json(staff).into_response())
    }
}

/// `POST /manpower/deployments`: `employee_id`, `client_contract_id`,
/// `site_id`, `role`, `shift_type`, `start_date` and, optionally,
/// `request_id`, `shift_start`, `shift_end`, `work_days` (`mon,tue,fri`) and
/// `end_date`.
async fn plan_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let bad = || refused(t(&lang, "manpower.bad_input"));
    let new = NewDeployment {
        employee: EmployeeId(number(&lang, &form, "employee_id")?.ok_or_else(bad)?),
        contract_id: number(&lang, &form, "client_contract_id")?.ok_or_else(bad)?,
        site_id: number(&lang, &form, "site_id")?.ok_or_else(bad)?,
        request_id: number(&lang, &form, "request_id")?,
        role: field(&form, "role"),
        shift_type: field(&form, "shift_type"),
        shift_start: Some(field(&form, "shift_start")),
        shift_end: Some(field(&form, "shift_end")),
        work_days: Some(field(&form, "work_days")),
        start_date: date(&lang, &form, "start_date")?.ok_or_else(bad)?,
        end_date: date(&lang, &form, "end_date")?,
    };
    let (_planned, warning) = deployments::plan_with_warning(&state, &user, new).await?;
    // Planning only warns about the documents (`MPW.ACL.07`).
    Ok(match warning {
        Some(warning) => (warning.toast(), back("/manpower/deployments")).into_response(),
        None => back("/manpower/deployments"),
    })
}

/// `POST /manpower/deployments/{id}/activate`.
async fn activate_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    deployments::activate(&state, &user, id, &lang.locale).await?;
    Ok(back("/manpower/deployments"))
}

/// `POST /manpower/deployments/{id}/extend`: `end_date`.
async fn extend_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let end =
        date(&lang, &form, "end_date")?.ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    deployments::extend(&state, &user, id, end, &lang.locale).await?;
    Ok(back("/manpower/deployments"))
}

/// `POST /manpower/deployments/{id}/suspend`: `reason`.
async fn suspend_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    suspension::suspend(&state, &user, id, &field(&form, "reason"), &lang.locale).await?;
    Ok(back("/manpower/deployments"))
}

/// `POST /manpower/deployments/{id}/resume`.
async fn resume_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    resumption::resume(&state, &user, id, &lang.locale).await?;
    Ok(back("/manpower/deployments"))
}

/// `POST /manpower/deployments/{id}/end`: `reason` and, for the reasons that
/// ask for it, `note` (the client's note), `conduct` (ticked: a complaint
/// ticket opens), `end_date` (client non-payment: the last day) and `waiver`.
async fn end_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let details = ends::EndDetails {
        note: Some(field(&form, "note")),
        conduct: matches!(field(&form, "conduct").as_str(), "on" | "true" | "1"),
        end_date: date(&lang, &form, "end_date")?,
        waiver: Some(field(&form, "waiver")),
    };
    ends::end(
        &state,
        &user,
        id,
        &field(&form, "reason"),
        details,
        &lang.locale,
    )
    .await?;
    Ok(back("/manpower/deployments"))
}

/// `POST /manpower/deployments/{id}/cancel`: `reason`.
async fn cancel_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    deployments::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(back("/manpower/deployments"))
}

/// `POST /manpower/deployments/{id}/cover`: `from` (`2027-03-01`) and,
/// optionally, `to`: a cover request for the days the worker is away.
async fn cover_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let from =
        date(&lang, &form, "from")?.ok_or_else(|| refused(t(&lang, "manpower.bad_input")))?;
    let to = date(&lang, &form, "to")?;
    crate::cover::open_by_hand(&state, &user, id, from, to).await?;
    Ok(back("/manpower/requests"))
}

/// An absence case as the list shows it (JSON).
#[derive(Serialize)]
struct AbsenceRow {
    id: i64,
    employee_id: i64,
    ticket_id: Option<i64>,
    last_present_on: NaiveDate,
    opened_on: NaiveDate,
    report_number: Option<String>,
    report_date: Option<NaiveDate>,
}

/// `GET /manpower/absence`: the open absence cases, for HR (`absence.manage`).
async fn absence_index(State(state): State<AppState>, user: AuthUser) -> Result<Response> {
    let rows: Vec<AbsenceRow> = absence::open_cases(&state, &user)
        .await?
        .into_iter()
        .map(|case| AbsenceRow {
            id: case.id,
            employee_id: case.employee_id,
            ticket_id: case.ticket_id,
            last_present_on: case.last_present_on,
            opened_on: case.opened_on,
            report_number: case.report_number,
            report_date: case.report_date,
        })
        .collect();
    Ok(Json(rows).into_response())
}

/// `POST /manpower/absence/{id}/report`: `number` and `date` of the MoHRE
/// absence report.
async fn absence_report(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let date = date(&lang, &form, "date")?.unwrap_or_else(today);
    absence::record_report(&state, &user, id, &field(&form, "number"), date).await?;
    Ok(back("/manpower/absence"))
}

/// `POST /manpower/absence/{id}/close`: `reason`.
async fn absence_close(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    absence::close(&state, &user, id, &field(&form, "reason")).await?;
    Ok(back("/manpower/absence"))
}
