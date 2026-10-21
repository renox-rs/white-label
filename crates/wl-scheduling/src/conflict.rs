//! Booking conflicts (wl-scheduling §3 *Booking: conflict*, §5.4 step 3, §5.5;
//! BL-SCH-05).
//!
//! A new availability block (sick leave, a manual *sick* block, leave, a
//! roster, a holiday) that overlaps bookings does not cancel them: each gets
//! the conflict flag with the cause `block`, a [`Conflict`] row says which
//! block caused it, and one [`BookingConflict`](wl_kernel::scheduling::BookingConflict)
//! lists them. Staff then act in bulk ([`bulk_resolve`]): reassign, cancel as
//! the business, or send the parties an offer of other times. The flag is
//! cleared by `reassign`, `reschedule` and `cancel` (the machine), and when the
//! block is removed ([`remove_block`]) for the conflicts that block caused.
//!
//! Decisions:
//!
//! - **Rest of the day.** `LeaveApproved` blocks the resource from the moment
//!   the event is handled (or the first day of the leave, if later) to the end
//!   of the last day, Asia/Dubai. A manual block of reason `sick` without an
//!   end runs to the end of the day ([`rest_of_day`]).
//! - **Bookings that can conflict** are `requested`, `booked` and `confirmed`:
//!   a party who is already checked in is not moved.
//! - **A `Conflict` row per booking, cause and block (or item).** It makes the
//!   flagging idempotent, lets a removed block clear only its own conflicts,
//!   and keeps the cause the machine's flag columns cannot name.
//! - **Bulk partial failure.** Every booking is its own transaction; a failure
//!   (the new resource was taken meanwhile) is listed and the booking stays
//!   conflicted.
//! - `Bookings::conflicts` and `bulk_resolve` are functions of this module;
//!   the kernel has no `Bookings` trait yet because no module calls them
//!   across a boundary.

use std::collections::BTreeMap;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::dubai_date;
use wl_kernel::scheduling::{BookingConflict, ConflictCause, availability_source};
use wl_kernel::workforce::{LeaveApproved, LeaveCancelled};

use crate::booking::{Booking, BookingResource, BookingStatus, default_initiator};
use crate::config::config_of;
use crate::notices::{self, ChangeEntry};
use crate::refusal::Refusal;
use crate::resource::{AvailabilityBlock, Resource, ServiceType, eligible_resources};
use crate::rules;
use crate::service::{self, Actor, CancelRequest};
use crate::slots;

/// The reason code of a change caused by a block: the resource is not
/// available. It is what the party's *cancel* link records.
pub const BLOCK_REASON_CODE: &str = "resource_unavailable";

/// Why one booking is in conflict.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_scheduling_conflicts", index(booking_id), index(block_id))]
pub struct Conflict {
    /// The primary key.
    pub id: i64,
    /// The booking.
    #[model(references = "wl_scheduling_bookings")]
    pub booking_id: i64,
    /// The resource that is not available.
    #[model(references = "wl_scheduling_resources")]
    pub resource_id: i64,
    /// `block` or `eligibility`.
    pub cause: String,
    /// The availability block that caused it (cause `block`).
    pub block_id: Option<i64>,
    /// The expiring qualification or document (cause `eligibility`).
    pub item: Option<String>,
    /// When it was raised.
    pub raised_at: DateTime,
    /// When it ended.
    pub cleared_at: Option<DateTime>,
    /// How it ended: `block_removed`, `reassign`, `reschedule` or `cancel`.
    pub cleared_by: Option<String>,
    /// When the party was last sent the offer of other times.
    pub offered_at: Option<DateTime>,
    /// When the conflict was escalated (`SCH.RUL.06`); once per conflict.
    pub escalated_at: Option<DateTime>,
}

impl Conflict {
    /// Whether it is still open.
    pub fn is_open(&self) -> bool {
        self.cleared_at.is_none()
    }
}

/// Whether a booking in `status` can be put in conflict: before the check-in.
pub fn can_conflict(status: BookingStatus) -> bool {
    matches!(
        status,
        BookingStatus::Requested | BookingStatus::Booked | BookingStatus::Confirmed
    )
}

