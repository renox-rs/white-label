//! The events a booking emits (wl-scheduling §6), built from the committed
//! booking and sent after the commit. The event types are the kernel's
//! (`wl_kernel::scheduling`), so that a vertical listens without depending on
//! this crate.

use renox::prelude::*;
use wl_kernel::scheduling::{
    BookingBooked, BookingCancelled, BookingCheckedIn, BookingCompleted, BookingNoShow,
    BookingReassigned, BookingRescheduled, BookingStarted, CancelStage, CancelledBy,
    DepositOutcome, Initiator, SlotReleased,
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
            deposit_carried: booking.deposit_held(),
        })
        .await
}

/// `reassign` moved the booking to other resources at the same time.
pub async fn reassigned(
    state: &AppState,
    booking: &Booking,
    old_resources: Vec<i64>,
    new_resources: Vec<i64>,
    reason_code: &str,
) -> Result {
    state
        .emit(BookingReassigned {
            booking: booking.booking_id(),
            old_resources,
            new_resources,
            reason_code: reason_code.to_owned(),
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

/// `start` began the service.
pub async fn started(state: &AppState, booking: &Booking) -> Result {
    state
        .emit(BookingStarted {
            booking: booking.booking_id(),
            subject: booking.for_subject(),
            at: booking.started_at.unwrap_or_else(renox::db::now),
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

/// `cancel` or `decline` ended the booking. The reason, who initiated it, the
/// stage and whether it was late are read from the committed booking; the
/// deposit outcome is what the deposit rules recorded on it.
pub async fn cancelled(state: &AppState, booking: &Booking, by: CancelledBy) -> Result {
    let initiated_by = booking
        .cancel_initiated_by
        .as_deref()
        .and_then(Initiator::parse)
        .unwrap_or(Initiator::Business);
    let stage = match booking.cancel_stage.as_deref() {
        Some("checked_in") => CancelStage::CheckedIn,
        Some("in_progress") => CancelStage::InProgress,
        _ => CancelStage::BeforeStart,
    };
    let deposit_outcome = match booking.deposit_outcome.as_deref() {
        Some("refunded") => Some(DepositOutcome::Refunded),
        Some("forfeited") => Some(DepositOutcome::Forfeited),
        Some("applied_to_work_done") => Some(DepositOutcome::AppliedToWorkDone),
        _ => None,
    };
    state
        .emit(BookingCancelled {
            booking: booking.booking_id(),
            by,
            initiated_by,
            reason_code: booking.cancel_code.clone().unwrap_or_default(),
            stage,
            work_done: booking.work_done(),
            late: booking.cancel_late,
            deposit_outcome,
            work_done_amount: booking.work_done_amount,
            deposit_applied: booking.deposit_applied,
        })
        .await
}
