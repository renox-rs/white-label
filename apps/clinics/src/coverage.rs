//! The coverage outcome of an insured visit (05 §4.2 *When it goes wrong*,
//! §3.3 `CLN.ACL.15`; BL-CLN-01).
//!
//! An appointment booked with an insurance policy is an *insured* visit
//! ([`crate::appointments::attach_policy`]). Before the patient is checked in
//! the receptionist records, on the pre-check-in screen, the **eligibility
//! result** of the policy ([`record_eligibility`]) and **one coverage
//! outcome** ([`choose`]):
//!
//! - [`CoverageOutcome::Insured`]: cover confirmed (a positive result today and
//!   a policy and card valid on the visit day). A positive result for a valid
//!   policy sets it by itself: nothing else is asked.
//! - [`CoverageOutcome::ReSplit`]: another valid policy of the same patient
//!   (or the same policy with an updated card) is used; the policy of the
//!   visit's charge split (`CLN.ENC.02`) becomes that one.
//! - [`CoverageOutcome::SelfPay`]: the patient pays the whole visit, after
//!   signing the self-pay consent that shows the estimated price
//!   ([`crate::consent`]).
//!
//! Or the appointment is rescheduled (the deposit is carried by
//! wl-scheduling), or cancelled with the reason `coverage_not_confirmed`
//! (initiated by the business, the deposit refunded in full:
//! [`crate::reasons`]).
//!
//! Nothing is stored on the booking. What the receptionist records lives in a
//! [`PendingCheckIn`] of the appointment until the check-in: [`CoverageGuard`]
//! refuses `check_in` with "coverage outcome required" until both are set,
//! `BookingCheckedIn` moves the data to the visit ([`move_to_visit`]), and it
//! is discarded when the appointment is cancelled or rescheduled or is not
//! checked in by 23:59 of its day in Asia/Dubai ([`discard_lapsed`]).
//!
//! After the check-in and before `start` the receptionist may change the
//! outcome to *insured* when the eligibility turned positive
//! ([`change_to_insured`]); the consent stays on file, marked *superseded*.
//! After `start` the outcome is fixed.
//!
//! Decisions:
//!
//! - *Re-split target.* A policy of the same patient whose dates and card
//!   cover the visit day and whose eligibility is recorded positive **today**
//!   (the day of the check-in, Asia/Dubai).
//! - *Recalculating the patient share* is the patient-share calculation's
//!   (`CLN.ENC.02`, with the payer contract, `CLN.INS.01`), which is not built:
//!   the visit records `coverage_policy_id`, the policy the split reads.
//! - *A non-insured appointment* is asked nothing.

use renox::chrono::{NaiveDate, NaiveTime};
use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::calendar::calendar;
use wl_kernel::documents::dubai_date;
use wl_kernel::scheduling::{
    BookingCancelled, BookingId, BookingRescheduled, CheckInAnswer, CheckInGuard, SchedulingFuture,
};

use crate::appointment::Appointment;
use crate::auth_check;
use crate::consent::{self, SelfPayConsent};
use crate::db::refused;
use crate::policies;
use crate::policy::{EligibilityCheck, InsurancePolicy};
use crate::texts::english;
use crate::visit::{Visit, VisitStatus};

/// What the cover of an insured visit turned out to be.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CoverageOutcome {
    /// Cover confirmed.
    #[default]
    Insured,
    /// Another valid policy, or an updated card, is used.
    ReSplit,
    /// The patient pays the whole visit, with a signed consent.
    SelfPay,
}

/// The reason code of a pending check-in discarded because the appointment
/// was cancelled.
pub const DISCARD_CANCELLED: &str = "cancelled";
/// … because it was rescheduled.
pub const DISCARD_RESCHEDULED: &str = "rescheduled";
/// … because the day ended without a check-in.
pub const DISCARD_LAPSED: &str = "lapsed";
/// … because the appointment was booked with another policy.
pub const DISCARD_POLICY_CHANGED: &str = "policy_changed";

/// When the day of an appointment ends for the pending data, Asia/Dubai.
pub const DAY_ENDS_AT: &str = "23:59";

/// The scheduled task and command that discard the lapsed data.
pub const TASK: &str = "clinics:pending_check_ins";

