//! `CLN.RUL.10`: ask for a new insurance card before an insured appointment
//! (05 §7.2, §9 *Patient messages after a change*, §3.3 `CLN.ACL.12`;
//! BL-CLN-04).
//!
//! Two days before an insured appointment, when the policy it was booked with
//! ends before the visit day or the card on file expires before it
//! ([`needs_card`]), the patient (the guardian, for a dependant) gets one
//! message with a portal link to upload the new card, and the appointment is
//! flagged for the front desk ([`FlagCause::InsuranceCard`]). The flag clears
//! when staff **accept** a card valid on the appointment day
//! ([`accept_card`]); an upload alone clears nothing.
//!
//! - *When.* The daily 02:00 run (Asia/Dubai) takes the appointments dated
//!   today + [`DAYS_AHEAD`] ([`run_daily`]). An appointment booked, moved or
//!   given a policy inside those days is checked at once ([`on_appointment`]).
//! - *Once.* The check is a kernel rule run per appointment and visit day, so
//!   one message and one flag; a reschedule is a new visit day and runs the
//!   check again.
//! - *The message* is a normal-urgency notification, so wl-notify holds it
//!   until the quiet hours end (08:00 by default) and never drops it. It names
//!   the clinic, the day and the time and carries the link; no service, no
//!   clinical detail. It is never sent for a self-pay appointment.
//! - *The link* is wl-portal's signed link for the party and the action
//!   `document_upload` (`/portal/link/{party}/document_upload`, signed). The
//!   page the patient uploads on is not built (wl-portal has only the guarded
//!   link page): the document kind [`CARD_KIND`] that the host registers lets
//!   wl-documents take the upload, and the app accepts the card when
//!   wl-documents accepts it ([`on_accepted`], from `DocumentAccepted`).
//! - *Accepting a card* sets the card's last valid day and, as a renewed card
//!   states the cover's last day, extends the policy's own last day when it is
//!   earlier. The card image itself is wl-documents' and needs the permission
//!   of its sensitivity class (`CLN.ACL.12`); this app stores none.
//! - *Dependants.* The message goes to the guardians of the patient
//!   (`PTY.REL.01`, through the party directory), else to the patient.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::serde_json::json;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::calendar::calendar;
use wl_kernel::documents::{DocumentAccepted, dubai_date, today};
use wl_kernel::notify::{self, GROUP_DOCUMENTS, NotificationTypeDef, NotifyChannel, NotifyRequest};
use wl_kernel::{ClientConfig, PartyId};

use crate::appointment::Appointment;
use crate::authorisations::addressees;
use crate::db::refused;
use crate::flags::{self, AppointmentFlag, FlagCause};
use crate::permissions::RECEPTIONIST_ROLE;
use crate::policy::{InsurancePolicy, POLICY_SUBJECT_TYPE};
use crate::texts::english;
use crate::visit::Visit;

/// The rule.
pub const RULE: &str = "CLN.RUL.10";
/// N: the days before the appointment the check runs.
pub const DAYS_AHEAD: i64 = 2;
/// The scheduled task, and the command, of the daily run.
pub const TASK: &str = "clinics:card_requests";
/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "02:00";
/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";
/// The notification type of the message.
pub const CARD_UPLOAD_REQUEST: &str = "clinics.card_upload_request";
/// The wl-documents kind of the insurance card; its subject is the policy.
pub const CARD_KIND: &str = "insurance_card";
/// wl-portal's action key for a document upload.
pub const UPLOAD_ACTION: &str = "document_upload";
/// How long the upload link works, in days.
pub const LINK_DAYS: u64 = 7;

/// Whether the policy ends before `visit_date`, or its card expires before
/// it: the patient has to bring a new card.
pub fn needs_card(policy: &InsurancePolicy, visit_date: NaiveDate) -> bool {
    policy.valid_to.is_some_and(|to| to < visit_date)
        || policy.card_valid_to.is_some_and(|to| to < visit_date)
}

/// What a check did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Told {
    /// The parties who got the message.
    pub parties: Vec<PartyId>,
    /// Whether the appointment is flagged.
    pub flagged: bool,
}

/// The portal link to upload a card, for `party`: wl-portal's signed link for
/// the action `document_upload`. Without the portal's route the plain path of
/// it is given.
pub fn upload_link(state: &AppState, party: PartyId) -> String {
    state
        .signed_url(
            "wl_portal.link",
            &[&party.0, &UPLOAD_ACTION],
            Duration::from_secs(LINK_DAYS * 24 * 3600),
        )
        .unwrap_or_else(|_| format!("/portal/link/{}/{UPLOAD_ACTION}", party.0))
}

/// The clinic's name for the message.
fn clinic_name(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map(|config| config.client.name.clone())
        .unwrap_or_default()
}

