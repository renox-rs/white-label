//! The visit: what happened at an appointment (05 §6 *Visit*, `CLN.ENC.01`).
//!
//! A visit is opened by the check-in of its booking ([`crate::visits::open`],
//! from `BookingCheckedIn`) and follows the booking: the service starting
//! ([`crate::visit_machine::START`]), taking place
//! ([`crate::visit_machine::COMPLETE`]) or being cancelled after the
//! check-in ([`crate::visit_machine::CANCEL`]). The status moves only through
//! the machine in [`crate::visit_machine`].
//!
//! Decision: the visit is built to be extended. Charge capture, the patient
//! share, the claims and the lots of the consumables hang on `visit_id`
//! later. The coverage columns (the eligibility result, the outcome, the
//! policy of the split, the self-pay consent) are moved to it by the check-in
//! from what the receptionist recorded ([`crate::coverage`]).

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::scheduling::BookingId;

use crate::coverage::CoverageOutcome;

/// The subject type of a visit.
pub const VISIT_SUBJECT_TYPE: &str = "clinics.visit";

/// Where a visit stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VisitStatus {
    /// The patient is checked in and waits.
    #[default]
    CheckedIn,
    /// The patient is with the practitioner.
    InProgress,
    /// Seen (terminal).
    Completed,
    /// Stopped after the check-in: a walk-out or the clinic's stop (terminal).
    Cancelled,
}

impl VisitStatus {
    /// Whether the visit can still change: `checked_in` or `in_progress`.
    pub fn is_open(self) -> bool {
        matches!(self, VisitStatus::CheckedIn | VisitStatus::InProgress)
    }
}

/// A visit.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_visits",
    state_machine,
    unique(booking_id),
    index(patient_party_id),
    index(status)
)]
pub struct Visit {
    /// The primary key.
    pub id: i64,
    /// The wl-scheduling booking (`scheduling.booking`).
    pub booking_id: i64,
    /// The clinic's record of the booking, when it has one.
    pub appointment_id: Option<i64>,
    /// The patient, a party.
    pub patient_party_id: Option<i64>,
    /// The insurance policy of an insured visit.
    pub policy_id: Option<i64>,
    /// The key of the booked service type.
    pub service_code: String,
    /// The day of the visit (Asia/Dubai).
    pub visit_date: NaiveDate,
    /// When the patient was checked in.
    pub checked_in_at: DateTime,
    /// When the service started.
    pub started_at: Option<DateTime>,
    /// When the visit was completed or cancelled.
    pub ended_at: Option<DateTime>,
    /// The cancellation reason code of the booking.
    pub cancel_reason: Option<String>,
    /// The eligibility result the receptionist recorded for the policy
    /// (`CLN.INS.02`).
    pub eligibility_check_id: Option<i64>,
    /// Whether that result is positive.
    pub eligibility_positive: Option<bool>,
    /// What the cover of an insured visit turned out to be (`CLN.ACL.15`);
    /// `None` for a visit that is not insured.
    pub coverage_outcome: Option<CoverageOutcome>,
    /// The policy the visit's charge split reads: the appointment's own for
    /// *insured*, the other one for *re-split*, none for *self-pay*.
    pub coverage_policy_id: Option<i64>,
    /// The signed self-pay consent of a *self-pay* visit.
    pub consent_id: Option<i64>,
    /// The lifecycle state; changed only by a transition.
    pub status: VisitStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Visit {
    /// The visit of booking `booking`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_booking(db: &Db, booking: i64) -> Result<Option<Visit>> {
        Visit::query()
            .where_(Visit::BOOKING_ID.eq(booking))
            .first(db)
            .await
    }

    /// The visit as a [`Subject`]: `clinics.visit:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(VISIT_SUBJECT_TYPE, self.id)
    }

    /// The booking as a [`BookingId`].
    pub fn booking(&self) -> BookingId {
        BookingId(self.booking_id)
    }

    /// The number shown to staff: `VIS-0007`.
    pub fn number(&self) -> String {
        format!("VIS-{:04}", self.id)
    }

    /// Whether the visit was booked with an insurance policy.
    pub fn is_insured(&self) -> bool {
        self.policy_id.is_some()
    }
}