/// One service of a visit with the units it will use (BL-CLN-02).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisitService {
    /// The service code.
    pub code: String,
    /// The units the visit needs of it; at least one.
    pub units: i64,
}

impl VisitService {
    /// `units` of the service `code`.
    pub fn new(code: &str, units: i64) -> Self {
        VisitService {
            code: code.to_owned(),
            units,
        }
    }
}

/// Reads `laser:2, consultation` into services: a code, a colon and the
/// units (one when left out).
///
/// # Errors
///
/// A 422 for no service, a blank code, units that are not a whole number of
/// at least one, or a code named twice.
pub fn parse_services(text: &str) -> Result<Vec<VisitService>> {
    let mut found: Vec<VisitService> = Vec::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (code, units) = match part.split_once(':') {
            Some((code, units)) => (code.trim(), units.trim().parse::<i64>().ok()),
            None => (part, Some(1)),
        };
        let Some(units) = units.filter(|u| *u >= 1) else {
            return Err(refused(english("clinics.coverage.problem.services", &[])));
        };
        if code.is_empty() || found.iter().any(|s| s.code == code) {
            return Err(refused(english("clinics.coverage.problem.services", &[])));
        }
        found.push(VisitService::new(code, units));
    }
    if found.is_empty() {
        return Err(refused(english("clinics.coverage.problem.services", &[])));
    }
    Ok(found)
}

/// What the receptionist recorded for an appointment before its check-in.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_pending_check_ins", index(booking_id))]
pub struct PendingCheckIn {
    /// The primary key.
    pub id: i64,
    /// The booking.
    pub booking_id: i64,
    /// The clinic's record of the booking.
    #[model(references = "clinics_appointments")]
    pub appointment_id: i64,
    /// The policy the appointment was booked with.
    pub policy_id: i64,
    /// The day of the visit.
    pub visit_date: NaiveDate,
    /// The eligibility result of the appointment's policy.
    pub eligibility_check_id: Option<i64>,
    /// Whether that result is positive.
    pub eligibility_positive: Option<bool>,
    /// The coverage outcome, once one is set.
    pub outcome: Option<CoverageOutcome>,
    /// The policy the visit's charge split reads.
    pub coverage_policy_id: Option<i64>,
    /// The self-pay consent of a `self_pay` outcome.
    pub consent_id: Option<i64>,
    /// The services the visit will use, with the units of each, as the
    /// receptionist entered them; empty means the booked service, one unit
    /// (BL-CLN-02).
    pub services: Json<Vec<VisitService>>,
    /// The visit the check-in moved it to.
    pub moved_visit_id: Option<i64>,
    /// The day it was discarded (Asia/Dubai).
    pub discarded_on: Option<NaiveDate>,
    /// Why: `cancelled`, `rescheduled`, `lapsed` or `policy_changed`.
    pub discard_reason: Option<String>,
    /// The user who recorded it last.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PendingCheckIn {
    /// The data in force for booking `booking`: not moved and not discarded.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn active_of(db: &Db, booking: i64) -> Result<Option<PendingCheckIn>> {
        PendingCheckIn::query()
            .where_(PendingCheckIn::BOOKING_ID.eq(booking))
            .where_(PendingCheckIn::MOVED_VISIT_ID.is_null())
            .where_(PendingCheckIn::DISCARDED_ON.is_null())
            .order_by_desc(PendingCheckIn::ID)
            .first(db)
            .await
    }

    /// Every row of booking `booking`, oldest first (the history of what was
    /// recorded and discarded).
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_booking(db: &Db, booking: i64) -> Result<Vec<PendingCheckIn>> {
        PendingCheckIn::query()
            .where_(PendingCheckIn::BOOKING_ID.eq(booking))
            .order_by(PendingCheckIn::ID)
            .get(db)
            .await
    }

    /// Whether the eligibility result and the outcome are both set.
    pub fn is_complete(&self) -> bool {
        self.eligibility_check_id.is_some() && self.outcome.is_some()
    }
}

/// The outcome the receptionist chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Cover confirmed.
    Insured,
    /// Re-split to this policy of the patient.
    ReSplit(i64),
    /// Self-pay, with the signed consent.
    SelfPay,
}

