//! The visit page (05 `CLN.ENC.01`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/visits/{id}` | `clinics.visit` | `visits.view` |
//! | `POST /clinics/visits/{id}/insured` | `clinics.visit_insured` | `coverage.record` |
//! | `POST /clinics/visits/{id}/charges` | `clinics.visit_charge` | `charges.capture` (a service), `charges.desk` or `charges.capture` (a consumable) |
//! | `POST /clinics/visits/{id}/none` | `clinics.visit_none` | `charges.desk` |
//! | `POST /clinics/visits/{id}/accept-work` | `clinics.visit_accept_work` | `charges.desk` |
//! | `POST /clinics/visits/{id}/not-suitable` | `clinics.visit_not_suitable` | `charges.capture` |
//! | `POST /clinics/visits/{id}/refund-deposit` | `clinics.visit_refund_deposit` | `charges.desk` |
//! | `POST /clinics/visits/{id}/consent` | `clinics.visit_consent` | `coverage.record` |
//! | `POST /clinics/visits/{id}/bill` | `clinics.visit_bill` | `charges.capture` or `charges.desk` |
//! | `POST /clinics/visits/{id}/lots` | `clinics.visit_lot` | `charges.desk` or `charges.capture` |
//! | `POST /clinics/visits/{id}/issue` | `clinics.visit_issue` | `charges.desk` or `charges.capture` (issues the stock again) |
//! | `GET /clinics/visits/{id}?code=X` | `clinics.visit` | the issue screen: the lot proposed for consumable `X` |
//! | `POST /clinics/visits/{id}/times` | `clinics.visit_times` | `charges.desk` (a time from the paper sheet after an outage, with the late-entry flag) |
//!
//! The page shows the cover the check-in recorded. When the eligibility turns
//! positive before the service starts, the receptionist changes the outcome
//! to *insured*; after `start` the outcome is fixed (`CLN.ACL.15`).

use renox::prelude::*;
use renox::state_machine::history;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_text;
use wl_kernel::integrations::web::banner_for;

use super::appointments::{outcome_label, policy_label};
use super::{Texts, invalid, parse_id, parse_moment, party_name, require, text, user_name};
use crate::authorisation::AuthorisationHold;
use crate::billing::{self, Basis};
use crate::charge::{ChargeKind, ChargeLine};
use crate::charges::{self, NewCharge};
use crate::consent::{self, SelfPayConsent};
use crate::consumable_issue::ConsumableIssue;
use crate::coverage::{self, CoverageOutcome};
use crate::late_times::{self, VisitTime};
use crate::lots;
use crate::money_text;
use crate::permissions;
use crate::policy::InsurancePolicy;
use crate::service_names;
use crate::texts;
use crate::visit::{Visit, VisitStatus};

/// The screen the payment gateway registers, so that it shows the banner
/// while the gateway is down: the visit takes its payments at the desk.
pub const SCREEN: &str = "clinics.visit";

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/visits/{id}", show)
        .name("clinics.visit")
        .post("/clinics/visits/{id}/insured", insured)
        .name("clinics.visit_insured")
        .post("/clinics/visits/{id}/charges", charge)
        .name("clinics.visit_charge")
        .post("/clinics/visits/{id}/none", none)
        .name("clinics.visit_none")
        .post("/clinics/visits/{id}/accept-work", accept_work)
        .name("clinics.visit_accept_work")
        .post("/clinics/visits/{id}/not-suitable", not_suitable)
        .name("clinics.visit_not_suitable")
        .post("/clinics/visits/{id}/refund-deposit", refund_deposit)
        .name("clinics.visit_refund_deposit")
        .post("/clinics/visits/{id}/times", enter_time)
        .name("clinics.visit_times")
        .post("/clinics/visits/{id}/consent", update_consent)
        .name("clinics.visit_consent")
        .post("/clinics/visits/{id}/bill", bill)
        .name("clinics.visit_bill")
        .post("/clinics/visits/{id}/issue", retry_issue)
        .name("clinics.visit_issue")
        .post("/clinics/visits/{id}/lots", set_lot)
        .name("clinics.visit_lot")
        .require_auth()
}

/// The badge kind of a visit status.
fn status_kind(status: VisitStatus) -> &'static str {
    match status {
        VisitStatus::CheckedIn | VisitStatus::InProgress => "info",
        VisitStatus::Completed => "success",
        VisitStatus::Cancelled => "warning",
    }
}

