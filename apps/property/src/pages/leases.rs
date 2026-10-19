//! The lease pages: the list, a lease with its terms, payment plan, cheques,
//! versions and history, and the actions on it (01 Property §4.1, §7.1).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /property/leases` | `property.leases` | `leases.view` in a building |
//! | `GET /property/leases/{id}` | `property.lease` | `leases.view` in the building |
//! | `POST /property/units/{id}/leases` | `property.lease_create` | `leases.create` in the building |
//! | `POST /property/leases/{id}/versions` | `property.lease_version` | `leases.create` in the building |
//! | `POST /property/leases/{id}/submit` | `property.lease_submit` | `leases.create` in the building |
//! | `POST /property/leases/{id}/activate` | `property.lease_activate` | `leases.activate` in the building |
//! | `POST /property/leases/{id}/cancel` | `property.lease_cancel` | `leases.cancel` in the building |
//! | `POST /property/leases/{id}/extend` | `property.lease_extend` | `leases.manage` in the building |
//! | `POST /property/leases/{id}/renewal-decision` | `property.lease_renewal_decision` | `leases.manage` in the building |
//! | `POST /property/leases/{id}/changed-terms-notice` | `property.lease_changed_terms_notice` | `leases.manage` in the building |
//! | `POST /property/leases/{id}/waive-deposit` | `property.lease_waive_deposit` | `leases.activate` in the building |
//! | `POST /property/leases/{id}/ejari` | `property.lease_ejari` | `leases.activate` in the building |
//! | `POST /property/leases/{id}/cheques` | `property.lease_cheques` | `leases.create` in the building |
//!
//! The actions answer with a toast and a refresh, as the sheets post them by
//! htmx; a refusal answers 422 with its reason.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{self, StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::cheques::ChequeSummary;
use wl_kernel::parties::directory;

use super::{Texts, buildings_with, lease_kind, text};
use crate::building::Building;
use crate::charge::{ChargeStatus, LeaseCharge, RENT};
use crate::cheques::{ChequeEntry, of_version, register_cheques};
use crate::lease::{Lease, LeaseStatus, RenewalDecision, VersionKind};
use crate::cancellation::{self, CancelRequest};
use crate::chase;
use crate::lease_machine::{ACTIVATE, CANCEL, SUBMIT};
use crate::leases::{self, EjariInput, NewLease, NewVersion};
use crate::parse::{parse_aed, parse_date};
use crate::permissions;
use crate::renewal;
use crate::texts;
use crate::unit::Unit;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/property/leases", index)
        .name("property.leases")
        .get("/property/leases/{id}", show)
        .name("property.lease")
        .post("/property/units/{id}/leases", create)
        .name("property.lease_create")
        .post("/property/leases/{id}/versions", create_version)
        .name("property.lease_version")
        .post("/property/leases/{id}/submit", submit)
        .name("property.lease_submit")
        .post("/property/leases/{id}/activate", activate)
        .name("property.lease_activate")
        .post("/property/leases/{id}/cancel", cancel)
        .name("property.lease_cancel")
        .post("/property/leases/{id}/extend", extend)
        .name("property.lease_extend")
        .post("/property/leases/{id}/renewal-decision", renewal_decision)
        .name("property.lease_renewal_decision")
        .post("/property/leases/{id}/changed-terms-notice", changed_terms_notice)
        .name("property.lease_changed_terms_notice")
        .post("/property/leases/{id}/waive-deposit", waive_deposit)
        .name("property.lease_waive_deposit")
        .post("/property/leases/{id}/ejari", ejari)
        .name("property.lease_ejari")
        .post("/property/leases/{id}/cheques", cheques)
        .name("property.lease_cheques")
        .require_auth()
}

/// The name of party `id`, else `#id`.
async fn party_name(state: &AppState, lang: &Lang, id: i64) -> String {
    directory(state)
        .display_name(&state.db, wl_kernel::PartyId(id), &lang.locale)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("#{id}"))
}

/// The name of the unit, `TWR-A / 1204`, else `#id`.
async fn unit_label(state: &AppState, unit_id: i64) -> String {
    let Ok(Some(unit)) = Unit::find(&state.db, unit_id).await else {
        return format!("#{unit_id}");
    };
    match Building::find(&state.db, unit.building_id).await {
        Ok(Some(building)) => format!("{} / {}", building.code, unit.number),
        _ => unit.number,
    }
}