/// An eligibility result to record.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct EligibilityInput {
    /// The policy asked about; the appointment's own when `None`.
    pub policy: Option<i64>,
    /// The answer.
    pub positive: bool,
    /// The payer's reference.
    pub reference: Option<String>,
    /// A note.
    pub note: Option<String>,
}

impl EligibilityInput {
    /// The appointment's policy answered `positive`.
    pub fn new(positive: bool) -> Self {
        EligibilityInput {
            policy: None,
            positive,
            reference: None,
            note: None,
        }
    }

    /// Asked about another policy of the patient (a re-split target).
    pub fn policy(mut self, policy: i64) -> Self {
        self.policy = Some(policy);
        self
    }

    /// With the payer's reference.
    pub fn reference(mut self, reference: &str) -> Self {
        self.reference = Some(reference.to_owned());
        self
    }

    /// With a note.
    pub fn note(mut self, note: &str) -> Self {
        self.note = Some(note.to_owned());
        self
    }
}

/// The appointment and policy a pre-check-in action is about.
async fn target(state: &AppState, booking: i64) -> Result<(Appointment, InsurancePolicy)> {
    let appointment = Appointment::of_booking(&state.db, booking)
        .await?
        .ok_or(Error::NotFound)?;
    if appointment.is_cancelled() {
        return Err(refused(english(
            "clinics.appointments.problem.cancelled",
            &[],
        )));
    }
    if Visit::of_booking(&state.db, booking).await?.is_some() {
        return Err(refused(english("clinics.coverage.problem.checked_in", &[])));
    }
    let Some(policy) = appointment.policy_id else {
        return Err(refused(english(
            "clinics.coverage.problem.not_insured",
            &[],
        )));
    };
    let policy = InsurancePolicy::find_or_404(&state.db, policy).await?;
    Ok((appointment, policy))
}

/// The data in force for the appointment, made when there is none.
async fn ensure_pending(
    state: &AppState,
    appointment: &Appointment,
    policy: &InsurancePolicy,
    by: Option<i64>,
) -> Result<PendingCheckIn> {
    if let Some(found) = PendingCheckIn::active_of(&state.db, appointment.booking_id).await? {
        return Ok(found);
    }
    let mut pending = PendingCheckIn {
        booking_id: appointment.booking_id,
        appointment_id: appointment.id,
        policy_id: policy.id,
        visit_date: appointment.visit_date,
        recorded_by: by,
        ..PendingCheckIn::default()
    };
    pending.save(&state.db).await?;
    Ok(pending)
}

/// Records the eligibility result of the appointment's policy (or of another
/// policy of the patient, a re-split target) on the pre-check-in screen. A
/// positive result for a policy and card valid on the visit day sets the
/// outcome *insured* when none is set; a negative one takes back an *insured*
/// outcome.
///
/// # Errors
///
/// A 404 for an unknown appointment; a 422 for an appointment that is not
/// insured, is cancelled or is already checked in, or for a policy of another
/// patient.
pub async fn record_eligibility(
    state: &AppState,
    by: Option<i64>,
    booking: i64,
    input: EligibilityInput,
    at: DateTime,
) -> Result<PendingCheckIn> {
    let (appointment, policy) = target(state, booking).await?;
    let other = match input.policy {
        Some(id) if id != policy.id => {
            let found = InsurancePolicy::find_or_404(&state.db, id).await?;
            if found.patient_party_id != policy.patient_party_id {
                return Err(refused(english(
                    "clinics.appointments.problem.other_patient",
                    &[],
                )));
            }
            Some(found)
        }
        _ => None,
    };
    let asked = other.as_ref().unwrap_or(&policy);
    let check = policies::record_eligibility(
        state,
        by,
        asked.id,
        input.positive,
        input.reference.as_deref(),
        input.note.as_deref(),
        at,
    )
    .await?;
    let pending = ensure_pending(state, &appointment, &policy, by).await?;
    if other.is_none() {
        let valid = policy.is_valid_on(appointment.visit_date);
        let (outcome, covered) = match (input.positive && valid, pending.outcome) {
            (true, None) => (Some(CoverageOutcome::Insured), Some(policy.id)),
            (false, Some(CoverageOutcome::Insured)) => (None, None),
            (_, current) => (current, pending.coverage_policy_id),
        };
        set(
            state,
            pending.id,
            &[
                ("eligibility_check_id", &Some(check.id)),
                ("eligibility_positive", &Some(input.positive)),
                ("outcome", &outcome),
                ("coverage_policy_id", &covered),
                ("recorded_by", &by),
            ],
        )
        .await?;
    }
    PendingCheckIn::find_or_404(&state.db, pending.id).await
}

