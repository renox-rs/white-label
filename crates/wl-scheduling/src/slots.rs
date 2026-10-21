//! Free time (wl-scheduling §3 *Availability*): the availability of a
//! resource at a time is its rules, less its blocks, less its bookings,
//! judged for every resource the service type needs.
//!
//! The pure parts ([`segments`], [`covered`], [`blocked`], [`fits`]) are
//! unit-tested here. [`check_free`] reads the database inside the caller's
//! transaction, so that the check and the insert of a booking are one unit.
//!
//! Decision: capacity is the lesser of the resource's capacity and the
//! service type's seats per slot, each at least 1: a room that holds 20 gives
//! 5 seats to a class of 5, and a practitioner (capacity 1) never gives more
//! than one. The time a booking occupies includes the buffers of its service
//! type.

use renox::chrono::{Datelike, NaiveDate, TimeDelta, Timelike};
use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use wl_kernel::calendar::BusinessCalendar;

use crate::booking::{Booking, BookingResource, BookingStatus};
use crate::config::{NEAREST_SLOTS, config_of};
use crate::resource::{
    AvailabilityBlock, AvailabilityRule, Resource, ServiceType, eligible_resources,
};
use crate::waiting::{WaitingEntry, WaitingStatus};

/// A stretch of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// The start.
    pub from: DateTime,
    /// The end.
    pub until: DateTime,
}

/// A free slot offered to a booker: the resources and the time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeSlot {
    /// The resources that deliver it, by id.
    pub resource_ids: Vec<i64>,
    /// The start.
    pub start: DateTime,
    /// The end.
    pub end: DateTime,
}

/// What to leave out when counting: the booking being moved or accepted.
#[derive(Debug, Clone, Copy, Default)]
pub struct Exclude {
    /// A booking whose own rows do not count.
    pub booking: Option<i64>,
    /// A waiting-list entry whose offer does not count (the entry that takes
    /// its own slot).
    pub entry: Option<i64>,
}

/// Why a time cannot be booked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unfit {
    /// The resource named is not open then (no rule covers the time).
    Closed(String),
    /// The resource named is blocked then.
    Blocked(String),
    /// Every seat is taken.
    Taken,
}

/// The time a booking of `service` from `start` to `end` occupies, buffers
/// included.
pub fn occupied_span(service: &ServiceType, start: DateTime, end: DateTime) -> Span {
    Span {
        from: start - TimeDelta::minutes(service.buffer_before_minutes),
        until: end + TimeDelta::minutes(service.buffer_after_minutes),
    }
}

/// The seats a resource gives to a service type.
pub fn capacity_of(resource: &Resource, service: &ServiceType) -> i64 {
    resource.capacity.max(1).min(service.capacity.max(1))
}

/// The local days (Asia/Dubai) from `start` to `end`, with the minutes
/// after midnight each day covers: `(day, from, to)`.
pub fn segments(start: DateTime, end: DateTime) -> Vec<(NaiveDate, i64, i64)> {
    let calendar = BusinessCalendar::default();
    let first = calendar.local(start).naive_local();
    let last = calendar.local(end).naive_local();
    let minutes = |time: renox::chrono::NaiveTime| i64::from(time.hour() * 60 + time.minute());
    let mut out = Vec::new();
    let mut day = first.date();
    loop {
        let from = if day == first.date() {
            minutes(first.time())
        } else {
            0
        };
        let to = if day == last.date() {
            minutes(last.time())
        } else {
            1440
        };
        if to > from {
            out.push((day, from, to));
        }
        if day >= last.date() {
            break;
        }
        match day.succ_opt() {
            Some(next) => day = next,
            None => break,
        }
    }
    out
}

