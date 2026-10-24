//! The back-office actions of the vertical (09 Manpower §7.1):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /manpower/contracts` | `manpower.contracts` | `contracts.view` or `contracts.manage` for the client |
//! | `GET /manpower/contracts/{id}` | `manpower.contract` | `contracts.view` or `contracts.manage` for the client (BL-MPW-22) |
//! | `POST /manpower/contracts` | `manpower.contract_create` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/signed` | `manpower.contract_signed` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/activate` | `manpower.contract_activate` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/cancel` | `manpower.contract_cancel` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/hold` | `manpower.contract_hold` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/terminate` | `manpower.contract_terminate` | `contracts.manage` for the client (the MD approves) |
//! | `POST /manpower/contracts/{id}/renew` | `manpower.contract_renew` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/release` | `manpower.contract_release` | `contracts.manage` for the client (the MD approves while a hold condition remains) |
//! | `POST /manpower/sites` | `manpower.site_create` | `sites.manage` for the client |
//! | `POST /manpower/sites/{id}` | `manpower.site_update` | `sites.manage` for the client |
//! | `GET /manpower/parties/options` | `manpower.party_options` | `contracts.manage` for some client (the picker of the forms) |
//! | `GET /manpower/requests/{id}` | `manpower.request` | as the list (BL-MPW-23) |
//! | `GET /manpower/requests` | `manpower.requests` | `requests.view`, `requests.create` or `requests.manage` for the client, or the client's portal access |
//! | `POST /manpower/requests` | `manpower.request_create` | `requests.create` for the client, or the client's approver |
//! | `POST /manpower/requests/{id}/submit` | `manpower.request_submit` | `requests.create` for the client, or the client's approver |
//! | `POST /manpower/requests/{id}/cancel` | `manpower.request_cancel` | `requests.manage` for the client, or the client's approver |
//! | `GET /portal/manpower/requests` | `manpower.portal_requests` | a user acting for a client company in the portal (BL-MPW-28) |
//! | `POST /portal/manpower/requests` | `manpower.portal_request_create` | the client's approver |
//! | `POST /portal/manpower/requests/{id}/submit` | `manpower.portal_request_submit` | the client's approver |
//! | `POST /portal/manpower/requests/{id}/cancel` | `manpower.portal_request_cancel` | the client's approver |
//! | `GET /portal/manpower/deployments` | `manpower.portal_deployments` | a user acting for a client company |
//! | `GET /portal/manpower/timesheets` | `manpower.portal_timesheets` | a user acting for a client company (the timesheet's page is `/timesheets/{id}`) |
//! | `POST /manpower/deployments/{id}/cover` | `manpower.deployment_cover` | `requests.manage` for the client |
//! | `GET /manpower/deployments` | `manpower.deployments` | `deployments.view` or `deployments.manage` for the client or the site, or the client's portal access (only the workers on site, name, role, site, shift and dates) |
//! | `GET /manpower/deployments/{id}` | `manpower.deployment` | `deployments.view` or `deployments.manage` for the client or the site (BL-MPW-24) |
//! | `POST /manpower/deployments` | `manpower.deployment_plan` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/activate` | `manpower.deployment_activate` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/extend` | `manpower.deployment_extend` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/suspend` | `manpower.deployment_suspend` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/resume` | `manpower.deployment_resume` | `deployments.manage` for the client |
//! | `POST /manpower/deployments/{id}/end` | `manpower.deployment_end` | `deployments.manage` for the client |
//! | `GET /manpower/absence` | `manpower.absence` | `absence.manage` (BL-MPW-25) |
//! | `GET /manpower/absence/{id}` | `manpower.absence_case` | `absence.manage` |
//! | `POST /manpower/absence/{id}/report` | `manpower.absence_report` | `absence.manage` |
//! | `POST /manpower/absence/{id}/close` | `manpower.absence_close` | `absence.manage` |
//! | `POST /manpower/deployments/{id}/cancel` | `manpower.deployment_cancel` | `deployments.manage` for the client |
//!
//! The lists and the record pages answer a page when the request names
//! `text/html` and JSON otherwise ([`wl_kernel::negotiate`]); the actions answer
//! 303 back to the page of the record with a toast ([`crate::screens::done`]) and
//! a refusal as its 422 with the words the sheet shows.

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
use crate::request::{ManpowerRequest, RequestKind};
use crate::requests::{self, NewRequest};
use crate::resumption;
use crate::screens::{self, contract_page, deployment_page, done, request_page};
use crate::site::{self, Site};
use crate::suspension::{self, DeploymentSuspension};
use crate::termination::TerminateAsk;
use crate::texts::english;
use crate::texts::{t, word};
use wl_kernel::documents::today;