/// Sets columns of the pending data `id` and stamps `updated_at`.
async fn set(
    state: &AppState,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    PendingCheckIn::query()
        .where_(PendingCheckIn::ID.eq(id))
        .update(&state.db, &all)
        .await?;
    Ok(())
}

/// Whether the policy has a positive eligibility result recorded on `day`.
async fn positive_on(state: &AppState, policy: i64, day: NaiveDate) -> Result<bool> {
    Ok(EligibilityCheck::latest_on(&state.db, policy, day)
        .await?
        .is_some_and(|check| check.positive))
}

/// Sets the coverage outcome of the appointment (`at` is the moment of the
/// check-in, Asia/Dubai).
///
/// # Errors
///
/// A 422 when *insured* has no positive result today or the policy or card
/// does not cover the visit day; when the *re-split* policy is not the
/// patient's, does not cover the visit day or has no positive result today;
/// when *self-pay* has a service with no self-pay price ("no self-pay price
/// for <service>") or no signed consent.
pub async fn choose(
    state: &AppState,
    by: Option<i64>,
    booking: i64,
    choice: Choice,
    at: DateTime,
) -> Result<PendingCheckIn> {
    let (appointment, policy) = target(state, booking).await?;
    let pending = ensure_pending(state, &appointment, &policy, by).await?;
    let today = dubai_date(at);
    let (covered, consent) = match choice {
        Choice::Insured => {
            if !policy.is_valid_on(appointment.visit_date)
                || !positive_on(state, policy.id, today).await?
            {
                return Err(refused(english("clinics.coverage.problem.insured", &[])));
            }
            (Some(policy.id), None)
        }
        Choice::ReSplit(id) => {
            let new = InsurancePolicy::find_or_404(&state.db, id).await?;
            if new.patient_party_id != policy.patient_party_id
                || !new.is_valid_on(appointment.visit_date)
                || !positive_on(state, new.id, today).await?
            {
                return Err(refused(english("clinics.coverage.problem.re_split", &[])));
            }
            (Some(new.id), None)
        }
        Choice::SelfPay => {
            // The price first: a service with none cannot be on a consent.
            consent::price_lines(
                state,
                std::slice::from_ref(&appointment.service_code),
                appointment.visit_date,
            )
            .await?;
            let signed = SelfPayConsent::current(&state.db, booking)
                .await?
                .filter(|c| c.lists(&appointment.service_code));
            let Some(signed) = signed else {
                return Err(refused(english("clinics.consent.problem.required", &[])));
            };
            (None, Some(signed.id))
        }
    };
    let outcome = match choice {
        Choice::Insured => CoverageOutcome::Insured,
        Choice::ReSplit(_) => CoverageOutcome::ReSplit,
        Choice::SelfPay => CoverageOutcome::SelfPay,
    };
    if consent.is_none() {
        // A consent signed earlier stays on file, marked superseded.
        consent::supersede_all(state, booking, today).await?;
    }
    set(
        state,
        pending.id,
        &[
            ("outcome", &Some(outcome)),
            ("coverage_policy_id", &covered),
            ("consent_id", &consent),
            ("recorded_by", &by),
        ],
    )
    .await?;
    PendingCheckIn::find_or_404(&state.db, pending.id).await
}

