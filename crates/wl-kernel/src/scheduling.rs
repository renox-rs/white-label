//! The scheduling contract (wl-scheduling §6, §8): the events a booking
//! emits and the strategy a vertical provides, so that no module depends on
//! the `wl-scheduling` crate.
//!
//! A booking is the subject `scheduling.booking:N` ([`BOOKING_SUBJECT_TYPE`]).
//! `wl-scheduling` emits [`BookingBooked`], [`BookingRescheduled`],
//! [`BookingCheckedIn`], [`BookingCompleted`], [`BookingNoShow`],
//! [`BookingCancelled`] and [`SlotReleased`] after the transaction that
//! changed the booking has committed. A vertical opens its own record on
//! [`BookingCheckedIn`] (a visit, a job card) and consumes a package credit on
//! [`BookingCompleted`].
//!
//! [`ResourceEligibility`] answers "may this resource deliver this service at
//! this time?" (`SCH.ACL.04`). The default says yes; a vertical provides its
//! own with [`provide_resource_eligibility`] (05 adds the practitioner's
//! licence). Without one, [`resource_eligibility`] gives
//! [`NoResourceEligibility`].

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::subject::Subject;

/// The subject type of a booking.
pub const BOOKING_SUBJECT_TYPE: &str = "scheduling.booking";

/// The id of a booking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BookingId(pub i64);

impl BookingId {
    /// The booking as a [`Subject`]: `scheduling.booking:7`.
    pub fn subject(self) -> Subject {
        Subject::new(BOOKING_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for BookingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i64> for BookingId {
    fn from(id: i64) -> Self {
        BookingId(id)
    }
}

/// Who a cancellation is charged to (wl-scheduling §4.1 *Cancellation reason
/// codes*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Initiator {
    /// The customer side.
    Party,
    /// The client company.
    Business,
}

impl Initiator {
    /// The stored name: `party` or `business`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Initiator::Party => "party",
            Initiator::Business => "business",
        }
    }

    /// The initiator a stored name stands for.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "party" => Some(Initiator::Party),
            "business" => Some(Initiator::Business),
            _ => None,
        }
    }
}

/// Who ran a cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelledBy {
    /// The party, from the portal or a link.
    Party,
    /// A member of staff.
    Staff,
    /// The system (a timer).
    System,
}

/// How far the booking had got when it was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelStage {
    /// Before the start time.
    BeforeStart,
    /// The party was checked in.
    CheckedIn,
    /// The service had started.
    InProgress,
}

impl CancelStage {
    /// The stored name: `before_start`, `checked_in` or `in_progress`.
    pub fn as_str(&self) -> &'static str {
        match self {
            CancelStage::BeforeStart => "before_start",
            CancelStage::CheckedIn => "checked_in",
            CancelStage::InProgress => "in_progress",
        }
    }
}

/// What became of a booking deposit (`SCH.ACL.05`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepositOutcome {
    /// Paid back in full.
    Refunded,
    /// Kept by the business.
    Forfeited,
    /// Applied to the work done first.
    AppliedToWorkDone,
}

impl DepositOutcome {
    /// The stored name: `refunded`, `forfeited` or `applied_to_work_done`.
    pub fn as_str(&self) -> &'static str {
        match self {
            DepositOutcome::Refunded => "refunded",
            DepositOutcome::Forfeited => "forfeited",
            DepositOutcome::AppliedToWorkDone => "applied_to_work_done",
        }
    }
}

/// A booking was made (`book`, or `accept` of a request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingBooked {
    /// The booking.
    pub booking: BookingId,
    /// Who booked.
    pub party: PartyId,
    /// The patient, vehicle or student the booking is for.
    pub subject: Option<Subject>,
    /// The key of the service type.
    pub service_type: String,
    /// The resources held.
    pub resources: Vec<i64>,
    /// The start.
    pub start: DateTime,
    /// The end.
    pub end: DateTime,
    /// `staff`, `portal`, `website` or `phone`.
    pub source: String,
}

impl Event for BookingBooked {}

/// A booking moved to another time (`reschedule`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingRescheduled {
    /// The booking.
    pub booking: BookingId,
    /// The old start.
    pub old_start: DateTime,
    /// The old end.
    pub old_end: DateTime,
    /// The old resources.
    pub old_resources: Vec<i64>,
    /// The new start.
    pub start: DateTime,
    /// The new end.
    pub end: DateTime,
    /// The new resources.
    pub resources: Vec<i64>,
    /// The booking deposit that stays held and now belongs to the new time,
    /// in fils; 0 when the service type has none (`SCH.ACL.05`).
    pub deposit_carried: i64,
}

impl Event for BookingRescheduled {}