/// The routes.
pub fn routes() -> Routes {
    Routes::new()
        .get("/manpower/contracts", contracts_index)
        .name("manpower.contracts")
        .post("/manpower/contracts", create_contract)
        .name("manpower.contract_create")
        .get("/manpower/contracts/{id}", screens::contract_page::show)
        .name("manpower.contract")
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
        .post("/manpower/sites/{id}", update_site)
        .name("manpower.site_update")
        .get("/manpower/parties/options", party_options)
        .name("manpower.party_options")
        .get("/manpower/requests", requests_index)
        .name("manpower.requests")
        .post("/manpower/requests", create_request)
        .name("manpower.request_create")
        .get("/manpower/requests/{id}", screens::request_page::show)
        .name("manpower.request")
        .post("/manpower/requests/{id}/submit", submit_request)
        .name("manpower.request_submit")
        .post("/manpower/requests/{id}/cancel", cancel_request)
        .name("manpower.request_cancel")
        .get("/portal/manpower/requests", screens::portal_page::requests)
        .name("manpower.portal_requests")
        .post("/portal/manpower/requests", screens::portal_page::create)
        .name("manpower.portal_request_create")
        .post(
            "/portal/manpower/requests/{id}/submit",
            screens::portal_page::submit,
        )
        .name("manpower.portal_request_submit")
        .post(
            "/portal/manpower/requests/{id}/cancel",
            screens::portal_page::cancel,
        )
        .name("manpower.portal_request_cancel")
        .get(
            "/portal/manpower/deployments",
            screens::portal_page::deployments,
        )
        .name("manpower.portal_deployments")
        .get(
            "/portal/manpower/timesheets",
            screens::portal_page::timesheets,
        )
        .name("manpower.portal_timesheets")
        .get("/manpower/deployments", deployments_index)
        .name("manpower.deployments")
        .get("/manpower/deployments/{id}", screens::deployment_page::show)
        .name("manpower.deployment")
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
        .get("/manpower/absence", screens::absence_page::list)
        .name("manpower.absence")
        .get("/manpower/absence/{id}", screens::absence_page::show)
        .name("manpower.absence_case")
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
    /// The contract's page.
    url: String,
    reference: String,
    /// `01 Jan 2026 - 31 Dec 2026`.
    period: String,
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
/// What a list may be narrowed to (`?status=on_hold`, `?ending=60`): the links
/// of the dashboard tiles. A value nobody knows narrows to nothing.
#[derive(serde::Deserialize, Default)]
pub struct ListFilter {
    /// A status key; for requests `missing` is the open and part-filled ones.
    #[serde(default)]
    status: String,
    /// Contracts that end within this many days.
    #[serde(default)]
    ending: Option<i64>,
}

async fn contracts_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<ListFilter>,
    session: Session,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    let mut visible = Vec::new();
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
        if !filter.status.is_empty() && contract.status.as_str() != filter.status {
            continue;
        }
        if let Some(days) = filter.ending {
            let day = today();
            let running = matches!(
                contract.status,
                ContractStatus::Active | ContractStatus::OnHold
            );
            if !(running
                && contract.end_date >= day
                && contract.end_date <= day + renox::chrono::Duration::days(days))
            {
                continue;
            }
        }
        visible.push(contract);
    }
    if visible.is_empty()
        && !user.has_permission_anywhere(permissions::CONTRACTS_VIEW)
        && !user.has_permission_anywhere(permissions::CONTRACTS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    if wl_kernel::negotiate::wants_html(&headers) {
        return screens::lists::contracts(&state, &user, &lang, &visible, &session, request)
            .await;
    }
    let mut rows = Vec::new();
    for contract in visible {
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
            url: contract_page::url(contract.id),
            reference: contract.reference.clone(),
            period: screens::period_text(&lang, contract.start_date, Some(contract.end_date)),
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
        .ok_or_else(|| refused(t(&lang, "manpower.problems.client_missing")))?;
    let start = date(&lang, &form, "start_date")?
        .ok_or_else(|| refused(t(&lang, "manpower.problems.dates_missing")))?;
    let end = date(&lang, &form, "end_date")?
        .ok_or_else(|| refused(t(&lang, "manpower.problems.dates_missing")))?;
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
    let created = contracts::create(&state, &user, new).await?;
    Ok(done(
        &lang,
        &contract_page::url(created.id),
        "manpower.screen.done.created",
    ))
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
    let on = date(&lang, &form, "signed_on")?
        .ok_or_else(|| refused(t(&lang, "manpower.problems.signed_date_missing")))?;
    contracts::record_signed(&state, &user, id, on, &field(&form, "document")).await?;
    Ok(done(
        &lang,
        &contract_page::url(id),
        "manpower.screen.done.signed",
    ))
}

/// `POST /manpower/contracts/{id}/activate`.
async fn activate_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    contracts::activate(&state, &user, id).await?;
    Ok(done(
        &lang,
        &contract_page::url(id),
        "manpower.screen.done.activated",
    ))
}

