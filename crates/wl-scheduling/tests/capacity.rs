//! The last seat goes to exactly one booker (#176, BL-SCH-09): the capacity
//! check and the insert run in one transaction that locks the resources,
//! holds count while they last, and the loser is shown the nearest slots.

mod support;

use std::sync::Arc;

use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::tokio::sync::Barrier;
use wl_kernel::PartyId;
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::waiting::{self, NewEntry, WaitingStatus};
use wl_scheduling::{Booking, BookingStatus, Refusal, Resource, ResourceType, ServiceType, WaitingEntry};

use support::{Booked, PATIENT, boot, in_hours, now};

/// A studio of five seats and the 30-minute class that gives five seats.
async fn class(t: &Booked) -> (Resource, ServiceType) {
    let mut studio = Resource::new("Studio", ResourceType::Room).capacity(5);
    studio.save(t.db()).await.unwrap();
    t.open_all_week(&studio).await;
    let class = t
        .service_with("class", &[&studio], |s| {
            s.capacity = 5;
            s.required_types = "room".to_owned();
        })
        .await;
    (studio, class)
}

/// The bookings that hold a seat from `start`.
async fn seats_at(t: &Booked, start: renox::db::DateTime) -> u64 {
    Booking::query()
        .where_(Booking::STARTS_AT.eq(start))
        .where_(Booking::STATUS.ne(BookingStatus::Cancelled))
        .count(t.db())
        .await
        .unwrap()
}

/// `callers` book the same slot behind a barrier, so that they reach the
/// capacity check together. The answers, in no order.
async fn together(
    t: &Booked,
    class: &ServiceType,
    studio: &Resource,
    start: renox::db::DateTime,
    callers: usize,
) -> Vec<Result<Booking, Refusal>> {
    let barrier = Arc::new(Barrier::new(callers));
    let mut tasks = Vec::new();
    for n in 0..callers {
        let (state, barrier) = (t.state().clone(), Arc::clone(&barrier));
        let (service_id, resource_id) = (class.id, studio.id);
        tasks.push(renox::tokio::spawn(async move {
            barrier.wait().await;
            service::book(
                &state,
                NewBooking::new(PartyId(500 + n as i64), service_id, &[resource_id], start),
                Actor::staff(1),
                now(),
            )
            .await
        }));
    }
    let mut answers = Vec::new();
    for task in tasks {
        answers.push(task.await.expect("the booking task ran"));
    }
    answers
}

/// Four of five seats are taken; two bookers reach the last seat together:
/// one wins, the other is told it was just taken and shown the nearest free
/// slots and the waiting list. Fifty times; every run ends with five seats.
#[renox::test]
async fn two_bookers_for_the_last_seat_one_wins() {
    let t = boot().await;
    let (studio, class) = class(&t).await;
    let base = in_hours(48);
    for run in 0..50 {
        let start = base + TimeDelta::hours(run);
        for seat in 0..4 {
            t.book_for(PartyId(100 + seat), &class, &[&studio], start)
                .await;
        }
        let answers = together(&t, &class, &studio, start, 2).await;
        let won = answers.iter().filter(|a| a.is_ok()).count();
        assert_eq!(won, 1, "run {run}: exactly one wins");
        assert_eq!(seats_at(&t, start).await, 5, "run {run}: five seats");
        let lost = answers
            .into_iter()
            .find_map(Result::err)
            .expect("one lost");
        let Refusal::Taken {
            nearest,
            waiting_list,
        } = lost
        else {
            panic!("run {run}: the loser is told the slot was just taken");
        };
        assert!(waiting_list, "run {run}: the waiting list is offered");
        assert_eq!(nearest.len(), 3, "run {run}: three nearest slots");
        // The first slot on the 15-minute grid after the five bookings end.
        assert!(
            nearest[0].start >= start + TimeDelta::minutes(30)
                && nearest[0].start < start + TimeDelta::minutes(45),
            "run {run}: {:?}",
            nearest[0].start
        );
        assert!(nearest.windows(2).all(|w| w[0].start < w[1].start), "run {run}");
        assert!(nearest.iter().all(|s| s.resource_ids == [studio.id]), "run {run}");
    }
}

/// Ten bookers for the last seat: five seats in all, nine told it was taken.
#[renox::test]
async fn ten_bookers_for_the_last_seat_one_wins() {
    let t = boot().await;
    let (studio, class) = class(&t).await;
    let start = in_hours(48);
    for seat in 0..4 {
        t.book_for(PartyId(100 + seat), &class, &[&studio], start)
            .await;
    }
    let answers = together(&t, &class, &studio, start, 10).await;
    let won = answers.iter().filter(|a| a.is_ok()).count();
    let taken = answers.iter().filter(|a| matches!(a, Err(Refusal::Taken { .. }))).count();
    assert_eq!((won, taken), (1, 9));
    assert_eq!(seats_at(&t, start).await, 5);
}

