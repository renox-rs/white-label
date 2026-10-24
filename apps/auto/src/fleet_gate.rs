//! A new booking for a vehicle of a suspended fleet contract is refused (08
//! §7.1 *Fleet contract* `suspend`, BL-AUT-13).
//!
//! wl-scheduling asks the app's [`BookingGate`] before it makes a **new**
//! booking (`book`, `request`); the answer here is *refused* when the booking
//! is about a vehicle (`auto.vehicle:N`) that is under a contract in the
//! state `suspended`. The bookings already made are not asked again: they are
//! kept and listed for the advisor to confirm or cancel
//! ([`crate::fleet_services::bookings_to_confirm`]), and the open job cards of
//! the contract finish. `AUT.RUL.04` requests no booking while suspended
//! either ([`crate::fleet_services::request_bookings`]).

use renox::prelude::*;
use wl_kernel::scheduling::{
    BookingGate, BookingGateQuery, CheckInAnswer, SchedulingFuture, provide_booking_gate,
};

use crate::fleet_contract::{FleetContract, FleetStatus};
use crate::seed::VEHICLE_SUBJECT_TYPE;
use crate::texts::english;
use crate::vehicle::Vehicle;

/// The suspended contract the vehicle of `subject_id` is under, if any.
///
/// # Errors
///
/// The database's errors.
pub async fn suspended_contract_of(db: &Db, vehicle_id: i64) -> Result<Option<FleetContract>> {
    let Some(vehicle) = Vehicle::find(db, vehicle_id).await? else {
        return Ok(None);
    };
    let Some(contract_id) = vehicle.fleet_contract_id else {
        return Ok(None);
    };
    Ok(FleetContract::find(db, contract_id)
        .await?
        .filter(|contract| contract.status == FleetStatus::Suspended))
}

/// The app's [`BookingGate`].
#[derive(Debug, Clone, Copy, Default)]
pub struct FleetBookingGate;

impl BookingGate for FleetBookingGate {
    fn check<'a>(
        &'a self,
        state: &'a AppState,
        query: BookingGateQuery,
    ) -> SchedulingFuture<'a, CheckInAnswer> {
        Box::pin(async move {
            let Some(subject) = query
                .subject
                .filter(|subject| subject.subject_type == VEHICLE_SUBJECT_TYPE)
            else {
                return Ok(CheckInAnswer::Allowed);
            };
            let Some(contract) = suspended_contract_of(&state.db, subject.subject_id).await? else {
                return Ok(CheckInAnswer::Allowed);
            };
            let number = contract.number();
            let text = english(
                "auto.fleet.problem.suspended_booking",
                &[("contract", &number)],
            );
            Ok(CheckInAnswer::Refused(text))
        })
    }
}

/// Makes [`FleetBookingGate`] the app's gate.
pub(crate) fn register(app: &mut Registry) {
    provide_booking_gate(app, FleetBookingGate);
}