/// The patient signs the self-pay consent for the appointment's service (and
/// `extra` ones) on the tablet; the estimate is each service's self-pay price
/// on the visit day. It does not set the outcome: [`choose`] with
/// [`Choice::SelfPay`] does.
///
/// # Errors
///
/// A 422 for a blank signer or a service with no self-pay price.
pub async fn sign_consent(
    state: &AppState,
    by: Option<i64>,
    booking: i64,
    signer_name: &str,
    ip: Option<&str>,
    extra: &[String],
    at: DateTime,
) -> Result<SelfPayConsent> {
    let (appointment, policy) = target(state, booking).await?;
    let pending = ensure_pending(state, &appointment, &policy, by).await?;
    let signed = consent::sign(state, by, &appointment, signer_name, ip, extra, at).await?;
    SelfPayConsent::query()
        .where_(SelfPayConsent::ID.eq(signed.id))
        .update(
            &state.db,
            &[("pending_id", &Some(pending.id)), ("updated_at", &now())],
        )
        .await?;
    // A new consent while the outcome is self-pay replaces the old one.
    if pending.outcome == Some(CoverageOutcome::SelfPay) {
        set(state, pending.id, &[("consent_id", &Some(signed.id))]).await?;
    }
    SelfPayConsent::find_or_404(&state.db, signed.id).await
}

/// Records the services the visit will use and the units of each (the
/// receptionist's entry at check-in; BL-CLN-02). It replaces what was entered
/// before.
///
/// # Errors
///
/// A 404 for an unknown appointment; a 422 for an appointment that is not
/// insured, is cancelled or checked in.
pub async fn set_services(
    state: &AppState,
    by: Option<i64>,
    booking: i64,
    services: Vec<VisitService>,
) -> Result<PendingCheckIn> {
    let (appointment, policy) = target(state, booking).await?;
    let pending = ensure_pending(state, &appointment, &policy, by).await?;
    set(
        state,
        pending.id,
        &[("services", &Json(services)), ("recorded_by", &by)],
    )
    .await?;
    PendingCheckIn::find_or_404(&state.db, pending.id).await
}

/// The patient signs the self-pay consent for the services a prior
/// authorisation does not cover (and only those); the coverage outcome stays
/// as it is and the consent is kept with the pending data
/// ([`crate::auth_check::sign_consent`]).
///
/// # Errors
///
/// A 422 when every service is covered, a service has no self-pay price or
/// the signer's name is blank.
pub async fn sign_authorisation_consent(
    state: &AppState,
    by: Option<i64>,
    booking: i64,
    signer_name: &str,
    ip: Option<&str>,
    at: DateTime,
) -> Result<SelfPayConsent> {
    let (appointment, policy) = target(state, booking).await?;
    let pending = ensure_pending(state, &appointment, &policy, by).await?;
    let signed =
        auth_check::sign_consent(state, by, &appointment, &pending, signer_name, ip, at).await?;
    SelfPayConsent::query()
        .where_(SelfPayConsent::ID.eq(signed.id))
        .update(
            &state.db,
            &[("pending_id", &Some(pending.id)), ("updated_at", &now())],
        )
        .await?;
    set(state, pending.id, &[("consent_id", &Some(signed.id))]).await?;
    SelfPayConsent::find_or_404(&state.db, signed.id).await
}

/// Discards the data in force for `booking`, if there is any: the appointment
/// was cancelled or rescheduled, booked with another policy, or the day ended.
/// A consent already signed stays on file, marked superseded. Returns whether
/// there was anything to discard.
///
/// # Errors
///
/// The database's errors.
pub async fn discard_for(
    state: &AppState,
    booking: i64,
    reason: &str,
    on: NaiveDate,
) -> Result<bool> {
    let Some(pending) = PendingCheckIn::active_of(&state.db, booking).await? else {
        return Ok(false);
    };
    set(
        state,
        pending.id,
        &[
            ("discarded_on", &Some(on)),
            ("discard_reason", &Some(reason.to_owned())),
        ],
    )
    .await?;
    consent::supersede_all(state, booking, on).await?;
    Ok(true)
}