/// The party arrived (`check_in`): a vertical opens its record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingCheckedIn {
    /// The booking.
    pub booking: BookingId,
    /// The patient, vehicle or student.
    pub subject: Option<Subject>,
    /// When the party was checked in.
    pub at: DateTime,
}

impl Event for BookingCheckedIn {}

/// The service took place (`complete`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingCompleted {
    /// The booking.
    pub booking: BookingId,
    /// The patient, vehicle or student.
    pub subject: Option<Subject>,
    /// The key of the service type.
    pub service_type: String,
    /// The resources that delivered it.
    pub resources: Vec<i64>,
    /// The start.
    pub start: DateTime,
    /// The end.
    pub end: DateTime,
}

impl Event for BookingCompleted {}

/// The party did not come (`mark_no_show`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingNoShow {
    /// The booking.
    pub booking: BookingId,
    /// Who booked.
    pub party: PartyId,
    /// The patient, vehicle or student.
    pub subject: Option<Subject>,
    /// Whether the service type has a deposit to forfeit.
    pub fee_applicable: bool,
}

impl Event for BookingNoShow {}

/// A booking was cancelled or declined (`cancel`, `decline`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingCancelled {
    /// The booking.
    pub booking: BookingId,
    /// Who ran the cancellation.
    pub by: CancelledBy,
    /// Whose side it is charged to.
    pub initiated_by: Initiator,
    /// The reason code, e.g. `party_request` or `site_closed`.
    pub reason_code: String,
    /// How far the booking had got.
    pub stage: CancelStage,
    /// The vertical record that holds the work done, if any.
    pub work_done: Option<Subject>,
    /// Made less than the forfeit window (or the cut-off, without a deposit)
    /// before the start.
    pub late: bool,
    /// What became of the deposit; `None` before the deposit rules ran or
    /// when the service type has none.
    pub deposit_outcome: Option<DepositOutcome>,
}

impl Event for BookingCancelled {}

/// A slot was freed (any transition that frees one): the waiting list is
/// offered it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotReleased {
    /// The resources freed.
    pub resources: Vec<i64>,
    /// The start of the freed time.
    pub start: DateTime,
    /// The end of the freed time.
    pub end: DateTime,
    /// The key of the service type.
    pub service_type: String,
}

impl Event for SlotReleased {}

/// What a [`ResourceEligibility`] asks about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EligibilityQuery {
    /// The resource (`scheduling.resource`).
    pub resource_id: i64,
    /// The employee behind a `person` resource, if any.
    pub employee_id: Option<i64>,
    /// The key of the service type.
    pub service_type: String,
    /// The resource's location (branch, workshop, campus), if any.
    pub location: Option<Subject>,
    /// The time to judge by: the booking's start, or now at check-in.
    pub at: DateTime,
}

/// The answer of a [`ResourceEligibility`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Eligibility {
    /// The resource may deliver the service.
    Eligible,
    /// It may not; the text names why, for staff only.
    Refused(String),
}

/// The future a [`ResourceEligibility`] method returns.
pub type SchedulingFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// May this resource deliver this service at this time (wl-scheduling §8,
/// `SCH.ACL.04`)? The default says yes.
pub trait ResourceEligibility: Send + Sync + 'static {
    /// Whether the resource of `query` may deliver the service at `query.at`.
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _query: EligibilityQuery,
    ) -> SchedulingFuture<'a, Eligibility> {
        Box::pin(async { Ok(Eligibility::Eligible) })
    }
}

/// The strategy when no vertical provides one: every resource is eligible.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoResourceEligibility;

impl ResourceEligibility for NoResourceEligibility {}

/// Makes `eligibility` the app's [`ResourceEligibility`]; a vertical calls it
/// in `register`.
pub fn provide_resource_eligibility(app: &mut Registry, eligibility: impl ResourceEligibility) {
    let eligibility: Arc<dyn ResourceEligibility> = Arc::new(eligibility);
    app.provide(eligibility);
}

/// The app's [`ResourceEligibility`], or [`NoResourceEligibility`].
pub fn resource_eligibility(state: &AppState) -> Arc<dyn ResourceEligibility> {
    match state.provided::<Arc<dyn ResourceEligibility>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoResourceEligibility),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_booking_is_a_scheduling_subject() {
        assert_eq!(BookingId(7).subject().to_string(), "scheduling.booking:7");
    }

    #[test]
    fn the_stored_names_round_trip() {
        assert_eq!(Initiator::parse(Initiator::Business.as_str()), Some(Initiator::Business));
        assert_eq!(Initiator::parse("nobody"), None);
        assert_eq!(CancelStage::CheckedIn.as_str(), "checked_in");
        assert_eq!(DepositOutcome::AppliedToWorkDone.as_str(), "applied_to_work_done");
    }
}