/// `POST /manpower/contracts/{id}/cancel`: `reason`.
async fn cancel_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    contracts::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &contract_page::url(id),
        "manpower.screen.done.cancelled",
    ))
}

/// `POST /manpower/contracts/{id}/hold`: `reason` (`overdue`,
/// `bounced_cheque` or `other`) and, for `other`, `text`.
async fn hold_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
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
    Ok(done(
        &lang,
        &contract_page::url(id),
        "manpower.screen.done.held",
    ))
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
    Ok(done(
        &lang,
        &contract_page::url(id),
        "manpower.screen.done.terms",
    ))
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
    let requested = contracts::terminate(&state, &user, id, ask).await?;
    let key = if requested.approval_request.is_some() {
        "manpower.screen.done.terminate_asked"
    } else {
        "manpower.screen.done.terminated"
    };
    Ok(done(&lang, &contract_page::url(id), key))
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
    Ok(done(
        &lang,
        &contract_page::url(id),
        "manpower.screen.done.renewed",
    ))
}

/// `POST /manpower/contracts/{id}/release`.
async fn release_contract(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let released = contracts::release(&state, &user, id).await?;
    let key = if released.approval_request.is_some() {
        "manpower.screen.done.release_asked"
    } else {
        "manpower.screen.done.released"
    };
    Ok(done(&lang, &contract_page::url(id), key))
}

/// `POST /manpower/sites`: `client_party_id`, `name`, `address` and
/// `required_licences`. Answers 303 back to the page the form was on (the
/// client's contract page).
async fn create_site(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back_to: renox::Back,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let client = number(&lang, &form, "client_party_id")?
        .ok_or_else(|| refused(t(&lang, "manpower.problems.client_missing")))?;
    site::create_with(
        &state,
        &user,
        PartyId(client),
        site::SiteInput {
            name: field(&form, "name"),
            address: Some(field(&form, "address")),
            required_licences: Some(field(&form, "required_licences")),
        },
    )
    .await?;
    Ok((
        Toast::success(t(&lang, "manpower.screen.done.site_created")),
        back_to,
    )
        .into_response())
}

/// `POST /manpower/sites/{id}`: `name`, `address` and `required_licences` of
/// a site; the deployments stay linked to it.
async fn update_site(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    back_to: renox::Back,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    site::update(
        &state,
        &user,
        id,
        site::SiteInput {
            name: field(&form, "name"),
            address: Some(field(&form, "address")),
            required_licences: Some(field(&form, "required_licences")),
        },
    )
    .await?;
    Ok((
        Toast::success(t(&lang, "manpower.screen.done.site_updated")),
        back_to,
    )
        .into_response())
}

/// `GET /manpower/parties/options`: the client companies a contract form's
/// picker offers as the person types (`?q=`), or the labels of the values it
/// already holds (`?values=`), as `number, name`. For those who manage
/// contracts, sites or terms. A client is a company: a person (a worker, a
/// contact) is not offered, nor a company that is a supplier, vendor or
/// subcontractor of the agency.
async fn party_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    if !user.has_permission_anywhere(permissions::CONTRACTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let directory = wl_kernel::parties::directory(&state);
    let mut parties = Vec::new();
    if query.is_lookup() {
        for id in query.values_as::<i64>() {
            parties.extend(directory.find(&state.db, PartyId(id)).await?);
        }
    } else {
        let today = wl_kernel::documents::today();
        for party in directory.search(&state.db, &query.q, 100).await? {
            if party.kind != wl_kernel::parties::PartyKind::Company {
                continue;
            }
            let mut supplier = false;
            for role in ["supplier", "vendor", "subcontractor"] {
                supplier |= directory.has_role(&state.db, party.id, role, today).await?;
            }
            if !supplier {
                parties.push(party);
            }
            if parties.len() == 20 {
                break;
            }
        }
    }
    Ok(Json(
        parties
            .into_iter()
            .map(|party| {
                renox::select::SelectOption::new(
                    party.id.0,
                    format!("{}, {}", party.code, party.display_name),
                )
            })
            .collect(),
    ))
}

