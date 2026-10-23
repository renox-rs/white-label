//! The scheduling contract (wl-scheduling §6, §8): the events a booking
//! emits and the strategy a vertical provides, so that no module depends on
//! the `wl-scheduling` crate.
//!
//! A booking is the subject `scheduling.booking:N` ([`BOOKING_SUBJECT_TYPE`]).
//! `wl-scheduling` emits [`BookingBooked`], [`BookingRescheduled`],
//! [`BookingCheckedIn`], [`BookingStarted`], [`BookingCompleted`], [`BookingNoShow`],
//! [`BookingReassigned`], [`BookingCancelled`] and [`SlotReleased`] after the transaction that
//! changed the booking has committed. A vertical opens its own record on
//! [`BookingCheckedIn`] (a visit, a job card) and consumes a package credit on
//! [`BookingCompleted`].
//!
//! [`WorkDoneStatus`] is the vertical's answer to "what was done for this
//! booking?" (`SCH.ACL.06`): a booking that is checked in is cancelled only
//! once the visit or job card has its work done captured or marked none.
//!
//! [`CheckInGuard`] lets a vertical refuse a check-in (05 asks for the
//! coverage outcome of an insured visit); [`BookingStarted`] tells it the
//! service began.
//!
//! [`BookingConflict`] is emitted when a new block or a lapsing licence puts
//! bookings in conflict; [`AvailabilitySource`] supplies other modules' blocks.
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

/// Staff moved a booking to another resource at the same time (`reassign`).
/// The reason code is for staff: the party is not told it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingReassigned {
    /// The booking.
    pub booking: BookingId,
    /// The resources before.
    pub old_resources: Vec<i64>,
    /// The resources now.
    pub new_resources: Vec<i64>,
    /// The cancellation reason code that says why, e.g.
    /// `resource_unavailable`.
    pub reason_code: String,
}

impl Event for BookingReassigned {}

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

/// The service started (`start`): the party is with the resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingStarted {
    /// The booking.
    pub booking: BookingId,
    /// The patient, vehicle or student.
    pub subject: Option<Subject>,
    /// When the service started.
    pub at: DateTime,
}

impl Event for BookingStarted {}

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
    /// The work done before the cancellation, in fils, as the vertical
    /// captured it; 0 for none, or for a cancellation before the check-in.
    /// The vertical bills it.
    pub work_done_amount: i64,
    /// The part of the deposit applied to the work done, in fils; the
    /// vertical bills the work less this. 0 unless a party-initiated
    /// cancellation came after the check-in.
    pub deposit_applied: i64,
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

/// What was done for a booking before it was cancelled after the check-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkDone {
    /// The record opened at check-in has not said yet: the booking cannot
    /// be cancelled.
    Pending,
    /// Nothing was done.
    Nothing,
    /// Work was done and captured on the vertical's record, worth `amount`
    /// fils at its price (normal or partial).
    Captured {
        /// The charge for the work done, in fils.
        amount: i64,
    },
}

/// What was done for a booking (wl-scheduling §8, `SCH.ACL.06`)? The vertical
/// answers from the record it opened at check-in. The default says nothing
/// was done.
pub trait WorkDoneStatus: Send + Sync + 'static {
    /// The work done for `booking`; `record` is the vertical record linked to
    /// the booking, if any.
    fn status<'a>(
        &'a self,
        _state: &'a AppState,
        _booking: BookingId,
        _record: Option<Subject>,
    ) -> SchedulingFuture<'a, WorkDone> {
        Box::pin(async { Ok(WorkDone::Nothing) })
    }
}

/// The strategy when no vertical provides one: nothing was done.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoWorkDoneStatus;

impl WorkDoneStatus for NoWorkDoneStatus {}

/// Makes `status` the app's [`WorkDoneStatus`]; a vertical calls it in
/// `register`.
pub fn provide_work_done_status(app: &mut Registry, status: impl WorkDoneStatus) {
    let status: Arc<dyn WorkDoneStatus> = Arc::new(status);
    app.provide(status);
}

/// The app's [`WorkDoneStatus`], or [`NoWorkDoneStatus`].
pub fn work_done_status(state: &AppState) -> Arc<dyn WorkDoneStatus> {
    match state.provided::<Arc<dyn WorkDoneStatus>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoWorkDoneStatus),
    }
}

/// The names of the services a module books, by code, in a language: what a
/// vertical shows where it has only a booking's service code
/// (`dental_checkup` is *Dental check-up*). The default knows none, so a code
/// is shown as it is.
pub trait ServiceNames: Send + Sync + 'static {
    /// The name of every service by its code, in `locale` (`en`, `ar`).
    fn names<'a>(
        &'a self,
        _state: &'a AppState,
        _locale: &'a str,
    ) -> SchedulingFuture<'a, std::collections::BTreeMap<String, String>> {
        Box::pin(async { Ok(std::collections::BTreeMap::new()) })
    }
}