/// The instant the Asia/Dubai day after `at`'s begins: the end of the rest of
/// the day.
pub fn rest_of_day(at: DateTime) -> DateTime {
    day_start(dubai_date(at) + TimeDelta::days(1))
}

fn day_start(day: NaiveDate) -> DateTime {
    BusinessCalendar::default().from_local(day.and_hms_opt(0, 0, 0).expect("midnight"))
}

/// The start and the end of the Asia/Dubai day `day`.
pub fn day_bounds(day: NaiveDate) -> (DateTime, DateTime) {
    (day_start(day), day_start(day + TimeDelta::days(1)))
}

/// The conflicts of `booking_id` that are still open.
pub async fn open_of(db: &Db, booking_id: i64) -> Result<Vec<Conflict>> {
    Conflict::query()
        .where_(Conflict::BOOKING_ID.eq(booking_id))
        .where_(Conflict::CLEARED_AT.is_null())
        .order_by(Conflict::ID)
        .get(db)
        .await
}

fn flag_column(cause: ConflictCause) -> &'static str {
    match cause {
        ConflictCause::Block => "conflict_block",
        ConflictCause::Eligibility => "conflict_eligibility",
    }
}

pub(crate) async fn set_flag(db: &Db, booking_id: i64, cause: ConflictCause, on: bool) -> Result {
    let stamp = now();
    Booking::query()
        .where_(Booking::ID.eq(booking_id))
        .update(db, &[(flag_column(cause), &on), ("updated_at", &stamp)])
        .await?;
    Ok(())
}

/// Flags `bookings` of `resource_id` with `cause`, once per block (or item):
/// a booking already flagged for the same block is left as it is. Emits one
/// `BookingConflict` for the bookings newly flagged and returns them.
///
/// # Errors
///
/// A database error, or the event listeners'.
pub async fn raise(
    state: &AppState,
    resource_id: i64,
    cause: ConflictCause,
    block_id: Option<i64>,
    item: Option<&str>,
    bookings: &[Booking],
    at: DateTime,
) -> Result<Vec<Booking>> {
    let mut flagged: Vec<Booking> = Vec::new();
    for booking in bookings {
        if !can_conflict(booking.status) {
            continue;
        }
        let same = open_of(&state.db, booking.id)
            .await?
            .into_iter()
            .any(|row| {
                row.cause == cause.as_str()
                    && row.block_id == block_id
                    && row.item.as_deref() == item
            });
        if same {
            continue;
        }
        Conflict {
            booking_id: booking.id,
            resource_id,
            cause: cause.as_str().to_owned(),
            block_id,
            item: item.map(str::to_owned),
            raised_at: at,
            ..Conflict::default()
        }
        .save(&state.db)
        .await?;
        set_flag(&state.db, booking.id, cause, true).await?;
        flagged.push(Booking::find_or_404(&state.db, booking.id).await?);
    }
    let Some(earliest) = flagged.iter().map(|booking| booking.starts_at).min() else {
        return Ok(flagged);
    };
    state
        .emit(BookingConflict {
            resource: resource_id,
            cause,
            block: block_id,
            item: item.map(str::to_owned),
            bookings: flagged.iter().map(Booking::booking_id).collect(),
            earliest_start: earliest,
        })
        .await?;
    // A conflict raised inside the escalation window is escalated at once
    // (`SCH.RUL.06`).
    for booking in &flagged {
        rules::escalate(state, booking.id, at).await?;
    }
    Ok(flagged)
}

/// The bookings of `resource_id` that can conflict and overlap `from`..`until`.
async fn overlapping(
    db: &Db,
    resource_id: i64,
    from: DateTime,
    until: DateTime,
) -> Result<Vec<Booking>> {
    let ids: Vec<i64> = BookingResource::query()
        .where_(BookingResource::RESOURCE_ID.eq(resource_id))
        .where_(BookingResource::RELEASED.eq(false))
        .get(db)
        .await?
        .into_iter()
        .map(|row| row.booking_id)
        .collect();
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut found: Vec<Booking> = Booking::query()
        .where_(Booking::ID.is_in(ids))
        .order_by(Booking::STARTS_AT)
        .get(db)
        .await?
        .into_iter()
        .filter(|booking| {
            can_conflict(booking.status) && booking.starts_at < until && from < booking.ends_at
        })
        .collect();
    found.sort_by_key(|booking| (booking.starts_at, booking.id));
    Ok(found)
}