/// One row of the history.
#[derive(Serialize, Debug, Clone)]
pub struct HistoryRow {
    /// When (Asia/Dubai).
    pub at: String,
    /// What happened.
    pub action: String,
    /// Who ran it, or the system.
    pub by: String,
}

/// One line of the charge capture.
#[derive(Serialize, Debug, Clone)]
pub struct ChargeRow {
    /// Service or consumable.
    pub kind: String,
    /// The code.
    pub code: String,
    /// The units.
    pub units: i64,
    /// The price of one unit.
    pub price: String,
    /// The patient's part.
    pub patient: String,
    /// The payer's part.
    pub payer: String,
    /// Why the line is not charged, or what it was.
    pub note: String,
    /// The lot of a consumable: the number, "Lot required" or "Lot uncertain".
    pub lot: String,
}

/// One part of a consumable that left the branch store.
#[derive(Serialize, Debug, Clone)]
pub struct IssueRow {
    /// The consumable.
    pub code: String,
    /// The lot's number, or "Lot uncertain" for the part waiting for stock.
    pub lot: String,
    /// The last day of use.
    pub expiry: String,
    /// The units.
    pub quantity: String,
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/visit.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    booking: i64,
    patient: String,
    service: String,
    visit_date: String,
    status: String,
    status_kind: &'static str,
    policy: String,
    outcome: String,
    outcome_kind: &'static str,
    split_policy: String,
    consent: String,
    estimate: String,
    holds: String,
    can_insure: bool,
    history: Vec<HistoryRow>,
    charges: Vec<ChargeRow>,
    work: String,
    patient_total: String,
    payer_total: String,
    invoice: String,
    payer_bill: String,
    deposit_applied: String,
    deposit_refunded: String,
    can_capture_open: bool,
    can_desk_open: bool,
    can_update_consent: bool,
    can_bill: bool,
    kind_options: Vec<[String; 2]>,
    vat_options: Vec<[String; 2]>,
    issues: Vec<IssueRow>,
    lines: Vec<[String; 2]>,
    proposal_code: String,
    proposal: String,
    can_set_lot: bool,
    can_desk_only: bool,
    stock_issue: String,
    can_retry_issue: bool,
    integration_banner: Option<renox::serde_json::Value>,
    can_enter_time: bool,
    time_kinds: Vec<[String; 2]>,
    times: Vec<TimeRow>,
}

/// A time entered for the visit, as the page lists it.
#[derive(Serialize, Debug, Clone)]
pub struct TimeRow {
    /// Check-in, check-out or consumable.
    pub kind: String,
    /// When it happened.
    pub occurred: String,
    /// When it was typed in.
    pub entered: String,
    /// The late-entry flag, or a dash.
    pub flag: String,
    /// Why it was late, or a dash.
    pub reason: String,
}

/// The query of the visit page: `code` asks which lot the issue screen
/// proposes for that consumable.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ShowQuery {
    /// The consumable code.
    #[serde(default)]
    pub code: String,
}

/// The work done as the page reads it.
fn work_text(lang: &Lang, visit: &Visit, lines: &[ChargeLine]) -> String {
    let key = if visit.not_suitable {
        "not_suitable"
    } else if visit.work_accepted_at.is_some() {
        "accepted"
    } else if !lines.is_empty() {
        "captured"
    } else if visit.work_none {
        "none"
    } else {
        "pending"
    };
    text(lang, &format!("charges.work.{key}"))
}

/// The note of a line: why it is not charged.
fn note_text(lang: &Lang, line: &ChargeLine) -> String {
    match line.reason.as_deref() {
        Some(reason) => text(lang, &format!("charges.reason.{reason}")),
        None => text(lang, "none"),
    }
}

/// The quantity of a lot row, in units.
fn units_text(quantity_milli: i64) -> String {
    if quantity_milli % 1_000 == 0 {
        (quantity_milli / 1_000).to_string()
    } else {
        format!("{}.{:03}", quantity_milli / 1_000, quantity_milli % 1_000)
    }
}

