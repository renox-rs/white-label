//! The job card pages (08 `AUT.JOB.01`, `AUT.JOB.02`, `AUT.JOB.03`, `AUT.ACL.08`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /auto/job-cards` | `auto.job_cards` | `job_cards.view`, or `job_cards.work` for the assigned ones |
//! | `POST /auto/job-cards` | `auto.job_card_create` | `job_cards.advise` |
//! | `GET /auto/vehicles/options` | `auto.vehicle_options` | `job_cards.advise` |
//! | `GET /auto/technician` | `auto.technician` | `job_cards.work` |
//! | `GET /auto/job-cards/{id}` | `auto.job_card` | as the list |
//! | `GET /auto/job-cards/{id}/estimate` | `auto.job_card_estimate` | as the list |
//! | `POST /auto/job-cards/{id}/<action>` | `auto.job_card_<action>` | the action's permission |
//! | `GET /auto/approve/{token}` | `auto.approve` | the customer's link |
//! | `POST /auto/approve/{token}` | `auto.approve_answer` | the customer's link |
//! | `POST /auto/approve/{token}/decline` | `auto.approve_decline` | the customer's link |
//!
//! The list is narrowed by status with link tabs and searched by plate or job
//! card number; a technician sees only the job cards assigned to them, and no
//! contact details (`AUT.ACL.08`). The estimate sheet lists the labour and
//! the parts with their condition, the totals with VAT, the promised day and
//! the warranty, and the link the customer approves on. The customer's page
//! needs no sign-in: the token in the link is the secret.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::documents::dubai_text;

use super::stops::{self, StopView};
use super::vehicles::{HistoryRow, km_text, vehicle_text};
use super::{
    Texts, parse_km, parse_milli, parse_money, parse_number_or_zero, parse_optional_date,
    parse_optional_id, party_name, require, role_options, text, ticked, user_name,
};
use crate::approval::{ApprovalStatus, EstimateApproval, MIN_WARRANTY_DAYS};
use crate::billing;
use crate::estimate::Estimate;
use crate::job_card::{FUEL_LEVELS, JobCard, JobCardStatus, QC_KEYS};
use crate::job_cards::{
    self, Delivery, Intake, NewJobCard, NewLabour, NewPart, Promise, may_work, visible_to,
};
use crate::job_stops;
use crate::labour_line::{LabourPricing, LineStatus, WorkStatus, hours_text};
use crate::labour_operation::LabourOperation;
use crate::money_text;
use crate::part_line::PartCondition;
use crate::parts;
use crate::permissions;
use crate::texts;
use crate::vehicle::Vehicle;
use crate::vehicles;

/// The routes of the back office.
pub fn routes() -> Routes {
    Routes::new()
        .get("/auto/job-cards", index)
        .name("auto.job_cards")
        .post("/auto/job-cards", create)
        .name("auto.job_card_create")
        .get("/auto/vehicles/options", vehicle_options)
        .name("auto.vehicle_options")
        .get("/auto/technician", technician)
        .name("auto.technician")
        .get("/auto/job-cards/{id}", show)
        .name("auto.job_card")
        .get("/auto/job-cards/{id}/estimate", estimate_sheet)
        .name("auto.job_card_estimate")
        .post("/auto/job-cards/{id}/intake", intake)
        .name("auto.job_card_intake")
        .post("/auto/job-cards/{id}/technician", assign)
        .name("auto.job_card_assign")
        .post("/auto/job-cards/{id}/inspect", inspect)
        .name("auto.job_card_inspect")
        .post("/auto/job-cards/{id}/findings", findings)
        .name("auto.job_card_findings")
        .post("/auto/job-cards/{id}/labour", add_labour)
        .name("auto.job_card_labour")
        .post("/auto/job-cards/{id}/parts", add_part)
        .name("auto.job_card_part")
        .post("/auto/job-cards/{id}/labour/{line}/remove", remove_labour)
        .name("auto.job_card_labour_remove")
        .post("/auto/job-cards/{id}/parts/{line}/remove", remove_part)
        .name("auto.job_card_part_remove")
        .post("/auto/job-cards/{id}/send", send)
        .name("auto.job_card_send")
        .post("/auto/job-cards/{id}/approve", approve)
        .name("auto.job_card_approve")
        .post("/auto/job-cards/{id}/decline", decline)
        .name("auto.job_card_decline")
        .post("/auto/job-cards/{id}/void", void)
        .name("auto.job_card_void")
        .post("/auto/job-cards/{id}/wait-parts", wait_parts)
        .name("auto.job_card_wait_parts")
        .post("/auto/job-cards/{id}/labour/{line}/work", work)
        .name("auto.job_card_work")
        .post("/auto/job-cards/{id}/parts/{line}/issue", issue)
        .name("auto.job_card_part_issue")
        .post("/auto/job-cards/{id}/parts/{line}/fit", fit)
        .name("auto.job_card_part_fit")
        .post("/auto/job-cards/{id}/parts/{line}/return", give_back)
        .name("auto.job_card_part_return")
        .post("/auto/job-cards/{id}/finish", finish)
        .name("auto.job_card_finish")
        .post("/auto/job-cards/{id}/qc", save_qc)
        .name("auto.job_card_qc")
        .post("/auto/job-cards/{id}/pass-qc", pass_qc)
        .name("auto.job_card_pass_qc")
        .post("/auto/job-cards/{id}/fail-qc", fail_qc)
        .name("auto.job_card_fail_qc")
        .post("/auto/job-cards/{id}/deliver", deliver)
        .name("auto.job_card_deliver")
        .post("/auto/job-cards/{id}/invoice", invoice)
        .name("auto.job_card_invoice")
        .require_auth()
}

