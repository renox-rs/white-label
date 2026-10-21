//! The appointment page: the pre-check-in screen of an insured visit (05
//! §4.2, `CLN.ACL.15`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /clinics/appointments` | `clinics.appointments` | `visits.view` |
//! | `GET /clinics/appointments/{booking}` | `clinics.appointment` | `visits.view` |
//! | `POST /clinics/appointments/{booking}/policy` | `clinics.appointment_policy` | `coverage.record` |
//! | `POST /clinics/appointments/{booking}/eligibility` | `clinics.appointment_eligibility` | `coverage.record` |
//! | `POST /clinics/appointments/{booking}/outcome` | `clinics.appointment_outcome` | `coverage.record` |
//! | `POST /clinics/appointments/{booking}/consent` | `clinics.appointment_consent` | `coverage.record` |
//!
//! The receptionist records the eligibility result and one coverage outcome
//! here; Check in itself is wl-scheduling's route (`bookings.checkin`) and is
//! refused with "coverage outcome required" until both are set. The page shows
//! the schedule, the demographics and the cover, never a visit reason or
//! billing codes (`CLN.ACL.02`), and never the card image (`CLN.ACL.12`).

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::{Texts, invalid, parse_id, party_name, require, text};
use crate::appointment::Appointment;
use crate::appointments;
use crate::consent::SelfPayConsent;
use crate::coverage::{self, Choice, CoverageOutcome, EligibilityInput, PendingCheckIn};
use crate::money_text;
use crate::permissions;
use crate::policy::{EligibilityCheck, InsurancePolicy};
use crate::texts;
use crate::visit::Visit;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/clinics/appointments", index)
        .name("clinics.appointments")
        .get("/clinics/appointments/{booking}", show)
        .name("clinics.appointment")
        .post("/clinics/appointments/{booking}/policy", policy)
        .name("clinics.appointment_policy")
        .post("/clinics/appointments/{booking}/eligibility", eligibility)
        .name("clinics.appointment_eligibility")
        .post("/clinics/appointments/{booking}/outcome", outcome)
        .name("clinics.appointment_outcome")
        .post("/clinics/appointments/{booking}/consent", consent)
        .name("clinics.appointment_consent")
        .require_auth()
}

/// The outcome as the page reads it.
pub(crate) fn outcome_label(lang: &Lang, outcome: CoverageOutcome) -> String {
    text(lang, &format!("coverage.outcome.{}", outcome.as_str()))
}

/// `Plan — member id (valid to …)` for a select.
pub(crate) fn policy_label(lang: &Lang, policy: &InsurancePolicy) -> String {
    let to = policy
        .valid_to
        .map_or_else(|| text(lang, "none"), |d| d.to_string());
    format!("{} — {} ({})", policy.plan, policy.member_id, to)
}

/// One appointment of the list.
#[derive(Serialize, Debug, Clone)]
pub struct AppointmentRow {
    /// The booking.
    pub booking: i64,
    /// The patient's name.
    pub patient: String,
    /// The service code.
    pub service: String,
    /// The start (Asia/Dubai).
    pub starts: String,
    /// Where the cover stands, in the page's language.
    pub cover: String,
    /// The badge kind.
    pub cover_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/appointments.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    appointments: Vec<AppointmentRow>,
}

/// `GET /clinics/appointments`: the appointments from today on, soonest first,
/// with where the cover of each insured one stands.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::VISITS_VIEW)?;
    let all = Appointment::query()
        .where_(Appointment::VISIT_DATE.gte(wl_kernel::documents::today()))
        .where_(Appointment::CANCELLED_ON.is_null())
        .order_by(Appointment::STARTS_AT)
        .get(&state.db)
        .await?;
    let mut rows = Vec::with_capacity(all.len());
    for appointment in &all {
        let visit = Visit::of_booking(&state.db, appointment.booking_id).await?;
        let pending = PendingCheckIn::active_of(&state.db, appointment.booking_id).await?;
        let (cover, kind) = if visit.is_some() {
            (text(&lang, "coverage.checked_in"), "success")
        } else if !appointment.is_insured() {
            (text(&lang, "coverage.not_insured"), "info")
        } else if let Some(outcome) = pending.as_ref().and_then(|p| p.outcome) {
            (outcome_label(&lang, outcome), "success")
        } else {
            (text(&lang, "coverage.waiting"), "warning")
        };
        rows.push(AppointmentRow {
            booking: appointment.booking_id,
            patient: match appointment.patient_party_id {
                Some(id) => party_name(&state, &lang, wl_kernel::PartyId(id)).await,
                None => text(&lang, "none"),
            },
            service: appointment.service_code.clone(),
            starts: wl_kernel::documents::dubai_text(appointment.starts_at),
            cover,
            cover_kind: kind,
        });
    }
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        appointments: rows,
    }))
}