/// An empty slot of five seats takes five of ten bookers.
#[renox::test]
async fn ten_bookers_for_five_empty_seats_five_win() {
    let t = boot().await;
    let (studio, class) = class(&t).await;
    let start = in_hours(48);
    let answers = together(&t, &class, &studio, start, 10).await;
    let won = answers.iter().filter(|a| a.is_ok()).count();
    assert_eq!(won, 5);
    assert_eq!(seats_at(&t, start).await, 5);
}

/// A request holds the slot for its hold time (15 minutes). A second `book`
/// is refused at 10 minutes and succeeds at 16, and the request, accepted
/// after that, finds the slot taken.
#[renox::test]
async fn a_request_holds_the_slot_only_for_its_hold_time() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("by_request", &[&doctor], |s| s.requires_acceptance = true)
        .await;
    let start = in_hours(48);
    let t0 = now();
    let request = service::request(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start).source("portal"),
        Actor::party(None),
        t0,
    )
    .await
    .unwrap();
    let book = |minutes: i64| {
        service::book(
            t.state(),
            NewBooking::new(support::OTHER_PATIENT, service.id, &[doctor.id], start),
            Actor::staff(1),
            t0 + TimeDelta::minutes(minutes),
        )
    };

    let held = book(10).await;
    assert!(matches!(held, Err(Refusal::Taken { .. })), "{held:?}");
    // Hold made at t0 lasts to t0 + 15: at 15 minutes it is over.
    let lapsed = book(16).await.expect("the lapsed hold no longer protects the slot");
    assert_eq!(lapsed.status, BookingStatus::Booked);

    let late = service::accept(t.state(), request.id, Actor::staff(1), t0 + TimeDelta::minutes(16)).await;
    assert!(matches!(late, Err(Refusal::Taken { .. })), "{late:?}");
    assert_eq!(t.reload(request.id).await.status, BookingStatus::Requested);
}

/// The lapsed hold is not cleaned up by any job: the clock decides. A request
/// whose hold ended a minute ago does not stop a booking.
#[renox::test]
async fn a_hold_that_ended_a_minute_ago_does_not_stop_a_booking() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("by_request", &[&doctor], |s| s.requires_acceptance = true)
        .await;
    let start = in_hours(48);
    let t0 = now();
    let request = service::request(
        t.state(),
        NewBooking::new(PATIENT, service.id, &[doctor.id], start),
        Actor::party(None),
        t0,
    )
    .await
    .unwrap();
    assert_eq!(
        request.hold_until,
        Some(t0 + TimeDelta::minutes(15)),
        "the hold is the request hold time"
    );
    let booked = service::book(
        t.state(),
        NewBooking::new(support::OTHER_PATIENT, service.id, &[doctor.id], start),
        Actor::staff(1),
        t0 + TimeDelta::minutes(16),
    )
    .await;
    assert!(booked.is_ok(), "{booked:?}");
    assert_eq!(t.reload(request.id).await.status, BookingStatus::Requested);
}

/// An unpaid self-booking holds the slot until its payment link expires; a
/// paid one holds it for good.
#[renox::test]
async fn an_unpaid_booking_holds_the_slot_until_the_link_expires() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    let t0 = now();
    let unpaid = t.booking(&service, &[&doctor], start).await;
    Booking::query()
        .where_(Booking::ID.eq(unpaid.id))
        .update(
            t.db(),
            &[
                ("payment_pending", &true),
                ("hold_until", &Some(t0 + TimeDelta::minutes(15))),
            ],
        )
        .await
        .unwrap();
    let again = |minutes: i64| {
        service::book(
            t.state(),
            NewBooking::new(support::OTHER_PATIENT, service.id, &[doctor.id], start),
            Actor::staff(1),
            t0 + TimeDelta::minutes(minutes),
        )
    };
    assert!(matches!(again(14).await, Err(Refusal::Taken { .. })));
    assert!(again(16).await.is_ok(), "the payment link has expired");
}

