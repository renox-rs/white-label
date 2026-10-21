//! The appointment follows its booking: `BookingBooked` creates it,
//! `BookingRescheduled` moves it, `BookingCancelled` closes it. The listeners
//! are the app's side of the scheduling contract (`wl_kernel::scheduling`);
//! the app depends on no scheduling crate.

use renox::db::now;
use renox::prelude::*;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::scheduling::{BookingBooked, BookingCancelled, BookingRescheduled};

use crate::appointment::Appointment;
use crate::db::refused;
use crate::policy::InsurancePolicy;
use crate::texts::english;
use crate::visit::Visit;

/// `BookingBooked`: the clinic keeps its record of the booking. The patient
/// is the booking's subject when that is a party; a second event for the
/// same booking changes nothing.
///
/// # Errors
///
/// The database's errors.
pub async fn on_booked(state: &AppState, event: &BookingBooked) -> Result<Appointment> {
    if let Some(found) = Appointment::of_booking(&state.db, event.booking.0).await? {
        return Ok(found);
    }
    let patient = event
        .subject
        .as_ref()
        .filter(|subject| subject.subject_type == PARTY_SUBJECT_TYPE)
        .map(|subject| subject.subject_id);
    let mut appointment = Appointment {
        booking_id: event.booking.0,
        patient_party_id: patient,
        booker_party_id: event.party.0,
        service_code: event.service_type.clone(),
        starts_at: event.start,
        ends_at: event.end,
        visit_date: dubai_date(event.start),
        ..Appointment::default()
    };
    appointment.save(&state.db).await?;
    Ok(appointment)
}

/// `BookingRescheduled`: the appointment moves with its booking.
///
/// # Errors
///
/// The database's errors.
pub async fn on_rescheduled(state: &AppState, event: &BookingRescheduled) -> Result {
    Appointment::query()
        .where_(Appointment::BOOKING_ID.eq(event.booking.0))
        .update(
            &state.db,
            &[
                ("starts_at", &event.start),
                ("ends_at", &event.end),
                ("visit_date", &dubai_date(event.start)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// `BookingCancelled`: the appointment is closed.
///
/// # Errors
///
/// The database's errors.
pub async fn on_cancelled(state: &AppState, event: &BookingCancelled) -> Result {
    Appointment::query()
        .where_(Appointment::BOOKING_ID.eq(event.booking.0))
        .update(
            &state.db,
            &[("cancelled_on", &Some(today())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// Books the appointment of `booking` with the insurance policy `policy`: it
/// is an *insured* visit from now on, and the check-in asks for the
/// eligibility result and the coverage outcome (`CLN.ACL.15`).
///
/// # Errors
///
/// A 404 for an unknown appointment or policy; a 422 when the appointment is
/// cancelled, the patient is already checked in, or the policy is another
/// patient's.
pub async fn attach_policy(state: &AppState, booking: i64, policy: i64) -> Result<Appointment> {
    let appointment = Appointment::of_booking(&state.db, booking)
        .await?
        .ok_or(Error::NotFound)?;
    let found = InsurancePolicy::find_or_404(&state.db, policy).await?;
    if appointment.is_cancelled() {
        return Err(refused(english("clinics.appointments.problem.cancelled", &[])));
    }
    if Visit::of_booking(&state.db, booking).await?.is_some() {
        return Err(refused(english(
            "clinics.appointments.problem.checked_in",
            &[],
        )));
    }
    if appointment
        .patient_party_id
        .is_some_and(|patient| patient != found.patient_party_id)
    {
        return Err(refused(english(
            "clinics.appointments.problem.other_patient",
            &[],
        )));
    }
    Appointment::query()
        .where_(Appointment::ID.eq(appointment.id))
        .update(
            &state.db,
            &[("policy_id", &Some(policy)), ("updated_at", &now())],
        )
        .await?;
    Appointment::find_or_404(&state.db, appointment.id).await
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: BookingBooked, state: AppState| async move {
        on_booked(&state, &event).await.map(|_| ())
    });
    app.listen(|event: BookingRescheduled, state: AppState| async move {
        on_rescheduled(&state, &event).await
    });
    app.listen(|event: BookingCancelled, state: AppState| async move {
        on_cancelled(&state, &event).await
    });
}
