//! The rules about the prior authorisation an appointment needs (05 §7.2
//! `CLN.RUL.07`, `CLN.RUL.09`; base and BL-CLN-03).
//!
//! `CLN.RUL.09`, when an authorisation is denied, used up or expires, and
//! [`LOOK_AHEAD_DAYS`] days before its last valid day: the future
//! appointments of the patient that relied on it (an insured appointment on a
//! policy of the authorisation's payer, for a service it covers, not yet
//! checked in) and that no other valid authorisation with units left covers
//! on their day are flagged, and the billing coordinator gets **one task per
//! authorisation and trigger** listing them ([`run_for`], [`look_ahead`]).
//! On *denied* and *used* every such appointment from today on is flagged; on
//! *expired* and on the look-ahead only the ones dated after the last valid
//! day (the ones inside the validity are still covered). The run is a kernel
//! rule run per authorisation and trigger, so a second run changes nothing; a
//! flag is also never raised twice, and the task is deduplicated by its key.
//! It cancels and reschedules nothing and tells the patient nothing (the
//! denial's own notice is the authorisation's).
//!
//! `CLN.RUL.07`, when an appointment is booked for a service that needs a
//! prior authorisation and no `approved` authorisation is valid on its date:
//! the appointment is flagged ([`FlagCause::NeedsAuthorisation`]) and the
//! billing coordinator gets a task to request one. The service needs one when
//! the payer's list has it ([`crate::authorisation::PriorAuthService`]).
//!
//! Decisions:
//!
//! - *The appointment is insured when it has a policy.* The payer comes from
//!   the policy, so the rule is evaluated when the appointment is booked, when
//!   a policy is attached to it, and when it is rescheduled ([`evaluate`]).
//!   An appointment with no policy is self-pay and is never flagged; one that
//!   is checked in is left as it is.
//! - *"Notify the billing coordinator"* is a staff task for the role
//!   ([`needed_key`]), one per appointment, closed when the flag is cleared.
//! - *Valid* means [`crate::authorisations::covering`]: usable, valid through
//!   the visit day, with units left.
//! - A new authorisation approved for the patient re-evaluates the open flags
//!   of the patient's appointments ([`reevaluate_patient`]); a cancellation
//!   or a check-in clears them.

use std::sync::{Arc, Mutex, PoisonError};

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::automation::{RuleRunSpec, RunOutcome, run_rule};
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::task_creator;

use crate::appointment::Appointment;
use crate::authorisation::{PriorAuthService, PriorAuthorisation};
use crate::authorisation_machine::{APPROVE, APPROVE_PARTIALLY, CONSUME_LAST, DENY, EXPIRE};
use crate::authorisations::covering;
use crate::flags::{self, AppointmentFlag, FlagCause};
use crate::permissions::BILLING_COORDINATOR_ROLE;
use crate::coverage::{CoverageOutcome, PendingCheckIn};
use crate::policy::InsurancePolicy;
use crate::texts::english;
use crate::visit::Visit;

/// The rule of the flag at booking.
pub const RULE_BOOKED: &str = "CLN.RUL.07";

/// The rule of the flags when an authorisation stops covering.
pub const RULE_STOPPED: &str = "CLN.RUL.09";

/// D: the days before the last valid day the look-ahead runs.
pub const LOOK_AHEAD_DAYS: i64 = 3;

/// What stopped an authorisation covering the appointments that relied on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The payer refused it.
    Denied,
    /// The last unit was consumed.
    Used,
    /// Its last valid day passed.
    Expired,
    /// [`LOOK_AHEAD_DAYS`] days before its last valid day.
    LookAhead,
}

impl Trigger {
    /// The name the flag and the keys carry.
    pub fn key(self) -> &'static str {
        match self {
            Trigger::Denied => "denied",
            Trigger::Used => "used",
            Trigger::Expired => "expired",
            Trigger::LookAhead => "look_ahead",
        }
    }

    /// Whether only the appointments after the last valid day lost their
    /// cover: on *expired* and on the look-ahead the ones inside the validity
    /// are still covered.
    fn after_validity_only(self) -> bool {
        matches!(self, Trigger::Expired | Trigger::LookAhead)
    }
}

