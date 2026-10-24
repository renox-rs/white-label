//! The prior authorisation pages (05 `CLN.INS.03`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/authorisations` | `clinics.authorisations` | `insurance_policies.view` |
//! | `POST /clinics/authorisations` | `clinics.authorisation_create` | `insurance.manage` |
//! | `GET /clinics/authorisations/{id}` | `clinics.authorisation` | `insurance_policies.view` |
//! | `POST /clinics/authorisations/{id}/request` | `clinics.authorisation_request` | `insurance.manage` |
//! | `POST /clinics/authorisations/{id}/approve` | `clinics.authorisation_approve` | `insurance.manage` |
//! | `POST /clinics/authorisations/{id}/approve-partially` | `clinics.authorisation_approve_partially` | `insurance.manage` |
//! | `POST /clinics/authorisations/{id}/deny` | `clinics.authorisation_deny` | `insurance.manage` |
//! | `POST /clinics/authorisations/{id}/cancel` | `clinics.authorisation_cancel` | `insurance.manage` |
//! | `POST /clinics/prior-auth-services` | `clinics.prior_auth_service_add` | `insurance.manage` |
//! | `POST /clinics/prior-auth-services/{id}/remove` | `clinics.prior_auth_service_remove` | `insurance.manage` |
//!
//! The list shows each authorisation with the units left and the payer's
//! list of services that need one. The transitions are the machine's: the
//! pages only read the form and call [`crate::authorisations`]. A page never
//! shows a clinical reason: the denial reason is staff's.

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::documents::today;

use super::{
    Texts, invalid, parse_date, parse_id, parse_party, party_exists, party_name, require, text,
};
use crate::authorisation::{AuthStatus, PriorAuthService, PriorAuthorisation};
use crate::authorisations::{self, NewAuthorisation};
use crate::permissions;
use crate::policy::InsurancePolicy;
use crate::service_names;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/authorisations", index)
        .name("clinics.authorisations")
        .post("/clinics/authorisations", create)
        .name("clinics.authorisation_create")
        .get("/clinics/authorisations/{id}", show)
        .name("clinics.authorisation")
        .post("/clinics/authorisations/{id}/request", request)
        .name("clinics.authorisation_request")
        .post("/clinics/authorisations/{id}/approve", approve)
        .name("clinics.authorisation_approve")
        .post(
            "/clinics/authorisations/{id}/approve-partially",
            approve_partially,
        )
        .name("clinics.authorisation_approve_partially")
        .post("/clinics/authorisations/{id}/deny", deny)
        .name("clinics.authorisation_deny")
        .post("/clinics/authorisations/{id}/cancel", cancel)
        .name("clinics.authorisation_cancel")
        .post("/clinics/prior-auth-services", service_add)
        .name("clinics.prior_auth_service_add")
        .post("/clinics/prior-auth-services/{id}/remove", service_remove)
        .name("clinics.prior_auth_service_remove")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: AuthStatus) -> String {
    text(
        lang,
        &format!("authorisation.status.{}", status_key(status)),
    )
}

/// The stored name of a status.
fn status_key(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::Draft => "draft",
        AuthStatus::Requested => "requested",
        AuthStatus::Approved => "approved",
        AuthStatus::PartiallyApproved => "partially_approved",
        AuthStatus::Denied => "denied",
        AuthStatus::Used => "used",
        AuthStatus::Expired => "expired",
        AuthStatus::Cancelled => "cancelled",
    }
}

/// The badge kind of a status.
fn status_kind(status: AuthStatus) -> &'static str {
    match status {
        AuthStatus::Draft | AuthStatus::Expired | AuthStatus::Cancelled => "info",
        AuthStatus::Requested => "info",
        AuthStatus::Approved | AuthStatus::Used => "success",
        AuthStatus::PartiallyApproved => "warning",
        AuthStatus::Denied => "error",
    }
}

/// One service on a payer's list.
#[derive(Serialize, Debug, Clone)]
pub struct RequiredRow {
    /// The primary key.
    pub id: i64,
    /// The payer's name.
    pub payer: String,
    /// The service code.
    pub code: String,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/authorisations.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    authorisations: GridPage<PriorAuthorisation>,
    required: Vec<RequiredRow>,
    policy_options: Vec<[String; 2]>,
    policy_selected: String,
    can_manage: bool,
}