/// The routes the customer opens from the link: no sign-in.
pub fn public_routes() -> Routes {
    Routes::new()
        .get("/auto/approve/{token}", approve_page)
        .name("auto.approve")
        .post("/auto/approve/{token}", approve_answer)
        .name("auto.approve_answer")
        .post("/auto/approve/{token}/decline", decline_answer)
        .name("auto.approve_decline")
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: JobCardStatus) -> String {
    text(lang, &format!("jobs.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: JobCardStatus) -> &'static str {
    match status {
        JobCardStatus::AwaitingApproval | JobCardStatus::AwaitingParts => "warning",
        JobCardStatus::Ready | JobCardStatus::Delivered => "success",
        JobCardStatus::CheckedIn
        | JobCardStatus::Inspecting
        | JobCardStatus::InProgress
        | JobCardStatus::QualityCheck
        | JobCardStatus::Declined
        | JobCardStatus::Cancelled
        | JobCardStatus::Voided => "info",
    }
}

/// The standing of a line with the customer, in words.
fn line_text(lang: &Lang, status: LineStatus) -> String {
    text(lang, &format!("jobs.line.{}", status.as_str()))
}

/// The badge kind of a line's standing.
fn line_kind(status: LineStatus) -> &'static str {
    match status {
        LineStatus::Approved => "success",
        LineStatus::Sent => "warning",
        LineStatus::Declined => "error",
        LineStatus::Draft | LineStatus::Removed => "info",
    }
}

/// The toast and refresh that answer an action.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// One job card of a list.
#[derive(Serialize, Debug, Clone)]
pub struct JobRow {
    /// The primary key.
    pub id: i64,
    /// `JOB-00007`.
    pub number: String,
    /// The day it was checked in.
    pub date: String,
    /// `Dubai A 12345`.
    pub plate: String,
    /// `Toyota Hilux 2022`.
    pub vehicle: String,
    /// The customer's name, or a dash for a technician.
    pub customer: String,
    /// The technician's name, or a dash.
    pub technician: String,
    /// The complaint.
    pub complaint: String,
    /// The estimate with VAT, `AED 1,250.00`.
    pub total: String,
    /// The status in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// The rows of `cards`, newest first.
async fn rows_of(
    state: &AppState,
    lang: &Lang,
    user: &AuthUser,
    cards: &[JobCard],
) -> Result<Vec<JobRow>> {
    let see_customer = user.has_permission(permissions::JOB_CARDS_VIEW);
    let mut ids: Vec<i64> = cards.iter().map(|c| c.vehicle_id).collect();
    ids.sort_unstable();
    ids.dedup();
    let found = Vehicle::query().where_in("id", ids).get(&state.db).await?;
    let mut rows = Vec::new();
    for card in cards {
        let vehicle = found.iter().find(|v| v.id == card.vehicle_id);
        let estimate = Estimate::load(&state.db, card.id).await?;
        rows.push(JobRow {
            id: card.id,
            number: card.number(),
            date: card.created_at.map_or_else(String::new, |at| {
                wl_kernel::documents::dubai_date(at).to_string()
            }),
            plate: vehicle.map_or_else(String::new, Vehicle::plate),
            vehicle: vehicle.map_or_else(String::new, vehicle_text),
            customer: if see_customer {
                party_name(state, &lang.locale, card.customer_party_id).await
            } else {
                text(lang, "none")
            },
            technician: match card.technician_user_id {
                Some(id) => user_name(&state.db, id).await,
                None => text(lang, "none"),
            },
            complaint: card.complaint.clone(),
            total: money_text(estimate.live_totals().gross()),
            status: status_text(lang, card.status),
            status_kind: status_kind(card.status),
        });
    }
    Ok(rows)
}

/// The job cards on a vehicle's page.
///
/// # Errors
///
/// The database's errors.
pub async fn history_rows(
    state: &AppState,
    lang: &Lang,
    vehicle_id: i64,
) -> Result<Vec<HistoryRow>> {
    let cards = JobCard::of_vehicle(&state.db, vehicle_id).await?;
    Ok(cards
        .iter()
        .map(|card| HistoryRow {
            id: card.id,
            number: card.number(),
            date: card.created_at.map_or_else(String::new, |at| {
                wl_kernel::documents::dubai_date(at).to_string()
            }),
            complaint: card.complaint.clone(),
            odometer: km_text(card.odometer_in),
            status: status_text(lang, card.status),
            status_kind: status_kind(card.status),
        })
        .collect())
}

/// What the list can be narrowed by.
#[derive(Deserialize, Default, Debug, Clone)]
pub struct Filter {
    /// `open`, `closed` or a status; blank for every job card.
    #[serde(default)]
    status: String,
    /// Text in the plate or the job card number.
    #[serde(default)]
    q: String,
}

/// The standings of the tabs, in order.
const STANDINGS: [&str; 6] = [
    "open",
    "awaiting_approval",
    "in_progress",
    "awaiting_parts",
    "ready",
    "closed",
];

impl Filter {
    /// The list's address with this filter.
    fn href(&self) -> String {
        if self.status.is_empty() {
            "/auto/job-cards".to_owned()
        } else {
            format!("/auto/job-cards?status={}", self.status)
        }
    }

    fn with_status(&self, status: &str) -> Filter {
        Filter {
            status: status.to_owned(),
            ..self.clone()
        }
    }

    /// Whether `card` passes the standing filter.
    fn accepts(&self, card: &JobCard) -> bool {
        match self.status.as_str() {
            "" => true,
            "open" => card.status.is_open(),
            "closed" => card.status.is_closed(),
            other => card.status.as_str() == other,
        }
    }
}

/// The options of the fuel list, `[value, label]`.
fn fuel_options(lang: &Lang) -> Vec<[String; 2]> {
    FUEL_LEVELS
        .iter()
        .map(|level| {
            [
                (*level).to_owned(),
                text(lang, &format!("jobs.fuel.{level}")),
            ]
        })
        .collect()
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/job_cards.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    rows: Vec<JobRow>,
    status_tabs: Vec<(String, String)>,
    status_current: String,
    status: String,
    q: String,
    fuel_options: Vec<[String; 2]>,
    technician_options: Vec<[String; 2]>,
    can_check_in: bool,
    title_key: &'static str,
}

/// `GET /auto/job-cards`.
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Query(filter): Query<Filter>,
) -> Result<View> {
    if !(user.has_permission(permissions::JOB_CARDS_VIEW)
        || user.has_permission(permissions::JOB_CARDS_WORK))
    {
        return Err(Error::Forbidden);
    }
    let mut cards = visible_to(&state, &user).await?;
    cards.retain(|card| filter.accepts(card));
    let wanted = filter.q.trim().to_lowercase();
    if !wanted.is_empty() {
        let mut kept = Vec::new();
        for card in cards {
            let plate = Vehicle::find(&state.db, card.vehicle_id)
                .await?
                .map(|v| v.plate())
                .unwrap_or_default();
            let haystack = format!("{} {plate}", card.number()).to_lowercase();
            if wanted
                .split_whitespace()
                .all(|word| haystack.contains(word))
            {
                kept.push(card);
            }
        }
        cards = kept;
    }
    cards.truncate(300);
    let rows = rows_of(&state, &lang, &user, &cards).await?;
    let mut status_tabs = vec![(
        filter.with_status("").href(),
        text(&lang, "jobs.filter.all"),
    )];
    for key in STANDINGS {
        status_tabs.push((
            filter.with_status(key).href(),
            text(&lang, &format!("jobs.filter.{key}")),
        ));
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        rows,
        status_tabs,
        status_current: filter.href(),
        status: filter.status.clone(),
        q: filter.q.clone(),
        fuel_options: fuel_options(&lang),
        technician_options: role_options(
            &state.db,
            permissions::TECHNICIAN_ROLE,
            text(&lang, "jobs.technician_none"),
        )
        .await,
        can_check_in: user.has_permission(permissions::JOB_CARDS_ADVISE),
        title_key: "jobs.title",
    }))
}

/// `GET /auto/technician`: the technician's own open job cards.
async fn technician(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::JOB_CARDS_WORK)?;
    let mut cards = JobCard::query()
        .where_(JobCard::TECHNICIAN_USER_ID.eq(Some(user.id)))
        .order_by_desc(JobCard::ID)
        .get(&state.db)
        .await?;
    cards.retain(|card| card.status.is_open());
    let rows = rows_of(&state, &lang, &user, &cards).await?;
    Ok(View::typed(TechnicianPage {
        tx: texts::all(&lang),
        rows,
    }))
}