/// The strategy when no module provides one: no names.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoServiceNames;

impl ServiceNames for NoServiceNames {}

/// Makes `names` the app's [`ServiceNames`]; the scheduling module calls it
/// in `register`.
pub fn provide_service_names(app: &mut Registry, names: impl ServiceNames) {
    let names: Arc<dyn ServiceNames> = Arc::new(names);
    app.provide(names);
}

/// The app's [`ServiceNames`], or [`NoServiceNames`].
pub fn service_names(state: &AppState) -> Arc<dyn ServiceNames> {
    match state.provided::<Arc<dyn ServiceNames>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoServiceNames),
    }
}

/// The names of the resources a module books (a tutor, a room), by id: what a
/// vertical shows where it holds only a resource's id. The default knows none,
/// so the page falls back to the number.
pub trait ResourceNames: Send + Sync + 'static {
    /// The name of each of `ids` that exists.
    fn names<'a>(
        &'a self,
        _state: &'a AppState,
        _ids: &'a [i64],
    ) -> SchedulingFuture<'a, std::collections::BTreeMap<i64, String>> {
        Box::pin(async { Ok(std::collections::BTreeMap::new()) })
    }

    /// The active resources of `kind`, by name: what a form offers where a
    /// person picks a tutor or a room, instead of typing a resource number.
    /// The default knows none.
    fn choices<'a>(
        &'a self,
        _state: &'a AppState,
        _kind: ResourceChoiceKind,
    ) -> SchedulingFuture<'a, Vec<ResourceChoice>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// Which resources a form offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceChoiceKind {
    /// People: tutors, practitioners.
    Person,
    /// Rooms.
    Room,
}

/// A resource a form offers ([`ResourceNames::choices`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceChoice {
    /// The resource's id.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// The employee behind a person, if any.
    pub employee_id: Option<i64>,
    /// How many it holds: bookings that may overlap, or the persons a room
    /// holds.
    pub capacity: i64,
}

/// The strategy when no module provides one: no names.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoResourceNames;

impl ResourceNames for NoResourceNames {}

/// Makes `names` the app's [`ResourceNames`]; the scheduling module calls it
/// in `register`.
pub fn provide_resource_names(app: &mut Registry, names: impl ResourceNames) {
    let names: Arc<dyn ResourceNames> = Arc::new(names);
    app.provide(names);
}

/// The app's [`ResourceNames`], or [`NoResourceNames`].
pub fn resource_names(state: &AppState) -> Arc<dyn ResourceNames> {
    match state.provided::<Arc<dyn ResourceNames>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoResourceNames),
    }
}

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

/// The answer of a [`CheckInGuard`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckInAnswer {
    /// The party may be checked in.
    Allowed,
    /// The party may not; the text is safe to show to the staff who check in.
    Refused(String),
}

/// May this party be checked in (wl-scheduling §4.1 `check_in`)? The
/// vertical answers from what it holds about the booking: 05 wants the
/// coverage outcome of an insured visit (`CLN.ACL.15`). The default allows
/// every check-in.
pub trait CheckInGuard: Send + Sync + 'static {
    /// Whether `booking` may be checked in at `at`.
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _booking: BookingId,
        _at: DateTime,
    ) -> SchedulingFuture<'a, CheckInAnswer> {
        Box::pin(async { Ok(CheckInAnswer::Allowed) })
    }
}

/// The guard when no vertical provides one: every check-in is allowed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCheckInGuard;

impl CheckInGuard for NoCheckInGuard {}

/// May this booking be completed (wl-scheduling §4.1 `complete`)? The
/// vertical answers from its record of the visit: 05 refuses the check-out of
/// a visit with a lot-tracked consumable that has no lot (`STK.LOT.01`). The
/// answer is a [`CheckInAnswer`]: allowed, or refused with a text safe to show
/// to the staff. The default allows every completion.
pub trait CompleteGuard: Send + Sync + 'static {
    /// Whether `booking` may be completed at `at`.
    fn check<'a>(
        &'a self,
        _state: &'a AppState,
        _booking: BookingId,
        _at: DateTime,
    ) -> SchedulingFuture<'a, CheckInAnswer> {
        Box::pin(async { Ok(CheckInAnswer::Allowed) })
    }
}

/// The guard when no vertical provides one: every completion is allowed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCompleteGuard;

impl CompleteGuard for NoCompleteGuard {}

/// Makes `guard` the app's [`CompleteGuard`]; a vertical calls it in
/// `register`.
pub fn provide_complete_guard(app: &mut Registry, guard: impl CompleteGuard) {
    let guard: Arc<dyn CompleteGuard> = Arc::new(guard);
    app.provide(guard);
}

/// The app's [`CompleteGuard`], or [`NoCompleteGuard`].
pub fn complete_guard(state: &AppState) -> Arc<dyn CompleteGuard> {
    match state.provided::<Arc<dyn CompleteGuard>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoCompleteGuard),
    }
}