/// What the page needs.
#[derive(Serialize, ViewData)]
#[view(template = "clinics/appointment.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    booking: i64,
    patient: String,
    service: String,
    visit_date: String,
    cancelled: bool,
    insured: bool,
    policy: String,
    policy_valid: bool,
    eligibility: String,
    outcome: String,
    outcome_kind: &'static str,
    consent: String,
    estimate: String,
    checked_in: bool,
    visit_id: i64,
    can_record: bool,
    can_cover: bool,
    can_check_in: bool,
    policy_options: Vec<[String; 2]>,
    policy_selected: String,
    result_options: Vec<[String; 2]>,
    outcome_options: Vec<[String; 2]>,
}

/// `GET /clinics/appointments/{booking}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(booking): Path<i64>,
) -> Result<View> {
    require(&user, permissions::VISITS_VIEW)?;
    let appointment = Appointment::of_booking(&state.db, booking)
        .await?
        .ok_or(Error::NotFound)?;
    let patient = match appointment.patient_party_id {
        Some(id) => party_name(&state, &lang, wl_kernel::PartyId(id)).await,
        None => text(&lang, "none"),
    };
    let policies = match appointment.patient_party_id {
        Some(id) => {
            InsurancePolicy::query()
                .where_(InsurancePolicy::PATIENT_PARTY_ID.eq(id))
                .order_by(InsurancePolicy::ID)
                .get(&state.db)
                .await?
        }
        None => Vec::new(),
    };
    let current = match appointment.policy_id {
        Some(id) => InsurancePolicy::find(&state.db, id).await?,
        None => None,
    };
    let pending = PendingCheckIn::active_of(&state.db, booking).await?;
    let visit = Visit::of_booking(&state.db, booking).await?;
    let none = text(&lang, "none");
    let (eligibility, outcome, consent_id) = match (&visit, &pending) {
        (Some(v), _) => (
            v.eligibility_check_id,
            v.coverage_outcome,
            v.consent_id,
        ),
        (None, Some(p)) => (p.eligibility_check_id, p.outcome, p.consent_id),
        (None, None) => (None, None, None),
    };
    let eligibility_text = match eligibility {
        Some(id) => match EligibilityCheck::find(&state.db, id).await? {
            Some(check) => text(
                &lang,
                if check.positive {
                    "coverage.eligibility_positive"
                } else {
                    "coverage.eligibility_negative"
                },
            ) + &format!(" ({})", check.checked_on),
            None => none.clone(),
        },
        None => text(&lang, "coverage.eligibility_missing"),
    };
    let consent = match consent_id {
        Some(id) => SelfPayConsent::find(&state.db, id).await?,
        None => None,
    };
    let selected = appointment
        .policy_id
        .map(|id| id.to_string())
        .or_else(|| policies.first().map(|p| p.id.to_string()))
        .unwrap_or_default();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        booking,
        patient,
        service: appointment.service_code.clone(),
        visit_date: appointment.visit_date.to_string(),
        cancelled: appointment.is_cancelled(),
        insured: appointment.is_insured(),
        policy: current
            .as_ref()
            .map_or_else(|| none.clone(), |p| policy_label(&lang, p)),
        policy_valid: current
            .as_ref()
            .is_some_and(|p| p.is_valid_on(appointment.visit_date)),
        eligibility: eligibility_text,
        outcome: outcome.map_or_else(
            || text(&lang, "coverage.outcome_missing"),
            |o| outcome_label(&lang, o),
        ),
        outcome_kind: match outcome {
            Some(CoverageOutcome::Insured) => "success",
            Some(_) => "warning",
            None => "info",
        },
        consent: consent.as_ref().map_or_else(
            || none.clone(),
            |c| {
                let mut line = format!("{} — {}", c.signer_name, c.signed_on);
                if c.is_superseded() {
                    line.push_str(" (");
                    line.push_str(&text(&lang, "consent.superseded"));
                    line.push(')');
                }
                line
            },
        ),
        estimate: consent
            .as_ref()
            .map_or_else(|| none.clone(), |c| money_text(c.estimated_fils)),
        checked_in: visit.is_some(),
        visit_id: visit.as_ref().map_or(0, |v| v.id),
        can_record: user.has_permission(permissions::COVERAGE_RECORD)
            && visit.is_none()
            && !appointment.is_cancelled(),
        can_cover: user.has_permission(permissions::COVERAGE_RECORD)
            && visit.is_none()
            && !appointment.is_cancelled()
            && appointment.is_insured(),
        can_check_in: user.has_permission("bookings.checkin")
            && visit.is_none()
            && !appointment.is_cancelled(),
        policy_options: policies
            .iter()
            .map(|p| [p.id.to_string(), policy_label(&lang, p)])
            .collect(),
        policy_selected: selected,
        result_options: vec![
            ["positive".to_owned(), text(&lang, "coverage.eligibility_positive")],
            ["negative".to_owned(), text(&lang, "coverage.eligibility_negative")],
        ],
        outcome_options: CoverageOutcome::ALL
            .iter()
            .map(|o| [o.as_str().to_owned(), outcome_label(&lang, *o)])
            .collect(),
    }))
}