/// The authorisations grid: search on the payer's reference, filters by
/// status, services and last valid day. The names, the number and the units
/// left are drawn by the template (`custom`).
pub(crate) fn grid(
    lang: &Lang,
    names: &std::collections::BTreeMap<String, String>,
    can_export: bool,
) -> Grid {
    let statuses: Vec<(String, String)> = AuthStatus::ALL
        .iter()
        .map(|status| (status.as_str().to_owned(), status_text(lang, *status)))
        .collect();
    let services: Vec<(String, String)> = names
        .keys()
        .map(|code| (code.clone(), service_names::name_with_code(names, code)))
        .collect();
    let grid = Grid::new("clinics-authorisations")
        .title(&text(lang, "authorisation.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::custom("number", &text(lang, "authorisation.number"))
                .frozen()
                .mobile(),
        )
        .column(Column::custom("patient", &text(lang, "policies.patient")).mobile())
        .column(Column::custom("payer", &text(lang, "policies.payer")))
        .column(Column::tags(
            "services",
            &text(lang, "authorisation.services_shown"),
            services,
        ))
        .column(Column::custom("units", &text(lang, "authorisation.units")).mobile())
        .column(
            Column::text("reference", &text(lang, "authorisation.reference"))
                .searchable()
                .hidden(),
        )
        .column(Column::date(
            "valid_until",
            &text(lang, "authorisation.valid_until"),
        ))
        .column(
            Column::select("status", &text(lang, "fields.status"), statuses)
                .badges(&[
                    ("draft", "info"),
                    ("requested", "info"),
                    ("expired", "info"),
                    ("cancelled", "info"),
                    ("approved", "success"),
                    ("used", "success"),
                    ("partially_approved", "warning"),
                    ("denied", "danger"),
                ])
                .mobile(),
        )
        .sort_by("-id")
        .row_url("/clinics/authorisations/{id}")
        .cards_on_mobile()
        .empty_state(
            &text(lang, "authorisation.empty"),
            Some(&text(lang, "authorisation.subtitle")),
        );
    if can_export { grid.exports() } else { grid }
}

/// The units as the list reads them.
fn units_text(lang: &Lang, a: &PriorAuthorisation) -> String {
    if a.units_approved > 0 {
        format!(
            "{} {} {}",
            a.units_left(),
            text(lang, "authorisation.of"),
            a.units_approved
        )
    } else {
        a.units_requested.to_string()
    }
}

/// The services of an authorisation for people: names with the codes the
/// forms take.
fn services_text(names: &std::collections::BTreeMap<String, String>, codes: &[String]) -> String {
    codes
        .iter()
        .map(|code| service_names::name_with_code(names, code))
        .collect::<Vec<_>>()
        .join(", ")
}

/// `GET /clinics/authorisations` (`?export=` needs `insurance.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::POLICIES_VIEW)?;
    let service_names = service_names::names(&state, &lang.locale).await?;
    let can_manage = user.has_permission(permissions::AUTHORISATIONS_MANAGE);
    let grid = grid(&lang, &service_names, can_manage);
    if request.param("export").is_some() && !can_manage {
        return Err(Error::Forbidden);
    }
    let query = PriorAuthorisation::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?;
    let mut parties = BTreeMap::new();
    for a in page.items() {
        for id in [a.patient_party_id, a.payer_party_id] {
            if let std::collections::btree_map::Entry::Vacant(slot) = parties.entry(id) {
                slot.insert(party_name(&state, &lang, PartyId(id)).await);
            }
        }
    }
    let page = page.extend(|a| {
        json!({
            "number": a.number(),
            "url": format!("/clinics/authorisations/{}", a.id),
            "patient": parties.get(&a.patient_party_id),
            "payer": parties.get(&a.payer_party_id),
            "units": units_text(&lang, a),
        })
    });
    let listed = PriorAuthService::query()
        .order_by(PriorAuthService::PAYER_PARTY_ID)
        .order_by(PriorAuthService::SERVICE_CODE)
        .get(&state.db)
        .await?;
    let mut required = Vec::with_capacity(listed.len());
    for row in &listed {
        required.push(RequiredRow {
            id: row.id,
            payer: party_name(&state, &lang, PartyId(row.payer_party_id)).await,
            code: service_names::name_with_code(&service_names, &row.service_code),
        });
    }
    let policies = InsurancePolicy::query()
        .order_by_desc(InsurancePolicy::ID)
        .get(&state.db)
        .await?;
    let mut policy_options = Vec::with_capacity(policies.len());
    for p in &policies {
        let patient = party_name(&state, &lang, p.patient()).await;
        policy_options.push([
            p.id.to_string(),
            format!("{patient} — {} {}", p.plan, p.member_id),
        ]);
    }
    let policy_selected = policy_options
        .first()
        .map(|option| option[0].clone())
        .unwrap_or_default();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        authorisations: page,
        required,
        policy_options,
        policy_selected,
        can_manage,
    })
    .into_response())
}

/// The form that prepares an authorisation.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The policy's id.
    #[serde(default)]
    pub policy: String,
    /// The service codes, comma separated.
    #[serde(default)]
    pub services: String,
    /// The units asked for.
    #[serde(default)]
    pub units: String,
}

/// `POST /clinics/authorisations`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::AUTHORISATIONS_MANAGE)?;
    let policy = parse_id(&form.policy).ok_or_else(|| invalid("policy: 1, 2, 3 ..."))?;
    let units = parse_id(&form.units).ok_or_else(|| invalid("units: 1, 2, 3 ..."))?;
    let mut new = NewAuthorisation::new(policy, units);
    for code in form.services.split(',') {
        new = new.service(code);
    }
    authorisations::create(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "authorisation.created")),
        renox::HxRefresh,
    )
        .into_response())
}

