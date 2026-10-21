//! The booking record (wl-scheduling §3 *Booking*) and the cancellation
//! reason codes (§4.1).
//!
//! The status moves only through the machine
//! ([`crate::booking_machine`]); the services that run it are in
//! [`crate::service`]. A booking is created `requested` or `booked`
//! (`request`, `book`: there is no source state, so the service inserts it
//! with a history row).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::scheduling::{BOOKING_SUBJECT_TYPE, BookingId, Initiator};
use wl_kernel::{PartyId, Series, Subject};

/// Where a booking stands (§4.1).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BookingStatus {
    /// Asked for; waiting for staff to accept.
    Requested,
    /// Slot and resources reserved.
    #[default]
    Booked,
    /// The party confirmed they will come.
    Confirmed,
    /// The party arrived.
    CheckedIn,
    /// The service started.
    InProgress,
    /// Took place.
    Completed,
    /// The party did not come.
    NoShow,
    /// Cancelled or declined.
    Cancelled,
}

impl BookingStatus {
    /// Whether the party is already in the building: after `check_in` and
    /// before the end.
    pub fn is_after_check_in(self) -> bool {
        matches!(self, BookingStatus::CheckedIn | BookingStatus::InProgress)
    }

    /// Whether the booking can still change: not `completed`, `no_show` or
    /// `cancelled`.
    pub fn is_open(self) -> bool {
        !matches!(
            self,
            BookingStatus::Completed | BookingStatus::NoShow | BookingStatus::Cancelled
        )
    }
}

/// One reserved slot.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_scheduling_bookings",
    state_machine,
    index(status, starts_at),
    index(party_id)
)]
pub struct Booking {
    /// The primary key.
    pub id: i64,
    /// `BKG-2026-0001`.
    pub number: String,
    /// Who booked.
    pub party_id: i64,
    /// The type of the subject (patient, vehicle, student), if any.
    pub subject_type: Option<String>,
    /// The id of the subject.
    pub subject_id: Option<i64>,
    /// What is booked.
    #[model(references = "wl_scheduling_service_types")]
    pub service_type_id: i64,
    /// The type of the location, if any.
    pub location_type: Option<String>,
    /// The id of the location.
    pub location_id: Option<i64>,
    /// The link of an online meeting, typed by staff.
    pub meeting_link: Option<String>,
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
    /// `staff`, `portal`, `website` or `phone`.
    pub source: String,
    /// Notes of the person who booked.
    pub notes: Option<String>,
    /// Where it stands; changed only by the machine.
    pub status: BookingStatus,
    /// A `requested` or unpaid booking holds its slot until then.
    pub hold_until: Option<DateTime>,
    /// Whether the prepayment is still awaited.
    pub payment_pending: bool,
    /// The type of the payment link of the prepayment, if any.
    pub prepayment_type: Option<String>,
    /// The id of the payment link.
    pub prepayment_id: Option<i64>,
    /// The path of the public page where the party pays it.
    pub prepayment_path: Option<String>,
    /// The user who booked, when a user did.
    pub created_by: Option<i64>,
    /// A block overlaps the booking (`BookingConflict`, cause `block`).
    pub conflict_block: bool,
    /// The resource will not be eligible at the start (cause `eligibility`).
    pub conflict_eligibility: bool,
    /// The deposit held, in fils; 0 for none.
    pub deposit_amount: i64,
    /// `refunded`, `forfeited` or `applied_to_work_done`, once settled.
    pub deposit_outcome: Option<String>,
    /// The refund asked for the deposit (`payments.refund`); the last one
    /// when a waived forfeiture was refunded later.
    pub deposit_refund_id: Option<i64>,
    /// The part of the deposit applied to the work done, in fils.
    pub deposit_applied: i64,
    /// The part of the deposit paid back, in fils.
    pub deposit_refunded: i64,
    /// The part of the deposit the business keeps, in fils.
    pub deposit_forfeited: i64,
    /// The user who waived the forfeiture (`bookings.waive_deposit`).
    pub deposit_waived_by: Option<i64>,
    /// When the forfeiture was waived.
    pub deposit_waived_at: Option<DateTime>,
    /// Why the forfeiture was waived.
    pub deposit_waive_reason: Option<String>,
    /// The cancellation reason code.
    pub cancel_code: Option<String>,
    /// `party` or `business`.
    pub cancel_initiated_by: Option<String>,
    /// `party`, `staff` or `system`: who ran the cancellation.
    pub cancel_by: Option<String>,
    /// The reason text staff gave (a decline, an override).
    pub cancel_note: Option<String>,
    /// `before_start`, `checked_in` or `in_progress`.
    pub cancel_stage: Option<String>,
    /// The cancellation came inside the forfeit window or the cut-off.
    pub cancel_late: bool,
    /// When it was cancelled.
    pub cancelled_at: Option<DateTime>,
    /// The type of the vertical record that holds the work done.
    pub work_done_type: Option<String>,
    /// The id of that record.
    pub work_done_id: Option<i64>,
    /// The work done before a cancellation after check-in, in fils, as the
    /// record captured it; 0 for none.
    pub work_done_amount: i64,
    /// When the party confirmed.
    pub confirmed_at: Option<DateTime>,
    /// When the party was checked in.
    pub checked_in_at: Option<DateTime>,
    /// When the service started.
    pub started_at: Option<DateTime>,
    /// When the service was completed.
    pub completed_at: Option<DateTime>,
    /// When the no-show was recorded.
    pub no_show_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The time one resource of a booking is occupied.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_scheduling_booking_resources",
    index(booking_id),
    index(resource_id, occupies_from)
)]
pub struct BookingResource {
    /// The primary key.
    pub id: i64,
    /// The booking.
    #[model(references = "wl_scheduling_bookings")]
    pub booking_id: i64,
    /// The resource.
    #[model(references = "wl_scheduling_resources")]
    pub resource_id: i64,
    /// The start of the occupied time, buffer before included.
    pub occupies_from: DateTime,
    /// The end of the occupied time, buffer after included.
    pub occupies_until: DateTime,
    /// The slot was released: the row no longer counts.
    pub released: bool,
}

