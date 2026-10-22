//! BL-EDU-08 (#443): a tutor is assigned to a batch or a session only if their
//! documents are valid on **every** session date they would teach (`EDU.ACL.14`):
//! the police clearance when minors are enrolled, the private tutor permit or
//! freelance visa for a freelance tutor. `open` and `reschedule` are refused
//! naming the missing kind and the first failing date, with no minors' data;
//! the batch and the session register `BlockSubjects` and `FutureWork`.
//!
//! The real `wl-hr` answers `qualification_valid`; the documents behind it are
//! the stand-in guard of the support, filled by each test.

mod support;

use edtech::batches;
use edtech::enrolments::{self, NewEnrolment};
use edtech::sessions;
use edtech::tutors;
use edtech::{Batch, BatchStatus, SessionStatus};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use support::{Centre, boot, day, next_weekday};
use wl_kernel::PartyId;
use wl_kernel::documents::{BlockEntry, BlockMode, OverrideMode, block_subjects, future_work};
use wl_scheduling::{AvailabilityRule, Booking, Resource, ServiceType, resource, service};

/// The first Monday of the batches of the tests.
fn monday() -> NaiveDate {
    next_weekday(day(7), 1)
}

/// The message of a refusal.
fn refusal(error: &Error) -> String {
    match error {
        Error::Status(status, message) => {
            assert_eq!(*status, StatusCode::UNPROCESSABLE_ENTITY, "{message}");
            message.clone()
        }
        other => panic!("not a refusal: {other:?}"),
    }
}

/// The police clearance of employee `employee` is valid through `until`.
fn clearance_until(t: &Centre, employee: i64, until: NaiveDate) {
    t.documents.set(
        &Centre::employee_subject(employee),
        "police_clearance",
        Some(until),
    );
}

/// A batch for minors of `layla` from `monday()`, Monday and Wednesday.
async fn minors_batch(t: &Centre, layla: &Resource, employee: i64, room: &str) -> Batch {
    let room = t.room(room, 14).await;
    let course = t.minors_course().await;
    t.batch_with(&course, (layla, Some(employee)), &room, monday(), &[1, 3])
        .await
}

/// Given tutor T with a police clearance valid to the Tuesday of the second week
/// and planned batch B (minors) with sessions to the end of the fourth, `open`
/// with T is refused naming the clearance and the first failing date, the
/// Wednesday after; B stays planned. Renewing it to cover the last session lets
/// `open` through.
#[renox::test]
async fn edu_08_open_is_refused_naming_the_clearance_and_the_first_failing_date() {
    let t = boot().await;
    let (layla, employee) = t.employed_tutor("Layla Hassan", false).await;
    let batch = minors_batch(&t, &layla, employee, "Room 1").await;
    clearance_until(&t, employee, monday() + TimeDelta::days(8));
    let amal = t.coordinator().await;

    let error = batches::open(t.state(), &amal, batch.id).await.unwrap_err();

    let message = refusal(&error);
    assert!(message.contains("police clearance"), "{message}");
    let first_failing = monday() + TimeDelta::days(9);
    assert!(message.contains(&first_failing.to_string()), "{message}");
    assert!(
        message.contains(&(monday() + TimeDelta::days(23)).to_string()),
        "the guard looked at every date, not only the first: {message}"
    );
    assert!(message.contains("Layla Hassan"), "{message}");
    assert!(
        !message.to_lowercase().contains("minor"),
        "no minors' data: {message}"
    );
    let same = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(same.status, BatchStatus::Planned);
    assert!(t.sessions_of(&batch).await.is_empty());

    // The page shows the refusal too.
    let amal_user = t.coordinator_user().await;
    t.app.acting_as(&amal_user);
    let page = t
        .app
        .htmx()
        .post(&format!("/edtech/batches/{}/open", batch.id), &[("x", "y")])
        .await;
    page.assert_status(422);
    assert!(page.text().contains("police clearance"));

    // A clearance that covers the last session date lets it through.
    clearance_until(&t, employee, monday() + TimeDelta::days(27));
    let opened = batches::open(t.state(), &amal, batch.id).await.unwrap();
    assert_eq!(opened.status, BatchStatus::Open);
}