/// What the lot column of a consumable line says.
fn lot_text(lang: &Lang, line: &ChargeLine, issues: &[ConsumableIssue]) -> String {
    if line.kind != ChargeKind::Consumable {
        return text(lang, "none");
    }
    let mine: Vec<String> = issues
        .iter()
        .filter(|issue| issue.charge_line_id == line.id)
        .map(|issue| {
            if issue.is_uncertain() {
                text(lang, "lots.uncertain")
            } else {
                issue
                    .lot_number
                    .clone()
                    .unwrap_or_else(|| text(lang, "none"))
            }
        })
        .collect();
    if !mine.is_empty() {
        return mine.join(", ");
    }
    match &line.lot_number {
        Some(number) => number.clone(),
        None if line.lot_tracked => text(lang, "lots.required"),
        None => text(lang, "none"),
    }
}

/// `GET /clinics/visits/{id}` (`?code=X`: the issue screen proposes the lot
/// of consumable `X` with the earliest expiry).
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Query(query): Query<ShowQuery>,
) -> Result<View> {
    require(&user, permissions::VISITS_VIEW)?;
    let visit = Visit::find_or_404(&state.db, id).await?;
    let service_names = service_names::names(&state, &lang.locale).await?;
    let none = text(&lang, "none");
    let patient = match visit.patient_party_id {
        Some(party) => party_name(&state, &lang, wl_kernel::PartyId(party)).await,
        None => none.clone(),
    };
    let policy_text = |policy: Option<InsurancePolicy>| {
        policy.map_or_else(|| none.clone(), |p| policy_label(&lang, &p))
    };
    let booked_with = match visit.policy_id {
        Some(policy) => InsurancePolicy::find(&state.db, policy).await?,
        None => None,
    };
    let split_with = match visit.coverage_policy_id {
        Some(policy) => InsurancePolicy::find(&state.db, policy).await?,
        None => None,
    };
    let consent = match visit.consent_id {
        Some(consent) => SelfPayConsent::find(&state.db, consent).await?,
        None => None,
    };
    let held = AuthorisationHold::of_visit(&state.db, visit.id).await?;
    let holds = if held.is_empty() {
        none.clone()
    } else {
        held.iter()
            .map(|hold| {
                format!(
                    "{}: PA-{:04} ({})",
                    service_names::name_of(&service_names, &hold.service_code),
                    hold.authorisation_id,
                    hold.units
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut rows = Vec::new();
    for row in history(&state.db, Visit::TABLE, visit.id).await? {
        let by = match row.actor_id {
            Some(actor) => user_name(&state.db, actor).await,
            None => text(&lang, "history.system"),
        };
        // The row of the visit coming to be is the framework's `insert`: it
        // is the check-in.
        let transition = if row.transition == "insert" {
            crate::visit_machine::CHECK_IN
        } else {
            row.transition.as_str()
        };
        rows.push(HistoryRow {
            at: dubai_text(row.at),
            action: text(&lang, &format!("visits.transition.{transition}")),
            by,
        });
    }
    let lines = ChargeLine::active_of(&state.db, visit.id).await?;
    let issued = ConsumableIssue::of_visit(&state.db, visit.id).await?;
    let issue_rows: Vec<IssueRow> = issued
        .iter()
        .map(|issue| IssueRow {
            code: lines
                .iter()
                .find(|line| line.id == issue.charge_line_id)
                .map_or_else(|| text(&lang, "none"), |line| line.code.clone()),
            lot: if issue.is_uncertain() {
                text(&lang, "lots.uncertain")
            } else {
                issue
                    .lot_number
                    .clone()
                    .unwrap_or_else(|| text(&lang, "none"))
            },
            expiry: issue
                .expiry_date
                .map_or_else(|| text(&lang, "none"), |day| day.to_string()),
            quantity: units_text(issue.quantity_milli),
        })
        .collect();
    let lot_lines: Vec<[String; 2]> = lines
        .iter()
        .filter(|line| {
            line.kind == ChargeKind::Consumable && line.lot_tracked && line.issued_at.is_none()
        })
        .map(|line| {
            [
                line.id.to_string(),
                format!("{} x {}", line.code, line.units),
            ]
        })
        .collect();
    let proposal_code = query.code.trim().to_owned();
    let proposal = if proposal_code.is_empty() {
        String::new()
    } else {
        match lots::proposal(&state, &visit, &proposal_code).await? {
            Some(lot) => format!(
                "{} ({} {})",
                lot.number,
                text(&lang, "lots.expiry"),
                lot.expiry_date
                    .map_or_else(|| text(&lang, "none"), |day| day.to_string())
            ),
            None => text(&lang, "lots.proposed_none"),
        }
    };
    let quoted = if visit.billed_at.is_none() && visit.status.is_open() {
        charges::quote(&state, &visit).await.ok()
    } else {
        None
    };
    let shown = quoted.as_ref().map_or(lines.clone(), |q| q.lines.clone());
    let charge_rows: Vec<ChargeRow> = shown
        .iter()
        .map(|line| ChargeRow {
            kind: text(
                &lang,
                match line.kind {
                    ChargeKind::Service => "charges.kind.service",
                    ChargeKind::Consumable => "charges.kind.consumable",
                },
            ),
            code: match line.kind {
                ChargeKind::Service => service_names::name_of(&service_names, &line.code),
                ChargeKind::Consumable => line.code.clone(),
            },
            units: line.units,
            price: money_text(line.unit_price),
            patient: money_text(line.patient_share),
            payer: money_text(line.payer_share),
            note: note_text(&lang, line),
            lot: lot_text(&lang, line, &issued),
        })
        .collect();
    let (patient_total, payer_total) = match &quoted {
        Some(q) => (q.patient_total, q.payer_total),
        None => (visit.patient_total, visit.payer_total),
    };
    let can_capture = user.has_permission(permissions::CHARGES_CAPTURE);
    let can_desk = user.has_permission(permissions::CHARGES_DESK);
    let dash = none.clone();
    let open = visit.status.is_open() && visit.billed_at.is_none();
    let option = |key: &str, label: &str| [key.to_owned(), text(&lang, label)];
    let time_rows: Vec<TimeRow> = VisitTime::of_visit(&state.db, visit.id)
        .await?
        .iter()
        .map(|t| TimeRow {
            kind: text(&lang, &format!("late.kind.{}", t.kind)),
            occurred: dubai_text(t.occurred_at),
            entered: dubai_text(t.entered_at),
            flag: if t.late {
                text(&lang, "late.flag")
            } else {
                text(&lang, "none")
            },
            reason: t.late_reason.clone().unwrap_or_else(|| text(&lang, "none")),
        })
        .collect();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: visit.id,
        number: visit.number(),
        booking: visit.booking_id,
        patient,
        service: service_names::name_of(&service_names, &visit.service_code),
        visit_date: visit.visit_date.to_string(),
        status: text(&lang, &format!("visits.status.{}", visit.status.as_str())),
        status_kind: status_kind(visit.status),
        policy: policy_text(booked_with),
        outcome: visit.coverage_outcome.map_or_else(
            || text(&lang, "coverage.outcome_none"),
            |o| outcome_label(&lang, o),
        ),
        outcome_kind: match visit.coverage_outcome {
            Some(CoverageOutcome::Insured) => "success",
            Some(_) => "warning",
            None => "info",
        },
        split_policy: policy_text(split_with),
        consent: consent.as_ref().map_or_else(
            || none.clone(),
            |c| format!("{} — {}", c.signer_name, c.signed_on),
        ),
        estimate: consent.as_ref().map_or_else(
            || none.clone(),
            |c| format!("{} ({})", money_text(c.estimated_fils), c.lines_text()),
        ),
        holds,
        can_insure: user.has_permission(permissions::COVERAGE_RECORD)
            && visit.is_insured()
            && visit.status == VisitStatus::CheckedIn
            && visit.coverage_outcome != Some(CoverageOutcome::Insured),
        history: rows,
        charges: charge_rows,
        work: work_text(&lang, &visit, &lines),
        patient_total: money_text(patient_total),
        payer_total: money_text(payer_total),
        invoice: visit
            .invoice_id
            .map_or_else(|| dash.clone(), |id| format!("#{id}")),
        payer_bill: visit
            .payer_bill_id
            .map_or_else(|| dash.clone(), |id| format!("#{id}")),
        deposit_applied: money_text(visit.deposit_applied),
        deposit_refunded: money_text(visit.deposit_refunded),
        can_capture_open: can_capture && open,
        can_desk_open: can_desk && open,
        can_update_consent: user.has_permission(permissions::COVERAGE_RECORD)
            && visit.coverage_outcome == Some(CoverageOutcome::SelfPay)
            && visit.status.is_open(),
        can_bill: (can_capture || can_desk) && visit.billed_at.is_none() && !visit.status.is_open(),
        kind_options: vec![
            option("service", "charges.kind.service"),
            option("consumable", "charges.kind.consumable"),
        ],
        vat_options: vec![
            option("zero", "charges.vat.zero"),
            option("standard", "charges.vat.standard"),
        ],
        issues: issue_rows,
        lines: lot_lines,
        proposal_code,
        proposal,
        can_set_lot: (can_capture || can_desk) && open,
        can_desk_only: can_desk && !can_capture && open,
        stock_issue: match visit.issue_pending_reason.as_deref() {
            None if open => none.clone(),
            None => text(&lang, "lots.issue_done"),
            Some(lots::UNKNOWN_STORE) => text(&lang, "lots.issue_unknown_store"),
            Some(_) => text(&lang, "lots.issue_waiting"),
        },
        can_retry_issue: (can_capture || can_desk)
            && visit.issue_pending_at.is_some()
            && !visit.status.is_open(),
        integration_banner: banner_for(&state.db, SCREEN, &lang).await?,
        can_enter_time: can_desk,
        time_kinds: vec![
            option(late_times::CHECK_IN, "late.kind.check_in"),
            option(late_times::CHECK_OUT, "late.kind.check_out"),
            option(late_times::CONSUMABLE, "late.kind.consumable"),
        ],
        times: time_rows,
    }))
}

/// The form that changes the outcome to insured.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct InsuredForm {
    /// The payer's reference of the positive result.
    #[serde(default)]
    pub reference: String,
}

