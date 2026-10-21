//! The visit follows its booking: the check-in opens it, the booking's
//! events move it (05 §4.2, `CLN.ENC.01`).
//!
//! The listeners are the app's side of the scheduling contract
//! (`wl_kernel::scheduling`): [`on_checked_in`] on `BookingCheckedIn`,
//! [`on_started`] on `BookingStarted`, [`on_completed`] on
//! `BookingCompleted`, [`on_cancelled`] on `BookingCancelled`. Each runs
//! after the booking's transaction committed and re-reads the visit. A
//! cancellation before the check-in has no visit and does nothing here.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::dubai_date;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::scheduling::{
    BookingCancelled, BookingCheckedIn, BookingCompleted, BookingStarted, CancelStage,
};

use crate::appointment::Appointment;
use crate::visit::{Visit, VisitStatus};
use crate::visit_machine::{CANCEL, CHECK_IN, COMPLETE, START, VisitInput};

/// `BookingCheckedIn`: opens the visit of the booking, in `checked_in`, with
/// the appointment's patient, service, day and policy. A second event for the
/// booking returns the visit that exists.
///
/// # Errors
///
/// The database's errors.
pub async fn on_checked_in(state: &AppState, event: &BookingCheckedIn) -> Result<Visit> {
    if let Some(found) = Visit::of_booking(&state.db, event.booking.0).await? {
        return Ok(found);
    }
    let appointment = Appointment::of_booking(&state.db, event.booking.0).await?;
    let from_event = event
        .subject
        .as_ref()
        .filter(|subject| subject.subject_type == PARTY_SUBJECT_TYPE)
        .map(|subject| subject.subject_id);
    let mut visit = Visit {
        booking_id: event.booking.0,
        appointment_id: appointment.as_ref().map(|a| a.id),
        patient_party_id: appointment
            .as_ref()
            .and_then(|a| a.patient_party_id)
            .or(from_event),
        policy_id: appointment.as_ref().and_then(|a| a.policy_id),
        service_code: appointment
            .as_ref()
            .map(|a| a.service_code.clone())
            .unwrap_or_default(),
        visit_date: appointment
            .as_ref()
            .map_or_else(|| dubai_date(event.at), |a| a.visit_date),
        checked_in_at: event.at,
        ..Visit::default()
    };
    let mut tx = state.db.begin().await?;
    visit
        .insert_in_state(&mut tx, VisitStatus::CheckedIn, CHECK_IN)
        .await?;
    tx.commit().await?;
    // What the receptionist recorded before the check-in moves to the visit.
    crate::coverage::move_to_visit(state, &visit).await?;
    // The flags of the appointment no longer hold once the patient is in.
    crate::prior_auth_rules::on_checked_in(state, visit.booking_id).await?;
    Visit::find_or_404(&state.db, visit.id).await
}

/// Runs the system transition `name` on the visit of `booking`, if it has one
/// that can take it. A visit that is already past it is left as it is.
async fn follow(
    state: &AppState,
    booking: i64,
    name: &str,
    input: &VisitInput,
) -> Result<Option<Visit>> {
    let Some(visit) = Visit::of_booking(&state.db, booking).await? else {
        return Ok(None);
    };
    let outcome = visit
        .transition(name)
        .by_system()
        .input(input)
        .run(state)
        .await;
    match outcome {
        Ok(_) => Ok(Some(Visit::find_or_404(&state.db, visit.id).await?)),
        Err(TransitionError::NotFrom(_)) => Ok(Some(visit)),
        Err(other) => Err(other.into_error()),
    }
}

/// A visit that is over gives back the units it held against prior
/// authorisations (BL-CLN-02).
async fn release(state: &AppState, visit: Option<&Visit>) -> Result {
    if let Some(visit) = visit
        && !visit.status.is_open()
    {
        crate::auth_check::release_holds(state, visit.id).await?;
    }
    Ok(())
}

/// `BookingStarted`: the visit is `in_progress`.
///
/// # Errors
///
/// The database's and the machine's errors.
pub async fn on_started(state: &AppState, event: &BookingStarted) -> Result<Option<Visit>> {
    let input = VisitInput {
        at: Some(event.at),
        reason: None,
    };
    follow(state, event.booking.0, START, &input).await
}

/// `BookingCompleted`: the visit is `completed`.
///
/// # Errors
///
/// The database's and the machine's errors.
pub async fn on_completed(state: &AppState, event: &BookingCompleted) -> Result<Option<Visit>> {
    let input = VisitInput {
        at: Some(renox::db::now()),
        reason: None,
    };
    let visit = follow(state, event.booking.0, COMPLETE, &input).await?;
    release(state, visit.as_ref()).await?;
    Ok(visit)
}

/// `BookingCancelled` after the check-in: the visit is `cancelled` with the
/// booking's reason code. A cancellation before the check-in has no visit.
///
/// # Errors
///
/// The database's and the machine's errors.
pub async fn on_cancelled(state: &AppState, event: &BookingCancelled) -> Result<Option<Visit>> {
    if event.stage == CancelStage::BeforeStart {
        return Ok(None);
    }
    let input = VisitInput {
        at: Some(renox::db::now()),
        reason: Some(event.reason_code.clone()),
    };
    let visit = follow(state, event.booking.0, CANCEL, &input).await?;
    release(state, visit.as_ref()).await?;
    Ok(visit)
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: BookingCheckedIn, state: AppState| async move {
        on_checked_in(&state, &event).await.map(|_| ())
    });
    app.listen(|event: BookingStarted, state: AppState| async move {
        on_started(&state, &event).await.map(|_| ())
    });
    app.listen(|event: BookingCompleted, state: AppState| async move {
        on_completed(&state, &event).await.map(|_| ())
    });
    app.listen(|event: BookingCancelled, state: AppState| async move {
        on_cancelled(&state, &event).await.map(|_| ())
    });
}