/// One lease of the list.
#[derive(Serialize, Debug, Clone)]
pub struct LeaseRow {
    /// The primary key.
    pub id: i64,
    /// The lease number with its version.
    pub number: String,
    /// The unit.
    pub unit: String,
    /// The tenant's name.
    pub tenant: String,
    /// The term, `2027-01-01 to 2027-12-31`.
    pub term: String,
    /// The annual rent.
    pub annual_rent: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Renewal, amendment or empty for the first version.
    pub kind: String,
}

/// What the list needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/leases.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    leases: Vec<LeaseRow>,
}

/// The text of a version kind; empty for the first version.
pub(crate) fn kind_text(lang: &Lang, kind: VersionKind) -> String {
    match kind {
        VersionKind::New => String::new(),
        other => text(lang, &format!("leases.kind.{}", other.as_str())),
    }
}

/// `GET /property/leases`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    let scopes = buildings_with(&user, permissions::LEASES_VIEW)?;
    let query = scopes.apply(Lease::query().order_by_desc(Lease::ID), &["building_id"]);
    let mut rows = Vec::new();
    for lease in query.get(&state.db).await? {
        rows.push(LeaseRow {
            id: lease.id,
            number: lease.number_with_version(),
            unit: unit_label(&state, lease.unit_id).await,
            tenant: party_name(&state, &lang, lease.tenant_party_id).await,
            term: format!("{} → {}", lease.start_date, lease.end_date),
            annual_rent: crate::money_text(lease.annual_rent),
            status: text(&lang, &format!("leases.status.{}", lease.status.as_str())),
            status_kind: lease_kind(lease.status),
            kind: kind_text(&lang, lease.version_kind),
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        leases: rows,
    }))
}

/// A charge of the payment plan.
#[derive(Serialize, Debug, Clone)]
pub struct ChargeRow {
    /// The charge's position in the plan.
    pub number: i32,
    /// What it is for.
    pub description: String,
    /// The due date.
    pub due_date: String,
    /// The amount before VAT.
    pub amount: String,
    /// The VAT treatment.
    pub vat: String,
    /// The status in the page's language.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// The invoice made for it.
    pub invoice_id: Option<i64>,
}

/// A cheque received with the lease.
#[derive(Serialize, Debug, Clone)]
pub struct ChequeRow {
    /// The cheque number.
    pub number: String,
    /// The bank.
    pub bank: String,
    /// The date on it.
    pub date: String,
    /// The amount.
    pub amount: String,
    /// The status in the page's language.
    pub status: String,
}

/// A version of the lease.
#[derive(Serialize, Debug, Clone)]
pub struct VersionRow {
    /// The version's id.
    pub id: i64,
    /// `v2`.
    pub label: String,
    /// Renewal, amendment or empty.
    pub kind: String,
    /// The term.
    pub term: String,
    /// The status in the page's language.
    pub status: String,
    /// Whether it is the version the page shows.
    pub current: bool,
}

/// The lease as the page shows it.
#[derive(Serialize, Debug, Clone)]
pub struct LeaseView {
    /// The primary key.
    pub id: i64,
    /// The number with its version.
    pub number: String,
    /// The unit.
    pub unit: String,
    /// The unit's id.
    pub unit_id: i64,
    /// The building's id.
    pub building_id: i64,
    /// The tenant's name.
    pub tenant: String,
    /// Renewal, amendment or empty.
    pub kind: String,
    /// The first day of the term.
    pub start_date: String,
    /// The last day.
    pub end_date: String,
    /// The annual rent.
    pub annual_rent: String,
    /// The deposit, or a note that it is waived.
    pub deposit: String,
    /// The VAT treatment of the rent.
    pub vat: String,
    /// The days of notice the contract needs.
    pub notice_days: i64,
    /// The day an amendment applies from.
    pub effective_date: String,
    /// The clauses.
    pub terms: String,
    /// The Ejari or Tawtheeq number.
    pub ejari_number: String,
    /// The day it expires.
    pub ejari_expires_on: String,
    /// Why it was cancelled.
    pub cancel_reason: String,
    /// The part of the deposit forfeited on cancellation.
    pub cancel_forfeit: String,
    /// The clause of the forfeit.
    pub cancel_forfeit_clause: String,
    /// The follow-up date the property manager extended to.
    pub follow_up_until: String,
    /// The last day a notice of changed terms can be delivered (`PRP.RUL.09`);
    /// empty unless the lease is in force.
    pub notice_deadline: String,
    /// The renewal decision in the page's language.
    pub renewal_decision: String,
    /// Whether the notice of changed terms was sent and delivered.
    pub notice_of_changes: String,
    /// "Current terms only" once the marker is set, else empty.
    pub current_terms_only: String,
}