/// `POST /clinics/visits/{id}/insured`.
async fn insured(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<InsuredForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let reference = Some(form.reference.trim()).filter(|r| !r.is_empty());
    coverage::change_to_insured(&state, Some(user.id), id, reference, renox::db::now()).await?;
    Ok((
        Toast::success(text(&lang, "coverage.changed_to_insured")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The toast and the refresh of an action that worked.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// The form that records a line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ChargeForm {
    /// The service or consumable code.
    #[serde(default)]
    pub code: String,
    /// The units (one when blank).
    #[serde(default)]
    pub units: String,
    /// `service` (the default) or `consumable`.
    #[serde(default)]
    pub kind: String,
    /// `standard` for a standard-rated line, else zero-rated.
    #[serde(default)]
    pub vat: String,
    /// The number of the lot scanned or picked (a consumable).
    #[serde(default)]
    pub lot: String,
}

/// The units typed in a form: one when blank.
fn units_of(text: &str) -> std::result::Result<i64, Error> {
    if text.trim().is_empty() {
        return Ok(1);
    }
    parse_id(text).ok_or_else(|| invalid("units: 1, 2, 3 ..."))
}

/// `POST /clinics/visits/{id}/charges`: a service delivered needs
/// `charges.capture`; a consumable issued, `charges.desk` or
/// `charges.capture`.
async fn charge(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<ChargeForm>,
) -> Result<Response> {
    let mut new = NewCharge::new(&form.code, units_of(&form.units)?);
    if form.vat == "standard" {
        new = new.standard_rated();
    }
    if form.kind == "consumable" {
        new = new.lot(&form.lot);
        if !user.has_permission(permissions::CHARGES_DESK) {
            require(&user, permissions::CHARGES_CAPTURE)?;
        }
        charges::record_consumable(&state, Some(user.id), id, new).await?;
    } else {
        require(&user, permissions::CHARGES_CAPTURE)?;
        charges::record_service(&state, Some(user.id), id, new).await?;
    }
    done(&lang, "charges.recorded")
}

/// The form that enters a time from the paper sheet.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct TimeForm {
    /// `check_in`, `check_out` or `consumable`.
    #[serde(default)]
    pub kind: String,
    /// When it happened, Asia/Dubai wall time (`2026-11-10T09:30`).
    #[serde(default)]
    pub time: String,
    /// Why it is entered late (asked when the time is behind the entry).
    #[serde(default)]
    pub reason: String,
    /// The consumable line, for `consumable`.
    #[serde(default)]
    pub line: String,
}

/// `POST /clinics/visits/{id}/times`: a check-in, check-out or consumable
/// time from the paper sheet, with the late-entry flag when it is late. A
/// time beyond the kernel's limit waits for approval.
async fn enter_time(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<TimeForm>,
) -> Result<Response> {
    require(&user, permissions::CHARGES_DESK)?;
    let at = parse_moment(&form.time).ok_or_else(|| invalid(&text(&lang, "late.problem.time")))?;
    let entered = late_times::enter(
        &state,
        &user,
        id,
        form.kind.trim(),
        at,
        Some(form.reason.as_str()),
        parse_id(&form.line),
    )
    .await?;
    done(
        &lang,
        if entered.is_pending() {
            "late.sent_for_approval"
        } else {
            "late.entered"
        },
    )
}

/// `POST /clinics/visits/{id}/none`.
async fn none(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::CHARGES_DESK)?;
    charges::mark_none(&state, id).await?;
    done(&lang, "charges.none_done")
}

/// `POST /clinics/visits/{id}/accept-work`.
async fn accept_work(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::CHARGES_DESK)?;
    charges::accept_work(&state, Some(user.id), id).await?;
    done(&lang, "charges.accepted")
}

