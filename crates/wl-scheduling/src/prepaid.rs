//! Whether a booking still waits for the money paid for it (wl-payments §4
//! `late_confirm`, wl-scheduling §3 *No double-booking*).
//!
//! `wl-payments` asks [`PrepaidSubjects`] when a gateway confirms a payment
//! late: a booking that is `cancelled`, `no_show` or `completed` no longer
//! takes the money, so the payment is refunded instead of allocated. Any
//! other subject is not ours and counts as active.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::payments::{PaymentFuture, PrepaidSubjects};
use wl_kernel::scheduling::BOOKING_SUBJECT_TYPE;

use crate::booking::Booking;

/// The bookings of this module as prepaid subjects.
pub struct PrepaidBookings;

impl PrepaidSubjects for PrepaidBookings {
    fn is_active<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> PaymentFuture<'a, bool> {
        Box::pin(async move {
            if subject.subject_type != BOOKING_SUBJECT_TYPE {
                return Ok(true);
            }
            Ok(Booking::find(&state.db, subject.subject_id)
                .await?
                .is_some_and(|booking| booking.status.is_open()))
        })
    }
}
