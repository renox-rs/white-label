//! The fleet contract pages (08 §4.5, `AUT.FLT.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/fleet` | `auto.fleet_contracts` | `fleet_contracts.manage`, `job_cards.advise` or `invoices.issue` |
//! | `POST /auto/fleet` | `auto.fleet_create` | `fleet_contracts.manage` |
//! | `GET /auto/fleet/{id}` | `auto.fleet_contract` | as the list |
//! | `POST /auto/fleet/{id}` | `auto.fleet_update` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/document` | `auto.fleet_document` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/vehicles` | `auto.fleet_vehicle_add` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/vehicles/remove` | `auto.fleet_vehicle_remove` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/plans` | `auto.fleet_plan_add` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/plans/{plan_id}/remove` | `auto.fleet_plan_remove` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/activate` | `auto.fleet_activate` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/suspend` | `auto.fleet_suspend` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/resume` | `auto.fleet_resume` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/renew` | `auto.fleet_renew` | `fleet_contracts.manage` |
//! | `POST /auto/fleet/{id}/invoice` | `auto.fleet_invoice` | `fleet_contracts.manage` or `invoices.issue` |
//! | `POST /auto/fleet/jobs/{job_card_id}/po` | `auto.fleet_po` | `fleet_contracts.manage` or `job_cards.advise` |
//! | `GET /auto/fleet-vehicle-options` | `auto.fleet_vehicle_options` | `fleet_contracts.manage` |
//!
//! The fleet and its contact are picked from the parties and the vehicles from
//! the register, never typed as ids.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    Texts, invalid, parse_money, parse_number, parse_optional_date, parse_optional_id, party_name,
    party_of, require, text,
};
use crate::fleet_billing;
use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::fleet_contracts::{self, NewContract};
use crate::fleet_invoice::{FleetInvoice, KIND_MONTHLY};
use crate::fleet_job::FleetJob;
use crate::fleet_services::{self, NewPlan};
use crate::job_card::JobCard;
use crate::money_text;
use crate::permissions;
use crate::texts;
use crate::vehicle::Vehicle;
use crate::vehicles;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/fleet", index)
        .name("auto.fleet_contracts")
        .post("/auto/fleet", create)
        .name("auto.fleet_create")
        .get("/auto/fleet-vehicle-options", vehicle_options)
        .name("auto.fleet_vehicle_options")
        .post("/auto/fleet/plans/{plan_id}/remove", plan_remove)
        .name("auto.fleet_plan_remove")
        .post("/auto/fleet/jobs/{job_card_id}/po", set_po)
        .name("auto.fleet_po")
        .get("/auto/fleet/{id}", show)
        .name("auto.fleet_contract")
        .post("/auto/fleet/{id}", update)
        .name("auto.fleet_update")
        .post("/auto/fleet/{id}/document", document)
        .name("auto.fleet_document")
        .post("/auto/fleet/{id}/vehicles", vehicle_add)
        .name("auto.fleet_vehicle_add")
        .post("/auto/fleet/{id}/vehicles/remove", vehicle_remove)
        .name("auto.fleet_vehicle_remove")
        .post("/auto/fleet/{id}/plans", plan_add)
        .name("auto.fleet_plan_add")
        .post("/auto/fleet/{id}/activate", activate)
        .name("auto.fleet_activate")
        .post("/auto/fleet/{id}/suspend", suspend)
        .name("auto.fleet_suspend")
        .post("/auto/fleet/{id}/resume", resume)
        .name("auto.fleet_resume")
        .post("/auto/fleet/{id}/renew", renew)
        .name("auto.fleet_renew")
        .post("/auto/fleet/{id}/invoice", invoice)
        .name("auto.fleet_invoice")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: FleetStatus) -> String {
    text(lang, &format!("fleet.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: FleetStatus) -> &'static str {
    match status {
        FleetStatus::Active => "success",
        FleetStatus::Suspended | FleetStatus::RenewalDue => "warning",
        FleetStatus::Draft | FleetStatus::Ended => "info",
    }
}

/// Whether `user` may see the contracts.
fn may_view(user: &AuthUser) -> bool {
    user.has_permission(permissions::FLEET_CONTRACTS_MANAGE)
        || user.has_permission(permissions::JOB_CARDS_ADVISE)
        || user.has_permission(permissions::INVOICES_ISSUE)
}

/// Success toast and a refresh of the page.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// What the list can be narrowed by.
#[derive(Deserialize, Default, Debug, Clone)]
pub struct Filter {
    /// Text in the number or the title.
    #[serde(default)]
    q: String,
    /// `draft`, `active` or `ended`; blank for every contract.
    #[serde(default)]
    tab: String,
}

impl Filter {
    fn href(&self) -> String {
        if self.tab.is_empty() {
            "/auto/fleet".to_owned()
        } else {
            format!("/auto/fleet?tab={}", self.tab)
        }
    }

    fn with_tab(&self, tab: &str) -> Filter {
        Filter {
            tab: tab.to_owned(),
            ..self.clone()
        }
    }

    /// Whether a contract in `status` belongs to the tab.
    fn takes(&self, status: FleetStatus) -> bool {
        match self.tab.as_str() {
            "draft" => status == FleetStatus::Draft,
            "active" => status.is_in_force(),
            "ended" => status == FleetStatus::Ended,
            _ => true,
        }
    }
}

/// One contract of the list.
#[derive(Serialize, Debug, Clone)]
pub struct ContractRow {
    /// The primary key.
    pub id: i64,
    /// `FC-0007`.
    pub number: String,
    /// What the contract is called.
    pub title: String,
    /// The fleet's name.
    pub fleet: String,
    /// The term.
    pub term: String,
    /// How many vehicles are under it.
    pub vehicles: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// The term as people read it: `2026-01-01 to 2026-12-31`.
fn term_text(lang: &Lang, contract: &FleetContract) -> String {
    texts::t_with(
        lang,
        "auto.fleet.term_text",
        &[("from", &contract.start_date), ("to", &contract.end_date)],
    )
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/fleet_contracts.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<ContractRow>,
    tabs: Vec<(String, String)>,
    tab_current: String,
    tab: String,
    q: String,
    can_create: bool,
}

/// `GET /auto/fleet`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
    let found = fleet_contracts::search(&state.db, &filter.q, None).await?;
    let mut rows = Vec::new();
    for contract in found.into_iter().filter(|c| filter.takes(c.status)) {
        let count = fleet_contracts::vehicles_of(&state.db, contract.id)
            .await?
            .len();
        rows.push(ContractRow {
            id: contract.id,
            number: contract.number(),
            title: contract.title.clone(),
            fleet: party_name(&state, &lang.locale, contract.fleet_party_id).await,
            term: term_text(&lang, &contract),
            vehicles: count.to_string(),
            status: status_text(&lang, contract.status),
            status_kind: status_kind(contract.status),
        });
    }
    let tabs = vec![
        (filter.with_tab("").href(), text(&lang, "fleet.tab.all")),
        (
            filter.with_tab("active").href(),
            text(&lang, "fleet.tab.active"),
        ),
        (
            filter.with_tab("draft").href(),
            text(&lang, "fleet.tab.draft"),
        ),
        (
            filter.with_tab("ended").href(),
            text(&lang, "fleet.tab.ended"),
        ),
    ];
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        tabs,
        tab_current: filter.href(),
        tab: filter.tab.clone(),
        q: filter.q.clone(),
        can_create: user.has_permission(permissions::FLEET_CONTRACTS_MANAGE),
    }))
}