/// The form that records the consultation given instead of the procedure.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct NotSuitableForm {
    /// The consultation service the practitioner gave.
    #[serde(default)]
    pub consultation: String,
    /// `standard` for a standard-rated line, else zero-rated.
    #[serde(default)]
    pub vat: String,
}

/// `POST /clinics/visits/{id}/not-suitable`.
async fn not_suitable(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<NotSuitableForm>,
) -> Result<Response> {
    require(&user, permissions::CHARGES_CAPTURE)?;
    let mut new = NewCharge::new(&form.consultation, 1);
    if form.vat == "standard" {
        new = new.standard_rated();
    }
    charges::mark_not_suitable(&state, Some(user.id), id, new).await?;
    done(&lang, "charges.not_suitable_done")
}

/// `POST /clinics/visits/{id}/refund-deposit`.
async fn refund_deposit(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    require(&user, permissions::CHARGES_DESK)?;
    charges::refund_deposit_in_full(&state, id).await?;
    done(&lang, "charges.refund_deposit_done")
}

/// The form of an updated self-pay consent.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct UpdateConsentForm {
    /// The patient's name as they sign it.
    #[serde(default)]
    pub signer_name: String,
    /// The services added, comma separated.
    #[serde(default)]
    pub services: String,
}

/// `POST /clinics/visits/{id}/consent`: the patient signs an updated
/// self-pay consent with the services added during the visit.
async fn update_consent(
    State(state): State<AppState>,
    user: AuthUser,
    renox::ClientIp(ip): renox::ClientIp,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<UpdateConsentForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let ip = ip.map(|ip| ip.to_string());
    let extra: Vec<String> = form
        .services
        .split(',')
        .map(|code| code.trim().to_owned())
        .filter(|code| !code.is_empty())
        .collect();
    consent::sign_updated(
        &state,
        Some(user.id),
        id,
        &form.signer_name,
        ip.as_deref(),
        &extra,
        renox::db::now(),
    )
    .await?;
    done(&lang, "consent.updated")
}

