//! Booking conflicts (#172, BL-SCH-05): a new block or approved leave flags
//! the bookings it overlaps without cancelling them, `BookingConflict` lists
//! them, staff resolve them in bulk, and the parties are told once, with the
//! links and without the reason.

mod support;

use std::time::Duration;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::dubai_date;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::scheduling::{ConflictCause, Initiator, SourceBlock};
use wl_kernel::workforce::{EmployeeId, LeaveApproved, LeaveCancelled};
use wl_scheduling::pages::local_text;
use wl_scheduling::service::{self, Actor, CancelRequest};
use wl_scheduling::{
    AvailabilityBlock, AvailabilityRule, Booking, BookingStatus, BulkAction, Resource, ServiceType,
    WlScheduling, conflict, links,
};

use support::{Booked, Messages, OTHER_PATIENT, PATIENT, boot, boot_hr, boot_with, dubai, now};

const HOUR: Duration = Duration::from_secs(3600);
const DAY: &str = "2030-03-12";
const NEXT: &str = "2030-03-13";
const EMPLOYEE: i64 = 7;

fn date(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

fn leave(request: i64, leave_type: &str, from: &str, to: &str) -> LeaveApproved {
    LeaveApproved {
        employee: EmployeeId(EMPLOYEE),
        request,
        leave_type: leave_type.to_owned(),
        from: date(from),
        to: date(to),
    }
}

/// Dr A (the employee behind her), Dr B, a service with a deposit of AED 200
/// that both deliver, and three bookings of Dr A on `DAY`: 11:00 and 15:00 of
/// the patient, 12:00 of the other patient.
struct Scene {
    a: Resource,
    b: Resource,
    service: ServiceType,
    a11: Booking,
    a12: Booking,
    a15: Booking,
}

async fn scene(t: &Booked) -> Scene {
    let mut a = t.doctor("Dr A").await;
    a.employee_id = Some(EMPLOYEE);
    a.save(t.db()).await.unwrap();
    let b = t.doctor("Dr B").await;
    let service = t
        .service_with("consultation", &[&a, &b], |service| {
            service.deposit_amount = 20_000;
            service.deposit_forfeit_hours = 24;
            service.cancel_cutoff_hours = 24;
            service.check_in_required = true;
        })
        .await;
    let a11 = t
        .book_for(PATIENT, &service, &[&a], dubai(DAY, 11, 0))
        .await;
    let a12 = t
        .book_for(OTHER_PATIENT, &service, &[&a], dubai(DAY, 12, 0))
        .await;
    let a15 = t
        .book_for(PATIENT, &service, &[&a], dubai(DAY, 15, 0))
        .await;
    Scene {
        a,
        b,
        service,
        a11,
        a12,
        a15,
    }
}

fn ids(bookings: &[&Booking]) -> Vec<i64> {
    bookings.iter().map(|booking| booking.id).collect()
}

async fn open_conflicts(t: &Booked, id: i64) -> usize {
    conflict::open_of(t.db(), id).await.unwrap().len()
}

/// Dr A has bookings at 11:00, 12:00 and 15:00; same-day sick leave
/// (`LeaveApproved`) flags all three with cause `block`, cancels none, and
/// emits one `BookingConflict` with the earliest start 11:00.
/// `Bookings::conflicts(Dr A, today)` returns the three.
#[renox::test]
async fn sick_leave_flags_the_bookings_of_the_day_and_cancels_none() {
    let t = boot().await;
    let s = scene(&t).await;

    t.state()
        .emit(leave(501, "sick", DAY, DAY))
        .await
        .expect("the leave is handled");

    for booking in [&s.a11, &s.a12, &s.a15] {
        let flagged = t.reload(booking.id).await;
        assert!(flagged.conflict_block, "{} is flagged", booking.number);
        assert!(!flagged.conflict_eligibility);
        assert_eq!(
            flagged.status,
            BookingStatus::Booked,
            "nothing is cancelled"
        );
    }
    assert!(t.events.cancelled.all().is_empty());
    let raised = t.events.conflicts.all();
    assert_eq!(raised.len(), 1, "one event for the block");
    assert_eq!(raised[0].resource, s.a.id);
    assert_eq!(raised[0].cause, ConflictCause::Block);
    assert!(raised[0].block.is_some());
    let mut listed: Vec<i64> = raised[0].bookings.iter().map(|id| id.0).collect();
    listed.sort_unstable();
    assert_eq!(listed, ids(&[&s.a11, &s.a12, &s.a15]));
    assert_eq!(raised[0].earliest_start, dubai(DAY, 11, 0));

    let listed = conflict::conflicts(t.db(), Some(s.a.id), Some(conflict::day_bounds(date(DAY))))
        .await
        .unwrap();
    assert_eq!(
        listed.iter().map(|b| b.id).collect::<Vec<_>>(),
        ids(&[&s.a11, &s.a12, &s.a15])
    );
    let blocks = AvailabilityBlock::query().get(t.db()).await.unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].source, "leave");
    assert_eq!(blocks[0].reason, "sick");
    assert_eq!(blocks[0].source_ref.as_deref(), Some("501"));
    // Dr B is not Dr A's employee: no block, no conflict for their calendar.
    assert_eq!(blocks[0].resource_id, s.a.id);
}