/// Makes `guard` the app's [`CheckInGuard`]; a vertical calls it in
/// `register`.
pub fn provide_check_in_guard(app: &mut Registry, guard: impl CheckInGuard) {
    let guard: Arc<dyn CheckInGuard> = Arc::new(guard);
    app.provide(guard);
}

/// The app's [`CheckInGuard`], or [`NoCheckInGuard`].
pub fn check_in_guard(state: &AppState) -> Arc<dyn CheckInGuard> {
    match state.provided::<Arc<dyn CheckInGuard>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoCheckInGuard),
    }
}

/// Why bookings conflict (wl-scheduling §3 *Booking: conflict*).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictCause {
    /// A new availability block (leave, sick, roster, holiday) overlaps them.
    Block,
    /// The resource will not be eligible at their start (`SCH.RUL.05`).
    Eligibility,
}

impl ConflictCause {
    /// The stored name: `block` or `eligibility`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ConflictCause::Block => "block",
            ConflictCause::Eligibility => "eligibility",
        }
    }
}

/// Bookings were flagged as conflicted: a new block overlaps them, or the
/// resource will not be eligible at their start. Nothing is cancelled; staff
/// move the bookings (wl-scheduling §5.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookingConflict {
    /// The resource that is not available.
    pub resource: i64,
    /// Why.
    pub cause: ConflictCause,
    /// The availability block, for the cause `block`.
    pub block: Option<i64>,
    /// The expiring qualification or document, for the cause `eligibility`.
    pub item: Option<String>,
    /// The bookings flagged.
    pub bookings: Vec<BookingId>,
    /// The earliest start among them.
    pub earliest_start: DateTime,
}

impl Event for BookingConflict {}

/// A new availability block was added to a resource (a manual *sick* or
/// *away* block, a leave, a roster, a holiday), whether or not it overlaps a
/// booking. A vertical that has its own work on the resource, such as the
/// class sessions of a tutor (10 `EDU.RUL.12`), reads it to find that work.
/// [`BookingConflict`] says only which bookings the block overlaps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailabilityBlockAdded {
    /// The resource that is not available.
    pub resource: i64,
    /// The employee behind the resource, if it is a person.
    pub employee: Option<i64>,
    /// The block.
    pub block: i64,
    /// `manual`, `leave`, `roster` or `holiday`.
    pub source: String,
    /// Why, e.g. `sick`.
    pub reason: String,
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
}

impl Event for AvailabilityBlockAdded {}

/// A stretch of time a source says a resource is not bookable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceBlock {
    /// The start.
    pub starts_at: DateTime,
    /// The end.
    pub ends_at: DateTime,
    /// Why, e.g. `sick`, `annual`, `roster`.
    pub reason: String,
    /// `leave`, `roster`, `session` or `holiday`.
    pub source: String,
    /// The record that caused it (a leave request, a roster), as the source
    /// names it; two blocks of one source and reference are one block.
    pub reference: String,
}

/// Another module's blocks of a resource (wl-scheduling §8): leave from
/// `wl-hr`, rosters from `wl-attendance`, sessions from an app. The default
/// supplies none.
pub trait AvailabilitySource: Send + Sync + 'static {
    /// The blocks of the resource (and the employee behind it, if any) that
    /// touch `from`..`until`.
    fn blocks<'a>(
        &'a self,
        _state: &'a AppState,
        _resource_id: i64,
        _employee_id: Option<i64>,
        _from: DateTime,
        _until: DateTime,
    ) -> SchedulingFuture<'a, Vec<SourceBlock>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The source when no module provides one: no blocks.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAvailabilitySource;

impl AvailabilitySource for NoAvailabilitySource {}

/// Makes `source` the app's [`AvailabilitySource`].
pub fn provide_availability_source(app: &mut Registry, source: impl AvailabilitySource) {
    let source: Arc<dyn AvailabilitySource> = Arc::new(source);
    app.provide(source);
}

/// The app's [`AvailabilitySource`], or [`NoAvailabilitySource`].
pub fn availability_source(state: &AppState) -> Arc<dyn AvailabilitySource> {
    match state.provided::<Arc<dyn AvailabilitySource>>() {
        Some(found) => Arc::clone(&found),
        None => Arc::new(NoAvailabilitySource),
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
        assert_eq!(
            Initiator::parse(Initiator::Business.as_str()),
            Some(Initiator::Business)
        );
        assert_eq!(Initiator::parse("nobody"), None);
        assert_eq!(CancelStage::CheckedIn.as_str(), "checked_in");
        assert_eq!(ConflictCause::Block.as_str(), "block");
        assert_eq!(ConflictCause::Eligibility.as_str(), "eligibility");
        assert_eq!(
            DepositOutcome::AppliedToWorkDone.as_str(),
            "applied_to_work_done"
        );
    }
}