/// Whether the rules open the resource for the whole of `start`..`end` for
/// `service_type_id`: every local day of the stretch lies inside one rule of
/// its weekday.
pub fn covered(
    rules: &[AvailabilityRule],
    service_type_id: i64,
    start: DateTime,
    end: DateTime,
) -> bool {
    segments(start, end).into_iter().all(|(day, from, to)| {
        let weekday = i64::from(day.weekday().number_from_monday());
        rules.iter().any(|rule| {
            rule.weekday == weekday
                && rule.applies(day, service_type_id)
                && rule.from_minute <= from
                && to <= rule.to_minute
        })
    })
}

/// Whether a block overlaps `start`..`end`.
pub fn blocked(blocks: &[AvailabilityBlock], start: DateTime, end: DateTime) -> bool {
    blocks
        .iter()
        .any(|block| block.starts_at < end && start < block.ends_at)
}

/// Whether `from`..`until` can take one more booking when `busy` is already
/// held and `capacity` seats exist: at no instant of it are all seats taken.
pub fn fits(busy: &[Span], from: DateTime, until: DateTime, capacity: i64) -> bool {
    let mut points = vec![from];
    points.extend(
        busy.iter()
            .map(|span| span.from)
            .filter(|point| *point > from && *point < until),
    );
    points.into_iter().all(|point| {
        let held = busy
            .iter()
            .filter(|span| span.from <= point && point < span.until)
            .count();
        (held as i64) < capacity
    })
}

/// Whether `booking` takes a seat at `now` (§3 *No double-booking*).
///
/// A booking past its request takes a seat until it ends. Holds count only
/// while they last, whether or not the cleanup job has run: a `requested`
/// booking counts until its hold time, and a booking that still waits for its
/// prepayment counts until the payment link expires.
pub fn holds(booking: &Booking, now: DateTime) -> bool {
    match booking.status {
        BookingStatus::Requested => booking.hold_until.is_some_and(|until| until > now),
        BookingStatus::Booked
        | BookingStatus::Confirmed
        | BookingStatus::CheckedIn
        | BookingStatus::InProgress => {
            !(booking.payment_pending && booking.hold_until.is_some_and(|until| until <= now))
        }
        BookingStatus::Completed | BookingStatus::NoShow | BookingStatus::Cancelled => false,
    }
}

/// The stretches of `resource` that are taken and overlap `from`..`until`.
pub async fn busy(
    tx: &mut Transaction,
    resource: i64,
    from: DateTime,
    until: DateTime,
    now: DateTime,
    exclude: Exclude,
) -> Result<Vec<Span>> {
    let rows = BookingResource::query()
        .where_(BookingResource::RESOURCE_ID.eq(resource))
        .where_(BookingResource::RELEASED.eq(false))
        .where_(BookingResource::OCCUPIES_FROM.lt(until))
        .where_(BookingResource::OCCUPIES_UNTIL.gt(from))
        .get(&mut *tx)
        .await?;
    let ids: Vec<i64> = rows.iter().map(|row| row.booking_id).collect();
    let bookings = if ids.is_empty() {
        Vec::new()
    } else {
        Booking::query()
            .where_(Booking::ID.is_in(ids))
            .get(&mut *tx)
            .await?
    };
    let mut spans = Vec::new();
    for row in &rows {
        let Some(booking) = bookings.iter().find(|b| b.id == row.booking_id) else {
            continue;
        };
        if exclude.booking == Some(booking.id) || !holds(booking, now) {
            continue;
        }
        spans.push(Span {
            from: row.occupies_from,
            until: row.occupies_until,
        });
    }
    // A slot offered to a waiting-list entry is held until the offer lapses.
    let offers = WaitingEntry::query()
        .where_(WaitingEntry::STATUS.eq(WaitingStatus::Offered))
        .where_(WaitingEntry::OFFERED_RESOURCE_ID.eq(resource))
        .where_(WaitingEntry::OFFERED_FROM.lt(until))
        .where_(WaitingEntry::OFFERED_UNTIL.gt(from))
        .where_(WaitingEntry::OFFER_HOLD_UNTIL.gt(now))
        .get(&mut *tx)
        .await?;
    for offer in offers {
        if exclude.entry == Some(offer.id) {
            continue;
        }
        if let (Some(from), Some(until)) = (offer.offered_from, offer.offered_until) {
            spans.push(Span { from, until });
        }
    }
    Ok(spans)
}