/// Sends the message and raises the flag for `appointment`.
async fn request_card(
    state: &AppState,
    appointment: &Appointment,
    policy: &InsurancePolicy,
    origin: &str,
) -> Result<Told> {
    let patient = PartyId(
        appointment
            .patient_party_id
            .unwrap_or(policy.patient_party_id),
    );
    let local = calendar(state).local(appointment.starts_at);
    let date = local.format("%d %b %Y").to_string();
    let time = local.format("%H:%M").to_string();
    let clinic = clinic_name(state);
    let mut told = Told::default();
    for party in addressees(state, patient).await {
        let request = NotifyRequest::new(CARD_UPLOAD_REQUEST, appointment.subject())
            .to_party(party)
            .data(json!({
                "clinic": clinic,
                "date": date,
                "time": time,
                "link": upload_link(state, party),
            }));
        notify::notifier(state).notify(state, request).await?;
        told.parties.push(party);
    }
    let detail = english(
        "clinics.flags.card_detail",
        &[("date", &appointment.visit_date)],
    );
    flags::raise(
        state,
        appointment,
        FlagCause::InsuranceCard,
        origin,
        None,
        Some(&detail),
    )
    .await?;
    told.flagged = true;
    Ok(told)
}

/// `CLN.RUL.10` for `appointment`: when its policy ends before the visit day
/// or its card has expired, the patient is asked for a new card and the
/// appointment is flagged, once for the visit day. When the cover is fine
/// the flag, if there was one, is cleared. `None` when the appointment is not
/// insured, is cancelled or checked in, or fine; else the run's outcome.
///
/// # Errors
///
/// The database's and the notifier's errors; a failed run is tried again by
/// the kernel.
pub async fn check(
    state: &AppState,
    appointment: &Appointment,
    origin: &str,
) -> Result<Option<(RunOutcome, Told)>> {
    let booking = appointment.booking_id;
    let Some(policy_id) = appointment
        .policy_id
        .filter(|_| !appointment.is_cancelled())
    else {
        flags::clear(state, booking, FlagCause::InsuranceCard, "not_insured").await?;
        return Ok(None);
    };
    if Visit::of_booking(&state.db, booking).await?.is_some() {
        return Ok(None);
    }
    let policy = InsurancePolicy::find_or_404(&state.db, policy_id).await?;
    if !needs_card(&policy, appointment.visit_date) {
        flags::clear(state, booking, FlagCause::InsuranceCard, "valid").await?;
        return Ok(None);
    }
    let key = format!("{RULE}:{booking}:{}", appointment.visit_date);
    let spec = RuleRunSpec::new(RULE, appointment.subject(), key);
    let told = Arc::new(Mutex::new(Told::default()));
    let out = Arc::clone(&told);
    let (appointment, origin) = (appointment.clone(), origin.to_owned());
    let outcome = run_rule(state, spec, move |state: AppState| async move {
        let done = request_card(&state, &appointment, &policy, &origin).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = done;
        Ok(())
    })
    .await?;
    let told = told.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok(Some((outcome, told)))
}

/// An appointment was booked, moved or given a policy: when its day is today
/// or within [`DAYS_AHEAD`] days it is checked at once. Returns the check's
/// result.
///
/// # Errors
///
/// As [`check`].
pub async fn on_appointment(
    state: &AppState,
    appointment: &Appointment,
    origin: &str,
) -> Result<Option<(RunOutcome, Told)>> {
    let ahead = (appointment.visit_date - today()).num_days();
    if (0..=DAYS_AHEAD).contains(&ahead) {
        check(state, appointment, origin).await
    } else {
        Ok(None)
    }
}

/// The daily run: the insured appointments dated two days after the day of
/// `at` (Asia/Dubai) are checked. Returns the bookings whose check ran.
///
/// # Errors
///
/// As [`check`].
pub async fn run_daily(state: &AppState, at: DateTime) -> Result<Vec<i64>> {
    let target = dubai_date(at) + TimeDelta::days(DAYS_AHEAD);
    let due = Appointment::query()
        .where_(Appointment::VISIT_DATE.eq(target))
        .where_(Appointment::CANCELLED_ON.is_null())
        .order_by(Appointment::STARTS_AT)
        .get(&state.db)
        .await?;
    let mut ran = Vec::new();
    for appointment in due.into_iter().filter(Appointment::is_insured) {
        if let Some((RunOutcome::Succeeded, _)) = check(state, &appointment, "card").await? {
            ran.push(appointment.booking_id);
        }
    }
    Ok(ran)
}

/// What accepting a card did.
#[derive(Debug, Clone)]
pub struct Accepted {
    /// The policy as it is now.
    pub policy: InsurancePolicy,
    /// The bookings whose flag was cleared.
    pub cleared: Vec<i64>,
}