/// #172 point 1 with the real `wl-hr`: a manager records Dr A's same-day sick
/// leave at 10:30 (`sick_leave::record`, which emits `LeaveApproved` itself);
/// the bookings from 10:30 on are flagged and none is cancelled, the one before
/// is not. The clock is moved to the day, as `record` only takes today.
#[renox::test]
async fn the_sick_leave_recorded_in_hr_flags_the_bookings_of_the_day() {
    let t = boot_hr().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.service(&doctor).await;
    let employee = doctor.employee_id.expect("an employee");
    let day = dubai_date(now()) + renox::chrono::TimeDelta::days(3);
    let day_text = day.to_string();
    let before = t
        .booking(&service, &[&doctor], dubai(&day_text, 10, 0))
        .await;
    let at_11 = t
        .booking(&service, &[&doctor], dubai(&day_text, 11, 0))
        .await;
    let at_15 = t
        .booking(&service, &[&doctor], dubai(&day_text, 15, 0))
        .await;
    let manager = t
        .user("Hanan", &[wl_hr::permissions::LEAVE_APPROVE])
        .await;

    t.app
        .travel((dubai(&day_text, 10, 30) - now()).to_std().expect("ahead"));
    t.app
        .at_travelled_time(async {
            wl_hr::sick_leave::record(t.state(), manager.id, employee, day, day)
                .await
                .expect("the sick leave is recorded")
        })
        .await;

    for flagged in [&at_11, &at_15] {
        let row = t.reload(flagged.id).await;
        assert!(row.conflict_block, "{} is flagged", row.number);
        assert_eq!(row.status, BookingStatus::Booked, "nothing is cancelled");
    }
    assert!(!t.reload(before.id).await.has_conflict(), "it ended at 10:30");
    assert!(t.events.cancelled.all().is_empty());
    let raised = t.events.conflicts.all();
    assert_eq!(raised.len(), 1);
    assert_eq!(raised[0].resource, doctor.id);
    assert_eq!(raised[0].cause, ConflictCause::Block);
    assert_eq!(raised[0].earliest_start, dubai(&day_text, 11, 0));
    let listed = conflict::conflicts(t.db(), Some(doctor.id), Some(conflict::day_bounds(day)))
        .await
        .unwrap();
    assert_eq!(
        listed.iter().map(|b| b.id).collect::<Vec<_>>(),
        ids(&[&at_11, &at_15])
    );
    let block = AvailabilityBlock::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(block.source, "leave");
    assert_eq!(block.reason, "sick");
    let late = block.starts_at - dubai(&day_text, 10, 30);
    assert!(
        late >= renox::chrono::TimeDelta::zero() && late < renox::chrono::TimeDelta::minutes(5),
        "the block starts when the leave is recorded"
    );
}