/// What a lease page needs.
#[derive(Serialize, ViewData)]
#[view(template = "property/lease.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    lease: LeaseView,
    status: String,
    status_kind: &'static str,
    charges: Vec<ChargeRow>,
    cheques: Vec<ChequeRow>,
    versions: Vec<VersionRow>,
    approvals: Vec<ApprovalCard>,
    timeline: Vec<Value>,
    pending: bool,
    can_submit: bool,
    can_activate: bool,
    can_cancel: bool,
    can_extend: bool,
    can_decide: bool,
    can_send_notice: bool,
    can_waive: bool,
    can_ejari: bool,
    can_version: bool,
    can_cheques: bool,
    today: String,
}

/// Whether `actions` offers the transition `name`.
fn offers(actions: &[state_machine::Available], name: &str) -> bool {
    actions.iter().any(|a| a.name == name)
}

/// The label of the VAT category of a charge.
fn vat_text(lang: &Lang, category: wl_kernel::VatCategory) -> String {
    text(lang, &format!("vat.{}", category.as_str()))
}

/// `GET /property/leases/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    let scope = lease.scope();
    if !user.has_permission_in(permissions::LEASES_VIEW, &scope) {
        return Err(Error::Forbidden);
    }
    let actions = lease.available(&user, &state.db).await?;
    let subject = lease.subject();
    let pending = !approvals::pending_for(&state.db, &subject).await?.is_empty();
    let approval_cards = approvals::cards_for(&state.db, &subject, &user, &lang).await?;
    let history = state_machine::history(&state.db, Lease::TABLE, lease.id).await?;
    let mut timeline = state_machine::timeline(&history, Some(&state_machine::info_of::<Lease>()));
    timeline.extend(approvals::timeline_for(&state.db, &subject, &lang).await?);
    timeline.sort_by(|a, b| a["time"].as_str().cmp(&b["time"].as_str()));

    let charges = lease
        .charges(&state.db)
        .await?
        .into_iter()
        .map(|c| ChargeRow {
            number: c.position,
            description: c.description.clone(),
            due_date: c.due_date.to_string(),
            amount: crate::money_text(c.amount),
            vat: vat_text(&lang, c.vat_category),
            status: text(&lang, &format!("charges.status.{}", c.status.as_str())),
            status_kind: match c.status {
                ChargeStatus::Invoiced => "success",
                ChargeStatus::Scheduled => "info",
                ChargeStatus::Planned | ChargeStatus::Cancelled => "info",
            },
            invoice_id: c.invoice_id,
        })
        .collect();
    let cheque_rows = of_version(&state, &lease)
        .await?
        .into_iter()
        .map(|c: ChequeSummary| ChequeRow {
            number: c.number,
            bank: c.bank,
            date: c.date.to_string(),
            amount: crate::money_text(c.amount),
            status: text(&lang, &format!("cheques.status.{}", c.status.as_str())),
        })
        .collect();
    let versions = Lease::versions_of(&state.db, lease.root_id)
        .await?
        .into_iter()
        .map(|v| VersionRow {
            id: v.id,
            label: format!("v{}", v.version),
            kind: kind_text(&lang, v.version_kind),
            term: format!("{} → {}", v.start_date, v.end_date),
            status: text(&lang, &format!("leases.status.{}", v.status.as_str())),
            current: v.id == lease.id,
        })
        .collect();

    let running = LeaseStatus::RUNNING.contains(&lease.status);
    let delivered = if running {
        renewal::delivered_on(&state, &lease).await?
    } else {
        lease.notice_of_changes_delivered_on
    };
    let notice_of_changes = match (delivered, lease.notice_of_changes_sent_on) {
        (Some(day), _) => texts::t_with(&lang, "property.renewal.notice_delivered", &[("delivered", &day)]),
        (None, Some(sent)) => texts::t_with(&lang, "property.renewal.notice_undelivered", &[("sent", &sent)]),
        (None, None) => text(&lang, "renewal.notice_not_sent"),
    };
    let manages = user.has_permission_in(permissions::LEASES_MANAGE, &scope);
    let in_window = matches!(lease.status, LeaseStatus::Active | LeaseStatus::RenewalDue);
    let creates = user.has_permission_in(permissions::LEASES_CREATE, &scope);
    let activates = user.has_permission_in(permissions::LEASES_ACTIVATE, &scope);
    let in_progress = LeaseStatus::IN_PROGRESS.contains(&lease.status);
    let can_version = creates
        && LeaseStatus::AMENDABLE.contains(&lease.status)
        && Lease::follow_ups_in_progress(&state.db, lease.root_id)
            .await?
            .is_empty();
    let deposit = if lease.deposit_waived {
        text(&lang, "leases.deposit_waived")
    } else {
        crate::money_text(lease.deposit_amount)
    };
    let view = LeaseView {
        id: lease.id,
        number: lease.number_with_version(),
        unit: unit_label(&state, lease.unit_id).await,
        unit_id: lease.unit_id,
        building_id: lease.building_id,
        tenant: party_name(&state, &lang, lease.tenant_party_id).await,
        kind: kind_text(&lang, lease.version_kind),
        start_date: lease.start_date.to_string(),
        end_date: lease.end_date.to_string(),
        annual_rent: crate::money_text(lease.annual_rent),
        deposit,
        vat: vat_text(&lang, lease.vat_category),
        notice_days: lease.notice_days(),
        effective_date: lease.effective_date.map(|d| d.to_string()).unwrap_or_default(),
        terms: lease.terms.clone().unwrap_or_default(),
        ejari_number: lease.ejari_number.clone().unwrap_or_default(),
        ejari_expires_on: lease.ejari_expires_on.map(|d| d.to_string()).unwrap_or_default(),
        cancel_reason: lease.cancel_reason.clone().unwrap_or_default(),
        cancel_forfeit: if lease.cancel_forfeit > 0 {
            crate::money_text(lease.cancel_forfeit)
        } else {
            String::new()
        },
        cancel_forfeit_clause: lease.cancel_forfeit_clause.clone().unwrap_or_default(),
        follow_up_until: lease.follow_up_until.map(|d| d.to_string()).unwrap_or_default(),
        notice_deadline: if running {
            renewal::notice_deadline(&lease).to_string()
        } else {
            String::new()
        },
        renewal_decision: text(
            &lang,
            &format!("renewal.decision_{}", lease.renewal_decision.as_str()),
        ),
        notice_of_changes,
        current_terms_only: if lease.current_terms_only {
            text(&lang, "renewal.current_terms_only_yes")
        } else {
            String::new()
        },
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        lease: view,
        status: text(&lang, &format!("leases.status.{}", lease.status.as_str())),
        status_kind: lease_kind(lease.status),
        charges,
        cheques: cheque_rows,
        versions,
        approvals: approval_cards,
        timeline,
        pending,
        can_submit: offers(&actions, SUBMIT),
        can_activate: offers(&actions, ACTIVATE) && !pending,
        can_cancel: offers(&actions, CANCEL) && !pending,
        can_extend: lease.status == LeaseStatus::PendingSignature
            && user.has_permission_in(permissions::LEASES_MANAGE, &scope),
        can_decide: manages && in_window,
        can_send_notice: manages
            && in_window
            && lease.renewal_decision == RenewalDecision::ChangedTerms
            && delivered.is_none(),
        can_waive: activates
            && in_progress
            && !lease.is_follow_up()
            && !lease.deposit_waived
            && lease.deposit_amount > 0,
        can_ejari: activates && LeaseStatus::RUNNING.contains(&lease.status),
        can_version,
        can_cheques: creates
            && !matches!(
                lease.status,
                LeaseStatus::Cancelled
                    | LeaseStatus::Terminated
                    | LeaseStatus::Renewed
                    | LeaseStatus::Amended
            ),
        today: wl_kernel::documents::today().to_string(),
    }))
}