/// Locks the resources `ids` for the rest of the transaction and reads them,
/// in a fixed order (by id), so that two bookers who need the same resources
/// ask for them in the same order and cannot deadlock.
///
/// Call it in a transaction that began with `begin_immediate`: on SQLite that
/// holds the whole database for writing, on PostgreSQL `FOR UPDATE` locks the
/// resource rows. Either way a second booker waits here until the first has
/// committed, and then counts the seats it took ([`check_free`]).
pub async fn lock(tx: &mut Transaction, ids: &[i64]) -> Result<Vec<Resource>> {
    Resource::query()
        .where_(Resource::ID.is_in(ids.to_vec()))
        .order_by(Resource::ID)
        .lock_for_update()
        .get(&mut *tx)
        .await
}

/// Whether every one of `resources` is open, unblocked and has a free seat
/// for `service` from `start` to `end` (all or nothing). Run it in the
/// transaction that inserts the booking.
pub async fn check_free(
    tx: &mut Transaction,
    service: &ServiceType,
    resources: &[Resource],
    start: DateTime,
    end: DateTime,
    now: DateTime,
    exclude: Exclude,
) -> Result<std::result::Result<(), Unfit>> {
    let occupied = occupied_span(service, start, end);
    for resource in resources {
        let rules = AvailabilityRule::query()
            .where_(AvailabilityRule::RESOURCE_ID.eq(resource.id))
            .get(&mut *tx)
            .await?;
        if !covered(&rules, service.id, start, end) {
            return Ok(Err(Unfit::Closed(resource.name.clone())));
        }
        let blocks = AvailabilityBlock::query()
            .where_(AvailabilityBlock::RESOURCE_ID.eq(resource.id))
            .where_(AvailabilityBlock::STARTS_AT.lt(end))
            .where_(AvailabilityBlock::ENDS_AT.gt(start))
            .get(&mut *tx)
            .await?;
        if blocked(&blocks, start, end) {
            return Ok(Err(Unfit::Blocked(resource.name.clone())));
        }
        let taken = busy(tx, resource.id, occupied.from, occupied.until, now, exclude).await?;
        if !fits(
            &taken,
            occupied.from,
            occupied.until,
            capacity_of(resource, service),
        ) {
            return Ok(Err(Unfit::Taken));
        }
    }
    Ok(Ok(()))
}

/// The first slot on the grid at or after `from`, `step` minutes apart.
fn align(from: DateTime, step_minutes: i64) -> DateTime {
    let step = step_minutes.max(1) * 60;
    let mut ts = from.timestamp();
    if from.timestamp_subsec_nanos() > 0 {
        ts += 1;
    }
    let aligned = (ts + step - 1).div_euclid(step) * step;
    from.with_nanosecond(0).unwrap_or(from) + TimeDelta::seconds(aligned - from.timestamp())
}

