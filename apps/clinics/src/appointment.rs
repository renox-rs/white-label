//! The clinic's side of a booking (05 §6 *Appointment*, §7.1 *Appointment*).
//!
//! wl-scheduling owns the booking and its machine; the app never reads its
//! tables. An [`Appointment`] is what the clinic keeps beside it, made from
//! the kernel's `BookingBooked` ([`crate::appointments::on_booked`]): who the
//! patient is, the service code, the day of the visit in Asia/Dubai, and the
//! insurance policy the appointment was booked with (an *insured* visit,
//! `CLN.ACL.15`). Decision: the booking's number and status stay in
//! wl-scheduling; the app asks the booking by `booking_id` through the
//! kernel's events and guard.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::scheduling::BookingId;

/// An appointment of the clinic.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_appointments",
    unique(booking_id),
    index(patient_party_id),
    index(visit_date)
)]
pub struct Appointment {
    /// The primary key.
    pub id: i64,
    /// The wl-scheduling booking (`scheduling.booking`).
    pub booking_id: i64,
    /// The patient, a party; `None` when the booking names no party subject.
    pub patient_party_id: Option<i64>,
    /// The party that booked (the patient, or a guardian).
    pub booker_party_id: i64,
    /// The key of the booking's service type.
    pub service_code: String,
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
    /// The day of the visit, the Dubai date of the start: a policy valid to
    /// that day is valid all day.
    pub visit_date: NaiveDate,
    /// The insurance policy the appointment was booked with; `None` for a
    /// self-pay appointment (no eligibility or outcome is asked).
    pub policy_id: Option<i64>,
    /// The day the booking was cancelled or declined (Asia/Dubai).
    pub cancelled_on: Option<NaiveDate>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Appointment {
    /// The appointment of booking `booking`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_booking(db: &Db, booking: i64) -> Result<Option<Appointment>> {
        Appointment::query()
            .where_(Appointment::BOOKING_ID.eq(booking))
            .first(db)
            .await
    }

    /// The booking as a [`BookingId`].
    pub fn booking(&self) -> BookingId {
        BookingId(self.booking_id)
    }

    /// The booking as a [`Subject`]: `scheduling.booking:7`.
    pub fn subject(&self) -> Subject {
        self.booking().subject()
    }

    /// Whether the appointment was booked with an insurance policy.
    pub fn is_insured(&self) -> bool {
        self.policy_id.is_some()
    }

    /// Whether the booking was cancelled or declined.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled_on.is_some()
    }
}