/// The form that books the appointment with a policy.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct PolicyForm {
    /// The policy's id.
    #[serde(default)]
    pub policy: String,
}

/// `POST /clinics/appointments/{booking}/policy`.
async fn policy(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(booking): Path<i64>,
    Form(form): Form<PolicyForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let policy = parse_id(&form.policy).ok_or_else(|| invalid("policy: 1, 2, 3 ..."))?;
    appointments::attach_policy(&state, booking, policy).await?;
    Ok((
        Toast::success(text(&lang, "coverage.policy_attached")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that records an eligibility result.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct EligibilityForm {
    /// `positive` or `negative`.
    #[serde(default)]
    pub result: String,
    /// The payer's reference.
    #[serde(default)]
    pub reference: String,
    /// A note.
    #[serde(default)]
    pub note: String,
    /// Another policy of the patient (a re-split target); blank for the
    /// appointment's own.
    #[serde(default)]
    pub policy: String,
}

/// `POST /clinics/appointments/{booking}/eligibility`.
async fn eligibility(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(booking): Path<i64>,
    Form(form): Form<EligibilityForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let positive = match form.result.trim() {
        "positive" => true,
        "negative" => false,
        _ => return Err(invalid("result: positive, negative")),
    };
    let mut input = EligibilityInput::new(positive);
    if let Some(policy) = parse_id(&form.policy) {
        input = input.policy(policy);
    }
    if !form.reference.trim().is_empty() {
        input = input.reference(&form.reference);
    }
    if !form.note.trim().is_empty() {
        input = input.note(&form.note);
    }
    coverage::record_eligibility(&state, Some(user.id), booking, input, renox::db::now()).await?;
    Ok((
        Toast::success(text(&lang, "coverage.eligibility_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that sets the coverage outcome.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct OutcomeForm {
    /// `insured`, `re_split` or `self_pay`.
    #[serde(default)]
    pub outcome: String,
    /// The policy of a re-split.
    #[serde(default)]
    pub policy: String,
}

/// `POST /clinics/appointments/{booking}/outcome`.
async fn outcome(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(booking): Path<i64>,
    Form(form): Form<OutcomeForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let choice = match form.outcome.trim() {
        "insured" => Choice::Insured,
        "re_split" => Choice::ReSplit(
            parse_id(&form.policy).ok_or_else(|| invalid("policy: 1, 2, 3 ..."))?,
        ),
        "self_pay" => Choice::SelfPay,
        _ => return Err(invalid("outcome: insured, re_split, self_pay")),
    };
    coverage::choose(&state, Some(user.id), booking, choice, renox::db::now()).await?;
    Ok((
        Toast::success(text(&lang, "coverage.outcome_recorded")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form of the self-pay consent on the tablet.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ConsentForm {
    /// The patient's name as they sign it.
    #[serde(default)]
    pub signer_name: String,
    /// More services than the booked one, comma separated.
    #[serde(default)]
    pub services: String,
}

/// `POST /clinics/appointments/{booking}/consent`: the patient signs on the
/// tablet and the outcome becomes self-pay.
async fn consent(
    State(state): State<AppState>,
    user: AuthUser,
    renox::ClientIp(ip): renox::ClientIp,
    lang: Lang,
    Path(booking): Path<i64>,
    Form(form): Form<ConsentForm>,
) -> Result<Response> {
    require(&user, permissions::COVERAGE_RECORD)?;
    let ip = ip.map(|ip| ip.to_string());
    let extra: Vec<String> = form
        .services
        .split(',')
        .map(|code| code.trim().to_owned())
        .filter(|code| !code.is_empty())
        .collect();
    let at = renox::db::now();
    coverage::sign_consent(
        &state,
        Some(user.id),
        booking,
        &form.signer_name,
        ip.as_deref(),
        &extra,
        at,
    )
    .await?;
    coverage::choose(&state, Some(user.id), booking, Choice::SelfPay, at).await?;
    Ok((
        Toast::success(text(&lang, "consent.signed")),
        renox::HxRefresh,
    )
        .into_response())
}
