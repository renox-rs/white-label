//! BL-EDU-02 (#437): seats are taken atomically (`EDU.ACL.17`). The seats of a
//! batch are the enrolments that hold one (`pending_payment`, `active`,
//! `frozen`); two takers of the last seat never both win, a counsellor stops at
//! the capacity, the centre manager at the room's limit, `transfer` into a full
//! batch is refused, and a lapsed hold frees its seat.

mod support;

use edtech::enrolments::{self, NewEnrolment};
use edtech::transfer::transfer;
use edtech::waiting::{self, BatchWaiting};
use edtech::{Batch, Enrolment, EnrolmentStatus, seats};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use support::{Centre, PRICE, boot, day, next_weekday};

/// An open batch of 12 seats (the room holds 14) starting on the first Monday
/// on or after `from`, with a tutor and a room of its own.
async fn open_batch(t: &Centre, name: &str, from: NaiveDate) -> Batch {
    let tutor = t.tutor(&format!("Tutor {name}")).await;
    let room = t.room(&format!("Room {name}"), 14).await;
    t.open_batch(&tutor, &room, next_weekday(from, 1)).await
}

/// `count` enrolments of `status` in `batch`, for the students from `first`.
async fn fill(t: &Centre, batch: &Batch, first: i64, count: i64, status: EnrolmentStatus) {
    for n in 0..count {
        t.enrolment_in(batch, first + n, status).await;
    }
}

/// A second counsellor at the front desk.
async fn other_desk(t: &Centre) -> renox::AuthUser {
    let user = t
        .user(
            "Dalia",
            &[
                edtech::permissions::COURSES_VIEW,
                edtech::permissions::BATCHES_VIEW,
                edtech::permissions::ENROLMENTS_VIEW,
                edtech::permissions::ENROLMENTS_MANAGE,
            ],
        )
        .await;
    t.auth(&user).await
}

/// Batch B has capacity 12 and 11 seats taken. Counsellor A and counsellor B
/// try at the same moment: exactly one succeeds, the other is refused with the
/// other open batch of the course that has a seat and the waiting list. (The
/// portal has no enrolment screen yet; it would call the same service.)
#[renox::test]
async fn edu_02_two_takers_of_the_last_seat_only_one_wins() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let b2 = open_batch(&t, "B2", day(21)).await;
    fill(&t, &b, 100, 11, EnrolmentStatus::Active).await;
    let (sara, omar) = (t.person(1, "Sara Ali"), t.person(2, "Omar Khan"));
    let (a, other) = (t.desk().await, other_desk(&t).await);

    let (first, second) = renox::tokio::join!(
        enrolments::create(t.state(), &a, NewEnrolment::course(sara, b.id, PRICE)),
        enrolments::create(t.state(), &other, NewEnrolment::course(omar, b.id, PRICE)),
    );

    let (won, refused) = match (first, second) {
        (Ok(won), Err(refused)) | (Err(refused), Ok(won)) => (won, refused),
        (Ok(_), Ok(_)) => panic!("both took the last seat"),
        (Err(one), Err(two)) => panic!("neither took it: {one} / {two}"),
    };
    assert_eq!(won.status, EnrolmentStatus::PendingPayment);
    let message = refused.to_string();
    assert!(message.contains("no free seat"), "{message}");
    assert!(message.contains(&b2.number()), "the other batch: {message}");
    assert!(message.contains("waiting list"), "{message}");
    assert_eq!(seats::available(t.db(), b.id).await.unwrap(), 0);
    assert_eq!(
        Batch::find_or_404(t.db(), b.id).await.unwrap().seats_taken,
        12
    );
}

/// A refusal with no other open batch names only the waiting list.
#[renox::test]
async fn edu_02_without_another_batch_the_refusal_offers_the_waiting_list() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    fill(&t, &b, 100, 12, EnrolmentStatus::Active).await;
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;

    let refused = enrolments::create(t.state(), &desk, NewEnrolment::course(sara, b.id, PRICE))
        .await
        .unwrap_err()
        .to_string();

    assert!(refused.contains("no free seat"), "{refused}");
    assert!(refused.contains("waiting list"), "{refused}");
    assert!(!refused.contains("BAT-"), "no other batch is offered: {refused}");
}

