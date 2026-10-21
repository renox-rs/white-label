//! The rules about the prior authorisation an appointment needs (05 §7.2
//! `CLN.RUL.07`, `CLN.RUL.09`; base and BL-CLN-03).
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

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::task_creator;

use crate::appointment::Appointment;
use crate::authorisation::{PriorAuthService, PriorAuthorisation};
use crate::authorisation_machine::{APPROVE, APPROVE_PARTIALLY};
use crate::authorisations::covering;
use crate::flags::{self, AppointmentFlag, FlagCause};
use crate::permissions::BILLING_COORDINATOR_ROLE;
use crate::policy::InsurancePolicy;
use crate::texts::english;
use crate::visit::Visit;

/// The rule of the flag at booking.
pub const RULE_BOOKED: &str = "CLN.RUL.07";

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

/// Registers the listener: an approval re-evaluates the patient's flags.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<PriorAuthorisation>, state: AppState| async move {
            if event.transition == APPROVE || event.transition == APPROVE_PARTIALLY {
                let authorisation = PriorAuthorisation::find_or_404(&state.db, event.key).await?;
                reevaluate_patient(&state, authorisation.patient_party_id, "approved").await?;
            }
            Ok(())
        },
    );
}