/// Flags the bookings a new `block` overlaps (cause `block`). Nothing is
/// cancelled.
///
/// # Errors
///
/// A database error.
pub async fn raise_for_block(
    state: &AppState,
    block: &AvailabilityBlock,
    at: DateTime,
) -> Result<Vec<Booking>> {
    let affected =
        overlapping(&state.db, block.resource_id, block.starts_at, block.ends_at).await?;
    raise(
        state,
        block.resource_id,
        ConflictCause::Block,
        Some(block.id),
        None,
        &affected,
        at,
    )
    .await
}

/// Adds `block` to its resource and flags the bookings it overlaps. A block
/// of the same source and reference on the same resource is not added twice
/// (a leave event delivered again); its bookings are checked again.
///
/// # Errors
///
/// An unknown resource, or a block that ends before it starts.
pub async fn add_block(
    state: &AppState,
    mut block: AvailabilityBlock,
    at: DateTime,
) -> std::result::Result<(AvailabilityBlock, Vec<Booking>), Refusal> {
    if block.ends_at <= block.starts_at {
        return Err(Refusal::invalid("wl_scheduling.problem.block_period"));
    }
    if Resource::find(&state.db, block.resource_id)
        .await?
        .is_none()
    {
        return Err(Refusal::invalid("wl_scheduling.problem.unknown_resource"));
    }
    let existing = match &block.source_ref {
        Some(reference) => {
            AvailabilityBlock::query()
                .where_(AvailabilityBlock::RESOURCE_ID.eq(block.resource_id))
                .where_(AvailabilityBlock::SOURCE.eq(block.source.clone()))
                .where_(AvailabilityBlock::SOURCE_REF.eq(reference.clone()))
                .first(&state.db)
                .await?
        }
        None => None,
    };
    let block = match existing {
        Some(found) => found,
        None => {
            block.created_at = Some(at);
            block.save(&state.db).await?;
            block
        }
    };
    let flagged = raise_for_block(state, &block, at).await?;
    Ok((block, flagged))
}

/// Ends the conflicts `block_id` caused and recomputes the flag of each
/// booking: it stays flagged while another block still conflicts with it.
async fn clear_for_block(state: &AppState, block_id: i64, at: DateTime) -> Result<usize> {
    let rows = Conflict::query()
        .where_(Conflict::BLOCK_ID.eq(block_id))
        .where_(Conflict::CLEARED_AT.is_null())
        .get(&state.db)
        .await?;
    let cleared = rows.len();
    for row in rows {
        Conflict::query()
            .where_(Conflict::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("cleared_at", &Some(at)),
                    ("cleared_by", &Some("block_removed".to_owned())),
                ],
            )
            .await?;
        let still = open_of(&state.db, row.booking_id)
            .await?
            .iter()
            .any(|other| other.cause == ConflictCause::Block.as_str());
        if !still {
            set_flag(&state.db, row.booking_id, ConflictCause::Block, false).await?;
        }
    }
    Ok(cleared)
}

/// Removes `block_id` (the leave was cancelled, staff took the block away) and
/// clears the conflicts it caused. Returns how many were cleared.
///
/// # Errors
///
/// An unknown block.
pub async fn remove_block(state: &AppState, block_id: i64, at: DateTime) -> Result<usize> {
    let block = AvailabilityBlock::find_or_404(&state.db, block_id).await?;
    let cleared = clear_for_block(state, block.id, at).await?;
    AvailabilityBlock::query()
        .where_(AvailabilityBlock::ID.eq(block.id))
        .delete(&state.db)
        .await?;
    Ok(cleared)
}