/// `GET /auto/fleet-vehicle-options`: the vehicles the picker offers.
async fn vehicle_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, permissions::FLEET_CONTRACTS_MANAGE)?;
    let found = if query.is_lookup() {
        let mut found = Vec::new();
        for id in query.values_as::<i64>() {
            found.extend(Vehicle::find(&state.db, id).await?);
        }
        found
    } else {
        let mut all = vehicles::search(&state.db, &query.q).await?;
        all.truncate(20);
        all
    };
    Ok(Json(
        found
            .into_iter()
            .map(|vehicle| renox::select::SelectOption::new(vehicle.id, vehicle.title()))
            .collect(),
    ))
}

/// The form that agrees or changes a contract.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ContractForm {
    /// The fleet, picked from the parties.
    #[serde(default)]
    pub fleet: String,
    /// The fleet contact, picked from the parties.
    #[serde(default)]
    pub contact: String,
    /// What the contract is called.
    #[serde(default)]
    pub title: String,
    /// The first day of the term.
    #[serde(default)]
    pub start_date: String,
    /// The last day of the term.
    #[serde(default)]
    pub end_date: String,
    /// The hourly labour rate in dirhams.
    #[serde(default)]
    pub labour_rate: String,
    /// The part price tier.
    #[serde(default)]
    pub price_tier: String,
    /// What is included.
    #[serde(default)]
    pub inclusions: String,
    /// The notice period in days.
    #[serde(default)]
    pub notice_days: String,
    /// The early termination fee in dirhams.
    #[serde(default)]
    pub fee: String,
    /// The renewal window in days.
    #[serde(default)]
    pub renewal_window: String,
}