/// The source key of the coordinator's task for `authorisation` and `trigger`.
pub fn task_key(authorisation: i64, trigger: Trigger) -> String {
    format!("clinics.prior_auth:{authorisation}:{}", trigger.key())
}

/// What a run of `CLN.RUL.09` flagged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flagged {
    /// The bookings that are flagged because of the run, by start.
    pub bookings: Vec<i64>,
    /// Whether the coordinator's task was opened or already was.
    pub task: bool,
}

/// The source key of the billing coordinator's task for the appointment of
/// `booking`.
pub fn needed_key(booking: i64) -> String {
    format!("clinics.prior_auth_needed:{booking}")
}

/// What [`evaluate`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The appointment is not insured, is cancelled or checked in, or its
    /// service needs no authorisation: no flag.
    NotApplicable,
    /// A valid authorisation with units left covers it: no flag.
    Covered,
    /// It needs one and has none: flagged. `true` when this call raised the
    /// flag.
    Flagged(bool),
}

/// Closes the billing coordinator's task `key`, if it is still open.
pub(crate) async fn close_task(state: &AppState, key: &str, note: &str) -> Result<bool> {
    let creator = task_creator(state);
    match creator.find_open(state, key).await? {
        Some(found) => creator.resolve_from_source(state, found.id, note).await,
        None => Ok(false),
    }
}

/// Clears the needs-authorisation flag of `booking` and closes its task.
async fn clear_needed(state: &AppState, booking: i64, reason: &str) -> Result<bool> {
    let cleared = flags::clear(state, booking, FlagCause::NeedsAuthorisation, reason)
        .await?
        .is_some();
    if cleared {
        close_task(state, &needed_key(booking), reason).await?;
    }
    Ok(cleared)
}

/// `CLN.RUL.07` for `appointment`: flags it when its service needs a prior
/// authorisation and none covers its day; clears the flag when one does (or
/// the appointment stopped being insured, or needing one); idempotent.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn evaluate(
    state: &AppState,
    appointment: &Appointment,
    origin: &str,
) -> Result<Verdict> {
    let booking = appointment.booking_id;
    let Some(policy_id) = appointment.policy_id.filter(|_| !appointment.is_cancelled()) else {
        clear_needed(state, booking, "not_insured").await?;
        return Ok(Verdict::NotApplicable);
    };
    if Visit::of_booking(&state.db, booking).await?.is_some() {
        return Ok(Verdict::NotApplicable);
    }
    let policy = InsurancePolicy::find_or_404(&state.db, policy_id).await?;
    let code = &appointment.service_code;
    if !PriorAuthService::needed(&state.db, policy.payer_party_id, code).await? {
        clear_needed(state, booking, "not_needed").await?;
        return Ok(Verdict::NotApplicable);
    }
    let found = covering(
        &state.db,
        policy.patient_party_id,
        policy.payer_party_id,
        code,
        appointment.visit_date,
        None,
    )
    .await?;
    if !found.is_empty() {
        clear_needed(state, booking, "covered").await?;
        return Ok(Verdict::Covered);
    }
    let detail = english(
        "clinics.flags.needed_detail",
        &[("service", code), ("date", &appointment.visit_date)],
    );
    let (_, created) = flags::raise(
        state,
        appointment,
        FlagCause::NeedsAuthorisation,
        origin,
        None,
        Some(&detail),
    )
    .await?;
    // A reschedule asks the coordinator again: the task is opened if the
    // earlier one was closed (a task still open is the same one).
    if created || origin == "rescheduled" {
        let title = english(
            "clinics.flags.needed_task",
            &[("service", code), ("date", &appointment.visit_date)],
        );
        let task = RoleTask::new(
            appointment.subject(),
            BILLING_COORDINATOR_ROLE,
            &title,
            &needed_key(booking),
        )
        .description(&detail);
        role_tasks::open(state, task).await?;
    }
    Ok(Verdict::Flagged(created))
}