/// Ends every open conflict of `booking_id` for `how` (`reassign`,
/// `reschedule`, `cancel`); the machine has cleared the flags already.
///
/// # Errors
///
/// A database error.
pub async fn close_all(db: &Db, booking_id: i64, how: &str, at: DateTime) -> Result {
    for row in open_of(db, booking_id).await? {
        Conflict::query()
            .where_(Conflict::ID.eq(row.id))
            .update(
                db,
                &[
                    ("cleared_at", &Some(at)),
                    ("cleared_by", &Some(how.to_owned())),
                ],
            )
            .await?;
    }
    Ok(())
}

/// `LeaveApproved` (a leave was approved, or sick leave recorded at once):
/// blocks the resources of the employee from `at` (or the first day, if
/// later) to the end of the last day, and flags the bookings they overlap.
///
/// # Errors
///
/// A database error.
pub async fn leave_approved(
    state: &AppState,
    event: &LeaveApproved,
    at: DateTime,
) -> Result<Vec<Booking>> {
    let starts_at = day_start(event.from).max(at);
    let ends_at = day_start(event.to + TimeDelta::days(1));
    let mut flagged = Vec::new();
    if starts_at >= ends_at {
        return Ok(flagged);
    }
    for resource in Resource::query()
        .where_(Resource::EMPLOYEE_ID.eq(event.employee.0))
        .order_by(Resource::ID)
        .get(&state.db)
        .await?
    {
        let mut block =
            AvailabilityBlock::manual(resource.id, starts_at, ends_at, &event.leave_type);
        block.source = "leave".to_owned();
        block.source_ref = Some(event.request.to_string());
        let (_, newly) = add_block(state, block, at).await.map_err(|refusal| {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, refusal_text(&refusal))
        })?;
        flagged.extend(newly);
    }
    Ok(flagged)
}

/// `LeaveCancelled`: takes the leave's blocks away and clears the conflicts
/// they caused.
///
/// # Errors
///
/// A database error.
pub async fn leave_cancelled(state: &AppState, event: &LeaveCancelled, at: DateTime) -> Result {
    for resource in Resource::query()
        .where_(Resource::EMPLOYEE_ID.eq(event.employee.0))
        .get(&state.db)
        .await?
    {
        for block in AvailabilityBlock::query()
            .where_(AvailabilityBlock::RESOURCE_ID.eq(resource.id))
            .where_(AvailabilityBlock::SOURCE.eq("leave"))
            .where_(AvailabilityBlock::SOURCE_REF.eq(event.request.to_string()))
            .get(&state.db)
            .await?
        {
            remove_block(state, block.id, at).await?;
        }
    }
    Ok(())
}

/// Asks the app's [`AvailabilitySource`] for the blocks of `resource_id`
/// between `from` and `until` and adds those not yet here, flagging the
/// bookings they overlap. Returns the bookings newly flagged.
///
/// # Errors
///
/// A database error, or the source's.
pub async fn pull_blocks(
    state: &AppState,
    resource_id: i64,
    from: DateTime,
    until: DateTime,
    at: DateTime,
) -> Result<Vec<Booking>> {
    let resource = Resource::find_or_404(&state.db, resource_id).await?;
    let source = availability_source(state);
    let blocks = source
        .blocks(state, resource.id, resource.employee_id, from, until)
        .await?;
    let mut flagged = Vec::new();
    for found in blocks {
        let mut block =
            AvailabilityBlock::manual(resource.id, found.starts_at, found.ends_at, &found.reason);
        block.source = found.source;
        block.source_ref = Some(found.reference);
        let (_, newly) = add_block(state, block, at).await.map_err(|refusal| {
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, refusal_text(&refusal))
        })?;
        flagged.extend(newly);
    }
    Ok(flagged)
}

fn refusal_text(refusal: &Refusal) -> String {
    refusal.plain()
}

