//! What a vertical may do to a booking from its own record (wl-kernel
//! `BookingActions`, wl-scheduling §8 `Bookings`): link the record it opened at
//! check-in as the booking's work done, and complete the booking as the system
//! (08: the job card is delivered).

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::scheduling::{BookingActions, BookingId, SchedulingFuture};

use crate::service::{self, Actor};

/// The module's [`BookingActions`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SchedulingActions;

impl BookingActions for SchedulingActions {
    fn link_work_done<'a>(
        &'a self,
        state: &'a AppState,
        booking: BookingId,
        record: Subject,
    ) -> SchedulingFuture<'a, ()> {
        Box::pin(async move { service::link_work_done(&state.db, booking.0, &record).await })
    }

    fn complete<'a>(&'a self, state: &'a AppState, booking: BookingId) -> SchedulingFuture<'a, ()> {
        Box::pin(async move {
            service::complete(state, booking.0, Actor::system(), renox::db::now())
                .await
                .map(|_| ())
                .map_err(|refusal| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, refusal.plain()))
        })
    }
}