/// A day typed in a form that is required.
fn required_date(text_value: &str, key: &str) -> Result<NaiveDate> {
    parse_optional_date(text_value)?.ok_or_else(|| invalid(&texts::english(key, &[])))
}

/// The contract the form asks for.
async fn new_contract(
    state: &AppState,
    form: &ContractForm,
    current_fleet: Option<i64>,
) -> Result<NewContract> {
    let fleet = match current_fleet {
        Some(fleet) if form.fleet.trim().is_empty() => fleet,
        _ => party_of(state, &form.fleet).await?,
    };
    let mut new = NewContract::new(
        fleet,
        &form.title,
        required_date(&form.start_date, "auto.fleet.problem.start")?,
        required_date(&form.end_date, "auto.fleet.problem.end")?,
    )
    .labour_rate(parse_money(&form.labour_rate)?);
    if !form.contact.trim().is_empty() {
        new = new.contact(party_of(state, &form.contact).await?);
    }
    if !form.price_tier.trim().is_empty() {
        new = new.tier(&form.price_tier);
    }
    if !form.inclusions.trim().is_empty() {
        new = new.inclusions(&form.inclusions);
    }
    if !form.notice_days.trim().is_empty() {
        new = new.notice_days(parse_number(&form.notice_days)?);
    }
    new = new.fee(parse_money(&form.fee)?);
    if !form.renewal_window.trim().is_empty() {
        new = new.renewal_window(parse_number(&form.renewal_window)?);
    }
    Ok(new)
}

/// `POST /auto/fleet`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<ContractForm>,
) -> Result<Response> {
    require(&user, permissions::FLEET_CONTRACTS_MANAGE)?;
    let new = new_contract(&state, &form, None).await?;
    let contract = fleet_contracts::create(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "fleet.created")),
        renox::HxRedirect(format!("/auto/fleet/{}", contract.id)),
    )
        .into_response())
}

/// One vehicle under the contract.
#[derive(Serialize, Debug, Clone)]
pub struct VehicleRow {
    /// The primary key.
    pub id: i64,
    /// The plate.
    pub plate: String,
    /// What it is.
    pub vehicle: String,
    /// The odometer.
    pub odometer: String,
}

/// One service plan.
#[derive(Serialize, Debug, Clone)]
pub struct PlanRow {
    /// The primary key.
    pub id: i64,
    /// The plate of the vehicle.
    pub plate: String,
    /// The service.
    pub service: String,
    /// `every 10,000 km or 6 months`.
    pub interval: String,
    /// The day and reading the next service falls due at.
    pub next_due: String,
    /// The booking asked for it, or a dash.
    pub booking: String,
    /// Whether the plan is in use, in words.
    pub active: String,
}

/// One job card under the contract.
#[derive(Serialize, Debug, Clone)]
pub struct JobRow {
    /// The job card.
    pub job_card_id: i64,
    /// `JOB-00007`.
    pub number: String,
    /// The plate.
    pub plate: String,
    /// The PO reference, or a dash.
    pub po: String,
    /// The raw PO reference for the form.
    pub raw_po: String,
    /// The job card's status in words.
    pub status: String,
    /// The invoice that billed it, or a dash.
    pub invoice: String,
    /// Whether the PO can still be set.
    pub open: bool,
}