/// The form of a new lease.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct LeaseForm {
    /// The tenant's party number.
    #[serde(default)]
    pub tenant: String,
    /// The first day of the term.
    #[serde(default)]
    pub start_date: String,
    /// The last day of the term.
    #[serde(default)]
    pub end_date: String,
    /// The annual rent in dirhams.
    #[serde(default)]
    pub annual_rent: String,
    /// The deposit in dirhams.
    #[serde(default)]
    pub deposit: String,
    /// The number of rent instalments.
    #[serde(default)]
    pub instalments: String,
    /// The due date of the first instalment.
    #[serde(default)]
    pub first_due: String,
    /// The contract's notice period in days.
    #[serde(default)]
    pub notice_days: String,
    /// The clauses.
    #[serde(default)]
    pub terms: String,
}

/// A required date, a 422 naming the field when it is blank or malformed.
fn required_date(value: &str, field: &str) -> Result<NaiveDate> {
    parse_date(value).ok_or_else(|| {
        Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{field}: YYYY-MM-DD"),
        )
    })
}

/// A required amount in dirhams.
fn required_amount(value: &str, field: &str) -> Result<i64> {
    parse_aed(value).ok_or_else(|| {
        Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{field}: AED 0.00"),
        )
    })
}

/// An optional amount: blank is 0.
fn optional_amount(value: &str, field: &str) -> Result<i64> {
    if value.trim().is_empty() {
        return Ok(0);
    }
    required_amount(value, field)
}