/// `Bookings::conflicts(resource?, period?)`: the conflicted bookings, by
/// start, optionally of one `resource` and starting inside `period`.
///
/// # Errors
///
/// A database error.
pub async fn conflicts(
    db: &Db,
    resource: Option<i64>,
    period: Option<(DateTime, DateTime)>,
) -> Result<Vec<Booking>> {
    let mut found: BTreeMap<i64, Booking> = BTreeMap::new();
    for booking in Booking::query()
        .where_(Booking::CONFLICT_BLOCK.eq(true))
        .get(db)
        .await?
    {
        found.insert(booking.id, booking);
    }
    for booking in Booking::query()
        .where_(Booking::CONFLICT_ELIGIBILITY.eq(true))
        .get(db)
        .await?
    {
        found.insert(booking.id, booking);
    }
    let mut out = Vec::new();
    for booking in found.into_values() {
        if !can_conflict(booking.status) {
            continue;
        }
        if let Some((from, until)) = period
            && !(booking.starts_at >= from && booking.starts_at < until)
        {
            continue;
        }
        if let Some(resource) = resource
            && !booking.resource_ids(db).await?.contains(&resource)
        {
            continue;
        }
        out.push(booking);
    }
    out.sort_by_key(|booking| (booking.starts_at, booking.id));
    Ok(out)
}

/// What staff do to the selected conflicted bookings in one action
/// (§5.5 step 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkAction {
    /// Move each booking to another eligible resource at the same time: `to`,
    /// or the first resource that can take it when `None`.
    Reassign {
        /// The resource that takes over, if staff chose one.
        to: Option<i64>,
    },
    /// Cancel each booking as the business, with this reason code
    /// (`resource_unavailable` by default).
    Cancel {
        /// The reason code.
        reason_code: String,
    },
    /// Send each party up to three next free times of the same resource.
    /// Nothing is moved or held; the booking stays conflicted.
    Reschedule,
}

/// Why one booking of a bulk action failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkFailure {
    /// The booking.
    pub booking: i64,
    /// What went wrong, in English.
    pub reason: String,
}

/// The result of [`bulk_resolve`]: what worked and what did not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkResult {
    /// The bookings the action was done for.
    pub done: Vec<i64>,
    /// The bookings it failed for; they stay as they were.
    pub failed: Vec<BulkFailure>,
    /// Bookings the action was done for but with a problem after it (a
    /// deposit refund `wl-payments` refused).
    pub warnings: Vec<BulkFailure>,
}

fn failure(booking: i64, refusal: &Refusal) -> BulkFailure {
    BulkFailure {
        booking,
        reason: refusal_text(refusal),
    }
}