/// One period billed.
#[derive(Serialize, Debug, Clone)]
pub struct InvoiceRow {
    /// `2026-10-01 to 2026-10-31`.
    pub period: String,
    /// Monthly or final, in words.
    pub kind: String,
    /// How many jobs it lists.
    pub jobs: String,
    /// The total before VAT.
    pub net: String,
    /// The invoice, or a dash when there was nothing to bill.
    pub invoice: String,
}

/// What the page of one contract needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/fleet_contract.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    title: String,
    status: String,
    status_kind: &'static str,
    fleet: String,
    contact: String,
    term: String,
    labour_rate: String,
    price_tier: String,
    inclusions: String,
    notice: String,
    fee: String,
    renewal_window: String,
    signed: String,
    invoiced_through: String,
    suspended_reason: String,
    termination: String,
    termination_scheduled: bool,
    vehicles: Vec<VehicleRow>,
    plans: Vec<PlanRow>,
    bookings: Vec<PlanRow>,
    jobs: Vec<JobRow>,
    invoices: Vec<InvoiceRow>,
    vehicle_options: Vec<[String; 2]>,
    can_edit: bool,
    can_activate: bool,
    can_suspend: bool,
    can_resume: bool,
    can_renew: bool,
    can_invoice: bool,
    can_po: bool,
    is_draft: bool,
    is_ended: bool,
    raw_fleet: String,
    raw_contact: String,
    raw_start: String,
    raw_end: String,
    raw_labour_rate: String,
    raw_tier: String,
    raw_inclusions: String,
    raw_notice: String,
    raw_fee: String,
    raw_window: String,
    fleet_options: Vec<[String; 2]>,
    contact_options: Vec<[String; 2]>,
    previous_from: String,
    previous_to: String,
}

/// Dirhams as typed in a form: `250.00`; blank for 0.
fn typed_money(fils: i64) -> String {
    if fils == 0 {
        String::new()
    } else {
        format!("{}.{:02}", fils / 100, fils % 100)
    }
}