/// A whole number, a 422 naming the field when it is not one.
fn whole<T: std::str::FromStr>(value: &str, field: &str) -> Result<T> {
    value.trim().parse().map_err(|_| {
        Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("{field}: 1, 2, 3 ..."),
        )
    })
}

/// `POST /property/units/{id}/leases`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(unit_id): Path<i64>,
    Form(form): Form<LeaseForm>,
) -> Result<Response> {
    let unit = Unit::find(&state.db, unit_id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_CREATE, &unit.scope()) {
        return Err(Error::Forbidden);
    }
    let mut new = NewLease::new(
        unit_id,
        wl_kernel::PartyId(whole::<i64>(&form.tenant, "tenant")?),
        required_date(&form.start_date, "start_date")?,
        required_date(&form.end_date, "end_date")?,
        required_amount(&form.annual_rent, "annual_rent")?,
        whole::<u32>(&form.instalments, "instalments")?,
    )
    .deposit(optional_amount(&form.deposit, "deposit")?);
    if !form.first_due.trim().is_empty() {
        new = new.first_due(required_date(&form.first_due, "first_due")?);
    }
    if !form.notice_days.trim().is_empty() {
        new.contract_notice_days = Some(whole::<i32>(&form.notice_days, "notice_days")?);
    }
    new.terms = Some(form.terms);
    let lease = leases::create_draft(&state, Some(user.id), new).await?;
    Ok((
        Toast::success(text(&lang, "leases.created")),
        renox::HxRedirect(format!("/property/leases/{}", lease.id)),
    )
        .into_response())
}

/// The form of a renewal or an amendment.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct VersionForm {
    /// `renewal` or `amendment`.
    #[serde(default)]
    pub kind: String,
    /// The first day of the term.
    #[serde(default)]
    pub start_date: String,
    /// The last day of the term.
    #[serde(default)]
    pub end_date: String,
    /// The annual rent in dirhams.
    #[serde(default)]
    pub annual_rent: String,
    /// The deposit in dirhams.
    #[serde(default)]
    pub deposit: String,
    /// The number of rent instalments.
    #[serde(default)]
    pub instalments: String,
    /// The day an amendment applies from.
    #[serde(default)]
    pub effective_date: String,
}

/// `POST /property/leases/{id}/versions`.
async fn create_version(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<VersionForm>,
) -> Result<Response> {
    let previous = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_CREATE, &previous.scope()) {
        return Err(Error::Forbidden);
    }
    let kind: VersionKind = form
        .kind
        .trim()
        .parse()
        .map_err(|_| Error::BadRequest("kind".to_owned()))?;
    let effective = if form.effective_date.trim().is_empty() {
        None
    } else {
        Some(required_date(&form.effective_date, "effective_date")?)
    };
    let new = NewVersion {
        kind,
        start_date: required_date(&form.start_date, "start_date")?,
        end_date: required_date(&form.end_date, "end_date")?,
        annual_rent: required_amount(&form.annual_rent, "annual_rent")?,
        deposit_amount: optional_amount(&form.deposit, "deposit")?,
        instalments: whole::<u32>(&form.instalments, "instalments")?,
        first_due: None,
        effective_date: effective,
        terms: None,
    };
    let lease = leases::create_version(&state, Some(user.id), id, new).await?;
    Ok((
        Toast::success(text(&lang, "leases.version_created")),
        renox::HxRedirect(format!("/property/leases/{}", lease.id)),
    )
        .into_response())
}