/// What the technician's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/technician.html")]
pub(crate) struct TechnicianPage {
    tx: Texts,
    rows: Vec<JobRow>,
}

/// `GET /auto/vehicles/options`: the vehicles the picker offers as the person
/// types (`?q=`), or the labels of the values it holds (`?values=`).
async fn vehicle_options(
    State(state): State<AppState>,
    user: AuthUser,
    query: renox::select::OptionQuery,
) -> Result<Json<Vec<renox::select::SelectOption>>> {
    require(&user, permissions::JOB_CARDS_ADVISE)?;
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

/// The form that checks a vehicle in.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CheckInForm {
    /// The vehicle, picked from the register.
    #[serde(default)]
    pub vehicle: String,
    /// The customer's complaint.
    #[serde(default)]
    pub complaint: String,
    /// The odometer reading in kilometres.
    #[serde(default)]
    pub odometer: String,
    /// The fuel level.
    #[serde(default)]
    pub fuel: String,
    /// The condition of the vehicle at intake.
    #[serde(default)]
    pub condition: String,
    /// The customer who acknowledged it.
    #[serde(default)]
    pub acknowledged_by: String,
    /// The damage is from an accident.
    #[serde(default)]
    pub accident: String,
    /// The police report number.
    #[serde(default)]
    pub police_report: String,
    /// The inspection fee agreed, in dirhams.
    #[serde(default)]
    pub inspection_fee: String,
    /// The daily storage fee agreed, in dirhams.
    #[serde(default)]
    pub storage_fee: String,
    /// The technician assigned.
    #[serde(default)]
    pub technician: String,
}

/// `POST /auto/job-cards`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CheckInForm>,
) -> Result<Response> {
    require(&user, permissions::JOB_CARDS_ADVISE)?;
    let vehicle = parse_optional_id(&form.vehicle)?
        .ok_or_else(|| super::invalid(&texts::english("auto.jobs.problem.vehicle", &[])))?;
    let mut new = NewJobCard::new(vehicle, &form.complaint)
        .odometer(parse_km(&form.odometer)?)
        .condition(&form.condition)
        .inspection_fee(parse_money(&form.inspection_fee)?)
        .storage_fee(parse_money(&form.storage_fee)?);
    if !form.fuel.trim().is_empty() {
        new = new.fuel(&form.fuel);
    }
    if !form.acknowledged_by.trim().is_empty() {
        new = new.acknowledged_by(&form.acknowledged_by);
    }
    if ticked(&form.accident) {
        new = new.accident(&form.police_report);
    }
    if let Some(technician) = parse_optional_id(&form.technician)? {
        new = new.technician(technician);
    }
    let card = job_cards::check_in(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "jobs.checked_in")),
        renox::HxRedirect(format!("/auto/job-cards/{}", card.id)),
    )
        .into_response())
}

/// One labour line of the job card page.
#[derive(Serialize, Debug, Clone)]
pub struct LabourRow {
    /// The primary key.
    pub id: i64,
    /// What is done.
    pub description: String,
    /// `2.5 h`, or the flat hours.
    pub hours: String,
    /// The hourly rate or the flat price.
    pub rate: String,
    /// Hourly or flat-rate, in words.
    pub pricing: String,
    /// The price, `AED 300.00`.
    pub price: String,
    /// Where the line stands with the customer.
    pub approval: String,
    /// The badge kind.
    pub approval_kind: &'static str,
    /// How much was done, in words.
    pub work: String,
    /// The hours worked.
    pub hours_done: String,
    /// Whether the work done can be recorded.
    pub can_work: bool,
    /// Whether the line can be taken off.
    pub can_remove: bool,
}

/// One part line of the job card page.
#[derive(Serialize, Debug, Clone)]
pub struct PartRow {
    /// The primary key.
    pub id: i64,
    /// What the part is.
    pub description: String,
    /// The stock item code, or a dash.
    pub item: String,
    /// New, used or renovated, in words.
    pub condition: String,
    /// Whether it is a special order, in words.
    pub special: String,
    /// The quantity.
    pub quantity: String,
    /// The unit price.
    pub unit_price: String,
    /// The price of the line.
    pub price: String,
    /// Where the line stands with the customer.
    pub approval: String,
    /// The badge kind.
    pub approval_kind: &'static str,
    /// Issued, fitted and returned, `2 / 1 / 0`.
    pub moved: String,
    /// Whether the outstanding quantity can be issued.
    pub can_issue: bool,
    /// Whether the part can be recorded as fitted.
    pub can_fit: bool,
    /// Whether what is not fitted can be given back.
    pub can_return: bool,
    /// Whether the line can be taken off.
    pub can_remove: bool,
}

/// One round of the estimate.
#[derive(Serialize, Debug, Clone)]
pub struct RoundRow {
    /// The number of the round.
    pub revision: i64,
    /// When it was sent.
    pub sent: String,
    /// The total before VAT.
    pub total: String,
    /// The standing in words.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Who answered, how and when; a dash while it waits.
    pub answered: String,
}

/// One check of the QC checklist.
#[derive(Serialize, Debug, Clone)]
pub struct QcRow {
    /// The check.
    pub key: String,
    /// What it is.
    pub label: String,
    /// Whether it was done, in words.
    pub done: String,
}

/// One change of status.
#[derive(Serialize, Debug, Clone)]
pub struct StepRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// What happened.
    pub action: String,
    /// Who ran it, or the system.
    pub by: String,
    /// The reason, if one was given.
    pub reason: String,
}

/// The standing of a round in words.
fn round_text(lang: &Lang, status: ApprovalStatus) -> String {
    text(lang, &format!("jobs.round.{}", status.as_str()))
}

/// What the page of one job card needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/job_card.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    plate: String,
    vehicle_id: i64,
    vehicle: String,
    customer: String,
    technician: String,
    advisor: String,
    status: String,
    status_kind: &'static str,
    complaint: String,
    odometer_in: String,
    fuel: String,
    condition: String,
    acknowledged: String,
    accident: String,
    inspection_fee: String,
    storage_fee: String,
    findings: String,
    qc_findings: String,
    promised: String,
    warranty: String,
    closed_reason: String,
    delivered: String,
    invoice: String,
    labour: Vec<LabourRow>,
    parts: Vec<PartRow>,
    net: String,
    vat: String,
    gross: String,
    link: String,
    rounds: Vec<RoundRow>,
    qc: Vec<QcRow>,
    qc_done: Vec<String>,
    qc_options: Vec<[String; 2]>,
    steps: Vec<StepRow>,
    // What the user may do.
    can_intake: bool,
    can_assign: bool,
    can_inspect: bool,
    can_findings: bool,
    can_add: bool,
    can_send: bool,
    can_approve: bool,
    can_decline: bool,
    can_cancel: bool,
    can_void: bool,
    can_wait: bool,
    can_finish: bool,
    can_qc: bool,
    can_deliver: bool,
    can_invoice: bool,
    show_qc: bool,
    // The forms.
    fuel_options: Vec<[String; 2]>,
    technician_options: Vec<[String; 2]>,
    condition_options: Vec<[String; 2]>,
    work_options: Vec<[String; 2]>,
    pricing_options: Vec<[String; 2]>,
    operation_options: Vec<[String; 2]>,
    raw_complaint: String,
    raw_odometer: String,
    raw_fuel: String,
    raw_condition: String,
    raw_acknowledged: String,
    raw_police_report: String,
    raw_findings: String,
    raw_technician: String,
    is_accident: bool,
    // The cancel, the release and the special orders (BL-AUT-01).
    stop: StopView,
    cancel_waits: bool,
    approvals: Vec<ApprovalCard>,
}