/// What the page of one authorisation needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/authorisation.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    patient: String,
    payer: String,
    services: String,
    status: String,
    status_kind: &'static str,
    units_requested: String,
    units_approved: String,
    units_consumed: String,
    units_left: String,
    reference: String,
    valid_until: String,
    denial_reason: String,
    today: String,
    can_request: bool,
    can_decide: bool,
    can_cancel: bool,
}

/// `GET /clinics/authorisations/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::POLICIES_VIEW)?;
    let a = PriorAuthorisation::find_or_404(&state.db, id).await?;
    let service_names = service_names::names(&state, &lang.locale).await?;
    let dash = text(&lang, "none");
    let manage = user.has_permission(permissions::AUTHORISATIONS_MANAGE);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: a.id,
        number: a.number(),
        patient: party_name(&state, &lang, PartyId(a.patient_party_id)).await,
        payer: party_name(&state, &lang, PartyId(a.payer_party_id)).await,
        services: services_text(&service_names, &a.services.0),
        status: status_text(&lang, a.status),
        status_kind: status_kind(a.status),
        units_requested: a.units_requested.to_string(),
        units_approved: a.units_approved.to_string(),
        units_consumed: a.units_consumed.to_string(),
        units_left: a.units_left().to_string(),
        reference: a.reference.clone().unwrap_or_else(|| dash.clone()),
        valid_until: a
            .valid_until
            .map_or_else(|| dash.clone(), |d| d.to_string()),
        denial_reason: a.denial_reason.clone().unwrap_or_else(|| dash.clone()),
        today: today().to_string(),
        can_request: manage && a.status == AuthStatus::Draft,
        can_decide: manage && a.status == AuthStatus::Requested,
        can_cancel: manage && matches!(a.status, AuthStatus::Draft | AuthStatus::Requested),
    }))
}

/// The toast and refresh that answer a transition.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// The form of `request`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RequestForm {
    /// The payer's reference, if it gave one with the request.
    #[serde(default)]
    pub reference: String,
}

/// `POST /clinics/authorisations/{id}/request`.
async fn request(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RequestForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    authorisations::request(&state, &user, id, reference, today()).await?;
    done(&lang, "authorisation.requested")
}

/// The form of `approve` and `approve_partially`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ApproveForm {
    /// The payer's approval reference.
    #[serde(default)]
    pub reference: String,
    /// The last day it is valid.
    #[serde(default)]
    pub valid_until: String,
    /// The units approved (`approve_partially`).
    #[serde(default)]
    pub units: String,
}

/// `POST /clinics/authorisations/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ApproveForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    let until = parse_date(&form.valid_until)?;
    authorisations::approve(&state, &user, id, reference, until, today()).await?;
    done(&lang, "authorisation.approved")
}

/// `POST /clinics/authorisations/{id}/approve-partially`.
async fn approve_partially(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ApproveForm>,
) -> Result<Response> {
    let reference = Some(form.reference.as_str()).filter(|r| !r.trim().is_empty());
    let until = parse_date(&form.valid_until)?;
    let units = parse_id(&form.units).ok_or_else(|| invalid("units: 1, 2, 3 ..."))?;
    authorisations::approve_partially(&state, &user, id, units, reference, until, today()).await?;
    done(&lang, "authorisation.approved_partially")
}

/// The form of `deny`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DenyForm {
    /// Why the payer refused.
    #[serde(default)]
    pub reason: String,
}

/// `POST /clinics/authorisations/{id}/deny`.
async fn deny(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DenyForm>,
) -> Result<Response> {
    authorisations::deny(&state, &user, id, &form.reason, today()).await?;
    done(&lang, "authorisation.denied")
}

/// `POST /clinics/authorisations/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    authorisations::cancel(&state, &user, id).await?;
    done(&lang, "authorisation.cancelled")
}

/// The form that adds a service to a payer's list.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ServiceForm {
    /// The payer, a party's id.
    #[serde(default)]
    pub payer: String,
    /// The service code.
    #[serde(default)]
    pub code: String,
}

/// `POST /clinics/prior-auth-services`.
async fn service_add(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<ServiceForm>,
) -> Result<Response> {
    require(&user, permissions::AUTHORISATIONS_MANAGE)?;
    let payer =
        parse_party(&form.payer).ok_or_else(|| invalid(&text(&lang, "policies.problem.payer")))?;
    if !party_exists(&state, payer).await? {
        return Err(invalid(&text(&lang, "policies.problem.payer_unknown")));
    }
    authorisations::require_for(&state, PartyId(payer), &form.code).await?;
    done(&lang, "authorisation.required_added")
}

/// `POST /clinics/prior-auth-services/{id}/remove`.
async fn service_remove(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::AUTHORISATIONS_MANAGE)?;
    let row = PriorAuthService::find_or_404(&state.db, id).await?;
    authorisations::stop_requiring(&state, PartyId(row.payer_party_id), &row.service_code).await?;
    done(&lang, "authorisation.required_removed")
}
