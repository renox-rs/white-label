//! What a vertical may do to a booking from its own record (wl-kernel
//! `BookingActions`, wl-scheduling §8 `Bookings`): link the record it opened at
//! check-in as the booking's work done, complete the booking as the system
//! (08: the job card is delivered), book a service for a party from a schedule
//! (08 `AUT.RUL.04`) and cancel a booking as the system (08: a fleet contract
//! that ended).

use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::scheduling::{BookingActions, BookingId, SchedulingFuture, ServiceBooking};

use crate::refusal::Refusal;
use crate::resource::ServiceType;
use crate::service::{self, Actor, CancelRequest, NewBooking};

/// The days after the day asked for that [`SchedulingActions::book_service`]
/// looks for a free slot in.
pub const BOOK_DAYS: i64 = 14;

/// A 422 with the English text of `refusal`.
fn unprocessable(refusal: &Refusal) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, refusal.plain())
}

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

    fn book_service<'a>(
        &'a self,
        state: &'a AppState,
        request: ServiceBooking,
    ) -> SchedulingFuture<'a, BookingId> {
        Box::pin(async move {
            let service = ServiceType::query()
                .where_(ServiceType::CODE.eq(request.service_code.as_str()))
                .where_(ServiceType::ACTIVE.eq(true))
                .first(&state.db)
                .await?
                .ok_or_else(|| {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        format!("no service `{}`", request.service_code),
                    )
                })?;
            let at = renox::db::now();
            let mut last = None;
            for day in 0..BOOK_DAYS {
                let start = request.from + TimeDelta::days(day);
                if start < at {
                    continue;
                }
                let resources = match service::pick_resources(state, &service, start, at).await {
                    Ok(resources) => resources,
                    Err(refusal) => return Err(unprocessable(&refusal)),
                };
                let mut new = NewBooking::new(request.party, service.id, &resources, start);
                new.subject = request.subject.clone();
                new.location = request.location.clone();
                new.notes = request.notes.clone();
                match service::book(state, new, Actor::system(), at).await {
                    Ok(booking) => return Ok(BookingId(booking.id)),
                    Err(refusal @ (Refusal::Taken { .. } | Refusal::Unavailable(_))) => {
                        last = Some(refusal);
                    }
                    Err(other) => return Err(unprocessable(&other)),
                }
            }
            Err(last.map_or_else(
                || {
                    Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "no slot is in the future".to_owned(),
                    )
                },
                |refusal| unprocessable(&refusal),
            ))
        })
    }

    fn cancel_booking<'a>(
        &'a self,
        state: &'a AppState,
        booking: BookingId,
        reason_code: &'a str,
    ) -> SchedulingFuture<'a, bool> {
        Box::pin(async move {
            match service::cancel(
                state,
                booking.0,
                CancelRequest::new(reason_code),
                Actor::system(),
                renox::db::now(),
            )
            .await
            {
                Ok(_) => Ok(true),
                Err(Refusal::Transition(_)) => Ok(false),
                Err(other) => Err(unprocessable(&other)),
            }
        })
    }
}