/// Whether a user sees the customer's name on a job card.
fn sees_customer(user: &AuthUser) -> bool {
    user.has_permission(permissions::JOB_CARDS_VIEW)
}

/// The job card `id` for a user who may see it: the holders of
/// `job_cards.view`, and a technician for the job cards assigned to them
/// (`AUT.ACL.08`).
async fn card_for_view(state: &AppState, user: &AuthUser, id: i64) -> Result<JobCard> {
    let card = JobCard::find_or_404(&state.db, id).await?;
    if user.has_permission(permissions::JOB_CARDS_VIEW) {
        return Ok(card);
    }
    if user.has_permission(permissions::JOB_CARDS_WORK) && card.technician_user_id == Some(user.id)
    {
        return Ok(card);
    }
    Err(Error::Forbidden)
}

/// The estimate rows of the page and the totals.
fn lines_of(
    lang: &Lang,
    user: &AuthUser,
    card: &JobCard,
    estimate: &Estimate,
) -> (Vec<LabourRow>, Vec<PartRow>) {
    let works = may_work(user, card);
    let issues = works || user.has_permission(permissions::STOCK_ISSUE);
    let under_way = matches!(
        card.status,
        JobCardStatus::InProgress | JobCardStatus::AwaitingParts
    );
    let labour = estimate
        .labour
        .iter()
        .filter(|line| line.approval != LineStatus::Removed)
        .map(|line| LabourRow {
            id: line.id,
            description: line.description.clone(),
            hours: line.hours_text(),
            rate: money_text(line.rate),
            pricing: text(lang, &format!("jobs.pricing.{}", line.pricing.as_str())),
            price: money_text(line.price()),
            approval: line_text(lang, line.approval),
            approval_kind: line_kind(line.approval),
            work: text(lang, &format!("jobs.work.{}", line.work.as_str())),
            hours_done: hours_text(line.hours_done_milli),
            can_work: works
                && line.approval == LineStatus::Approved
                && card.status == JobCardStatus::InProgress,
            can_remove: works && line.approval == LineStatus::Draft && card.status.takes_lines(),
        })
        .collect();
    let parts = estimate
        .parts
        .iter()
        .filter(|line| line.approval != LineStatus::Removed)
        .map(|line| PartRow {
            id: line.id,
            description: line.description.clone(),
            item: line.item_code.clone().unwrap_or_else(|| text(lang, "none")),
            condition: text(
                lang,
                &format!("parts.condition.{}", line.part_condition.as_str()),
            ),
            special: if line.special_order {
                text(lang, "jobs.special_yes")
            } else {
                String::new()
            },
            quantity: hours_text(line.quantity_milli),
            unit_price: money_text(line.unit_price),
            price: money_text(line.price()),
            approval: line_text(lang, line.approval),
            approval_kind: line_kind(line.approval),
            moved: format!(
                "{} / {} / {}",
                hours_text(line.issued_milli),
                hours_text(line.fitted_milli),
                hours_text(line.returned_milli)
            ),
            can_issue: issues
                && under_way
                && line.approval == LineStatus::Approved
                && line.outstanding_milli() > 0,
            can_fit: works && card.status == JobCardStatus::InProgress && line.unfitted_milli() > 0,
            can_return: issues && card.status.is_open() && line.unfitted_milli() > 0,
            can_remove: works && line.approval == LineStatus::Draft && card.status.takes_lines(),
        })
        .collect();
    (labour, parts)
}

/// The changes of status of a job card, oldest first.
async fn steps_of(state: &AppState, lang: &Lang, card: &JobCard) -> Result<Vec<StepRow>> {
    let mut steps = Vec::new();
    for row in renox::state_machine::history(&state.db, JobCard::TABLE, card.id).await? {
        let by = match row.actor_id {
            Some(actor) => user_name(&state.db, actor).await,
            None => text(lang, "history.system"),
        };
        let transition = if row.transition == "insert" {
            crate::job_cards::CHECK_IN
        } else {
            row.transition.as_str()
        };
        steps.push(StepRow {
            at: dubai_text(row.at),
            action: text(lang, &format!("jobs.transition.{transition}")),
            by,
            reason: row.reason.clone().unwrap_or_default(),
        });
    }
    Ok(steps)
}

/// The rounds of the estimate.
async fn rounds_of(state: &AppState, lang: &Lang, card: &JobCard) -> Result<Vec<RoundRow>> {
    let mut rounds = Vec::new();
    for round in EstimateApproval::of_job_card(&state.db, card.id).await? {
        let answered = match (&round.decided_by_name, round.decided_at) {
            (Some(name), Some(at)) => format!(
                "{name} · {} · {}",
                text(
                    lang,
                    &format!("jobs.method.{}", round.method.clone().unwrap_or_default())
                ),
                dubai_text(at)
            ),
            _ => text(lang, "none"),
        };
        rounds.push(RoundRow {
            revision: round.revision,
            sent: round.sent_at.map_or_else(String::new, dubai_text),
            total: money_text(round.net_total),
            status: round_text(lang, round.status),
            status_kind: match round.status {
                ApprovalStatus::Approved => "success",
                ApprovalStatus::Pending => "warning",
                ApprovalStatus::Declined => "error",
                ApprovalStatus::Superseded => "info",
            },
            answered,
        });
    }
    Ok(rounds)
}