/// A slot offered to a waiting-list entry is held for the offer hold time
/// (30 minutes): a staff `book` is refused until it lapses, and the clock
/// decides, not the lapse job.
#[renox::test]
async fn an_offered_slot_is_held_until_the_offer_lapses() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    let end = start + TimeDelta::minutes(30);
    let t0 = now();
    let entry = waiting::join(t.state(), NewEntry::new(PATIENT, service.id))
        .await
        .unwrap();
    let offered = waiting::offer(t.state(), entry.id, doctor.id, start, end, Actor::staff(1), t0)
        .await
        .unwrap();
    assert_eq!(offered.status, WaitingStatus::Offered);
    assert_eq!(offered.offer_hold_until, Some(t0 + TimeDelta::minutes(30)));

    let book = |minutes: i64| {
        service::book(
            t.state(),
            NewBooking::new(support::OTHER_PATIENT, service.id, &[doctor.id], start),
            Actor::staff(1),
            t0 + TimeDelta::minutes(minutes),
        )
    };
    let held = book(5).await;
    assert!(matches!(held, Err(Refusal::Taken { .. })), "{held:?}");
    // Not lapsed by any job, and still free once the hold time is over.
    assert_eq!(
        WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap().status,
        WaitingStatus::Offered
    );
    assert!(book(31).await.is_ok());
}

/// The entry that was offered the slot takes it: its own hold does not stop
/// its own booking, and a late `accept` is refused.
#[renox::test]
async fn the_offered_entry_takes_its_own_slot() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    let end = start + TimeDelta::minutes(30);
    let t0 = now();
    let entry = waiting::join(t.state(), NewEntry::new(PATIENT, service.id))
        .await
        .unwrap();
    waiting::offer(t.state(), entry.id, doctor.id, start, end, Actor::staff(1), t0)
        .await
        .unwrap();

    let late = waiting::accept(t.state(), entry.id, Actor::party(None), t0 + TimeDelta::minutes(31)).await;
    assert!(late.is_err());
    let booking = waiting::accept(t.state(), entry.id, Actor::party(None), t0 + TimeDelta::minutes(10))
        .await
        .unwrap();
    assert_eq!(booking.status, BookingStatus::Booked);
    assert_eq!(booking.starts_at, start);
    assert_eq!(booking.party_id, PATIENT.0);
    let done = WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap();
    assert_eq!(done.status, WaitingStatus::Booked);
    assert_eq!(done.booking_id, Some(booking.id));
    assert_eq!(done.offered_resource_id, None);
}

/// A lapsed offer waits again, and the hold cannot be lapsed early; an entry
/// expires only after its day; withdrawing frees the held slot.
#[renox::test]
async fn offers_lapse_entries_expire_and_withdrawing_frees_the_slot() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t.service(&doctor).await;
    let start = in_hours(48);
    let end = start + TimeDelta::minutes(30);
    let t0 = now();
    let entry = waiting::join(t.state(), NewEntry::new(PATIENT, service.id))
        .await
        .unwrap();
    waiting::offer(t.state(), entry.id, doctor.id, start, end, Actor::staff(1), t0)
        .await
        .unwrap();
    assert!(waiting::lapse(t.state(), entry.id, t0 + TimeDelta::minutes(29)).await.is_err());
    let back = waiting::lapse(t.state(), entry.id, t0 + TimeDelta::minutes(30))
        .await
        .unwrap();
    assert_eq!(back.status, WaitingStatus::Waiting);
    assert_eq!(back.offered_from, None);

    // An offer that does not fit what the entry waits for is refused.
    let picky = waiting::join(
        t.state(),
        NewEntry::new(PATIENT, service.id).between(start + TimeDelta::hours(5), start + TimeDelta::hours(8)),
    )
    .await
    .unwrap();
    assert!(
        waiting::offer(t.state(), picky.id, doctor.id, start, end, Actor::staff(1), t0)
            .await
            .is_err()
    );

    // Withdrawing an offered entry frees its slot at once.
    let offered = waiting::join(t.state(), NewEntry::new(PATIENT, service.id))
        .await
        .unwrap();
    waiting::offer(t.state(), offered.id, doctor.id, start, end, Actor::staff(1), now())
        .await
        .unwrap();
    assert!(service::book(
        t.state(),
        NewBooking::new(support::OTHER_PATIENT, service.id, &[doctor.id], start),
        Actor::staff(1),
        now()
    )
    .await
    .is_err());
    waiting::withdraw(t.state(), offered.id, Actor::staff(1)).await.unwrap();
    assert!(service::book(
        t.state(),
        NewBooking::new(support::OTHER_PATIENT, service.id, &[doctor.id], start),
        Actor::staff(1),
        now()
    )
    .await
    .is_ok());

    // Expiry needs the last day to have passed.
    let mut dated = NewEntry::new(PATIENT, service.id);
    dated.expires_on = Some(wl_kernel::documents::today());
    let dated = waiting::join(t.state(), dated).await.unwrap();
    assert!(waiting::expire(t.state(), dated.id, wl_kernel::documents::today()).await.is_err());
    let tomorrow = wl_kernel::documents::today().succ_opt().unwrap();
    let gone = waiting::expire(t.state(), dated.id, tomorrow).await.unwrap();
    assert_eq!(gone.status, WaitingStatus::Expired);
}