/// Capacity 12, room limit 14: the centre manager enrols a 13th and a 14th
/// student, not a 15th.
#[renox::test]
async fn edu_02_the_centre_manager_goes_over_the_capacity_up_to_the_room_limit() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    fill(&t, &b, 100, 12, EnrolmentStatus::Active).await;
    let manager = t.manager().await;
    let (s13, s14, s15) = (
        t.person(1, "Sara Ali"),
        t.person(2, "Omar Khan"),
        t.person(3, "Huda Khan"),
    );

    for student in [s13, s14] {
        enrolments::create(t.state(), &manager, NewEnrolment::course(student, b.id, PRICE))
            .await
            .expect("up to the room limit");
    }
    let over = enrolments::create(t.state(), &manager, NewEnrolment::course(s15, b.id, PRICE))
        .await
        .unwrap_err();

    assert!(over.to_string().contains("no free seat"));
    assert_eq!(seats::available(t.db(), b.id).await.unwrap(), 0);
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 14);
}

/// A room that states no limit leaves the capacity as the hard limit, even for
/// the centre manager.
#[renox::test]
async fn edu_02_a_room_without_a_limit_makes_the_capacity_the_hard_limit() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    Batch::query()
        .where_(Batch::ID.eq(b.id))
        .update(t.db(), &[("room_limit", &Option::<i64>::None)])
        .await
        .unwrap();
    fill(&t, &b, 100, 12, EnrolmentStatus::Active).await;
    let manager = t.manager().await;
    let sara = t.person(1, "Sara Ali");

    let over = enrolments::create(t.state(), &manager, NewEnrolment::course(sara, b.id, PRICE))
        .await;

    assert!(over.is_err());
}

/// A counsellor cannot go past 12.
#[renox::test]
async fn edu_02_a_counsellor_cannot_exceed_the_capacity() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    fill(&t, &b, 100, 12, EnrolmentStatus::Active).await;
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;

    let over = enrolments::create(t.state(), &desk, NewEnrolment::course(sara, b.id, PRICE))
        .await
        .unwrap_err();

    assert!(over.to_string().contains("no free seat"));
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 12);
}

/// `transfer` into a full batch is refused, with the alternatives, and the
/// enrolment stays where it was; into a batch with a seat it moves.
#[renox::test]
async fn edu_02_transfer_into_a_full_batch_is_refused() {
    let t = boot().await;
    let from = open_batch(&t, "From", day(7)).await;
    let full = open_batch(&t, "Full", day(21)).await;
    let roomy = open_batch(&t, "Roomy", day(35)).await;
    fill(&t, &full, 100, 12, EnrolmentStatus::Active).await;
    let student = t.person(1, "Sara Ali");
    let e = t.enrolment_in(&from, student, EnrolmentStatus::Active).await;
    let desk = t.desk().await;

    let refused = transfer(t.state(), Some(&desk), e.id, full.id)
        .await
        .unwrap_err()
        .to_string();

    assert!(refused.contains("no free seat"), "{refused}");
    assert!(refused.contains(&roomy.number()), "{refused}");
    let unchanged = Enrolment::find_or_404(t.db(), e.id).await.unwrap();
    assert_eq!(unchanged.status, EnrolmentStatus::Active);
    assert!(unchanged.seat_held);
    assert_eq!(seats::count_in(t.db(), from.id).await.unwrap(), 1);
    assert_eq!(seats::count_in(t.db(), full.id).await.unwrap(), 12);

    let moved = transfer(t.state(), Some(&desk), e.id, roomy.id)
        .await
        .expect("a batch with a seat takes the student");
    assert_eq!(moved.status, EnrolmentStatus::Active);
    assert_eq!(moved.batch_id, Some(roomy.id));
    assert_eq!(moved.transferred_from_id, Some(e.id));
    let old = Enrolment::find_or_404(t.db(), e.id).await.unwrap();
    assert_eq!(old.status, EnrolmentStatus::Transferred);
    assert!(!old.seat_held);
    assert_eq!(old.transferred_to_id, Some(moved.id));
    assert_eq!(seats::count_in(t.db(), from.id).await.unwrap(), 0);
    assert_eq!(seats::count_in(t.db(), roomy.id).await.unwrap(), 1);
}