/// `every 10,000 km or 6 months`.
fn interval_text(lang: &Lang, plan: &crate::fleet_service::FleetService) -> String {
    let km = plan.interval_km.map(|km| {
        texts::t_with(
            lang,
            "auto.fleet.every_km",
            &[("km", &crate::number_text(km))],
        )
    });
    let months = plan
        .interval_months
        .map(|months| texts::t_with(lang, "auto.fleet.every_months", &[("months", &months)]));
    [km, months]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `2026-11-10 · 60,000 km`.
fn due_text(lang: &Lang, plan: &crate::fleet_service::FleetService) -> String {
    let date = plan.next_due_date.map(|d| d.to_string());
    let km = plan
        .next_due_km
        .map(|km| format!("{} km", crate::number_text(km)));
    let joined = [date, km]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    if joined.is_empty() {
        text(lang, "none")
    } else {
        joined
    }
}

/// The plan rows of `plans`.
async fn plan_rows(
    state: &AppState,
    lang: &Lang,
    plans: &[crate::fleet_service::FleetService],
) -> Result<Vec<PlanRow>> {
    let mut rows = Vec::new();
    for plan in plans {
        let plate = Vehicle::find(&state.db, plan.vehicle_id)
            .await?
            .map_or_else(String::new, |v| v.plate());
        rows.push(PlanRow {
            id: plan.id,
            plate,
            service: plan.service_code.clone(),
            interval: interval_text(lang, plan),
            next_due: due_text(lang, plan),
            booking: plan.booking_id.map_or_else(
                || text(lang, "none"),
                |booking| match plan.requested_for_date {
                    Some(day) => format!("#{booking} · {day}"),
                    None => format!("#{booking}"),
                },
            ),
            active: text(
                lang,
                if plan.active {
                    "fleet.plan_active"
                } else {
                    "fleet.plan_stopped"
                },
            ),
        });
    }
    Ok(rows)
}

/// `GET /auto/fleet/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    if !may_view(&user) {
        return Err(Error::Forbidden);
    }
    let contract = FleetContract::find_or_404(&state.db, id).await?;
    let none = text(&lang, "none");
    let manage = user.has_permission(permissions::FLEET_CONTRACTS_MANAGE);
    let status = contract.status;
    let under = fleet_contracts::vehicles_of(&state.db, id).await?;
    let plans = fleet_services::plans_of(&state.db, id).await?;
    let waiting = fleet_services::bookings_to_confirm(&state.db, id).await?;
    let mut vehicle_rows = Vec::new();
    let mut vehicle_options = Vec::new();
    for vehicle in &under {
        vehicle_rows.push(VehicleRow {
            id: vehicle.id,
            plate: vehicle.plate(),
            vehicle: super::vehicles::vehicle_text(vehicle),
            odometer: super::vehicles::km_text(vehicle.odometer_km),
        });
        vehicle_options.push([vehicle.id.to_string(), vehicle.title()]);
    }
    let mut job_rows = Vec::new();
    for job in FleetJob::of_contract(&state.db, id).await? {
        let card = JobCard::find(&state.db, job.job_card_id).await?;
        let plate = Vehicle::find(&state.db, job.vehicle_id)
            .await?
            .map_or_else(String::new, |v| v.plate());
        job_rows.push(JobRow {
            job_card_id: job.job_card_id,
            number: card
                .as_ref()
                .map_or_else(|| format!("#{}", job.job_card_id), JobCard::number),
            plate,
            po: job.po_reference.clone().unwrap_or_else(|| none.clone()),
            raw_po: job.po_reference.clone().unwrap_or_default(),
            status: card.as_ref().map_or_else(
                || none.clone(),
                |card| super::job_cards::status_text(&lang, card.status),
            ),
            invoice: job
                .invoice_id
                .map_or_else(|| none.clone(), |i| format!("#{i}")),
            open: job.invoice_id.is_none(),
        });
    }
    let invoice_rows: Vec<InvoiceRow> = FleetInvoice::of_contract(&state.db, id)
        .await?
        .iter()
        .map(|invoice| InvoiceRow {
            period: texts::t_with(
                &lang,
                "auto.fleet.term_text",
                &[
                    ("from", &invoice.period_start_date),
                    ("to", &invoice.period_end_date),
                ],
            ),
            kind: text(&lang, &format!("fleet.invoice_kind.{}", invoice.kind)),
            jobs: invoice.jobs.to_string(),
            net: money_text(invoice.net_total),
            invoice: invoice
                .invoice_id
                .map_or_else(|| none.clone(), |i| format!("#{i}")),
        })
        .collect();
    let (from, to) = fleet_billing::previous_month(wl_kernel::documents::today());
    let fleet_name = party_name(&state, &lang.locale, contract.fleet_party_id).await;
    let contact_name = match contract.contact_party_id {
        Some(contact) => party_name(&state, &lang.locale, contact).await,
        None => none.clone(),
    };
    let termination = match (contract.notice_date, contract.effective_date) {
        (Some(notice), Some(effective)) => texts::t_with(
            &lang,
            "auto.fleet.termination_text",
            &[("notice", &notice), ("effective", &effective)],
        ),
        _ => none.clone(),
    };
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: contract.id,
        number: contract.number(),
        title: contract.title.clone(),
        status: status_text(&lang, status),
        status_kind: status_kind(status),
        fleet: fleet_name.clone(),
        contact: contact_name.clone(),
        term: term_text(&lang, &contract),
        labour_rate: money_text(contract.labour_rate),
        price_tier: contract.price_tier.clone().unwrap_or_else(|| none.clone()),
        inclusions: contract.inclusions.clone().unwrap_or_else(|| none.clone()),
        notice: texts::t_with(
            &lang,
            "auto.insurers.days",
            &[("days", &contract.notice_period_days)],
        ),
        fee: money_text(contract.early_termination_fee),
        renewal_window: texts::t_with(
            &lang,
            "auto.insurers.days",
            &[("days", &contract.renewal_window_days)],
        ),
        signed: contract
            .signed_document
            .clone()
            .unwrap_or_else(|| text(&lang, "fleet.unsigned")),
        invoiced_through: contract
            .invoiced_through_date
            .map_or_else(|| none.clone(), |d| d.to_string()),
        suspended_reason: contract.suspended_reason.clone().unwrap_or_default(),
        termination,
        termination_scheduled: contract.termination_scheduled,
        vehicles: vehicle_rows,
        plans: plan_rows(&state, &lang, &plans).await?,
        bookings: plan_rows(&state, &lang, &waiting).await?,
        jobs: job_rows,
        invoices: invoice_rows,
        vehicle_options,
        can_edit: manage && status != FleetStatus::Ended,
        can_activate: manage && status == FleetStatus::Draft,
        can_suspend: manage && matches!(status, FleetStatus::Active | FleetStatus::RenewalDue),
        can_resume: manage && status == FleetStatus::Suspended,
        can_renew: manage && status == FleetStatus::RenewalDue,
        can_invoice: (manage || user.has_permission(permissions::INVOICES_ISSUE))
            && status.is_in_force(),
        can_po: manage || user.has_permission(permissions::JOB_CARDS_ADVISE),
        is_draft: status == FleetStatus::Draft,
        is_ended: status == FleetStatus::Ended,
        raw_fleet: contract.fleet_party_id.to_string(),
        raw_contact: contract
            .contact_party_id
            .map(|c| c.to_string())
            .unwrap_or_default(),
        raw_start: contract.start_date.to_string(),
        raw_end: contract.end_date.to_string(),
        raw_labour_rate: typed_money(contract.labour_rate),
        raw_tier: contract.price_tier.clone().unwrap_or_default(),
        raw_inclusions: contract.inclusions.clone().unwrap_or_default(),
        raw_notice: contract.notice_period_days.to_string(),
        raw_fee: typed_money(contract.early_termination_fee),
        raw_window: contract.renewal_window_days.to_string(),
        fleet_options: vec![[contract.fleet_party_id.to_string(), fleet_name]],
        contact_options: contract
            .contact_party_id
            .map(|c| vec![[c.to_string(), contact_name]])
            .unwrap_or_default(),
        previous_from: from.to_string(),
        previous_to: to.to_string(),
    }))
}