/// The data not moved to a visit by the end of the appointment day is
/// discarded: every row of a day before today's, and, from 23:59 on, the rows
/// of today (Asia/Dubai). Returns how many were discarded.
///
/// # Errors
///
/// The database's errors.
pub async fn discard_lapsed(state: &AppState, at: DateTime) -> Result<usize> {
    let day = dubai_date(at);
    let cutoff = NaiveTime::parse_from_str(DAY_ENDS_AT, "%H:%M").expect("23:59 is a time");
    let ended = calendar(state).local(at).time() >= cutoff;
    let rows = PendingCheckIn::query()
        .where_(PendingCheckIn::MOVED_VISIT_ID.is_null())
        .where_(PendingCheckIn::DISCARDED_ON.is_null())
        .where_(PendingCheckIn::VISIT_DATE.lte(day))
        .order_by(PendingCheckIn::ID)
        .get(&state.db)
        .await?;
    let mut discarded = 0;
    for row in rows {
        if row.visit_date < day || ended {
            discard_for(state, row.booking_id, DISCARD_LAPSED, day).await?;
            discarded += 1;
        }
    }
    Ok(discarded)
}

/// The answer to `check_in` of the booking: an insured appointment may be
/// checked in only with an eligibility result and a coverage outcome (and,
/// for self-pay, the signed consent); any other appointment is let through.
///
/// # Errors
///
/// The database's errors.
pub async fn check_in_answer(state: &AppState, booking: i64) -> Result<CheckInAnswer> {
    let Some(appointment) = Appointment::of_booking(&state.db, booking).await? else {
        return Ok(CheckInAnswer::Allowed);
    };
    if appointment.policy_id.is_none() {
        return Ok(CheckInAnswer::Allowed);
    }
    let pending = PendingCheckIn::active_of(&state.db, booking)
        .await?
        .filter(PendingCheckIn::is_complete);
    let Some(pending) = pending else {
        return Ok(CheckInAnswer::Refused(english(
            "clinics.coverage.problem.outcome_required",
            &[],
        )));
    };
    if pending.outcome == Some(CoverageOutcome::SelfPay) {
        let signed = SelfPayConsent::current(&state.db, booking)
            .await?
            .is_some_and(|c| c.lists(&appointment.service_code));
        if !signed {
            return Ok(CheckInAnswer::Refused(english(
                "clinics.consent.problem.required",
                &[],
            )));
        }
        return Ok(CheckInAnswer::Allowed);
    }
    // A service that needs a prior authorisation goes ahead only with a valid
    // authorisation that has units left, or as self-pay with a consent that
    // lists it (BL-CLN-02).
    let needs = auth_check::plan(state, &appointment, Some(&pending)).await?;
    let short = auth_check::shortfalls(&needs);
    if !short.is_empty() && !auth_check::consented(state, booking, &short).await? {
        return Ok(CheckInAnswer::Refused(english(
            "clinics.authorisation.problem.required",
            &[],
        )));
    }
    Ok(CheckInAnswer::Allowed)
}

/// The guard wl-scheduling asks at `check_in` (`wl_kernel::scheduling`).
#[derive(Debug, Clone, Copy, Default)]
pub struct CoverageGuard;

impl CheckInGuard for CoverageGuard {
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        booking: BookingId,
        _at: DateTime,
    ) -> SchedulingFuture<'a, CheckInAnswer> {
        Box::pin(async move { check_in_answer(state, booking.0).await })
    }
}

/// The check-in opened `visit`: the data recorded for the appointment moves
/// to it (the eligibility result, the outcome, the policy of the split, the
/// consent) and the pending data is closed. A visit with no data in force
/// keeps nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn move_to_visit(state: &AppState, visit: &Visit) -> Result<bool> {
    let Some(pending) = PendingCheckIn::active_of(&state.db, visit.booking_id).await? else {
        return Ok(false);
    };
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[
                ("eligibility_check_id", &pending.eligibility_check_id),
                ("eligibility_positive", &pending.eligibility_positive),
                ("coverage_outcome", &pending.outcome),
                ("coverage_policy_id", &pending.coverage_policy_id),
                ("consent_id", &pending.consent_id),
                ("updated_at", &now()),
            ],
        )
        .await?;
    if let Some(consent) = pending.consent_id {
        SelfPayConsent::query()
            .where_(SelfPayConsent::ID.eq(consent))
            .update(
                &state.db,
                &[("visit_id", &Some(visit.id)), ("updated_at", &now())],
            )
            .await?;
    }
    // The units of the authorisations the visit relies on are held against
    // it until it is completed or cancelled (BL-CLN-02).
    auth_check::hold_units(state, visit, &pending).await?;
    set(state, pending.id, &[("moved_visit_id", &Some(visit.id))]).await?;
    Ok(true)
}