/// `POST /clinics/visits/{id}/bill`: bills a visit that is over and was not
/// billed (the bill stopped on an error), on the basis it ended with
/// ([`Basis::of_visit`]).
async fn bill(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::CHARGES_DESK) {
        require(&user, permissions::CHARGES_CAPTURE)?;
    }
    let visit = Visit::find_or_404(&state.db, id).await?;
    let basis =
        Basis::of_visit(&visit).ok_or_else(|| invalid(&text(&lang, "charges.problem.closed")))?;
    billing::bill(&state, id, basis).await?;
    done(&lang, "charges.billed")
}

/// The form that sets the lot of a consumable line.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct LotForm {
    /// The consumable line.
    #[serde(default)]
    pub line: String,
    /// The number of the lot scanned or picked; blank takes the proposed one.
    #[serde(default)]
    pub lot: String,
}

/// `POST /clinics/visits/{id}/lots`: sets or corrects the lot of a
/// consumable line before the check-out.
async fn set_lot(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<LotForm>,
) -> Result<Response> {
    if !user.has_permission(permissions::CHARGES_DESK) {
        require(&user, permissions::CHARGES_CAPTURE)?;
    }
    let line = parse_id(&form.line).ok_or_else(|| invalid(&text(&lang, "lots.problem.no_line")))?;
    lots::assign(&state, id, line, Some(form.lot.as_str())).await?;
    done(&lang, "lots.assigned")
}

/// `POST /clinics/visits/{id}/issue`: issues the stock of a finished visit
/// again after the first run stopped; nothing is issued twice.
async fn retry_issue(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    if !user.has_permission(permissions::CHARGES_DESK) {
        require(&user, permissions::CHARGES_CAPTURE)?;
    }
    lots::retry(&state, id).await?;
    done(&lang, "lots.issue_retried")
}