/// `GET /auto/job-cards/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let card = card_for_view(&state, &user, id).await?;
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let estimate = Estimate::load(&state.db, card.id).await?;
    let totals = estimate.live_totals();
    let (labour, parts_rows) = lines_of(&lang, &user, &card, &estimate);
    let pending = EstimateApproval::pending_of(&state.db, card.id).await?;
    let rounds = rounds_of(&state, &lang, &card).await?;
    let latest = EstimateApproval::of_job_card(&state.db, card.id)
        .await?
        .into_iter()
        .rev()
        .find(|r| r.status == ApprovalStatus::Approved || r.status == ApprovalStatus::Pending);
    let advises = user.has_permission(permissions::JOB_CARDS_ADVISE);
    let works = may_work(&user, &card);
    let open = card.status.is_open();
    let status = card.status;
    let qc: Vec<QcRow> = card
        .qc_checks
        .0
        .iter()
        .map(|check| QcRow {
            key: check.key.clone(),
            label: text(&lang, &format!("jobs.qc.{}", check.key)),
            done: if check.done {
                text(&lang, "jobs.qc_done")
            } else {
                text(&lang, "jobs.qc_open")
            },
        })
        .collect();
    let operations = LabourOperation::query()
        .where_(LabourOperation::ACTIVE.eq(true))
        .order_by(LabourOperation::CODE)
        .get(&state.db)
        .await?;
    let mut operation_options = vec![[String::new(), text(&lang, "jobs.operation_none")]];
    operation_options.extend(operations.iter().map(|op| [op.code.clone(), op.title()]));
    let customer = if sees_customer(&user) {
        party_name(&state, &lang.locale, card.customer_party_id).await
    } else {
        text(&lang, "none")
    };
    let none = || text(&lang, "none");
    let stop = stops::view_of(&lang, &user, &card, &estimate);
    let approval_cards =
        approvals::cards_for(&state.db, &card.subject(), &user, &lang).await?;
    let cancel_waits = job_stops::cancel_pending(&state.db, card.id).await?;
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: card.id,
        number: card.number(),
        plate: vehicle.plate(),
        vehicle_id: vehicle.id,
        vehicle: vehicle_text(&vehicle),
        customer,
        technician: match card.technician_user_id {
            Some(id) => user_name(&state.db, id).await,
            None => none(),
        },
        advisor: match card.advisor_user_id {
            Some(id) => user_name(&state.db, id).await,
            None => none(),
        },
        status: status_text(&lang, status),
        status_kind: status_kind(status),
        complaint: card.complaint.clone(),
        odometer_in: km_text(card.odometer_in),
        fuel: card
            .fuel_level
            .as_ref()
            .map_or_else(none, |level| text(&lang, &format!("jobs.fuel.{level}"))),
        condition: card.intake_condition.clone().unwrap_or_else(none),
        acknowledged: card.intake_acknowledged_by.clone().unwrap_or_else(none),
        accident: if card.accident_damage {
            card.police_report_no.clone().map_or_else(
                || text(&lang, "jobs.accident_no_report"),
                |no| format!("{} {no}", text(&lang, "jobs.accident_report")),
            )
        } else {
            text(&lang, "jobs.no")
        },
        inspection_fee: money_text(card.inspection_fee),
        storage_fee: money_text(card.storage_fee_daily),
        findings: card.findings.clone().unwrap_or_else(none),
        qc_findings: card.qc_findings.clone().unwrap_or_default(),
        promised: latest
            .as_ref()
            .and_then(|r| r.promised_for)
            .map_or_else(none, |d| d.to_string()),
        warranty: latest.as_ref().map_or_else(none, |r| {
            texts::t_with(
                &lang,
                "auto.jobs.warranty_days",
                &[("days", &r.warranty_days)],
            )
        }),
        closed_reason: card.closed_reason.clone().unwrap_or_default(),
        delivered: match (&card.delivered_to, card.delivered_at) {
            (Some(to), Some(at)) => format!("{to} · {}", dubai_text(at)),
            _ => String::new(),
        },
        invoice: card
            .invoice_id
            .map_or_else(String::new, |id| format!("#{id}")),
        labour,
        parts: parts_rows,
        net: money_text(totals.net()),
        vat: money_text(totals.vat),
        gross: money_text(totals.gross()),
        link: pending
            .as_ref()
            .map(EstimateApproval::path)
            .unwrap_or_default(),
        rounds,
        qc,
        qc_done: card
            .qc_checks
            .0
            .iter()
            .filter(|c| c.done)
            .map(|c| c.key.clone())
            .collect(),
        qc_options: QC_KEYS
            .iter()
            .map(|key| [(*key).to_owned(), text(&lang, &format!("jobs.qc.{key}"))])
            .collect(),
        steps: steps_of(&state, &lang, &card).await?,
        can_intake: advises
            && matches!(status, JobCardStatus::CheckedIn | JobCardStatus::Inspecting),
        can_assign: advises && open,
        can_inspect: works && status == JobCardStatus::CheckedIn,
        can_findings: works && open,
        can_add: works && status.takes_lines(),
        can_send: advises
            && matches!(
                status,
                JobCardStatus::Inspecting | JobCardStatus::InProgress
            )
            && estimate.drafts() > 0,
        can_approve: advises && status == JobCardStatus::AwaitingApproval && pending.is_some(),
        can_decline: advises
            && matches!(
                status,
                JobCardStatus::CheckedIn
                    | JobCardStatus::Inspecting
                    | JobCardStatus::AwaitingApproval
            ),
        can_cancel: advises
            && !cancel_waits
            && matches!(
                status,
                JobCardStatus::Inspecting
                    | JobCardStatus::AwaitingApproval
                    | JobCardStatus::InProgress
                    | JobCardStatus::AwaitingParts
                    | JobCardStatus::QualityCheck
            ),
        can_void: advises && status == JobCardStatus::CheckedIn,
        can_wait: works && status == JobCardStatus::InProgress,
        can_finish: works && status == JobCardStatus::InProgress,
        can_qc: user.has_permission(permissions::JOB_CARDS_QC)
            && status == JobCardStatus::QualityCheck,
        can_deliver: advises && status == JobCardStatus::Ready,
        can_invoice: advises && status == JobCardStatus::Delivered && card.invoice_id.is_none(),
        show_qc: !card.qc_checks.0.is_empty(),
        fuel_options: fuel_options(&lang),
        technician_options: role_options(
            &state.db,
            permissions::TECHNICIAN_ROLE,
            text(&lang, "jobs.technician_none"),
        )
        .await,
        condition_options: PartCondition::ALL
            .iter()
            .map(|c| {
                [
                    c.as_str().to_owned(),
                    text(&lang, &format!("parts.condition.{}", c.as_str())),
                ]
            })
            .collect(),
        work_options: [WorkStatus::NotDone, WorkStatus::PartDone, WorkStatus::Done]
            .into_iter()
            .map(|w| {
                [
                    w.as_str().to_owned(),
                    text(&lang, &format!("jobs.work.{}", w.as_str())),
                ]
            })
            .collect(),
        pricing_options: [LabourPricing::Hourly, LabourPricing::FlatRate]
            .into_iter()
            .map(|p| {
                [
                    p.as_str().to_owned(),
                    text(&lang, &format!("jobs.pricing.{}", p.as_str())),
                ]
            })
            .collect(),
        operation_options,
        raw_complaint: card.complaint.clone(),
        raw_odometer: card.odometer_in.to_string(),
        raw_fuel: card.fuel_level.clone().unwrap_or_default(),
        raw_condition: card.intake_condition.clone().unwrap_or_default(),
        raw_acknowledged: card.intake_acknowledged_by.clone().unwrap_or_default(),
        raw_police_report: card.police_report_no.clone().unwrap_or_default(),
        raw_findings: card.findings.clone().unwrap_or_default(),
        raw_technician: card
            .technician_user_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        is_accident: card.accident_damage,
        stop,
        cancel_waits,
        approvals: approval_cards,
    }))
}