/// Given an adult course without minor enrolments, the same `open` succeeds
/// with no police clearance at all.
#[renox::test]
async fn edu_08_an_adult_course_needs_no_clearance() {
    let t = boot().await;
    let (layla, employee) = t.employed_tutor("Layla Hassan", false).await;
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let batch = t
        .batch_with(&course, (&layla, Some(employee)), &room, monday(), &[1, 3])
        .await;
    let amal = t.coordinator().await;

    let opened = batches::open(t.state(), &amal, batch.id).await.unwrap();

    assert_eq!(opened.status, BatchStatus::Open);
}

/// Given freelance tutor F with an expired permit but a valid freelance visa
/// (two documents of the one kind), F can be assigned; one with only an expired
/// permit is refused naming the kind, and one with nothing at all too.
#[renox::test]
async fn edu_08_a_freelance_tutor_passes_with_either_the_permit_or_the_visa() {
    let t = boot().await;
    let (fiona, fiona_employee) = t.employed_tutor("Fiona Freelance", true).await;
    let (gina, gina_employee) = t.employed_tutor("Gina Freelance", true).await;
    let (hana, hana_employee) = t.employed_tutor("Hana Freelance", true).await;
    let course = t.course().await;
    let amal = t.coordinator().await;
    let subject = |employee| Centre::employee_subject(employee);

    // Fiona: the permit ran out, the visa did not.
    t.documents
        .set(&subject(fiona_employee), "tutor_permit", Some(day(-30)));
    t.documents
        .also(&subject(fiona_employee), "tutor_permit", day(300));
    let room1 = t.room("Room 1", 14).await;
    let batch = t
        .batch_with(
            &course,
            (&fiona, Some(fiona_employee)),
            &room1,
            monday(),
            &[1, 3],
        )
        .await;
    let opened = batches::open(t.state(), &amal, batch.id).await.unwrap();
    assert_eq!(opened.status, BatchStatus::Open);

    // Gina: only the expired permit.
    t.documents
        .set(&subject(gina_employee), "tutor_permit", Some(day(-30)));
    let room2 = t.room("Room 2", 14).await;
    let other = t
        .batch_with(
            &course,
            (&gina, Some(gina_employee)),
            &room2,
            monday(),
            &[1, 3],
        )
        .await;
    let error = batches::open(t.state(), &amal, other.id).await.unwrap_err();
    let message = refusal(&error);
    assert!(
        message.contains("private tutor permit or freelance visa"),
        "{message}"
    );
    assert!(message.contains(&monday().to_string()), "{message}");

    // Hana: no document at all.
    let room3 = t.room("Room 3", 14).await;
    let third = t
        .batch_with(
            &course,
            (&hana, Some(hana_employee)),
            &room3,
            monday(),
            &[1, 3],
        )
        .await;
    assert!(batches::open(t.state(), &amal, third.id).await.is_err());
}

/// Rescheduling a session of B to a Friday after the clearance ran out is
/// refused; to a Friday before it succeeds.
#[renox::test]
async fn edu_08_a_session_moves_only_to_a_date_the_clearance_covers() {
    let t = boot().await;
    let (layla, employee) = t.employed_tutor("Layla Hassan", false).await;
    clearance_until(&t, employee, monday() + TimeDelta::days(27));
    let batch = minors_batch(&t, &layla, employee, "Room 1").await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, batch.id).await.unwrap();
    // The clearance ends on the Tuesday of the second week.
    clearance_until(&t, employee, monday() + TimeDelta::days(8));
    let session = t.sessions_of(&batch).await.remove(0);

    let late = monday() + TimeDelta::days(11);
    let error = sessions::reschedule(t.state(), &amal, session.id, late, "18:00")
        .await
        .unwrap_err();
    let message = refusal(&error);
    assert!(message.contains("police clearance"), "{message}");
    assert!(message.contains(&late.to_string()), "{message}");
    let unchanged = edtech::Session::find_or_404(t.db(), session.id)
        .await
        .unwrap();
    assert_eq!(unchanged.session_date, session.session_date);

    let early = monday() + TimeDelta::days(4);
    let moved = sessions::reschedule(t.state(), &amal, session.id, early, "18:00")
        .await
        .unwrap();
    assert_eq!(moved.session_date, early);
    assert_eq!(moved.status, SessionStatus::Scheduled);
}