/// The first `limit` slots of `service` between `from` and `until` that are
/// free for all of `resources`, on the grid of `step_minutes`.
#[allow(clippy::too_many_arguments)]
pub async fn free_in(
    tx: &mut Transaction,
    service: &ServiceType,
    resources: &[Resource],
    from: DateTime,
    until: DateTime,
    step_minutes: i64,
    limit: usize,
    now: DateTime,
) -> Result<Vec<FreeSlot>> {
    let duration = TimeDelta::minutes(service.duration_minutes);
    let reach_from = from - TimeDelta::minutes(service.buffer_before_minutes);
    let reach_until = until + duration + TimeDelta::minutes(service.buffer_after_minutes);
    let mut loaded = Vec::new();
    for resource in resources {
        let rules = AvailabilityRule::query()
            .where_(AvailabilityRule::RESOURCE_ID.eq(resource.id))
            .get(&mut *tx)
            .await?;
        let blocks = AvailabilityBlock::query()
            .where_(AvailabilityBlock::RESOURCE_ID.eq(resource.id))
            .where_(AvailabilityBlock::STARTS_AT.lt(reach_until))
            .where_(AvailabilityBlock::ENDS_AT.gt(reach_from))
            .get(&mut *tx)
            .await?;
        let taken = busy(
            tx,
            resource.id,
            reach_from,
            reach_until,
            now,
            Exclude::default(),
        )
        .await?;
        loaded.push((resource, rules, blocks, taken));
    }
    let step = TimeDelta::minutes(step_minutes.max(1));
    let mut out = Vec::new();
    let mut candidate = align(from, step_minutes);
    while candidate <= until && out.len() < limit {
        let end = candidate + duration;
        let occupied = occupied_span(service, candidate, end);
        let free = loaded.iter().all(|(resource, rules, blocks, taken)| {
            covered(rules, service.id, candidate, end)
                && !blocked(blocks, candidate, end)
                && fits(
                    taken,
                    occupied.from,
                    occupied.until,
                    capacity_of(resource, service),
                )
        });
        if free {
            out.push(FreeSlot {
                resource_ids: resources.iter().map(|resource| resource.id).collect(),
                start: candidate,
                end,
            });
        }
        candidate += step;
    }
    Ok(out)
}