/// Staff accept the card of `policy`: valid through `card_valid_to` (`None`:
/// it does not expire). A renewed card states the cover's last day, so the
/// policy's own last day is extended to it when it is earlier.
///
/// Decision (05 §7.2, `CLN.RUL.10`, kept after review): the rule asks for a new
/// card when *the policy ends before the appointment or the card has expired*,
/// and the flag clears when staff accept "an uploaded card valid on the
/// appointment date". The second half only makes sense for the first trigger
/// if the accepted card carries the renewed cover with it, so accepting a card
/// valid later than the policy's own end moves that end to the card's date
/// (never earlier). The spec has no other renewal step for a policy. The flag of
/// every appointment of the policy whose day the policy and the card now
/// cover is cleared; one they do not cover stays.
///
/// # Errors
///
/// A 404 for an unknown policy; a 422 for a card that has already expired.
pub async fn accept_card(
    state: &AppState,
    policy: i64,
    card_valid_to: Option<NaiveDate>,
) -> Result<Accepted> {
    let found = InsurancePolicy::find_or_404(&state.db, policy).await?;
    if card_valid_to.is_some_and(|to| to < today()) {
        return Err(refused(english(
            "clinics.policies.problem.card_expired",
            &[],
        )));
    }
    let valid_to = match (found.valid_to, card_valid_to) {
        (Some(to), Some(card)) if to < card => Some(card),
        (current, _) => current,
    };
    InsurancePolicy::query()
        .where_(InsurancePolicy::ID.eq(found.id))
        .update(
            &state.db,
            &[
                ("card_valid_to", &card_valid_to),
                ("valid_to", &valid_to),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let updated = InsurancePolicy::find_or_404(&state.db, found.id).await?;
    let open = AppointmentFlag::query()
        .where_(AppointmentFlag::CAUSE.eq(FlagCause::InsuranceCard))
        .where_(AppointmentFlag::CLEARED_ON.is_null())
        .order_by(AppointmentFlag::ID)
        .get(&state.db)
        .await?;
    let mut cleared = Vec::new();
    for flag in open {
        let Some(appointment) = Appointment::of_booking(&state.db, flag.booking_id).await? else {
            continue;
        };
        if appointment.policy_id == Some(updated.id)
            && updated.is_valid_on(appointment.visit_date)
            && flags::clear(
                state,
                flag.booking_id,
                FlagCause::InsuranceCard,
                "card_accepted",
            )
            .await?
            .is_some()
        {
            cleared.push(flag.booking_id);
        }
    }
    Ok(Accepted {
        policy: updated,
        cleared,
    })
}

/// wl-documents accepted a document: an insurance card of a policy is the
/// card the patient uploaded, now accepted by staff.
///
/// # Errors
///
/// As [`accept_card`].
pub async fn on_accepted(state: &AppState, event: &DocumentAccepted) -> Result<Option<Accepted>> {
    if event.kind != CARD_KIND || event.subject.subject_type != POLICY_SUBJECT_TYPE {
        return Ok(None);
    }
    accept_card(state, event.subject.subject_id, event.expires_on)
        .await
        .map(Some)
}

/// The notification type of the message.
pub fn notification_types() -> Vec<NotificationTypeDef> {
    let subject = english("clinics.cards.subject", &[]);
    let body = english("clinics.cards.body", &[]);
    vec![
        NotificationTypeDef::new(CARD_UPLOAD_REQUEST, "clinics")
            .group(GROUP_DOCUMENTS)
            .staff_role(RECEPTIONIST_ROLE)
            .template(NotifyChannel::Email, "en", Some(&subject), &body),
    ]
}

/// Registers the notification type, the listener and the daily run.
pub(crate) fn register(app: &mut Registry) {
    for def in notification_types() {
        notify::register_type(def);
    }
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        on_accepted(&state, &event).await.map(|_| ())
    });
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run_daily(&state, now()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Ask the patients of insured appointments in two days for a new insurance card when the policy ends or the card expired (CLN.RUL.10)",
        |_args, state: AppState| async move {
            let ran = run_daily(&state, now()).await?;
            println!("{} insurance card requests sent.", ran.len());
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn policy(valid_to: Option<&str>, card_to: Option<&str>) -> InsurancePolicy {
        InsurancePolicy {
            valid_from: day("2026-01-01"),
            valid_to: valid_to.map(day),
            card_valid_to: card_to.map(day),
            ..InsurancePolicy::default()
        }
    }

    #[test]
    fn a_policy_that_ends_before_the_visit_needs_a_card() {
        assert!(needs_card(
            &policy(Some("2026-11-10"), None),
            day("2026-11-12")
        ));
        assert!(!needs_card(
            &policy(Some("2026-11-12"), None),
            day("2026-11-12")
        ));
    }

    #[test]
    fn an_expired_card_needs_a_new_one_even_on_a_policy_in_force() {
        assert!(needs_card(
            &policy(Some("2027-06-30"), Some("2026-11-11")),
            day("2026-11-12")
        ));
        assert!(!needs_card(&policy(None, None), day("2026-11-12")));
        assert!(!needs_card(
            &policy(Some("2027-06-30"), Some("2026-11-12")),
            day("2026-11-12")
        ));
    }
}