/// "The rest of their day": sick leave recorded at 10:30 blocks Dr A from
/// 10:30 to the end of the day. The booking that ended at 10:30, the one
/// already checked in, and tomorrow's are not flagged.
#[renox::test]
async fn the_rest_of_the_day_starts_when_the_leave_is_recorded() {
    let t = boot().await;
    let s = scene(&t).await;
    let before = t
        .book_for(PATIENT, &s.service, &[&s.a], dubai(DAY, 10, 0))
        .await;
    let arrived = t
        .book_for(OTHER_PATIENT, &s.service, &[&s.a], dubai(DAY, 10, 30))
        .await;
    service::check_in(t.state(), arrived.id, Actor::staff(1), dubai(DAY, 10, 25))
        .await
        .expect("checked in");
    let tomorrow = t
        .book_for(PATIENT, &s.service, &[&s.a], dubai(NEXT, 11, 0))
        .await;

    let flagged =
        conflict::leave_approved(t.state(), &leave(502, "sick", DAY, DAY), dubai(DAY, 10, 30))
            .await
            .unwrap();

    let mut got: Vec<i64> = flagged.iter().map(|b| b.id).collect();
    got.sort_unstable();
    assert_eq!(got, ids(&[&s.a11, &s.a12, &s.a15]));
    for untouched in [&before, &arrived, &tomorrow] {
        assert!(!t.reload(untouched.id).await.has_conflict());
    }
    let block = AvailabilityBlock::query()
        .order_by(AvailabilityBlock::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.starts_at, dubai(DAY, 10, 30));
    assert_eq!(block.ends_at, dubai(NEXT, 0, 0));
}

/// The same leave delivered again flags nothing twice: one block, one event,
/// one open conflict per booking.
#[renox::test]
async fn the_same_leave_twice_flags_each_booking_once() {
    let t = boot().await;
    let s = scene(&t).await;

    t.state().emit(leave(503, "sick", DAY, DAY)).await.unwrap();
    t.state().emit(leave(503, "sick", DAY, DAY)).await.unwrap();

    assert_eq!(t.events.conflicts.all().len(), 1);
    assert_eq!(
        AvailabilityBlock::query().get(t.db()).await.unwrap().len(),
        1
    );
    for booking in [&s.a11, &s.a12, &s.a15] {
        assert_eq!(open_conflicts(&t, booking.id).await, 1);
    }
}

/// Staff bulk-reassign 11:00 and 12:00 to Dr B: both conflicts clear, Dr B
/// holds them, and each party gets one message with *accept*, *pick another
/// time* and *cancel* links, naming Dr B and not the reason. The 15:00 booking
/// stays conflicted.
#[renox::test]
async fn a_bulk_reassign_clears_the_conflicts_and_tells_each_party_once() {
    let t = boot().await;
    let s = scene(&t).await;
    let desk = t.front_desk().await;
    t.state().emit(leave(504, "sick", DAY, DAY)).await.unwrap();

    t.app.acting_as(&desk);
    let first = s.a11.id.to_string();
    let second = s.a12.id.to_string();
    let to = s.b.id.to_string();
    t.app
        .htmx()
        .post(
            "/scheduling/conflicts/resolve",
            &[
                ("action", "reassign"),
                ("to", to.as_str()),
                ("bookings", first.as_str()),
                ("bookings", second.as_str()),
            ],
        )
        .await
        .assert_status(303);

    for booking in [&s.a11, &s.a12] {
        let moved = t.reload(booking.id).await;
        assert!(!moved.has_conflict());
        assert_eq!(moved.status, BookingStatus::Booked);
        assert_eq!(moved.starts_at, booking.starts_at);
        assert_eq!(moved.resource_ids(t.db()).await.unwrap(), [s.b.id]);
        assert_eq!(open_conflicts(&t, booking.id).await, 0);
    }
    assert!(t.reload(s.a15.id).await.conflict_block);
    assert_eq!(t.events.reassigned.all().len(), 2);

    let sent = t.messages.sent();
    assert_eq!(sent.len(), 2, "one message per party");
    for (request, booking, party) in [
        (&sent[0], &s.a11, PATIENT),
        (&sent[1], &s.a12, OTHER_PATIENT),
    ] {
        assert_eq!(request.type_key, "scheduling.bookings_changed");
        assert_eq!(request.recipients, [NotifyRecipient::Party { id: party }]);
        let body = Messages::body(request);
        assert!(body.contains(&booking.number), "{body}");
        assert!(body.contains("Dr B"), "{body}");
        assert!(!body.contains("Dr A"), "{body}");
        assert!(body.contains(&format!("/scheduling/links/{}/accept", booking.id)));
        assert!(body.contains(&format!("/scheduling/links/{}/reschedule", booking.id)));
        assert!(body.contains(&format!(
            "/scheduling/links/{}/cancel/resource_unavailable",
            booking.id
        )));
        assert!(!body.to_lowercase().contains("sick"), "{body}");
        assert!(!body.to_lowercase().contains("leave"), "{body}");
    }
}