/// `POST /property/leases/{id}/submit`.
async fn submit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    leases::submit(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((Toast::success(text(&lang, "leases.submitted")), renox::HxRefresh).into_response())
}

/// `POST /property/leases/{id}/activate`.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    leases::activate(&state, &user, id)
        .await
        .map_err(TransitionError::into_error)?;
    Ok((Toast::success(text(&lang, "leases.activated")), renox::HxRefresh).into_response())
}

/// The form of a cancellation.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CancelForm {
    /// Why the prospect backs out.
    #[serde(default)]
    pub reason: String,
    /// The part of the deposit forfeited, in dirhams.
    #[serde(default)]
    pub forfeit: String,
    /// The clause of the terms that allows it.
    #[serde(default)]
    pub clause: String,
}

/// `POST /property/leases/{id}/cancel`: cancels at once when nothing was
/// received, else sends the cancellation to a property manager.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CancelForm>,
) -> Result<Response> {
    let outcome = cancellation::request(
        &state,
        &user,
        id,
        CancelRequest {
            reason: form.reason,
            forfeit: optional_amount(&form.forfeit, "forfeit")?,
            clause: Some(form.clause).filter(|c| !c.trim().is_empty()),
        },
    )
    .await
    .map_err(TransitionError::into_error)?;
    let key = if outcome.is_pending() {
        "cancel.sent"
    } else {
        "cancel.done"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}

/// The form that extends the follow-up of a lease waiting for its signature.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ExtendForm {
    /// The date the follow-up is extended to.
    #[serde(default)]
    pub until: String,
}

/// `POST /property/leases/{id}/extend` (`PRP.RUL.12`).
async fn extend(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ExtendForm>,
) -> Result<Response> {
    let until = required_date(&form.until, "until")?;
    chase::extend(&state, &user, id, until).await?;
    Ok((Toast::success(text(&lang, "chase.extended_done")), renox::HxRefresh).into_response())
}

/// The form of the renewal decision.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DecisionForm {
    /// `undecided`, `same_terms`, `changed_terms` or `not_renewing`.
    #[serde(default)]
    pub decision: String,
}

/// `POST /property/leases/{id}/renewal-decision` (`PRP.RUL.01`).
async fn renewal_decision(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DecisionForm>,
) -> Result<Response> {
    let decision: RenewalDecision = form
        .decision
        .trim()
        .parse()
        .map_err(|_| Error::BadRequest("decision".to_owned()))?;
    renewal::set_decision(&state, &user, id, decision).await?;
    Ok((Toast::success(text(&lang, "renewal.decided")), renox::HxRefresh).into_response())
}

/// `POST /property/leases/{id}/changed-terms-notice` (`PRP.RUL.09`).
async fn changed_terms_notice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    renewal::send_changed_terms(&state, &user, id).await?;
    Ok((Toast::success(text(&lang, "renewal.notice_sent")), renox::HxRefresh).into_response())
}

/// `POST /property/leases/{id}/waive-deposit`.
async fn waive_deposit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    leases::waive_deposit(&state, &user, id).await?;
    Ok((Toast::success(text(&lang, "leases.deposit_waived_done")), renox::HxRefresh).into_response())
}

/// The form of the Ejari registration.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct EjariForm {
    /// The registration number.
    #[serde(default)]
    pub number: String,
    /// The day it was registered.
    #[serde(default)]
    pub registered_on: String,
    /// The day it expires.
    #[serde(default)]
    pub expires_on: String,
}

/// `POST /property/leases/{id}/ejari`.
async fn ejari(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<EjariForm>,
) -> Result<Response> {
    let expires_on = if form.expires_on.trim().is_empty() {
        None
    } else {
        Some(required_date(&form.expires_on, "expires_on")?)
    };
    leases::record_ejari(
        &state,
        &user,
        id,
        EjariInput {
            number: form.number,
            registered_on: required_date(&form.registered_on, "registered_on")?,
            expires_on,
        },
    )
    .await?;
    Ok((Toast::success(text(&lang, "ejari.recorded_done")), renox::HxRefresh).into_response())
}