/// What the estimate sheet needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/estimate.html")]
pub(crate) struct EstimatePage {
    tx: Texts,
    id: i64,
    number: String,
    plate: String,
    vehicle: String,
    customer: String,
    complaint: String,
    labour: Vec<LabourRow>,
    parts: Vec<PartRow>,
    net: String,
    vat: String,
    gross: String,
    promised: String,
    warranty: String,
    link: String,
    revision: i64,
}

/// `GET /auto/job-cards/{id}/estimate`: the estimate as the customer reads it.
async fn estimate_sheet(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    let card = card_for_view(&state, &user, id).await?;
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let estimate = Estimate::load(&state.db, card.id).await?;
    let totals = estimate.live_totals();
    let (labour, parts_rows) = lines_of(&lang, &user, &card, &estimate);
    let rounds = EstimateApproval::of_job_card(&state.db, card.id).await?;
    let latest = rounds.last();
    let pending = rounds.iter().rev().find(|r| r.is_pending());
    Ok(View::typed(EstimatePage {
        tx: texts::all(&lang),
        id: card.id,
        number: card.number(),
        plate: vehicle.plate(),
        vehicle: vehicle_text(&vehicle),
        customer: if sees_customer(&user) {
            party_name(&state, &lang.locale, card.customer_party_id).await
        } else {
            text(&lang, "none")
        },
        complaint: card.complaint.clone(),
        labour,
        parts: parts_rows,
        net: money_text(totals.net()),
        vat: money_text(totals.vat),
        gross: money_text(totals.gross()),
        promised: latest
            .and_then(|r| r.promised_for)
            .map_or_else(|| text(&lang, "none"), |d| d.to_string()),
        warranty: latest.map_or_else(
            || {
                texts::t_with(
                    &lang,
                    "auto.jobs.warranty_days",
                    &[("days", &MIN_WARRANTY_DAYS)],
                )
            },
            |r| {
                texts::t_with(
                    &lang,
                    "auto.jobs.warranty_days",
                    &[("days", &r.warranty_days)],
                )
            },
        ),
        link: pending.map(EstimateApproval::path).unwrap_or_default(),
        revision: latest.map_or(0, |r| r.revision),
    }))
}

/// The form of the intake.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct IntakeForm {
    /// The customer's complaint.
    #[serde(default)]
    pub complaint: String,
    /// The odometer reading in kilometres.
    #[serde(default)]
    pub odometer: String,
    /// The fuel level.
    #[serde(default)]
    pub fuel: String,
    /// The condition of the vehicle.
    #[serde(default)]
    pub condition: String,
    /// The customer who acknowledged it.
    #[serde(default)]
    pub acknowledged_by: String,
    /// The damage is from an accident.
    #[serde(default)]
    pub accident: String,
    /// The police report number.
    #[serde(default)]
    pub police_report: String,
    /// The inspection fee agreed, in dirhams.
    #[serde(default)]
    pub inspection_fee: String,
    /// The daily storage fee agreed, in dirhams.
    #[serde(default)]
    pub storage_fee: String,
}

/// `POST /auto/job-cards/{id}/intake`.
async fn intake(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<IntakeForm>,
) -> Result<Response> {
    let intake = Intake {
        complaint: form.complaint.clone(),
        odometer_in: parse_km(&form.odometer)?,
        fuel_level: Some(form.fuel.clone()),
        condition: form.condition.clone(),
        acknowledged_by: Some(form.acknowledged_by.clone()),
        signature: None,
        accident_damage: ticked(&form.accident),
        police_report_no: Some(form.police_report.clone()),
        inspection_fee: parse_money(&form.inspection_fee)?,
        storage_fee_daily: parse_money(&form.storage_fee)?,
    };
    job_cards::record_intake(&state, &user, id, intake).await?;
    done(&lang, "jobs.intake_saved")
}

/// The form of a technician assignment.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct AssignForm {
    /// The technician, a user.
    #[serde(default)]
    pub technician: String,
}

/// `POST /auto/job-cards/{id}/technician`.
async fn assign(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<AssignForm>,
) -> Result<Response> {
    let technician = parse_optional_id(&form.technician)?.ok_or_else(|| {
        super::invalid(&texts::english("auto.jobs.problem.not_a_technician", &[]))
    })?;
    job_cards::assign_technician(&state, &user, id, technician).await?;
    done(&lang, "jobs.assigned")
}

/// `POST /auto/job-cards/{id}/inspect`.
async fn inspect(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    job_cards::start_inspection(&state, &user, id).await?;
    done(&lang, "jobs.inspecting")
}

/// The form of the findings.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FindingsForm {
    /// What the technician found.
    #[serde(default)]
    pub findings: String,
}

/// `POST /auto/job-cards/{id}/findings`.
async fn findings(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FindingsForm>,
) -> Result<Response> {
    job_cards::record_findings(&state, &user, id, &form.findings).await?;
    done(&lang, "jobs.findings_saved")
}

/// The form of a labour line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct LabourForm {
    /// The menu operation, by code; blank for a line of its own.
    #[serde(default)]
    pub operation: String,
    /// What is done, for a line of its own.
    #[serde(default)]
    pub description: String,
    /// `hourly` or `flat_rate`, for a line of its own.
    #[serde(default)]
    pub pricing: String,
    /// The hours.
    #[serde(default)]
    pub hours: String,
    /// The hourly rate or the flat price in dirhams, for a line of its own.
    #[serde(default)]
    pub rate: String,
}

/// `POST /auto/job-cards/{id}/labour`.
async fn add_labour(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<LabourForm>,
) -> Result<Response> {
    let hours = parse_milli(&form.hours)?;
    if form.operation.trim().is_empty() {
        let rate = parse_money(&form.rate)?;
        let new = if form.pricing.trim() == "flat_rate" {
            NewLabour::flat(&form.description, hours, rate)
        } else {
            NewLabour::hourly(&form.description, hours, rate)
        };
        job_cards::add_labour(&state, &user, id, new).await?;
    } else {
        job_cards::add_operation(&state, &user, id, &form.operation, hours).await?;
    }
    done(&lang, "jobs.line_added")
}

/// The form of a part line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PartForm {
    /// The stock item code.
    #[serde(default)]
    pub item_code: String,
    /// What the part is.
    #[serde(default)]
    pub description: String,
    /// The quantity.
    #[serde(default)]
    pub quantity: String,
    /// The unit price in dirhams.
    #[serde(default)]
    pub unit_price: String,
    /// New, used or renovated.
    #[serde(default)]
    pub condition: String,
    /// Ordered for this job.
    #[serde(default)]
    pub special_order: String,
}

/// `POST /auto/job-cards/{id}/parts`.
async fn add_part(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<PartForm>,
) -> Result<Response> {
    let condition = PartCondition::parse(&form.condition).unwrap_or(PartCondition::New);
    let mut new = NewPart::new(
        &form.description,
        parse_milli(&form.quantity)?,
        parse_money(&form.unit_price)?,
    )
    .condition(condition);
    if !form.item_code.trim().is_empty() {
        new = new.item(&form.item_code);
    }
    if ticked(&form.special_order) {
        new = new.special_order();
    }
    job_cards::add_part(&state, &user, id, new).await?;
    done(&lang, "jobs.line_added")
}