/// The booking numbers, `BKG-2026-0001` (§7).
pub const BOOKING_SERIES: Series = Series {
    key: "booking",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "BKG",
    reset: wl_kernel::Reset::Yearly,
    gapless: false,
    per_entity: false,
};

impl Booking {
    /// The booking as a [`Subject`]: `scheduling.booking:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(BOOKING_SUBJECT_TYPE, self.id)
    }

    /// The booking as an id.
    pub fn booking_id(&self) -> BookingId {
        BookingId(self.id)
    }

    /// Who booked, as a party id.
    pub fn party(&self) -> PartyId {
        PartyId(self.party_id)
    }

    /// What the booking is for (patient, vehicle, student), if set.
    pub fn for_subject(&self) -> Option<Subject> {
        match (&self.subject_type, self.subject_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The location, if set.
    pub fn location(&self) -> Option<Subject> {
        match (&self.location_type, self.location_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The vertical record that holds the work done, if one was linked.
    pub fn work_done(&self) -> Option<Subject> {
        match (&self.work_done_type, self.work_done_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The deposit the party has paid and the business holds, in fils: 0
    /// without a deposit or while its payment is still awaited.
    pub fn deposit_held(&self) -> i64 {
        if self.payment_pending {
            0
        } else {
            self.deposit_amount
        }
    }

    /// Whether any cause of a conflict is open (§3).
    pub fn has_conflict(&self) -> bool {
        self.conflict_block || self.conflict_eligibility
    }

    /// The ids of the resources the booking holds, by id.
    pub async fn resource_ids(&self, db: &Db) -> Result<Vec<i64>> {
        let mut ids: Vec<i64> = BookingResource::query()
            .where_(BookingResource::BOOKING_ID.eq(self.id))
            .where_(BookingResource::RELEASED.eq(false))
            .get(db)
            .await?
            .into_iter()
            .map(|row| row.resource_id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// The ids of every resource the booking took, released ones included
    /// (a booking that is over has given its resources back but still lies
    /// where it was held).
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn held_resource_ids(&self, db: &Db) -> Result<Vec<i64>> {
        let mut ids: Vec<i64> = BookingResource::query()
            .where_(BookingResource::BOOKING_ID.eq(self.id))
            .get(db)
            .await?
            .into_iter()
            .map(|row| row.resource_id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }
}

/// The reason codes of the module with their default *initiated by* (§4.1).
/// `party_request`, `walked_out` and `unpaid` are party-initiated; the others
/// are business-initiated unless staff record otherwise with a note.
pub const REASON_CODES: &[(&str, Initiator)] = &[
    ("party_request", Initiator::Party),
    ("walked_out", Initiator::Party),
    ("unpaid", Initiator::Party),
    ("resource_unavailable", Initiator::Business),
    ("resource_ineligible", Initiator::Business),
    ("site_closed", Initiator::Business),
    ("equipment_failure", Initiator::Business),
    ("service_not_suitable", Initiator::Business),
    ("other", Initiator::Business),
];

/// The reason code of a party cancelling from an ordinary link or the portal.
pub const PARTY_REQUEST: &str = "party_request";

/// The reason code of a party who left after check-in.
pub const WALKED_OUT: &str = "walked_out";

/// The reason code of an unpaid hold the system cancels.
pub const UNPAID: &str = "unpaid";

/// The reason code of `decline`.
pub const OTHER: &str = "other";

/// The default *initiated by* of `code`: the module's codes, else the codes a
/// vertical added (`extra`); `None` for an unknown code.
pub fn default_initiator(code: &str, extra: &[(String, Initiator)]) -> Option<Initiator> {
    REASON_CODES
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, initiator)| *initiator)
        .or_else(|| {
            extra
                .iter()
                .find(|(known, _)| known == code)
                .map(|(_, initiator)| *initiator)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_codes_map_to_who_initiated() {
        assert_eq!(
            default_initiator("party_request", &[]),
            Some(Initiator::Party)
        );
        assert_eq!(default_initiator("walked_out", &[]), Some(Initiator::Party));
        assert_eq!(default_initiator("unpaid", &[]), Some(Initiator::Party));
        assert_eq!(
            default_initiator("site_closed", &[]),
            Some(Initiator::Business)
        );
        assert_eq!(default_initiator("other", &[]), Some(Initiator::Business));
        assert_eq!(default_initiator("nothing", &[]), None);
    }

    #[test]
    fn a_vertical_adds_a_code_but_cannot_change_a_module_code() {
        let extra = vec![
            ("patient_deceased".to_owned(), Initiator::Business),
            ("site_closed".to_owned(), Initiator::Party),
        ];
        assert_eq!(
            default_initiator("patient_deceased", &extra),
            Some(Initiator::Business)
        );
        assert_eq!(
            default_initiator("site_closed", &extra),
            Some(Initiator::Business)
        );
    }

    #[test]
    fn only_open_statuses_can_change() {
        assert!(BookingStatus::Requested.is_open());
        assert!(BookingStatus::InProgress.is_open());
        assert!(!BookingStatus::Cancelled.is_open());
        assert!(BookingStatus::CheckedIn.is_after_check_in());
        assert!(!BookingStatus::Booked.is_after_check_in());
    }
}
