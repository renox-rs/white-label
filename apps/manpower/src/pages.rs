//! The back-office actions of the vertical (09 Manpower §7.1):
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /manpower/contracts` | `manpower.contracts` | `contracts.view` or `contracts.manage` for the client |
//! | `POST /manpower/contracts` | `manpower.contract_create` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/signed` | `manpower.contract_signed` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/activate` | `manpower.contract_activate` | `contracts.manage` for the client |
//! | `POST /manpower/contracts/{id}/cancel` | `manpower.contract_cancel` | `contracts.manage` for the client |
//! | `POST /manpower/sites` | `manpower.site_create` | `sites.manage` for the client |
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

use crate::contract::{BillingCycle, ClientContract, ContractStatus};
use crate::contracts::{self, NewContract};
use crate::db::refused;
use crate::permissions;
use crate::site;
use crate::texts::{screen_texts, t, word};

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
        .post("/manpower/sites", create_site)
        .name("manpower.site_create")
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