/// A request as the list shows it.
#[derive(Debug, Serialize, Clone)]
pub(crate) struct RequestRow {
    id: i64,
    /// The request's page.
    url: String,
    /// `15 Nov 2026 - Open-ended`.
    period: String,
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
    /// The status as the pages word it (`open` is *Submitted*).
    status_label: String,
    status_kind: &'static str,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    replacement: bool,
    cause: Option<String>,
    replaces_deployment_id: Option<i64>,
}

/// `GET /manpower/requests`: the requests of the client companies the person
/// works for, or the one the portal grants them (`MPW.ACL.01`).
async fn requests_index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    headers: HeaderMap,
    Query(filter): Query<ListFilter>,
    session: Session,
    request: renox::grid::GridRequest,
) -> Result<Response> {
    let mut visible = Vec::new();
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
        let wanted = match filter.status.as_str() {
            "" => true,
            "missing" => request.status.takes_workers(),
            key => request.status.as_str() == key,
        };
        if wanted {
            visible.push(request);
        }
    }
    if !any
        && !user.has_permission_anywhere(permissions::REQUESTS_VIEW)
        && !user.has_permission_anywhere(permissions::REQUESTS_CREATE)
        && !user.has_permission_anywhere(permissions::REQUESTS_MANAGE)
    {
        return Err(Error::Forbidden);
    }
    if wl_kernel::negotiate::wants_html(&headers) {
        let ids = visible.iter().map(|request| request.id).collect();
        return screens::lists::requests(&state, &user, &lang, ids, &session, request).await;
    }
    let mut rows = Vec::new();
    for request in visible {
        let site = Site::find(&state.db, request.site_id)
            .await?
            .map(|site| site.name)
            .unwrap_or_default();
        rows.push(RequestRow {
            id: request.id,
            url: request_page::url(request.id),
            period: screens::period_text(&lang, request.start_date, request.end_date),
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
            status_label: request_page::status_text(&lang, request.status),
            status_kind: request_page::status_kind(request.status),
            start_date: request.start_date,
            end_date: request.end_date,
            replacement: request.kind == RequestKind::Replacement,
            cause: request.cause.clone(),
            replaces_deployment_id: request.replaces_deployment_id,
        });
    }
    Ok(Json(rows).into_response())
}

/// The request a *New request* form describes (the back office's sheet and the
/// client approver's in the portal send the same fields).
pub(crate) fn new_request_from(lang: &Lang, form: &BTreeMap<String, String>) -> Result<NewRequest> {
    Ok(NewRequest {
        contract_id: number(lang, form, "client_contract_id")?
            .ok_or_else(|| refused(t(lang, "manpower.problems.contract_missing")))?,
        site_id: number(lang, form, "site_id")?
            .ok_or_else(|| refused(t(lang, "manpower.problems.site_missing")))?,
        role: request_page::role_key(&field(form, "role")),
        headcount: number(lang, form, "headcount")?
            .ok_or_else(|| refused(t(lang, "manpower.problems.headcount_missing")))?,
        shift_type: field(form, "shift_type"),
        shift_start: Some(field(form, "shift_start")),
        shift_end: Some(field(form, "shift_end")),
        start_date: date(lang, form, "start_date")?
            .ok_or_else(|| refused(t(lang, "manpower.problems.start_missing")))?,
        end_date: date(lang, form, "end_date")?,
    })
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
    let new = new_request_from(&lang, &form)?;
    let created = requests::create(&state, &user, new).await?;
    Ok(done(
        &lang,
        &request_page::url(created.id),
        "manpower.screen.done.request_created",
    ))
}

/// `POST /manpower/requests/{id}/submit`. A contract on credit hold refuses it
/// (`MPW.ACL.14`): the page says so in words, then the service's line with the
/// date and the reason.
async fn submit_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    match requests::submit(&state, &user, id).await {
        Ok(_) => Ok(done(
            &lang,
            &request_page::url(id),
            "manpower.screen.done.request_submitted",
        )),
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message))
            if message.starts_with("Contract on credit hold") =>
        {
            Err(refused(format!(
                "{} {message}",
                t(&lang, "manpower.screen.requests.on_hold_blocked")
            )))
        }
        Err(other) => Err(other),
    }
}