/// A party with two affected bookings gets one message listing both.
#[renox::test]
async fn a_party_with_two_bookings_gets_one_message() {
    let t = boot().await;
    let s = scene(&t).await;
    t.state().emit(leave(505, "sick", DAY, DAY)).await.unwrap();

    let result = conflict::bulk_resolve(
        t.state(),
        &[s.a11.id, s.a15.id],
        &BulkAction::Reassign { to: Some(s.b.id) },
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(result.done, [s.a11.id, s.a15.id]);
    assert!(result.failed.is_empty());
    let sent = t.messages.sent();
    assert_eq!(sent.len(), 1);
    let body = Messages::body(&sent[0]);
    assert!(body.contains(&s.a11.number), "{body}");
    assert!(body.contains(&s.a15.number), "{body}");
}

/// Without a named resource each booking goes to the first that can take it.
#[renox::test]
async fn a_bulk_reassign_without_a_target_picks_one_that_is_free() {
    let t = boot().await;
    let s = scene(&t).await;
    t.state().emit(leave(506, "sick", DAY, DAY)).await.unwrap();

    let result = conflict::bulk_resolve(
        t.state(),
        &[s.a11.id],
        &BulkAction::Reassign { to: None },
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(result.done, [s.a11.id]);
    let moved = t.reload(s.a11.id).await;
    assert_eq!(moved.resource_ids(t.db()).await.unwrap(), [s.b.id]);
    assert!(!moved.has_conflict());
}

/// Staff bulk-cancel 15:00 with `resource_unavailable`: it is cancelled with
/// initiated by `business` and its AED 200 deposit refunded in full.
#[renox::test]
async fn a_bulk_cancel_is_business_initiated_and_refunds_the_deposit() {
    let t = boot().await;
    let s = scene(&t).await;
    let desk = t.front_desk().await;
    t.state().emit(leave(507, "sick", DAY, DAY)).await.unwrap();

    t.app.acting_as(&desk);
    let id = s.a15.id.to_string();
    t.app
        .htmx()
        .post(
            "/scheduling/conflicts/resolve",
            &[
                ("action", "cancel"),
                ("reason_code", "resource_unavailable"),
                ("bookings", id.as_str()),
            ],
        )
        .await
        .assert_status(303);

    let cancelled = t.reload(s.a15.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_code.as_deref(),
        Some("resource_unavailable")
    );
    assert_eq!(cancelled.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(cancelled.deposit_outcome.as_deref(), Some("refunded"));
    assert_eq!(t.refunds.amounts(), [20_000]);
    assert!(!cancelled.has_conflict());
    assert_eq!(open_conflicts(&t, s.a15.id).await, 0);
    let events = t.events.cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].initiated_by, Initiator::Business);
    // The other two stay conflicted and untouched.
    assert!(t.reload(s.a11.id).await.conflict_block);
    assert!(t.reload(s.a12.id).await.conflict_block);
}

/// Dr B became busy at 12:00 meanwhile: 11:00 succeeds, 12:00 is reported
/// failed, stays with Dr A and stays conflicted.
#[renox::test]
async fn a_booking_the_new_resource_cannot_take_fails_alone() {
    let t = boot().await;
    let s = scene(&t).await;
    t.book_for(PATIENT, &s.service, &[&s.b], dubai(DAY, 12, 0))
        .await;
    t.state().emit(leave(508, "sick", DAY, DAY)).await.unwrap();

    let result = conflict::bulk_resolve(
        t.state(),
        &[s.a11.id, s.a12.id],
        &BulkAction::Reassign { to: Some(s.b.id) },
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(result.done, [s.a11.id]);
    assert_eq!(result.failed.len(), 1);
    assert_eq!(result.failed[0].booking, s.a12.id);
    assert!(
        result.failed[0].reason.contains("taken"),
        "{:?}",
        result.failed
    );
    assert!(!t.reload(s.a11.id).await.has_conflict());
    let stays = t.reload(s.a12.id).await;
    assert!(stays.conflict_block);
    assert_eq!(stays.resource_ids(t.db()).await.unwrap(), [s.a.id]);
    // Only the party of the booking that moved was told.
    assert_eq!(t.messages.sent().len(), 1);
}

/// Only conflicted bookings are resolved in bulk; the others are reported.
#[renox::test]
async fn a_booking_without_a_conflict_is_reported_not_touched() {
    let t = boot().await;
    let s = scene(&t).await;

    let result = conflict::bulk_resolve(
        t.state(),
        &[s.a11.id],
        &BulkAction::Cancel {
            reason_code: "resource_unavailable".to_owned(),
        },
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();

    assert!(result.done.is_empty());
    assert_eq!(result.failed.len(), 1);
    assert_eq!(t.reload(s.a11.id).await.status, BookingStatus::Booked);
    assert!(t.refunds.amounts().is_empty());
}

/// Dr A's days are open 09:00 to 10:30 and 14:00 to 16:00, in steps of 30
/// minutes. With the 15:00 booking conflicted, a bulk reschedule sends the
/// party the next three free times of Dr A (09:00, 09:30 and 10:00 tomorrow),
/// moves nothing and holds nothing; the booking stays conflicted until the
/// party picks 09:30 from the link, which reschedules it and clears the
/// conflict.
#[renox::test]
async fn a_bulk_reschedule_offers_three_times_and_the_pick_clears_the_conflict() {
    let t = boot_with(WlScheduling::new().horizon_days(5000).slot_step_minutes(30)).await;
    let mut a = t
        .resource("Dr A", wl_scheduling::ResourceType::Person)
        .await;
    a.employee_id = Some(EMPLOYEE);
    a.save(t.db()).await.unwrap();
    for weekday in 1..=7 {
        for (from, to) in [(540, 630), (840, 960)] {
            AvailabilityRule::new(a.id, weekday, from, to)
                .save(t.db())
                .await
                .unwrap();
        }
    }
    let service = t
        .service_with("consultation", &[&a], |service| {
            service.deposit_amount = 20_000;
        })
        .await;
    let booking = t
        .book_for(PATIENT, &service, &[&a], dubai(DAY, 15, 0))
        .await;
    t.state().emit(leave(509, "sick", DAY, DAY)).await.unwrap();
    assert!(t.reload(booking.id).await.conflict_block);

    let result = conflict::bulk_resolve(
        t.state(),
        &[booking.id],
        &BulkAction::Reschedule,
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(result.done, [booking.id]);
    let sent = t.messages.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].type_key, "scheduling.reschedule_offer");
    let body = Messages::body(&sent[0]);
    for (hour, minute) in [(9, 0), (9, 30), (10, 0)] {
        let shown = wl_kernel::documents::dubai_text(dubai(NEXT, hour, minute));
        assert!(body.contains(&shown), "{shown} in {body}");
    }
    assert!(!body.to_lowercase().contains("sick"), "{body}");
    let same = t.reload(booking.id).await;
    assert_eq!(same.starts_at, dubai(DAY, 15, 0), "nothing is moved");
    assert!(same.conflict_block, "the booking stays conflicted");
    assert!(t.events.rescheduled.all().is_empty());
    assert_eq!(open_conflicts(&t, booking.id).await, 1);

    // The party picks 09:30 from the link of the message.
    let url = sent[0].data["reschedule_url"].as_str().expect("a link");
    t.app
        .htmx()
        .post(
            &t.path_of(url),
            &[("starts_at", local_text(dubai(NEXT, 9, 30)).as_str())],
        )
        .await
        .assert_ok()
        .assert_see("is moved");
    let moved = t.reload(booking.id).await;
    assert_eq!(moved.starts_at, dubai(NEXT, 9, 30));
    assert_eq!(moved.status, BookingStatus::Booked);
    assert!(!moved.has_conflict());
    assert_eq!(open_conflicts(&t, booking.id).await, 0);
    assert_eq!(t.events.rescheduled.all().len(), 1);
}

/// The party cancels from the conflict message, 15:00 booking, AED 200
/// deposit with a forfeit window of 24 hours, at 10:45 on the day: it is
/// cancelled by the business with the code `resource_unavailable` and the
/// AED 200 is refunded.
#[renox::test]
async fn cancelling_from_the_message_is_business_initiated_and_refunds() {
    let t = boot().await;
    let s = scene(&t).await;
    t.state().emit(leave(510, "sick", DAY, DAY)).await.unwrap();

    service::cancel(
        t.state(),
        s.a15.id,
        CancelRequest::from_change_message("resource_unavailable"),
        Actor::party(None),
        dubai(DAY, 10, 45),
    )
    .await
    .expect("the party cancels from the message");

    let cancelled = t.reload(s.a15.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(
        cancelled.cancel_code.as_deref(),
        Some("resource_unavailable")
    );
    assert_eq!(t.refunds.amounts(), [20_000]);
    assert_eq!(open_conflicts(&t, s.a15.id).await, 0);
}

/// The same through the link of the message.
#[renox::test]
async fn the_cancel_link_of_the_message_refunds_the_deposit() {
    let t = boot().await;
    let s = scene(&t).await;
    t.state().emit(leave(511, "sick", DAY, DAY)).await.unwrap();
    let url = links::change_cancel_url(t.state(), s.a15.id, "resource_unavailable", HOUR).unwrap();

    t.app
        .htmx()
        .post(&t.path_of(&url), &[("go", "1")])
        .await
        .assert_ok();

    let cancelled = t.reload(s.a15.id).await;
    assert_eq!(cancelled.status, BookingStatus::Cancelled);
    assert_eq!(cancelled.cancel_initiated_by.as_deref(), Some("business"));
    assert_eq!(t.refunds.amounts(), [20_000]);
}

/// The leave is cancelled: the block goes and the conflicts it caused clear;
/// the bookings stay as they were.
#[renox::test]
async fn a_cancelled_leave_clears_the_conflicts_it_caused() {
    let t = boot().await;
    let s = scene(&t).await;
    t.state()
        .emit(leave(512, "annual", DAY, DAY))
        .await
        .unwrap();
    assert!(t.reload(s.a11.id).await.conflict_block);

    t.state()
        .emit(LeaveCancelled {
            employee: EmployeeId(EMPLOYEE),
            request: 512,
            leave_type: "annual".to_owned(),
            from: date(DAY),
            to: date(DAY),
        })
        .await
        .unwrap();

    for booking in [&s.a11, &s.a12, &s.a15] {
        let back = t.reload(booking.id).await;
        assert!(!back.has_conflict());
        assert_eq!(back.status, BookingStatus::Booked);
        assert_eq!(open_conflicts(&t, booking.id).await, 0);
    }
    assert!(
        AvailabilityBlock::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    // The time can be booked again.
    t.book_for(PATIENT, &s.service, &[&s.a], dubai(DAY, 16, 0))
        .await;
}

/// A manual block of reason `sick` without an end runs to the end of the day;
/// it flags the bookings it overlaps. A booking under two blocks stays
/// conflicted when one is removed and clears when the other goes.
#[renox::test]
async fn manual_blocks_flag_and_each_clears_only_its_own_conflicts() {
    let t = boot().await;
    let s = scene(&t).await;
    let desk = t.front_desk().await;
    t.app.acting_as(&desk);
    let resource = s.a.id.to_string();

    t.app
        .htmx()
        .post(
            "/scheduling/blocks",
            &[
                ("resource", resource.as_str()),
                ("reason", "sick"),
                ("starts_at", local_text(dubai(DAY, 10, 30)).as_str()),
            ],
        )
        .await
        .assert_status(303);

    let first = AvailabilityBlock::query()
        .order_by(AvailabilityBlock::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.source, "manual");
    assert_eq!(first.starts_at, dubai(DAY, 10, 30));
    assert_eq!(first.ends_at, dubai(NEXT, 0, 0), "the rest of the day");
    for booking in [&s.a11, &s.a12, &s.a15] {
        assert!(t.reload(booking.id).await.conflict_block);
    }
    assert_eq!(t.events.conflicts.all().len(), 1);

    // A second block over 12:00.
    t.app
        .htmx()
        .post(
            "/scheduling/blocks",
            &[
                ("resource", resource.as_str()),
                ("reason", "other"),
                ("starts_at", local_text(dubai(DAY, 11, 45)).as_str()),
                ("ends_at", local_text(dubai(DAY, 12, 30)).as_str()),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(open_conflicts(&t, s.a12.id).await, 2);
    assert_eq!(t.events.conflicts.all().len(), 2);

    t.app
        .htmx()
        .post(
            &format!("/scheduling/blocks/{}/remove", first.id),
            &[("go", "1")],
        )
        .await
        .assert_status(303);
    assert!(!t.reload(s.a11.id).await.conflict_block);
    assert!(!t.reload(s.a15.id).await.conflict_block);
    let still = t.reload(s.a12.id).await;
    assert!(still.conflict_block, "the second block still conflicts");
    assert_eq!(open_conflicts(&t, s.a12.id).await, 1);

    let second = AvailabilityBlock::query()
        .order_by(AvailabilityBlock::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .htmx()
        .post(
            &format!("/scheduling/blocks/{}/remove", second.id),
            &[("go", "1")],
        )
        .await
        .assert_status(303);
    assert!(!t.reload(s.a12.id).await.has_conflict());
}

/// A block raised by sick leave is cleared by `reassign`, the conflict row
/// ends, and the party's booking keeps its state.
#[renox::test]
async fn a_reassign_clears_a_conflict_a_real_block_raised() {
    let t = boot().await;
    let s = scene(&t).await;
    let desk = t.front_desk().await;
    t.state().emit(leave(513, "sick", DAY, DAY)).await.unwrap();
    assert!(t.reload(s.a11.id).await.conflict_block);

    t.app.acting_as(&desk);
    let from = s.a.id.to_string();
    let to = s.b.id.to_string();
    t.app
        .htmx()
        .post(
            &format!("/scheduling/bookings/{}/reassign", s.a11.id),
            &[
                ("from", from.as_str()),
                ("to", to.as_str()),
                ("reason_code", "resource_unavailable"),
            ],
        )
        .await
        .assert_status(303);

    let moved = t.reload(s.a11.id).await;
    assert!(!moved.has_conflict());
    assert_eq!(moved.status, BookingStatus::Booked);
    assert_eq!(open_conflicts(&t, s.a11.id).await, 0);
    assert!(t.reload(s.a12.id).await.conflict_block);
}

/// Blocks another module supplies are added once and flag the bookings they
/// overlap.
#[renox::test]
async fn blocks_from_an_availability_source_are_added_once() {
    let t = boot().await;
    let s = scene(&t).await;
    t.sources.add(SourceBlock {
        starts_at: dubai(DAY, 10, 0),
        ends_at: dubai(DAY, 11, 30),
        reason: "roster".to_owned(),
        source: "roster".to_owned(),
        reference: "roster-9".to_owned(),
    });

    let flagged = conflict::pull_blocks(
        t.state(),
        s.a.id,
        dubai(DAY, 0, 0),
        dubai(NEXT, 0, 0),
        now(),
    )
    .await
    .unwrap();
    let again = conflict::pull_blocks(
        t.state(),
        s.a.id,
        dubai(DAY, 0, 0),
        dubai(NEXT, 0, 0),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(flagged.iter().map(|b| b.id).collect::<Vec<_>>(), [s.a11.id]);
    assert!(again.is_empty());
    assert_eq!(
        AvailabilityBlock::query().get(t.db()).await.unwrap().len(),
        1
    );
}

/// The conflict list shows the conflicted bookings; a user who may only look
/// cannot act, and a booking nobody blocked is not listed.
#[renox::test]
async fn the_list_shows_conflicts_and_resolving_needs_the_manage_permission() {
    let t = boot().await;
    let s = scene(&t).await;
    let desk = t.front_desk().await;
    let viewer = t.viewer().await;
    t.app.acting_as(&desk);
    t.app
        .get("/scheduling/conflicts")
        .await
        .assert_ok()
        .assert_see("No bookings in conflict");

    t.state().emit(leave(514, "sick", DAY, DAY)).await.unwrap();
    t.app
        .get("/scheduling/conflicts")
        .await
        .assert_ok()
        .assert_see(&s.a11.number)
        .assert_see(&s.a15.number);
    t.app
        .get(&format!("/scheduling/conflicts?resource={}", s.b.id))
        .await
        .assert_ok()
        .assert_see("No bookings in conflict");

    t.app.acting_as(&viewer);
    t.app.get("/scheduling/conflicts").await.assert_ok();
    let id = s.a11.id.to_string();
    t.app
        .htmx()
        .post(
            "/scheduling/conflicts/resolve",
            &[("action", "cancel"), ("bookings", id.as_str())],
        )
        .await
        .assert_forbidden();
    assert_eq!(t.reload(s.a11.id).await.status, BookingStatus::Booked);
    t.app
        .htmx()
        .post(
            "/scheduling/blocks",
            &[("resource", "1"), ("reason", "sick")],
        )
        .await
        .assert_forbidden();
}