/// `POST /auto/fleet/{id}`.
async fn update(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ContractForm>,
) -> Result<Response> {
    require(&user, permissions::FLEET_CONTRACTS_MANAGE)?;
    let contract = FleetContract::find_or_404(&state.db, id).await?;
    let new = new_contract(&state, &form, Some(contract.fleet_party_id)).await?;
    fleet_contracts::update(&state, &user, id, new).await?;
    done(&lang, "fleet.saved")
}

/// The form of the signed contract.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DocumentForm {
    /// The document reference.
    #[serde(default)]
    pub document: String,
}

/// `POST /auto/fleet/{id}/document`.
async fn document(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DocumentForm>,
) -> Result<Response> {
    fleet_contracts::attach_document(&state, &user, id, &form.document).await?;
    done(&lang, "fleet.document_saved")
}

/// The form of a vehicle.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct VehicleForm {
    /// The vehicle, picked from the register or from the contract's.
    #[serde(default)]
    pub vehicle: String,
}

/// `POST /auto/fleet/{id}/vehicles`.
async fn vehicle_add(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<VehicleForm>,
) -> Result<Response> {
    let vehicle = parse_number(&form.vehicle)?;
    fleet_contracts::add_vehicle(&state, &user, id, vehicle).await?;
    done(&lang, "fleet.vehicle_added")
}

/// `POST /auto/fleet/{id}/vehicles/remove`.
async fn vehicle_remove(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<VehicleForm>,
) -> Result<Response> {
    let vehicle = parse_number(&form.vehicle)?;
    fleet_contracts::remove_vehicle(&state, &user, id, vehicle).await?;
    done(&lang, "fleet.vehicle_removed")
}

/// The form of a service plan.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PlanForm {
    /// The vehicle, picked from the contract's.
    #[serde(default)]
    pub vehicle: String,
    /// The service type code.
    #[serde(default)]
    pub service: String,
    /// The mileage between two services.
    #[serde(default)]
    pub interval_km: String,
    /// The months between two services.
    #[serde(default)]
    pub interval_months: String,
    /// The day of the last service.
    #[serde(default)]
    pub last_date: String,
    /// The odometer reading of the last service.
    #[serde(default)]
    pub last_km: String,
}