/// The form that records the post-dated cheques.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ChequesForm {
    /// The bank the cheques are drawn on.
    #[serde(default)]
    pub bank: String,
    /// One cheque a line: `number | YYYY-MM-DD | amount | instalment`.
    #[serde(default)]
    pub cheques: String,
}

/// The cheques of `text`, one `number | date | amount | instalment` per line
/// (the instalment, from 1, is optional), linked to the rent charges of
/// `charges` by their order.
pub fn parse_cheque_lines(
    text: &str,
    charges: &[LeaseCharge],
) -> std::result::Result<Vec<ChequeEntry>, String> {
    let rent: Vec<&LeaseCharge> = charges
        .iter()
        .filter(|c| c.charge_type == RENT && c.status != ChargeStatus::Cancelled)
        .collect();
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let n = index + 1;
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        if parts.len() < 3 {
            return Err(format!("Line {n}: number | YYYY-MM-DD | amount | instalment"));
        }
        let date = parse_date(parts[1]).ok_or_else(|| format!("Line {n}: the date is YYYY-MM-DD"))?;
        let amount = parse_aed(parts[2]).ok_or_else(|| format!("Line {n}: the amount is AED 0.00"))?;
        let charge_id = match parts.get(3).filter(|p| !p.is_empty()) {
            None => None,
            Some(position) => {
                let position: usize = position
                    .parse()
                    .map_err(|_| format!("Line {n}: the instalment is 1, 2, 3 ..."))?;
                let charge = rent
                    .get(position.wrapping_sub(1))
                    .ok_or_else(|| format!("Line {n}: there is no instalment {position}"))?;
                Some(charge.id)
            }
        };
        entries.push(ChequeEntry::new(parts[0], date, amount, charge_id));
    }
    Ok(entries)
}

/// `POST /property/leases/{id}/cheques`.
async fn cheques(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ChequesForm>,
) -> Result<Response> {
    let lease = Lease::find(&state.db, id).await?.ok_or(Error::NotFound)?;
    if !user.has_permission_in(permissions::LEASES_CREATE, &lease.scope()) {
        return Err(Error::Forbidden);
    }
    let charges = lease.charges(&state.db).await?;
    let entries = parse_cheque_lines(&form.cheques, &charges)
        .map_err(|message| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message))?;
    register_cheques(&state, Some(user.id), &lease, &form.bank, entries).await?;
    Ok((Toast::success(text(&lang, "cheques.recorded")), renox::HxRefresh).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn charge(id: i64, status: ChargeStatus) -> LeaseCharge {
        LeaseCharge {
            id,
            charge_type: RENT.to_owned(),
            status,
            ..Default::default()
        }
    }

    #[test]
    fn cheque_lines_are_linked_to_the_rent_charges_in_order() {
        let charges = [
            charge(10, ChargeStatus::Cancelled),
            charge(11, ChargeStatus::Planned),
            charge(12, ChargeStatus::Planned),
        ];
        let text = "101 | 2027-01-01 | 25,000.00 | 1\n\n102 | 2027-04-01 | 25000 | 2\n103 | 2027-07-01 | 25000";
        let entries = parse_cheque_lines(text, &charges).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].charge_id, Some(11));
        assert_eq!(entries[1].charge_id, Some(12));
        assert_eq!(entries[2].charge_id, None);
        assert_eq!(entries[0].amount, 2_500_000);
        assert_eq!(entries[0].number, "101");
    }

    #[test]
    fn a_bad_line_names_its_number() {
        let charges = [charge(11, ChargeStatus::Planned)];
        for (text, line) in [
            ("101 | 2027-01-01", "Line 1"),
            ("101 | 01/01/2027 | 100", "Line 1"),
            ("\n101 | 2027-01-01 | abc", "Line 2"),
            ("101 | 2027-01-01 | 100 | 5", "Line 1"),
        ] {
            let error = parse_cheque_lines(text, &charges).unwrap_err();
            assert!(error.starts_with(line), "{error}");
        }
    }
}