/// A new authorisation was approved for `patient`: every open
/// needs-authorisation flag on their appointments is checked again and
/// cleared when something covers the appointment now. Returns how many were
/// cleared.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn reevaluate_patient(state: &AppState, patient: i64, origin: &str) -> Result<usize> {
    let open = AppointmentFlag::query()
        .where_(AppointmentFlag::CAUSE.eq(FlagCause::NeedsAuthorisation))
        .where_(AppointmentFlag::CLEARED_ON.is_null())
        .order_by(AppointmentFlag::ID)
        .get(&state.db)
        .await?;
    let mut cleared = 0;
    for flag in open {
        let Some(appointment) = Appointment::of_booking(&state.db, flag.booking_id).await? else {
            continue;
        };
        if appointment.patient_party_id != Some(patient) {
            continue;
        }
        if evaluate(state, &appointment, origin).await? == Verdict::Covered {
            cleared += 1;
        }
    }
    Ok(cleared)
}

/// The appointment of `booking` was cancelled: its flags go, and the task.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn on_cancelled(state: &AppState, booking: i64) -> Result<usize> {
    let cleared = flags::clear_all(state, booking, "cancelled").await?;
    close_task(state, &needed_key(booking), "cancelled").await?;
    Ok(cleared)
}

/// The patient was checked in: the flags no longer hold.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn on_checked_in(state: &AppState, booking: i64) -> Result<usize> {
    let cleared = flags::clear_all(state, booking, "checked_in").await?;
    close_task(state, &needed_key(booking), "checked_in").await?;
    Ok(cleared)
}

/// Flags the appointments that relied on `authorisation` and lost their
/// cover to `trigger`, and opens the one task listing them. The appointments
/// are the patient's, insured, not cancelled and not checked in, dated from
/// `from` on, for a service the authorisation covers, on a policy of its
/// payer, with no other valid authorisation with units left on their day.
async fn flag_relying(
    state: &AppState,
    authorisation: &PriorAuthorisation,
    trigger: Trigger,
    from: NaiveDate,
) -> Result<Flagged> {
    let appointments = Appointment::query()
        .where_(Appointment::PATIENT_PARTY_ID.eq(authorisation.patient_party_id))
        .where_(Appointment::CANCELLED_ON.is_null())
        .where_(Appointment::VISIT_DATE.gte(from))
        .order_by(Appointment::STARTS_AT)
        .get(&state.db)
        .await?;
    let mut done = Flagged::default();
    let mut lines = Vec::new();
    for appointment in appointments {
        if !authorisation.covers(&appointment.service_code) {
            continue;
        }
        if trigger.after_validity_only()
            && authorisation
                .valid_until
                .is_some_and(|until| appointment.visit_date <= until)
        {
            continue;
        }
        let Some(policy_id) = appointment.policy_id else {
            continue;
        };
        let policy = InsurancePolicy::find_or_404(&state.db, policy_id).await?;
        if policy.payer_party_id != authorisation.payer_party_id
            || Visit::of_booking(&state.db, appointment.booking_id).await?.is_some()
        {
            continue;
        }
        let pending = PendingCheckIn::active_of(&state.db, appointment.booking_id).await?;
        if pending.is_some_and(|p| p.outcome == Some(CoverageOutcome::SelfPay)) {
            continue;
        }
        let other = covering(
            &state.db,
            authorisation.patient_party_id,
            authorisation.payer_party_id,
            &appointment.service_code,
            appointment.visit_date,
            Some(authorisation.id),
        )
        .await?;
        if !other.is_empty() {
            continue;
        }
        let detail = english(
            "clinics.flags.stopped_detail",
            &[
                ("number", &authorisation.number()),
                ("cause", &english(&format!("clinics.flags.cause_{}", trigger.key()), &[])),
            ],
        );
        flags::raise(
            state,
            &appointment,
            FlagCause::NeedsAuthorisation,
            trigger.key(),
            Some(authorisation.id),
            Some(&detail),
        )
        .await?;
        done.bookings.push(appointment.booking_id);
        lines.push(format!("{} {}", appointment.visit_date, appointment.service_code));
    }
    if !done.bookings.is_empty() {
        let title = english(
            "clinics.flags.stopped_task",
            &[
                ("number", &authorisation.number()),
                ("cause", &english(&format!("clinics.flags.cause_{}", trigger.key()), &[])),
            ],
        );
        let task = RoleTask::new(
            authorisation.subject(),
            BILLING_COORDINATOR_ROLE,
            &title,
            &task_key(authorisation.id, trigger),
        )
        .description(&lines.join("; "));
        done.task = role_tasks::open(state, task).await?.is_some();
    }
    Ok(done)
}