/// `POST /auto/fleet/{id}/plans`.
async fn plan_add(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<PlanForm>,
) -> Result<Response> {
    let mut new = NewPlan::new(parse_number(&form.vehicle)?, &form.service);
    if let Some(km) = parse_optional_id(&form.interval_km)? {
        new = new.every_km(km);
    }
    if let Some(months) = parse_optional_id(&form.interval_months)? {
        new = new.every_months(months);
    }
    if let Some(date) = parse_optional_date(&form.last_date)? {
        new = new.last_service(date, super::parse_km(&form.last_km)?);
    }
    fleet_services::add_plan(&state, &user, id, new).await?;
    done(&lang, "fleet.plan_added")
}

/// `POST /auto/fleet/plans/{plan_id}/remove`.
async fn plan_remove(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(plan_id): Path<i64>,
) -> Result<Response> {
    fleet_services::remove_plan(&state, &user, plan_id).await?;
    done(&lang, "fleet.plan_removed")
}

/// `POST /auto/fleet/{id}/activate`.
async fn activate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    fleet_contracts::activate(&state, &user, id).await?;
    done(&lang, "fleet.activated")
}

/// The form of a reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[serde(default)]
    pub reason: String,
}

/// `POST /auto/fleet/{id}/suspend`.
async fn suspend(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    fleet_contracts::suspend(&state, &user, id, &form.reason).await?;
    done(&lang, "fleet.suspended")
}

/// `POST /auto/fleet/{id}/resume`.
async fn resume(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    fleet_contracts::resume(&state, &user, id).await?;
    done(&lang, "fleet.resumed")
}

/// The form of a renewal.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RenewForm {
    /// The new last day of the term.
    #[serde(default)]
    pub end_date: String,
    /// The new hourly labour rate in dirhams; blank keeps the old one.
    #[serde(default)]
    pub labour_rate: String,
}

/// `POST /auto/fleet/{id}/renew`.
async fn renew(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RenewForm>,
) -> Result<Response> {
    let end = required_date(&form.end_date, "auto.fleet.problem.end")?;
    let rate = parse_money(&form.labour_rate)?;
    fleet_contracts::renew(&state, &user, id, end, (rate > 0).then_some(rate)).await?;
    done(&lang, "fleet.renewed")
}

/// The form of a period to invoice.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PeriodForm {
    /// The first day.
    #[serde(default)]
    pub from: String,
    /// The last day.
    #[serde(default)]
    pub to: String,
}

/// `POST /auto/fleet/{id}/invoice`.
async fn invoice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<PeriodForm>,
) -> Result<Response> {
    let from = required_date(&form.from, "auto.fleet.problem.period")?;
    let to = required_date(&form.to, "auto.fleet.problem.period")?;
    fleet_billing::invoice_period(&state, Some(&user), id, from, to, KIND_MONTHLY).await?;
    done(&lang, "fleet.invoiced")
}

/// The form of a PO reference.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PoForm {
    /// The fleet's PO reference.
    #[serde(default)]
    pub po: String,
}

/// `POST /auto/fleet/jobs/{job_card_id}/po`.
async fn set_po(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(job_card_id): Path<i64>,
    Form(form): Form<PoForm>,
) -> Result<Response> {
    fleet_billing::set_po(&state, &user, job_card_id, &form.po).await?;
    done(&lang, "fleet.po_saved")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tabs_split_the_contracts_by_where_they_stand() {
        let active = Filter {
            tab: "active".to_owned(),
            ..Filter::default()
        };
        assert!(active.takes(FleetStatus::Active));
        assert!(active.takes(FleetStatus::Suspended));
        assert!(active.takes(FleetStatus::RenewalDue));
        assert!(!active.takes(FleetStatus::Draft));
        assert!(!active.takes(FleetStatus::Ended));
        assert!(active.with_tab("ended").takes(FleetStatus::Ended));
        assert!(Filter::default().takes(FleetStatus::Draft));
    }

    #[test]
    fn every_status_has_a_badge_kind_the_kit_knows() {
        for status in FleetStatus::ALL {
            assert!(
                ["info", "success", "warning", "error"].contains(&status_kind(*status)),
                "{status:?}"
            );
        }
    }

    #[test]
    fn dirhams_are_typed_with_two_decimals_or_blank() {
        assert_eq!(typed_money(12_050), "120.50");
        assert_eq!(typed_money(0), "");
    }
}