/// `POST /manpower/requests/{id}/cancel`: `reason`. The service refuses while
/// a deployment of the request is active (BL-MPW-23), whoever asks.
async fn cancel_request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    requests::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &request_page::url(id),
        "manpower.screen.done.request_cancelled",
    ))
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
    /// The deployment's page.
    url: String,
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

/// The line staff see on a `suspended` deployment: the reasons and the days
/// suspended; none otherwise.
pub(crate) async fn suspension_note(
    state: &AppState,
    deployment: &Deployment,
) -> Result<Vec<String>> {
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
    Query(filter): Query<ListFilter>,
    session: Session,
    request: renox::grid::GridRequest,
) -> Result<Response> {
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
        if !filter.status.is_empty() && deployment.status.as_str() != filter.status {
            continue;
        }
        if by_staff {
            staff.push(deployment);
        } else {
            clients.push(deployment);
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
    if wl_kernel::negotiate::wants_html(&headers) {
        // Staff see their deployments; a client user only the workers on site.
        let is_staff = !staff.is_empty();
        let shown = if is_staff { &staff } else { &clients };
        let ids = shown.iter().map(|deployment| deployment.id).collect();
        return screens::lists::deployments(
            &state, &user, &lang, ids, is_staff, &session, request,
        )
        .await;
    }
    if staff.is_empty() {
        let mut rows = Vec::new();
        for deployment in &clients {
            rows.push(ClientDeploymentRow {
                worker: deployments::worker_name(&state, &lang.locale, deployment.employee()).await,
                role: deployment.role.clone(),
                site: site_name(&state, deployment.site_id).await?,
                shift_type: deployment.shift_type.clone(),
                start_date: deployment.start_date,
                end_date: deployment.end_date,
                status: word(&lang, "status", deployment.status.as_str()),
                status_kind: deployment_kind(deployment.status),
            });
        }
        return Ok(Json(rows).into_response());
    }
    let mut rows = Vec::new();
    for deployment in &staff {
        rows.push(DeploymentRow {
            id: deployment.id,
            employee_id: deployment.employee_id,
            worker: deployments::worker_name(&state, &lang.locale, deployment.employee()).await,
            client_contract_id: deployment.client_contract_id,
            client_party_id: deployment.client_party_id,
            client: party_name(&state, &lang, deployment.client_party_id).await,
            site_id: deployment.site_id,
            site: site_name(&state, deployment.site_id).await?,
            request_id: deployment.request_id,
            role: deployment.role.clone(),
            shift_type: deployment.shift_type.clone(),
            start_date: deployment.start_date,
            end_date: deployment.end_date,
            cover: deployment.kind == DeploymentKind::Cover,
            status: word(&lang, "status", deployment.status.as_str()),
            status_kind: deployment_kind(deployment.status),
            end_reason: deployment.end_reason.clone(),
            document_flags: DeploymentDocumentFlag::open_for(&state.db, deployment.id)
                .await?
                .iter()
                .map(DeploymentDocumentFlag::text)
                .collect(),
            days_suspended: deployment.days_suspended(today()),
            url: deployment_page::url(deployment.id),
            suspension: suspension_note(&state, deployment).await?,
        });
    }
    Ok(Json(rows).into_response())
}

/// The name of a site, empty when it is gone.
async fn site_name(state: &AppState, id: i64) -> Result<String> {
    Ok(Site::find(&state.db, id)
        .await?
        .map(|site| site.name)
        .unwrap_or_default())
}

/// `POST /manpower/deployments`: `employee_id`, `client_contract_id`,
/// `site_id`, `role`, `shift_type`, `start_date` and, optionally,
/// `request_id`, `shift_start`, `shift_end`, `work_days` (`mon,tue,fri`) and
/// `end_date`. With a `request_id` the contract, site, role and shift that are
/// not given are the request's (the plan sheet of the list asks for the
/// request, the worker and the dates only).
async fn plan_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let bad = || refused(t(&lang, "manpower.bad_input"));
    let request_id = number(&lang, &form, "request_id")?;
    let request = match request_id {
        Some(id) => ManpowerRequest::find(&state.db, id).await?,
        None => None,
    };
    let given = |name: &str, own: Option<String>| {
        Some(field(&form, name))
            .filter(|value| !value.is_empty())
            .or(own)
            .unwrap_or_default()
    };
    let contract_id = match number(&lang, &form, "client_contract_id")? {
        Some(id) => id,
        None => request
            .as_ref()
            .map(|request| request.client_contract_id)
            .ok_or_else(bad)?,
    };
    let site_id = match number(&lang, &form, "site_id")? {
        Some(id) => id,
        None => request
            .as_ref()
            .map(|request| request.site_id)
            .ok_or_else(bad)?,
    };
    let new = NewDeployment {
        employee: EmployeeId(number(&lang, &form, "employee_id")?.ok_or_else(bad)?),
        contract_id,
        site_id,
        request_id,
        role: given("role", request.as_ref().map(|request| request.role.clone())),
        shift_type: given(
            "shift_type",
            request.as_ref().map(|request| request.shift_type.clone()),
        ),
        shift_start: Some(given(
            "shift_start",
            request
                .as_ref()
                .and_then(|request| request.shift_start.clone()),
        )),
        shift_end: Some(given(
            "shift_end",
            request
                .as_ref()
                .and_then(|request| request.shift_end.clone()),
        )),
        work_days: Some(field(&form, "work_days")),
        start_date: date(&lang, &form, "start_date")?.ok_or_else(bad)?,
        end_date: date(&lang, &form, "end_date")?,
    };
    let (planned, warning) = deployments::plan_with_warning(&state, &user, new).await?;
    // Planning only warns about the documents (`MPW.ACL.07`).
    let page = deployment_page::url(planned.id);
    Ok(match warning {
        Some(warning) => (warning.toast(), back(&page)).into_response(),
        None => done(&lang, &page, "manpower.screen.done.deployment_planned"),
    })
}