/// `CLN.RUL.09` for authorisation `id` and `trigger`, as a kernel rule run
/// (one per authorisation and trigger; the look-ahead one per last valid
/// day): `from` is the day the appointments must be dated on or after. A
/// second call for the same occurrence is [`RunOutcome::Skipped`].
///
/// # Errors
///
/// The database's and the task creator's errors; a run that fails is tried
/// again by the kernel.
pub async fn run_for(
    state: &AppState,
    id: i64,
    trigger: Trigger,
    from: NaiveDate,
) -> Result<(RunOutcome, Flagged)> {
    let authorisation = PriorAuthorisation::find_or_404(&state.db, id).await?;
    let key = match (trigger, authorisation.valid_until) {
        (Trigger::LookAhead, Some(until)) => format!("{RULE_STOPPED}:{id}:look_ahead:{until}"),
        _ => format!("{RULE_STOPPED}:{id}:{}", trigger.key()),
    };
    let spec = RuleRunSpec::new(RULE_STOPPED, authorisation.subject(), key);
    let found = Arc::new(Mutex::new(Flagged::default()));
    let out = Arc::clone(&found);
    let outcome = run_rule(state, spec, move |state: AppState| async move {
        let done = flag_relying(&state, &authorisation, trigger, from).await?;
        *out.lock().unwrap_or_else(PoisonError::into_inner) = done;
        Ok(())
    })
    .await?;
    let found = found.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Ok((outcome, found))
}

/// The look-ahead of the daily run: the usable authorisations whose last
/// valid day is [`LOOK_AHEAD_DAYS`] days after the day of `at` (Asia/Dubai)
/// run `CLN.RUL.09` once. Returns the ids that were run.
///
/// # Errors
///
/// The database's and the task creator's errors.
pub async fn look_ahead(state: &AppState, at: DateTime) -> Result<Vec<i64>> {
    let day = dubai_date(at);
    let target = day + TimeDelta::days(LOOK_AHEAD_DAYS);
    let due = PriorAuthorisation::query()
        .where_(PriorAuthorisation::VALID_UNTIL.eq(target))
        .order_by(PriorAuthorisation::ID)
        .get(&state.db)
        .await?;
    let mut ran = Vec::new();
    for authorisation in due.into_iter().filter(|a| a.status.is_usable()) {
        let (outcome, _) = run_for(state, authorisation.id, Trigger::LookAhead, day).await?;
        if outcome == RunOutcome::Succeeded {
            ran.push(authorisation.id);
        }
    }
    Ok(ran)
}

/// Registers the listeners: an approval re-evaluates the patient's flags; a
/// denied, used up or expired authorisation flags the appointments that
/// relied on it.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<PriorAuthorisation>, state: AppState| async move {
            let trigger = match event.transition.as_str() {
                APPROVE | APPROVE_PARTIALLY => {
                    let authorisation =
                        PriorAuthorisation::find_or_404(&state.db, event.key).await?;
                    reevaluate_patient(&state, authorisation.patient_party_id, "approved").await?;
                    return Ok(());
                }
                DENY => Trigger::Denied,
                CONSUME_LAST => Trigger::Used,
                EXPIRE => Trigger::Expired,
                _ => return Ok(()),
            };
            run_for(&state, event.key, trigger, today()).await?;
            Ok(())
        },
    );
}