/// The nearest free slots to offer a booker who just lost the slot at
/// `start` (§3 *No double-booking*): up to `limit` of the same service type
/// and location, from the requested time up to the booking horizon. First
/// the free slots of the same resources; when fewer than `limit`, the
/// earliest free slots of the other eligible resources of the location (any
/// time, not only the requested one), by start. Slots a resource may not
/// deliver (`ResourceEligibility`) are left out.
pub async fn nearest(
    state: &AppState,
    service: &ServiceType,
    requested: &[Resource],
    start: DateTime,
    now: DateTime,
    limit: usize,
) -> Result<Vec<FreeSlot>> {
    let cfg = config_of(state);
    let horizon = now + TimeDelta::days(cfg.horizon_days);
    let from = start.max(now);
    // Read before the transaction opens: a pool of one connection cannot
    // serve a second query while the transaction holds it.
    let others: Vec<Resource> = if requested.len() == 1 && service.required().len() == 1 {
        let location = requested[0].location();
        eligible_resources(&state.db, service.id)
            .await?
            .into_iter()
            .filter(|resource| {
                resource.id != requested[0].id
                    && resource.resource_type == requested[0].resource_type
                    && resource.location() == location
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut tx = state.db.begin().await?;
    // A few more than asked for, as some may be left out below.
    let wanted = limit * 3;
    let mut found = free_in(
        &mut tx,
        service,
        requested,
        from,
        horizon,
        cfg.slot_step_minutes,
        wanted,
        now,
    )
    .await?;
    let mut elsewhere = Vec::new();
    if found.len() < limit {
        for resource in &others {
            let slots = free_in(
                &mut tx,
                service,
                std::slice::from_ref(resource),
                now,
                horizon,
                cfg.slot_step_minutes,
                wanted,
                now,
            )
            .await?;
            elsewhere.extend(slots);
        }
    }
    tx.rollback().await?;

    let mut everyone: Vec<&Resource> = requested.iter().collect();
    everyone.extend(others.iter());
    let mut kept = Vec::new();
    for slot in found.drain(..) {
        if eligible_slot(state, service, &everyone, &slot).await {
            kept.push(slot);
        }
        if kept.len() >= limit {
            return Ok(kept);
        }
    }
    elsewhere.sort_by_key(|slot| (slot.start, slot.resource_ids.clone()));
    for slot in elsewhere {
        if eligible_slot(state, service, &everyone, &slot).await {
            kept.push(slot);
        }
        if kept.len() >= limit {
            break;
        }
    }
    kept.truncate(limit);
    Ok(kept)
}

/// Whether every resource of `slot` may deliver the service at its start.
async fn eligible_slot(
    state: &AppState,
    service: &ServiceType,
    resources: &[&Resource],
    slot: &FreeSlot,
) -> bool {
    let chosen: Vec<Resource> = resources
        .iter()
        .filter(|resource| slot.resource_ids.contains(&resource.id))
        .map(|resource| (*resource).clone())
        .collect();
    crate::service::check_eligible(state, service, &chosen, slot.start)
        .await
        .is_ok()
}

/// How many nearest slots a refused booker is shown (§3).
pub const SHOWN: usize = NEAREST_SLOTS;

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: u32, hour: u32, minute: u32) -> DateTime {
        // Asia/Dubai is UTC+4: 2026-11-<day> <hour>:<minute> local.
        BusinessCalendar::default().from_local(
            NaiveDate::from_ymd_opt(2026, 11, day)
                .unwrap()
                .and_hms_opt(hour, minute, 0)
                .unwrap(),
        )
    }

    fn span(from: DateTime, until: DateTime) -> Span {
        Span { from, until }
    }

    /// 2026-11-02 is a Monday.
    fn monday_nine_to_five() -> Vec<AvailabilityRule> {
        vec![AvailabilityRule::new(1, 1, 9 * 60, 17 * 60)]
    }

    #[test]
    fn the_grid_starts_at_the_next_step() {
        assert_eq!(align(at(2, 10, 0), 15), at(2, 10, 0));
        assert_eq!(align(at(2, 10, 1), 15), at(2, 10, 15));
        assert_eq!(align(at(2, 10, 14), 15), at(2, 10, 15));
        assert_eq!(align(at(2, 10, 46), 30), at(2, 11, 0));
    }

    fn booking(status: BookingStatus, hold: Option<DateTime>, pending: bool) -> Booking {
        Booking {
            status,
            hold_until: hold,
            payment_pending: pending,
            ..Booking::default()
        }
    }

    #[test]
    fn a_request_holds_the_slot_only_while_its_hold_lasts() {
        let now = at(2, 10, 0);
        let held = booking(BookingStatus::Requested, Some(at(2, 10, 5)), false);
        let lapsed = booking(BookingStatus::Requested, Some(at(2, 9, 59)), false);
        let ending_now = booking(BookingStatus::Requested, Some(now), false);
        assert!(holds(&held, now));
        assert!(!holds(&lapsed, now));
        assert!(!holds(&ending_now, now));
    }

    #[test]
    fn an_unpaid_booking_holds_the_slot_until_the_link_expires() {
        let now = at(2, 10, 0);
        let unpaid = booking(BookingStatus::Booked, Some(at(2, 10, 5)), true);
        let expired = booking(BookingStatus::Booked, Some(at(2, 9, 55)), true);
        let paid = booking(BookingStatus::Booked, Some(at(2, 9, 55)), false);
        assert!(holds(&unpaid, now));
        assert!(!holds(&expired, now));
        assert!(holds(&paid, now));
    }

    #[test]
    fn an_ended_booking_holds_nothing() {
        let now = at(2, 10, 0);
        for status in [
            BookingStatus::Completed,
            BookingStatus::NoShow,
            BookingStatus::Cancelled,
        ] {
            assert!(!holds(&booking(status, None, false), now));
        }
        assert!(holds(&booking(BookingStatus::CheckedIn, None, false), now));
    }

    #[test]
    fn a_stretch_inside_one_rule_is_covered() {
        let rules = monday_nine_to_five();
        assert!(covered(&rules, 7, at(2, 9, 0), at(2, 9, 30)));
        assert!(covered(&rules, 7, at(2, 16, 30), at(2, 17, 0)));
    }

    #[test]
    fn a_stretch_outside_the_rules_is_not_covered() {
        let rules = monday_nine_to_five();
        assert!(!covered(&rules, 7, at(2, 8, 45), at(2, 9, 15)));
        assert!(!covered(&rules, 7, at(2, 16, 45), at(2, 17, 15)));
        // Tuesday has no rule.
        assert!(!covered(&rules, 7, at(3, 10, 0), at(3, 10, 30)));
    }

    #[test]
    fn a_rule_for_another_service_type_does_not_open_this_one() {
        let mut rule = AvailabilityRule::new(1, 1, 0, 1440);
        rule.service_type_id = Some(8);
        assert!(!covered(&[rule.clone()], 7, at(2, 10, 0), at(2, 10, 30)));
        assert!(covered(&[rule], 8, at(2, 10, 0), at(2, 10, 30)));
    }

    #[test]
    fn a_stretch_over_midnight_needs_both_days() {
        let all_week: Vec<AvailabilityRule> = (1..=7)
            .map(|weekday| AvailabilityRule::new(1, weekday, 0, 1440))
            .collect();
        assert!(covered(&all_week, 7, at(2, 23, 45), at(3, 0, 15)));
        assert!(!covered(
            &monday_nine_to_five(),
            7,
            at(2, 16, 45),
            at(3, 0, 15)
        ));
        assert_eq!(segments(at(2, 23, 45), at(3, 0, 15)).len(), 2);
    }

    #[test]
    fn a_block_overlaps_only_the_time_it_covers() {
        let block = AvailabilityBlock::manual(1, at(2, 10, 0), at(2, 11, 0), "sick");
        assert!(blocked(
            std::slice::from_ref(&block),
            at(2, 10, 30),
            at(2, 11, 30)
        ));
        assert!(blocked(
            std::slice::from_ref(&block),
            at(2, 9, 30),
            at(2, 10, 1)
        ));
        assert!(!blocked(
            std::slice::from_ref(&block),
            at(2, 11, 0),
            at(2, 11, 30)
        ));
        assert!(!blocked(&[block], at(2, 9, 0), at(2, 10, 0)));
    }

    #[test]
    fn a_single_seat_is_taken_by_any_overlap() {
        let busy = [span(at(2, 10, 0), at(2, 10, 30))];
        assert!(!fits(&busy, at(2, 10, 15), at(2, 10, 45), 1));
        assert!(fits(&busy, at(2, 10, 30), at(2, 11, 0), 1));
        assert!(fits(&busy, at(2, 9, 30), at(2, 10, 0), 1));
    }

    #[test]
    fn a_group_slot_fills_up_seat_by_seat() {
        let busy = [
            span(at(2, 10, 0), at(2, 10, 30)),
            span(at(2, 10, 0), at(2, 10, 30)),
        ];
        assert!(fits(&busy, at(2, 10, 0), at(2, 10, 30), 3));
        assert!(!fits(&busy, at(2, 10, 0), at(2, 10, 30), 2));
    }

    #[test]
    fn bookings_one_after_the_other_do_not_add_up() {
        let busy = [
            span(at(2, 10, 0), at(2, 10, 30)),
            span(at(2, 10, 30), at(2, 11, 0)),
        ];
        // One seat is free at every instant of 10:00-11:00.
        assert!(fits(&busy, at(2, 10, 0), at(2, 11, 0), 2));
        assert!(!fits(&busy, at(2, 10, 0), at(2, 11, 0), 1));
    }

    #[test]
    fn the_buffers_count_as_occupied() {
        let mut service = ServiceType::new("visit", "Visit", 30);
        service.buffer_before_minutes = 5;
        service.buffer_after_minutes = 10;
        let occupied = occupied_span(&service, at(2, 10, 0), at(2, 10, 30));
        assert_eq!(occupied.from, at(2, 9, 55));
        assert_eq!(occupied.until, at(2, 10, 40));
    }

    #[test]
    fn the_seats_are_the_lesser_capacity() {
        let room = Resource::new("Room", crate::ResourceType::Room).capacity(20);
        let mut class = ServiceType::new("class", "Class", 60);
        class.capacity = 5;
        assert_eq!(capacity_of(&room, &class), 5);
        let doctor = Resource::new("Dr A", crate::ResourceType::Person);
        assert_eq!(capacity_of(&doctor, &class), 1);
    }
}