/// The eligibility turned positive after self-pay (or a re-split) was chosen:
/// before `start` the receptionist records the positive result and changes
/// the outcome to *insured*; a signed consent stays on file, marked
/// superseded. After `start` the outcome is fixed.
///
/// # Errors
///
/// A 422 for a visit that is not insured, is already *insured*, has started or
/// ended, or whose policy or card does not cover the visit day.
pub async fn change_to_insured(
    state: &AppState,
    by: Option<i64>,
    visit: i64,
    reference: Option<&str>,
    at: DateTime,
) -> Result<Visit> {
    let visit = Visit::find_or_404(&state.db, visit).await?;
    let Some(policy_id) = visit.policy_id else {
        return Err(refused(english(
            "clinics.coverage.problem.not_insured",
            &[],
        )));
    };
    if visit.status != VisitStatus::CheckedIn {
        return Err(refused(english("clinics.coverage.problem.fixed", &[])));
    }
    if visit.coverage_outcome == Some(CoverageOutcome::Insured) {
        return Err(refused(english("clinics.coverage.problem.already", &[])));
    }
    let policy = InsurancePolicy::find_or_404(&state.db, policy_id).await?;
    if !policy.is_valid_on(visit.visit_date) {
        return Err(refused(english("clinics.coverage.problem.insured", &[])));
    }
    let check =
        policies::record_eligibility(state, by, policy.id, true, reference, None, at).await?;
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[
                ("eligibility_check_id", &Some(check.id)),
                ("eligibility_positive", &Some(true)),
                ("coverage_outcome", &Some(CoverageOutcome::Insured)),
                ("coverage_policy_id", &Some(policy.id)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    consent::supersede_all(state, visit.booking_id, dubai_date(at)).await?;
    Visit::find_or_404(&state.db, visit.id).await
}

/// `BookingCancelled`: what was recorded for the appointment is discarded.
///
/// # Errors
///
/// The database's errors.
pub async fn on_cancelled(state: &AppState, event: &BookingCancelled) -> Result<bool> {
    discard_for(state, event.booking.0, DISCARD_CANCELLED, dubai_date(now())).await
}

/// `BookingRescheduled`: what was recorded for the old time is discarded.
///
/// # Errors
///
/// The database's errors.
pub async fn on_rescheduled(state: &AppState, event: &BookingRescheduled) -> Result<bool> {
    discard_for(
        state,
        event.booking.0,
        DISCARD_RESCHEDULED,
        dubai_date(now()),
    )
    .await
}

/// Registers the check-in guard, the listeners and the nightly discard.
pub(crate) fn register(app: &mut Registry) {
    wl_kernel::scheduling::provide_check_in_guard(app, CoverageGuard);
    app.listen(|event: BookingCancelled, state: AppState| async move {
        on_cancelled(&state, &event).await.map(|_| ())
    });
    app.listen(|event: BookingRescheduled, state: AppState| async move {
        on_rescheduled(&state, &event).await.map(|_| ())
    });
    app.schedule()
        .daily_at(DAY_ENDS_AT, TASK, |state: AppState| async move {
            discard_lapsed(&state, now()).await.map(|_| ())
        })
        .timezone("Asia/Dubai");
    app.command(
        TASK,
        "Discard the coverage data recorded for appointments not checked in by the end of their day (CLN.ACL.15)",
        |_args, state: AppState| async move {
            let discarded = discard_lapsed(&state, now()).await?;
            println!("{discarded} pending check-ins discarded.");
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outcomes_are_stored_by_name() {
        assert_eq!(CoverageOutcome::Insured.as_str(), "insured");
        assert_eq!(CoverageOutcome::ReSplit.as_str(), "re_split");
        assert_eq!(CoverageOutcome::SelfPay.as_str(), "self_pay");
    }

    #[test]
    fn the_data_is_complete_with_a_result_and_an_outcome() {
        let mut pending = PendingCheckIn::default();
        assert!(!pending.is_complete());
        pending.eligibility_check_id = Some(1);
        assert!(!pending.is_complete());
        pending.outcome = Some(CoverageOutcome::SelfPay);
        assert!(pending.is_complete());
    }
}
