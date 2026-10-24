//! The job card follows its booking (08 §7.1 *Booking*, `AUT.JOB.01`): the
//! check-in of a workshop booking opens the job card.
//!
//! The listener is the app's side of the scheduling contract
//! (`wl_kernel::scheduling`): [`on_checked_in`] on `BookingCheckedIn`. It runs
//! after the booking's transaction committed. The booking's subject is the
//! vehicle (`auto.vehicle:N`); the job card is opened for it, billed to its
//! owner, in the workshop (the scope) of the booking, and linked back to the
//! booking so that a cancellation after the check-in asks the job card what
//! was done (`SCH.ACL.06`). A second event for the booking returns the job
//! card that exists; a booking that is not for a vehicle opens nothing. The
//! advisor then records the intake ([`crate::job_cards::record_intake`]).

use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::scheduling::{BOOKING_SUBJECT_TYPE, BookingCheckedIn, BookingId, booking_actions};
use wl_kernel::{Subject, scopes};

use crate::job_card::{JobCard, JobCardStatus};
use crate::job_cards::CHECK_IN;
use crate::seed::VEHICLE_SUBJECT_TYPE;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// `BookingCheckedIn`: opens the job card of the booking, in `checked_in`,
/// without its intake. A booking that is not for a vehicle of the register
/// opens nothing.
///
/// # Errors
///
/// The database's and the scheduling contract's errors.
pub async fn on_checked_in(state: &AppState, event: &BookingCheckedIn) -> Result<Option<JobCard>> {
    if let Some(found) = JobCard::of_booking(&state.db, event.booking.0).await? {
        return Ok(Some(found));
    }
    let Some(subject) = event
        .subject
        .as_ref()
        .filter(|subject| subject.subject_type == VEHICLE_SUBJECT_TYPE)
    else {
        return Ok(None);
    };
    let Some(vehicle) = Vehicle::find(&state.db, subject.subject_id).await? else {
        return Ok(None);
    };
    let scope = scopes::scope_of(
        &state.db,
        &Subject::new(BOOKING_SUBJECT_TYPE, event.booking.0),
    )
    .await?;
    let workshop = if scope.is_global() {
        None
    } else {
        scope
            .id()
            .parse::<i64>()
            .ok()
            .map(|id| (scope.kind().to_owned(), id))
    };
    let mut card = JobCard {
        vehicle_id: vehicle.id,
        customer_party_id: vehicle.owner_party_id,
        booking_id: Some(event.booking.0),
        workshop_type: workshop.as_ref().map(|(kind, _)| kind.clone()),
        workshop_id: workshop.as_ref().map(|(_, id)| *id),
        complaint: english("auto.jobs.from_booking", &[]),
        odometer_in: vehicle.odometer_km,
        ..JobCard::default()
    };
    let mut tx = state.db.begin().await?;
    card.insert_in_state(&mut tx, JobCardStatus::CheckedIn, CHECK_IN)
        .await?;
    tx.commit().await?;
    booking_actions(state)
        .link_work_done(state, BookingId(event.booking.0), card.subject())
        .await?;
    // A vehicle under a fleet contract: the job is billed in its monthly
    // invoice.
    crate::fleet_billing::attach_job(state, &card).await?;
    Ok(Some(JobCard::find_or_404(&state.db, card.id).await?))
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: BookingCheckedIn, state: AppState| async move {
        on_checked_in(&state, &event).await.map(|_| ())
    });
}
