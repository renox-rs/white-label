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
use crate::resource::{AvailabilityBlock, AvailabilityRule, Resource, ServiceType};

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

/// Whether `booking` takes a seat at `now`.
pub fn holds(booking: &Booking, _now: DateTime) -> bool {
    matches!(
        booking.status,
        BookingStatus::Requested
            | BookingStatus::Booked
            | BookingStatus::Confirmed
            | BookingStatus::CheckedIn
            | BookingStatus::InProgress
    )
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
    Ok(spans)
}

/// Reads the resources `ids` in a fixed order (by id), so that two bookers
/// who need the same resources ask for them in the same order and cannot
/// deadlock.
pub async fn lock(tx: &mut Transaction, ids: &[i64]) -> Result<Vec<Resource>> {
    Resource::query()
        .where_(Resource::ID.is_in(ids.to_vec()))
        .order_by(Resource::ID)
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
        if !fits(&taken, occupied.from, occupied.until, capacity_of(resource, service)) {
            return Ok(Err(Unfit::Taken));
        }
    }
    Ok(Ok(()))
}

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
        assert!(!covered(&monday_nine_to_five(), 7, at(2, 16, 45), at(3, 0, 15)));
        assert_eq!(segments(at(2, 23, 45), at(3, 0, 15)).len(), 2);
    }

    #[test]
    fn a_block_overlaps_only_the_time_it_covers() {
        let block = AvailabilityBlock::manual(1, at(2, 10, 0), at(2, 11, 0), "sick");
        assert!(blocked(&[block.clone()], at(2, 10, 30), at(2, 11, 30)));
        assert!(blocked(&[block.clone()], at(2, 9, 30), at(2, 10, 1)));
        assert!(!blocked(&[block.clone()], at(2, 11, 0), at(2, 11, 30)));
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