/// The resource of `booking` that carries its open conflict, else its only
/// resource.
async fn conflicted_resource(db: &Db, booking: &Booking) -> Result<Option<i64>> {
    if let Some(row) = open_of(db, booking.id).await?.first() {
        return Ok(Some(row.resource_id));
    }
    let ids = booking.resource_ids(db).await?;
    Ok(match ids.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// Reassigns `booking` to `to`, or to the first resource of the same type and
/// location that can take it.
async fn reassign_one(
    state: &AppState,
    booking: &Booking,
    to: Option<i64>,
    actor: Actor,
    at: DateTime,
) -> std::result::Result<(Booking, String), Refusal> {
    let Some(from) = conflicted_resource(&state.db, booking).await? else {
        return Err(Refusal::invalid("wl_scheduling.problem.not_on_booking"));
    };
    if let Some(to) = to {
        return service::reassign_quietly(
            state,
            booking.id,
            from,
            to,
            BLOCK_REASON_CODE,
            actor,
            at,
        )
        .await;
    }
    let held = booking.resource_ids(&state.db).await?;
    let current = Resource::find_or_404(&state.db, from).await?;
    let mut last = Refusal::invalid("wl_scheduling.problem.no_replacement");
    for candidate in eligible_resources(&state.db, booking.service_type_id).await? {
        if held.contains(&candidate.id)
            || candidate.resource_type != current.resource_type
            || candidate.location() != current.location()
        {
            continue;
        }
        match service::reassign_quietly(
            state,
            booking.id,
            from,
            candidate.id,
            BLOCK_REASON_CODE,
            actor,
            at,
        )
        .await
        {
            Ok(done) => return Ok(done),
            Err(refusal) => last = refusal,
        }
    }
    Err(last)
}

/// The times of the next free slots of the booking's own resources, up to
/// three (not held).
pub async fn offer_slots(
    state: &AppState,
    booking: &Booking,
    at: DateTime,
) -> Result<Vec<DateTime>> {
    let service = ServiceType::find_or_404(&state.db, booking.service_type_id).await?;
    let ids = booking.resource_ids(&state.db).await?;
    let mut resources = Vec::new();
    for id in &ids {
        if let Some(resource) = Resource::find(&state.db, *id).await? {
            resources.push(resource);
        }
    }
    let mut times = Vec::new();
    for slot in slots::nearest(
        state,
        &service,
        &resources,
        booking.starts_at,
        at,
        slots::SHOWN,
    )
    .await?
    {
        let mut same = slot.resource_ids.clone();
        same.sort_unstable();
        if same == ids {
            times.push(slot.start);
        }
    }
    Ok(times)
}

/// Applies `action` to `bookings` (conflicted, before the check-in), each in
/// its own transaction, and tells the result. A party with several bookings
/// in the action gets one message listing them all, with *accept*, *pick
/// another time* and *cancel* links and no reason.
///
/// # Errors
///
/// A database error or the notifier's; a booking that cannot be handled is
/// listed in [`BulkResult::failed`], not returned as an error.
pub async fn bulk_resolve(
    state: &AppState,
    bookings: &[i64],
    action: &BulkAction,
    actor: Actor,
    at: DateTime,
) -> Result<BulkResult> {
    let cfg = config_of(state);
    if let BulkAction::Cancel { reason_code } = action
        && default_initiator(reason_code, &cfg.extra_reason_codes).is_none()
    {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            crate::texts::english("wl_scheduling.problem.unknown_reason", &[]),
        ));
    }
    let mut result = BulkResult::default();
    let mut told: BTreeMap<i64, Vec<ChangeEntry>> = BTreeMap::new();
    for id in bookings {
        let Some(booking) = Booking::find(&state.db, *id).await? else {
            result.failed.push(BulkFailure {
                booking: *id,
                reason: crate::texts::english("wl_scheduling.problem.unknown_booking", &[]),
            });
            continue;
        };
        if !booking.has_conflict() || !can_conflict(booking.status) {
            result.failed.push(BulkFailure {
                booking: *id,
                reason: crate::texts::english("wl_scheduling.problem.not_conflicted", &[]),
            });
            continue;
        }
        match action {
            BulkAction::Reassign { to } => {
                match reassign_one(state, &booking, *to, actor, at).await {
                    Ok((moved, resource)) => {
                        result.done.push(moved.id);
                        told.entry(moved.party_id).or_default().push(ChangeEntry {
                            booking: moved,
                            resource: Some(resource),
                            slots: Vec::new(),
                        });
                    }
                    Err(refusal) => result.failed.push(failure(*id, &refusal)),
                }
            }
            BulkAction::Cancel { reason_code } => {
                let outcome = service::cancel(
                    state,
                    booking.id,
                    CancelRequest::new(reason_code),
                    actor,
                    at,
                )
                .await;
                match outcome {
                    Ok(_) => result.done.push(booking.id),
                    Err(refusal) => {
                        let now_cancelled =
                            Booking::find_or_404(&state.db, booking.id).await?.status
                                == BookingStatus::Cancelled;
                        if now_cancelled {
                            // The cancellation stands; the deposit refund is
                            // asked again by `deposit::settle`.
                            result.done.push(booking.id);
                            result.warnings.push(failure(booking.id, &refusal));
                        } else {
                            result.failed.push(failure(booking.id, &refusal));
                        }
                    }
                }
            }
            BulkAction::Reschedule => {
                let slots = offer_slots(state, &booking, at).await?;
                Conflict::query()
                    .where_(Conflict::BOOKING_ID.eq(booking.id))
                    .where_(Conflict::CLEARED_AT.is_null())
                    .update(&state.db, &[("offered_at", &Some(at))])
                    .await?;
                result.done.push(booking.id);
                told.entry(booking.party_id).or_default().push(ChangeEntry {
                    booking,
                    resource: None,
                    slots,
                });
            }
        }
    }
    let type_key = match action {
        BulkAction::Reschedule => notices::RESCHEDULE_OFFER,
        _ => notices::CHANGED,
    };
    for (party, entries) in told {
        notices::changes(
            state,
            PartyId(party),
            &entries,
            type_key,
            BLOCK_REASON_CODE,
            at,
        )
        .await?;
    }
    Ok(result)
}