/// A new tutor whose clearance covers some of the remaining dates and not all
/// changes nothing: the batch is refused with the dates the document leaves out,
/// and a session goes only to a tutor valid on its day.
#[renox::test]
async fn edu_08_a_new_tutor_must_be_valid_on_every_remaining_date() {
    let t = boot().await;
    let (layla, layla_employee) = t.employed_tutor("Layla Hassan", false).await;
    let (nour, nour_employee) = t.employed_tutor("Nour Saleh", false).await;
    clearance_until(&t, layla_employee, monday() + TimeDelta::days(27));
    clearance_until(&t, nour_employee, monday() + TimeDelta::days(18));
    let batch = minors_batch(&t, &layla, layla_employee, "Room 1").await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, batch.id).await.unwrap();

    let from = monday() + TimeDelta::days(7);
    let error = tutors::change_batch_tutor(
        t.state(),
        &amal,
        batch.id,
        nour.id,
        Some(nour_employee),
        from,
    )
    .await
    .unwrap_err();
    let message = refusal(&error);
    assert!(
        message.contains(&(monday() + TimeDelta::days(21)).to_string()),
        "{message}"
    );
    assert!(
        message.contains(&(monday() + TimeDelta::days(23)).to_string()),
        "{message}"
    );
    assert!(
        t.sessions_of(&batch)
            .await
            .iter()
            .all(|s| s.tutor_resource_id == Some(layla.id))
    );

    // One session inside Nour's clearance goes to her; one outside it does not.
    let sessions_of = t.sessions_of(&batch).await;
    let inside = sessions_of
        .iter()
        .find(|s| s.session_date == monday() + TimeDelta::days(9))
        .unwrap();
    let outside = sessions_of
        .iter()
        .find(|s| s.session_date == monday() + TimeDelta::days(23))
        .unwrap();
    let moved = sessions::change_tutor(t.state(), &amal, inside.id, nour.id, Some(nour_employee))
        .await
        .unwrap();
    assert_eq!(moved.tutor_resource_id, Some(nour.id));
    let refused =
        sessions::change_tutor(t.state(), &amal, outside.id, nour.id, Some(nour_employee))
            .await
            .unwrap_err();
    assert!(refusal(&refused).contains("police clearance"));
}

/// A minor who enrols into an adult batch whose tutor has no clearance is
/// allowed to (enrolment is not in the `blocks` list); from then on the batch
/// needs the clearance for any further assignment.
#[renox::test]
async fn edu_08_a_minor_enrolling_makes_the_batch_need_the_clearance() {
    let t = boot().await;
    let (layla, employee) = t.employed_tutor("Layla Hassan", false).await;
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let batch = t
        .batch_with(&course, (&layla, Some(employee)), &room, monday(), &[1, 3])
        .await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, batch.id).await.unwrap();
    let child = t.person(31, "Omar Child");
    let mother = t.person(32, "Mother");
    t.people.guardian(child, mother);
    let desk = t.desk().await;

    let enrolled = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(child, batch.id, support::PRICE)
            .guardian(mother)
            .terms_accepted(),
    )
    .await
    .expect("the enrolment is not blocked");
    assert_eq!(enrolled.batch_id, Some(batch.id));

    let session = t.sessions_of(&batch).await.remove(1);
    let error = sessions::reschedule(
        t.state(),
        &amal,
        session.id,
        monday() + TimeDelta::days(4),
        "18:00",
    )
    .await
    .unwrap_err();
    let message = refusal(&error);
    assert!(message.contains("police clearance"), "{message}");
    assert!(!message.contains("Omar"), "no student's name: {message}");
}