/// After a `lapse` the seat is free and a new enrolment into the batch
/// succeeds.
#[renox::test]
async fn edu_02_after_a_lapse_a_new_enrolment_succeeds() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    fill(&t, &b, 100, 11, EnrolmentStatus::Active).await;
    let held = t
        .enrolment_in(&b, 200, EnrolmentStatus::PendingPayment)
        .await;
    let sara = t.person(1, "Sara Ali");
    let desk = t.desk().await;
    let full = enrolments::create(t.state(), &desk, NewEnrolment::course(sara, b.id, PRICE)).await;
    assert!(full.is_err(), "the batch is full while the hold lasts");

    let lapsed = t.lapse(&held).await;
    assert_eq!(lapsed.status, EnrolmentStatus::Lapsed);

    let again = enrolments::create(t.state(), &desk, NewEnrolment::course(sara, b.id, PRICE))
        .await
        .expect("the freed seat is taken");
    assert_eq!(again.status, EnrolmentStatus::PendingPayment);
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 12);
}

/// Enrolments that are over count for nothing, and a freeze keeps the seat.
#[renox::test]
async fn edu_02_only_active_frozen_and_held_enrolments_count() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    for (n, status) in [
        EnrolmentStatus::Lapsed,
        EnrolmentStatus::Withdrawn,
        EnrolmentStatus::Transferred,
        EnrolmentStatus::Completed,
    ]
    .into_iter()
    .enumerate()
    {
        t.enrolment_in(&b, 100 + n as i64, status).await;
    }
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 0);
    t.enrolment_in(&b, 200, EnrolmentStatus::Frozen).await;
    t.enrolment_in(&b, 201, EnrolmentStatus::PendingPayment).await;
    t.enrolment_in(&b, 202, EnrolmentStatus::Active).await;
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 3);
    assert_eq!(seats::available(t.db(), b.id).await.unwrap(), 9);
}

/// A refused student goes on the waiting list: a plain list the counsellors see
/// (student, payer, added at), holding no seat and offering nothing when a seat
/// frees.
#[renox::test]
async fn edu_02_the_waiting_list_is_a_plain_list_that_holds_no_seat() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    fill(&t, &b, 100, 11, EnrolmentStatus::Active).await;
    let held = t
        .enrolment_in(&b, 200, EnrolmentStatus::PendingPayment)
        .await;
    let (sara, mother) = (t.person(1, "Sara Ali"), t.person(2, "Huda Ali"));
    let desk = t.desk().await;

    let entry = waiting::join(t.state(), &desk, b.id, sara, Some(mother))
        .await
        .unwrap();
    assert_eq!((entry.student_party_id, entry.payer_party_id), (sara, mother));
    assert!(entry.created_at.is_some());
    let twice = waiting::join(t.state(), &desk, b.id, sara, None).await;
    assert!(twice.is_err(), "once on the list");

    t.lapse(&held).await;
    assert_eq!(seats::available(t.db(), b.id).await.unwrap(), 1);
    assert_eq!(
        BatchWaiting::of_batch(t.db(), b.id).await.unwrap().len(),
        1,
        "nothing is offered by itself and nobody is removed"
    );
    assert_eq!(seats::count_in(t.db(), b.id).await.unwrap(), 11);
}

/// The batch page lists the waiting list to a counsellor and offers to add.
#[renox::test]
async fn edu_02_the_batch_page_shows_the_waiting_list() {
    let t = boot().await;
    let b = open_batch(&t, "B", day(7)).await;
    let sara = t.person(1, "Sara Ali");
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    waiting::join(t.state(), &desk, b.id, sara, None)
        .await
        .unwrap();

    t.app.acting_as(&desk_user);
    t.app
        .get(&format!("/edtech/batches/{}", b.id))
        .await
        .assert_ok()
        .assert_see("Waiting list")
        .assert_see("Sara Ali");
}