/// `POST /auto/job-cards/{id}/labour/{line}/remove`.
async fn remove_labour(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
) -> Result<Response> {
    job_cards::remove_labour(&state, &user, line).await?;
    done(&lang, "jobs.line_removed")
}

/// `POST /auto/job-cards/{id}/parts/{line}/remove`.
async fn remove_part(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
) -> Result<Response> {
    job_cards::remove_part(&state, &user, line).await?;
    done(&lang, "jobs.line_removed")
}

/// The form that sends the estimate.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct SendForm {
    /// The day the vehicle is promised for.
    #[serde(default)]
    pub promised_for: String,
    /// The warranty in days.
    #[serde(default)]
    pub warranty_days: String,
}

/// `POST /auto/job-cards/{id}/send`.
async fn send(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<SendForm>,
) -> Result<Response> {
    let promise = Promise {
        promised_for: parse_optional_date(&form.promised_for)?,
        warranty_days: Some(parse_number_or_zero(&form.warranty_days)?).filter(|days| *days > 0),
    };
    job_cards::send_estimate(&state, &user, id, promise).await?;
    done(&lang, "jobs.estimate_sent")
}

/// The form that records the customer's approval.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ApproveForm {
    /// The customer who approved.
    #[serde(default)]
    pub name: String,
    /// `signature` or `recorded`.
    #[serde(default)]
    pub method: String,
    /// The customer's signature.
    #[serde(default)]
    pub signature: String,
}

/// `POST /auto/job-cards/{id}/approve`.
async fn approve(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ApproveForm>,
) -> Result<Response> {
    let method = if form.method.trim().is_empty() {
        "recorded"
    } else {
        form.method.trim()
    };
    job_cards::approve_recorded(&state, &user, id, &form.name, method, Some(&form.signature))
        .await?;
    done(&lang, "jobs.approved")
}

/// The form with a reason.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ReasonForm {
    /// Why.
    #[serde(default)]
    pub reason: String,
}

/// `POST /auto/job-cards/{id}/decline`.
async fn decline(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    job_cards::decline(&state, &user, id, &form.reason).await?;
    done(&lang, "jobs.declined")
}

/// `POST /auto/job-cards/{id}/void`.
async fn void(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ReasonForm>,
) -> Result<Response> {
    job_cards::void(&state, &user, id, &form.reason).await?;
    done(&lang, "jobs.voided")
}

/// `POST /auto/job-cards/{id}/wait-parts`.
async fn wait_parts(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    job_cards::wait_parts(&state, &user, id).await?;
    done(&lang, "jobs.waiting_parts")
}

/// The form of the work done on a labour line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct WorkForm {
    /// `not_done`, `part_done` or `done`.
    #[serde(default)]
    pub work: String,
    /// The hours worked.
    #[serde(default)]
    pub hours: String,
}

/// `POST /auto/job-cards/{id}/labour/{line}/work`.
async fn work(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
    Form(form): Form<WorkForm>,
) -> Result<Response> {
    let work = match form.work.trim() {
        "done" => WorkStatus::Done,
        "part_done" => WorkStatus::PartDone,
        _ => WorkStatus::NotDone,
    };
    let hours = if form.hours.trim().is_empty() {
        None
    } else {
        Some(parse_milli(&form.hours)?)
    };
    job_cards::record_work(&state, &user, line, work, hours).await?;
    done(&lang, "jobs.work_saved")
}

/// The form of a quantity.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct QuantityForm {
    /// The quantity; blank for all there is.
    #[serde(default)]
    pub quantity: String,
}

/// The quantity of a form: none when blank.
fn quantity_of(form: &QuantityForm) -> std::result::Result<Option<i64>, Error> {
    if form.quantity.trim().is_empty() {
        Ok(None)
    } else {
        parse_milli(&form.quantity).map(Some)
    }
}

/// `POST /auto/job-cards/{id}/parts/{line}/issue`.
async fn issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
    Form(form): Form<QuantityForm>,
) -> Result<Response> {
    parts::issue(&state, &user, line, quantity_of(&form)?).await?;
    done(&lang, "jobs.part_issued")
}

/// `POST /auto/job-cards/{id}/parts/{line}/fit`.
async fn fit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
    Form(form): Form<QuantityForm>,
) -> Result<Response> {
    parts::fit(&state, &user, line, quantity_of(&form)?).await?;
    done(&lang, "jobs.part_fitted")
}

/// `POST /auto/job-cards/{id}/parts/{line}/return`.
async fn give_back(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path((_id, line)): Path<(i64, i64)>,
) -> Result<Response> {
    parts::return_part(&state, &user, line).await?;
    done(&lang, "jobs.part_returned")
}

/// `POST /auto/job-cards/{id}/finish`.
async fn finish(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    job_cards::finish_work(&state, &user, id).await?;
    done(&lang, "jobs.work_finished")
}

/// The form of the QC checklist.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct QcForm {
    /// The checks that were done.
    #[serde(default)]
    pub done: Vec<String>,
}

/// `POST /auto/job-cards/{id}/qc`.
async fn save_qc(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<QcForm>,
) -> Result<Response> {
    job_cards::save_qc(&state, &user, id, &form.done).await?;
    done(&lang, "jobs.qc_saved")
}

/// `POST /auto/job-cards/{id}/pass-qc`.
async fn pass_qc(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    job_cards::pass_qc(&state, &user, id).await?;
    done(&lang, "jobs.qc_passed")
}

/// The form of the QC findings.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FindingsReasonForm {
    /// What the QC found.
    #[serde(default)]
    pub findings: String,
}

/// `POST /auto/job-cards/{id}/fail-qc`.
async fn fail_qc(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FindingsReasonForm>,
) -> Result<Response> {
    job_cards::fail_qc(&state, &user, id, &form.findings).await?;
    done(&lang, "jobs.qc_failed")
}

/// The form of the delivery.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct DeliverForm {
    /// Who collected the vehicle.
    #[serde(default)]
    pub delivered_to: String,
    /// The odometer reading at delivery.
    #[serde(default)]
    pub odometer: String,
    /// The date the next service falls due.
    #[serde(default)]
    pub next_service_date: String,
    /// The odometer reading the next service falls due at.
    #[serde(default)]
    pub next_service_odometer: String,
}

/// `POST /auto/job-cards/{id}/deliver`.
async fn deliver(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<DeliverForm>,
) -> Result<Response> {
    let delivery = Delivery {
        delivered_to: form.delivered_to.clone(),
        odometer_out: if form.odometer.trim().is_empty() {
            None
        } else {
            Some(parse_km(&form.odometer)?)
        },
        next_service_date: parse_optional_date(&form.next_service_date)?,
        next_service_odometer: if form.next_service_odometer.trim().is_empty() {
            None
        } else {
            Some(parse_km(&form.next_service_odometer)?)
        },
    };
    job_cards::deliver(&state, &user, id, delivery).await?;
    done(&lang, "jobs.delivered")
}