/// `FutureWork(T, expiry)` returns B's sessions after the expiry date and T's
/// one-to-one bookings after it; `BlockSubjects` give T's employee for the
/// transitions of the batch and the session.
#[renox::test]
async fn edu_08_future_work_lists_the_sessions_and_the_bookings_after_the_expiry() {
    let t = boot().await;
    let (layla, employee) = t.employed_tutor("Layla Hassan", false).await;
    clearance_until(&t, employee, monday() + TimeDelta::days(60));
    let batch = minors_batch(&t, &layla, employee, "Room 1").await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, batch.id).await.unwrap();

    // A one-to-one lesson of T that asks for the clearance, booked while it is valid.
    let mut service_type = ServiceType::new("one_to_one_maths", "One-to-one maths", 60);
    service_type.required_qualifications = "police_clearance".to_owned();
    service_type.save(t.db()).await.unwrap();
    resource::offer(t.db(), service_type.id, layla.id)
        .await
        .unwrap();
    for weekday in 1..=7 {
        AvailabilityRule::new(layla.id, weekday, 0, 1440)
            .save(t.db())
            .await
            .unwrap();
    }
    let parent = t.person(40, "Parent");
    let lesson_day = monday() + TimeDelta::days(12);
    let booking: Booking = service::book(
        t.state(),
        wl_scheduling::NewBooking::new(
            PartyId(parent),
            service_type.id,
            &[layla.id],
            support::at(lesson_day, 10, 0),
        ),
        wl_scheduling::Actor::staff(1),
        renox::db::now(),
    )
    .await
    .expect("the lesson is booked");

    // The clearance ends on the Tuesday of the second week.
    let expiry = monday() + TimeDelta::days(8);
    clearance_until(&t, employee, expiry);
    let subject = Centre::employee_subject(employee);
    let entry = |guarded: &str| BlockEntry {
        kind: "police_clearance".to_owned(),
        guarded: guarded.to_owned(),
        mode: BlockMode::Block,
        override_mode: OverrideMode::Never,
        condition: None,
        locked: true,
    };
    let from = expiry + TimeDelta::days(1);

    let sessions_work = future_work("session.change_tutor")
        .expect("the sessions register their FutureWork")
        .planned_from(t.state(), &entry("session.change_tutor"), &subject, from)
        .await
        .unwrap();
    let dates: Vec<NaiveDate> = sessions_work.iter().map(|r| r.date).collect();
    let wanted: Vec<NaiveDate> = [9, 14, 16, 21, 23]
        .iter()
        .map(|d| monday() + TimeDelta::days(*d))
        .collect();
    assert_eq!(dates, wanted, "B's sessions after the expiry");
    assert!(sessions_work[0].label.starts_with(&batch.number()));

    let bookings_work = future_work("booking.book")
        .expect("wl-scheduling registers its FutureWork")
        .planned_from(t.state(), &entry("booking.book"), &subject, from)
        .await
        .unwrap();
    let records: Vec<_> = bookings_work.iter().map(|r| r.record.clone()).collect();
    assert_eq!(records, vec![booking.subject()], "T's one-to-one booking");

    // A police clearance is asked of the batch's tutor for the transitions of the
    // batch and the session; the permit of a tutor who is not a freelancer is not.
    for guarded in [
        "batch.open",
        "batch.change_tutor",
        "session.change_tutor",
        "session.reschedule",
        "session.submit_attendance",
    ] {
        let provider = block_subjects(guarded).expect("a BlockSubjects provider");
        let record = if guarded.starts_with("batch") {
            batch.id
        } else {
            t.sessions_of(&batch).await[0].id
        };
        let found = provider
            .subjects(t.state(), &entry(guarded), record)
            .await
            .unwrap();
        assert_eq!(found, vec![subject.clone()], "{guarded}");
        let mut permit = entry(guarded);
        permit.kind = "tutor_permit".to_owned();
        assert!(
            provider
                .subjects(t.state(), &permit, record)
                .await
                .unwrap()
                .is_empty(),
            "{guarded}: the tutor is not a freelancer"
        );
    }
}
