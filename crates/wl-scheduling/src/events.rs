//! The events a booking emits (wl-scheduling §6), built from the committed
//! booking and sent after the commit. The event types are the kernel's
//! (`wl_kernel::scheduling`), so that a vertical listens without depending on
//! this crate.

use renox::prelude::*;
use wl_kernel::scheduling::{
    BookingBooked, BookingCheckedIn, BookingCompleted, BookingNoShow, BookingRescheduled,
    SlotReleased,
};

use crate::booking::Booking;
use crate::resource::ServiceType;

/// `book` or `accept` made the booking `booked`.
pub async fn booked(
    state: &AppState,
    booking: &Booking,
    service: &ServiceType,
    resources: Vec<i64>,
) -> Result {
    state
        .emit(BookingBooked {
            booking: booking.booking_id(),
            party: booking.party(),
            subject: booking.for_subject(),
            service_type: service.code.clone(),
            resources,
            start: booking.starts_at,
            end: booking.ends_at,
            source: booking.source.clone(),
        })
        .await
}

/// `reschedule` moved the booking.
pub async fn rescheduled(
    state: &AppState,
    booking: &Booking,
    old: &Booking,
    old_resources: Vec<i64>,
    resources: Vec<i64>,
) -> Result {
    state
        .emit(BookingRescheduled {
            booking: booking.booking_id(),
            old_start: old.starts_at,
            old_end: old.ends_at,
            old_resources,
            start: booking.starts_at,
            end: booking.ends_at,
            resources,
        })
        .await
}

/// `check_in` arrived.
pub async fn checked_in(state: &AppState, booking: &Booking) -> Result {
    state
        .emit(BookingCheckedIn {
            booking: booking.booking_id(),
            subject: booking.for_subject(),
            at: booking.checked_in_at.unwrap_or_else(renox::db::now),
        })
        .await
}

/// `complete` finished the service.
pub async fn completed(
    state: &AppState,
    booking: &Booking,
    service: &ServiceType,
    resources: Vec<i64>,
) -> Result {
    state
        .emit(BookingCompleted {
            booking: booking.booking_id(),
            subject: booking.for_subject(),
            service_type: service.code.clone(),
            resources,
            start: booking.starts_at,
            end: booking.ends_at,
        })
        .await
}

/// `mark_no_show` recorded an absence.
pub async fn no_show(state: &AppState, booking: &Booking, service: &ServiceType) -> Result {
    state
        .emit(BookingNoShow {
            booking: booking.booking_id(),
            party: booking.party(),
            subject: booking.for_subject(),
            fee_applicable: service.has_deposit(),
        })
        .await
}

/// A transition freed the slot of `resources` from `start` to `end`.
pub async fn slot_released(
    state: &AppState,
    service: &ServiceType,
    resources: Vec<i64>,
    start: renox::db::DateTime,
    end: renox::db::DateTime,
) -> Result {
    if resources.is_empty() || end <= start {
        return Ok(());
    }
    state
        .emit(SlotReleased {
            resources,
            start,
            end,
            service_type: service.code.clone(),
        })
        .await
}