/// With several resources the booking takes all of them or none: a busy room
/// leaves the practitioner free.
#[renox::test]
async fn a_busy_resource_leaves_the_others_free() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let room = t.resource("Room 1", ResourceType::Room).await;
    t.open_all_week(&room).await;
    let procedure = t
        .service_with("procedure", &[&doctor, &room], |s| {
            s.required_types = "person,room".to_owned();
        })
        .await;
    let just_room = t
        .service_with("room_only", &[&room], |s| s.required_types = "room".to_owned())
        .await;
    let start = in_hours(48);
    t.booking(&just_room, &[&room], start).await;
    let refused = service::book(
        t.state(),
        NewBooking::new(PATIENT, procedure.id, &[doctor.id, room.id], start),
        Actor::staff(1),
        now(),
    )
    .await;
    assert!(matches!(refused, Err(Refusal::Taken { .. })), "{refused:?}");
    // The doctor was not taken by the failed booking.
    let doctor_only = t.service_with("check_up", &[&doctor], |_| {}).await;
    t.booking(&doctor_only, &[&doctor], start).await;
}

/// A portal user books without naming a resource: the first free one is
/// taken; when none is free the answer is 409. The loser may join the
/// waiting list and withdraw from it; a stranger's party is a 404.
#[renox::test]
async fn the_portal_books_the_first_free_resource_and_joins_the_waiting_list() {
    let t = boot().await;
    let first = t.doctor("Dr A").await;
    let second = t.doctor("Dr B").await;
    let service = t.service_with("consultation", &[&first, &second], |_| {}).await;
    let sara = t.user("Sara", &[]).await;
    t.portal.allow(sara.id, PATIENT);
    let start = in_hours(48);
    let at = wl_scheduling::pages::local_text(start);
    let (party, svc) = (PATIENT.0.to_string(), service.id.to_string());
    let form = [
        ("party", party.as_str()),
        ("service", svc.as_str()),
        ("starts_at", at.as_str()),
    ];
    t.app.acting_as(&sara);

    t.app.htmx().post("/scheduling/portal/bookings", &form).await.assert_ok();
    t.app.htmx().post("/scheduling/portal/bookings", &form).await.assert_ok();
    let booked = Booking::query().order_by(Booking::ID).get(t.db()).await.unwrap();
    assert_eq!(booked.len(), 2);
    let mut ids = Vec::new();
    for booking in &booked {
        assert_eq!(booking.source, "portal");
        ids.extend(booking.resource_ids(t.db()).await.unwrap());
    }
    assert_eq!(ids, [first.id, second.id], "the first free resource each time");

    // Both doctors are busy now: just taken.
    t.app
        .htmx()
        .post("/scheduling/portal/bookings", &form)
        .await
        .assert_status(409);
    assert_eq!(Booking::query().count(t.db()).await.unwrap(), 2);

    // The loser joins the waiting list, and leaves it.
    t.app
        .htmx()
        .post("/scheduling/portal/waiting-list", &form)
        .await
        .assert_ok();
    let entry = WaitingEntry::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(entry.status, WaitingStatus::Waiting);
    assert_eq!(entry.party_id, PATIENT.0);
    t.app
        .htmx()
        .post(&format!("/scheduling/portal/waiting-list/{}/withdraw", entry.id), &[("x", "y")])
        .await
        .assert_ok();
    let gone = WaitingEntry::find_or_404(t.db(), entry.id).await.unwrap();
    assert_eq!(gone.status, WaitingStatus::Withdrawn);

    // Sara acts for one party only.
    let other = support::OTHER_PATIENT.0.to_string();
    t.app
        .htmx()
        .post(
            "/scheduling/portal/bookings",
            &[("party", other.as_str()), ("service", svc.as_str()), ("starts_at", at.as_str())],
        )
        .await
        .assert_not_found();
}

/// A service that needs acceptance is requested from the portal and holds the
/// slot for the request hold time.
#[renox::test]
async fn the_portal_requests_a_service_that_needs_acceptance() {
    let t = boot().await;
    let doctor = t.doctor("Dr A").await;
    let service = t
        .service_with("by_request", &[&doctor], |s| s.requires_acceptance = true)
        .await;
    let sara = t.user("Sara", &[]).await;
    t.portal.allow(sara.id, PATIENT);
    let start = in_hours(48);
    let at = wl_scheduling::pages::local_text(start);
    let (party, svc) = (PATIENT.0.to_string(), service.id.to_string());
    t.app.acting_as(&sara);
    t.app
        .htmx()
        .post(
            "/scheduling/portal/bookings",
            &[("party", party.as_str()), ("service", svc.as_str()), ("starts_at", at.as_str())],
        )
        .await
        .assert_ok();
    let request = Booking::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(request.status, BookingStatus::Requested);
    assert!(request.hold_until.is_some());
}