/// `POST /manpower/deployments/{id}/activate`: the service lists every
/// failing check, one line each; the page lists them in words too.
async fn activate_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    deployments::activate(&state, &user, id, &lang.locale).await?;
    Ok(done(
        &lang,
        &deployment_page::url(id),
        "manpower.screen.done.deployment_activated",
    ))
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
    Ok(done(
        &lang,
        &deployment_page::url(id),
        "manpower.screen.done.deployment_extended",
    ))
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
    Ok(done(
        &lang,
        &deployment_page::url(id),
        "manpower.screen.done.deployment_suspended",
    ))
}

/// `POST /manpower/deployments/{id}/resume`.
async fn resume_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    resumption::resume(&state, &user, id, &lang.locale).await?;
    Ok(done(
        &lang,
        &deployment_page::url(id),
        "manpower.screen.done.deployment_resumed",
    ))
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
    let reason = field(&form, "reason");
    let deployment = deployments::load(&state, id).await?;
    deployments::require_manage(&user, deployment.client())?;
    // A reason that is not a user's is refused in words, not by its key.
    if !crate::deployment::reasons::USER.contains(&reason.as_str()) {
        return Err(refused(t(
            &lang,
            "manpower.screen.deployment.end_reason_unknown",
        )));
    }
    let details = ends::EndDetails {
        note: Some(field(&form, "note")),
        conduct: matches!(field(&form, "conduct").as_str(), "on" | "true" | "1"),
        end_date: date(&lang, &form, "end_date")?,
        waiver: Some(field(&form, "waiver")),
    };
    ends::end(&state, &user, id, &reason, details, &lang.locale).await?;
    Ok(done(
        &lang,
        &deployment_page::url(id),
        "manpower.screen.done.deployment_ended",
    ))
}

/// `POST /manpower/deployments/{id}/cancel`: `reason`.
async fn cancel_deployment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    deployments::cancel(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &deployment_page::url(id),
        "manpower.screen.done.deployment_cancelled",
    ))
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
    let opened = crate::cover::open_by_hand(&state, &user, id, from, to).await?;
    Ok(done(
        &lang,
        &request_page::url(opened.id),
        "manpower.screen.done.cover_asked",
    ))
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

/// The open absence cases as JSON, for HR (`absence.manage`).
pub(crate) async fn absence_json(state: &AppState, user: &AuthUser) -> Result<Response> {
    let rows: Vec<AbsenceRow> = absence::open_cases(state, user)
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
    Ok(done(
        &lang,
        &screens::absence_page::url(id),
        "manpower.screen.done.absence_reported",
    ))
}

/// `POST /manpower/absence/{id}/close`: `reason`.
async fn absence_close(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    absence::close(&state, &user, id, &field(&form, "reason")).await?;
    Ok(done(
        &lang,
        &screens::absence_page::url(id),
        "manpower.screen.done.absence_closed",
    ))
}