/// `POST /auto/job-cards/{id}/invoice`: raises the invoice of a delivered job
/// card that has none (the billing failed or invoicing was off).
async fn invoice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::JOB_CARDS_ADVISE)?;
    billing::bill(&state, id).await?;
    done(&lang, "jobs.invoiced")
}

/// What the customer's page needs.
#[derive(Serialize, ViewData)]
#[view(template = "auto/approve.html")]
pub(crate) struct ApprovePage {
    tx: Texts,
    token: String,
    number: String,
    plate: String,
    vehicle: String,
    labour: Vec<LabourRow>,
    parts: Vec<PartRow>,
    net: String,
    vat: String,
    gross: String,
    promised: String,
    warranty: String,
    open: bool,
    answered: String,
    added_work: bool,
}

/// `GET /auto/approve/{token}`: the estimate round the customer reads and
/// answers.
async fn approve_page(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
) -> Result<View> {
    let round = EstimateApproval::by_token(&state.db, &token)
        .await?
        .ok_or(Error::NotFound)?;
    let card = JobCard::find_or_404(&state.db, round.job_card_id).await?;
    let vehicle = Vehicle::find_or_404(&state.db, card.vehicle_id).await?;
    let estimate = Estimate::load(&state.db, card.id).await?;
    // The lines of this round, as the customer reads them.
    let in_round = Estimate {
        labour: estimate
            .labour
            .iter()
            .filter(|l| l.approval_id == Some(round.id))
            .cloned()
            .collect(),
        parts: estimate
            .parts
            .iter()
            .filter(|l| l.approval_id == Some(round.id))
            .cloned()
            .collect(),
    };
    let totals = in_round.totals(|_| true);
    let (labour, parts_rows) = customer_lines(&lang, &in_round);
    let open = round.is_pending() && card.status == JobCardStatus::AwaitingApproval;
    let answered = match (&round.decided_by_name, round.decided_at) {
        (Some(name), Some(at)) => format!(
            "{} · {name} · {}",
            round_text(&lang, round.status),
            dubai_text(at)
        ),
        _ if round.status == ApprovalStatus::Superseded => round_text(&lang, round.status),
        _ => String::new(),
    };
    Ok(View::typed(ApprovePage {
        tx: texts::all(&lang),
        token,
        number: card.number(),
        plate: vehicle.plate(),
        vehicle: vehicle_text(&vehicle),
        labour,
        parts: parts_rows,
        net: money_text(totals.net()),
        vat: money_text(totals.vat),
        gross: money_text(totals.gross()),
        promised: round
            .promised_for
            .map_or_else(|| text(&lang, "none"), |d| d.to_string()),
        warranty: texts::t_with(
            &lang,
            "auto.jobs.warranty_days",
            &[("days", &round.warranty_days)],
        ),
        open,
        answered,
        added_work: round.revision > 1,
    }))
}

/// The lines of a round as the customer reads them: no actions.
fn customer_lines(lang: &Lang, estimate: &Estimate) -> (Vec<LabourRow>, Vec<PartRow>) {
    let labour = estimate
        .labour
        .iter()
        .map(|line| LabourRow {
            id: line.id,
            description: line.description.clone(),
            hours: line.hours_text(),
            rate: money_text(line.rate),
            pricing: text(lang, &format!("jobs.pricing.{}", line.pricing.as_str())),
            price: money_text(line.price()),
            approval: line_text(lang, line.approval),
            approval_kind: line_kind(line.approval),
            work: String::new(),
            hours_done: String::new(),
            can_work: false,
            can_remove: false,
        })
        .collect();
    let parts = estimate
        .parts
        .iter()
        .map(|line| PartRow {
            id: line.id,
            description: line.description.clone(),
            item: String::new(),
            condition: text(
                lang,
                &format!("parts.condition.{}", line.part_condition.as_str()),
            ),
            special: if line.special_order {
                text(lang, "jobs.special_yes")
            } else {
                String::new()
            },
            quantity: hours_text(line.quantity_milli),
            unit_price: money_text(line.unit_price),
            price: money_text(line.price()),
            approval: line_text(lang, line.approval),
            approval_kind: line_kind(line.approval),
            moved: String::new(),
            can_issue: false,
            can_fit: false,
            can_return: false,
            can_remove: false,
        })
        .collect();
    (labour, parts)
}

/// The form where the customer approves.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct AnswerForm {
    /// The customer's name.
    #[serde(default)]
    pub name: String,
    /// The customer's signature, typed.
    #[serde(default)]
    pub signature: String,
    /// The consent to the cost, the time and the warranty.
    #[serde(default)]
    pub consent: String,
    /// Why the added work is declined.
    #[serde(default)]
    pub reason: String,
}

/// `POST /auto/approve/{token}`.
async fn approve_answer(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
    Form(form): Form<AnswerForm>,
) -> Result<Response> {
    job_cards::approve_by_link(
        &state,
        &token,
        &form.name,
        Some(&form.signature),
        ticked(&form.consent),
    )
    .await?;
    done(&lang, "jobs.thanks")
}

/// `POST /auto/approve/{token}/decline`.
async fn decline_answer(
    State(state): State<AppState>,
    lang: Lang,
    Path(token): Path<String>,
    Form(form): Form<AnswerForm>,
) -> Result<Response> {
    job_cards::decline_by_link(&state, &token, &form.name, &form.reason).await?;
    done(&lang, "jobs.thanks_declined")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_status_has_a_badge_kind_the_kit_knows() {
        for status in [
            JobCardStatus::CheckedIn,
            JobCardStatus::Inspecting,
            JobCardStatus::AwaitingApproval,
            JobCardStatus::InProgress,
            JobCardStatus::AwaitingParts,
            JobCardStatus::QualityCheck,
            JobCardStatus::Ready,
            JobCardStatus::Delivered,
            JobCardStatus::Declined,
            JobCardStatus::Cancelled,
            JobCardStatus::Voided,
        ] {
            assert!(
                ["info", "success", "warning", "error"].contains(&status_kind(status)),
                "{status:?}"
            );
        }
    }

    #[test]
    fn a_filter_keeps_open_closed_or_one_status() {
        let ready = JobCard {
            status: JobCardStatus::Ready,
            ..JobCard::default()
        };
        let delivered = JobCard {
            status: JobCardStatus::Delivered,
            ..JobCard::default()
        };
        let by = |status: &str| Filter {
            status: status.to_owned(),
            ..Filter::default()
        };
        assert!(by("").accepts(&ready) && by("").accepts(&delivered));
        assert!(by("open").accepts(&ready) && !by("open").accepts(&delivered));
        assert!(!by("closed").accepts(&ready) && by("closed").accepts(&delivered));
        assert!(by("ready").accepts(&ready) && !by("ready").accepts(&delivered));
    }
}
