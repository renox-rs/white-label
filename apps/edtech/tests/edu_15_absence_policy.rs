//! BL-EDU-15 (#450): the credits a session uses follow the absence policy of
//! the catalogue item stored with the enrolment, not a fixed rule. By default an
//! unexcused absence uses a credit; an absence notified 24 hours or more before
//! is excused, uses none and gives a make-up right; a late notice stays unexcused
//! unless the coordinator excuses it with a reason; a membership, a course fee, a
//! cancelled session and a booking the centre cancels use none.
//!
//! The ledger of the packages is the stand-in of the support, which behaves as
//! wl-sales does for `consume` (once per source) and `reverse`.

mod support;

use edtech::attendance::{Attendance, Mark};
use edtech::credits;
use edtech::make_up::{self, MakeUpRight};
use edtech::notices::{FAMILY_NOTICE, NOTICE};
use edtech::roster;
use edtech::sessions;
use edtech::{Batch, Enrolment, EnrolmentKind, EnrolmentStatus, Session, SessionStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, Messages, at, boot, day, next_weekday};
use wl_kernel::PartyId;
use wl_scheduling::{
    Actor, AvailabilityRule, CancelRequest, NewBooking, ServiceType, resource, service,
};

const ITEM: &str = "pack10";

/// A planned batch whose sessions are fixtures: the students of the tests are
/// guests of its sessions.
async fn batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.batch_of(&tutor, &room, next_weekday(day(7), 1), &[1, 3])
        .await
}

/// The row of `enrolment` in `session`.
async fn row(t: &Centre, session: &Session, enrolment: &Enrolment) -> Attendance {
    Attendance::find_of(t.db(), session.id, enrolment.id)
        .await
        .unwrap()
        .expect("a row")
}

/// Given a 10-credit package (default policy) and P absent without notice,
/// `submit_attendance` consumes one credit; one who came uses one too (late
/// counts as present); the row says what was taken.
#[renox::test]
async fn edu_15_an_unexcused_absence_uses_a_credit_by_default() {
    let t = boot().await;
    let batch = batch(&t).await;
    let session = t.past_session(&batch, 1, 2).await;
    let (p, q) = (t.person(1, "Sara Ali"), t.person(2, "Omar Khan"));
    let (ep, eq) = (
        t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await,
        t.package_in(q, ITEM, 10, EnrolmentStatus::Active).await,
    );
    let desk = t.desk().await;
    roster::add_student(t.state(), &desk, session.id, ep.id)
        .await
        .unwrap();
    roster::add_student(t.state(), &desk, session.id, eq.id)
        .await
        .unwrap();
    let tariq = t.teacher().await;

    let held = sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        session.id,
        &[(ep.id, Mark::Absent), (eq.id, Mark::Late)],
        false,
    )
    .await
    .unwrap();

    assert_eq!(held.status, SessionStatus::Held);
    let absent = row(&t, &session, &ep).await;
    assert_eq!(absent.mark, Mark::Absent);
    assert!(absent.credit_due && absent.consumed);
    assert_eq!(t.packages.left(PartyId(p), ITEM), 9);
    let late = row(&t, &session, &eq).await;
    assert_eq!(late.mark, Mark::Late);
    assert!(late.consumed, "late arrival counts as present");
    assert_eq!(t.packages.left(PartyId(q), ITEM), 9);
    assert_eq!(
        t.packages.consumed_by(),
        vec![absent.subject(), late.subject()]
    );
}

/// A notice on the day before at 09:00 for a session at 10:00 the next day
/// excuses P: nothing is consumed, P has one make-up right, and the tutor sees P
/// pre-marked excused. A notice at 08:00 on the day itself leaves P unexcused.
#[renox::test]
async fn edu_15_a_notice_a_day_ahead_excuses_and_a_later_one_does_not() {
    let t = boot().await;
    let batch = batch(&t).await;
    let early = t.past_session(&batch, 1, 3).await;
    let late = t.past_session(&batch, 2, 2).await;
    let p = t.person(1, "Sara Ali");
    let ep = t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await;
    let desk = t.desk().await;
    for session in [&early, &late] {
        roster::add_student(t.state(), &desk, session.id, ep.id)
            .await
            .unwrap();
    }

    // 09:00 the day before the first session: 25 hours ahead.
    let noticed = roster::record_notice(
        t.state(),
        &desk,
        early.id,
        ep.id,
        at(early.session_date - TimeDelta::days(1), 9, 0),
    )
    .await
    .unwrap();
    assert_eq!(noticed.mark, Mark::Excused, "pre-marked for the tutor");
    // 08:00 on the day of the second session: two hours ahead.
    let tardy = roster::record_notice(
        t.state(),
        &desk,
        late.id,
        ep.id,
        at(late.session_date, 8, 0),
    )
    .await
    .unwrap();
    assert_eq!(tardy.mark, Mark::Pending, "too late to excuse");
    assert!(tardy.notice_at.is_some());

    let tariq = t.teacher().await;
    sessions::submit_attendance_marked(t.state(), &tariq, early.id, &[], false)
        .await
        .expect("the excused student needs no mark of the tutor");
    sessions::submit_attendance_marked(t.state(), &tariq, late.id, &[(ep.id, Mark::Absent)], false)
        .await
        .unwrap();

    let excused = row(&t, &early, &ep).await;
    assert_eq!(excused.mark, Mark::Excused);
    assert!(!excused.credit_due && !excused.consumed);
    let unexcused = row(&t, &late, &ep).await;
    assert_eq!(unexcused.mark, Mark::Absent);
    assert!(
        unexcused.consumed,
        "the late notice is an unexcused absence"
    );
    assert_eq!(
        t.packages.left(PartyId(p), ITEM),
        9,
        "one credit, for the second session only"
    );
    let rights = make_up::open_rights(t.db(), &ep).await.unwrap();
    assert_eq!(rights.len(), 1, "one right for the one excused absence");
    assert_eq!(rights[0].session_id, early.id);
}

/// The coordinator marks a late notice excused with a reason (without one it is
/// refused); a tutor cannot excuse an absence by themself; a notice after the
/// start is not a notice.
#[renox::test]
async fn edu_15_the_coordinator_excuses_a_late_notice_with_a_reason() {
    let t = boot().await;
    let batch = batch(&t).await;
    let session = t.past_session(&batch, 1, 2).await;
    let p = t.person(1, "Sara Ali");
    let ep = t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await;
    let desk = t.desk().await;
    let amal = t.coordinator().await;
    roster::add_student(t.state(), &desk, session.id, ep.id)
        .await
        .unwrap();
    roster::record_notice(
        t.state(),
        &desk,
        session.id,
        ep.id,
        at(session.session_date, 8, 0),
    )
    .await
    .unwrap();

    // Not without a reason, not by the counsellor.
    assert!(
        roster::excuse_late(t.state(), &amal, session.id, ep.id, " ")
            .await
            .is_err()
    );
    assert!(matches!(
        roster::excuse_late(t.state(), &desk, session.id, ep.id, "Illness").await,
        Err(Error::Forbidden)
    ));
    // The tutor cannot excuse it.
    let tariq = t.teacher().await;
    let refused = sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        session.id,
        &[(ep.id, Mark::Excused)],
        false,
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("24 hours"), "{refused}");
    // A notice after the start is an absence.
    let after = roster::record_notice(
        t.state(),
        &desk,
        session.id,
        ep.id,
        at(session.session_date, 12, 0),
    )
    .await;
    assert!(after.is_err());

    let excused = roster::excuse_late(t.state(), &amal, session.id, ep.id, "Hospital visit")
        .await
        .unwrap();
    assert_eq!(excused.mark, Mark::Excused);
    assert_eq!(excused.excuse_reason.as_deref(), Some("Hospital visit"));
    sessions::submit_attendance_marked(t.state(), &tariq, session.id, &[], false)
        .await
        .unwrap();
    assert_eq!(t.packages.left(PartyId(p), ITEM), 10, "nothing consumed");
    assert_eq!(make_up::open_rights(t.db(), &ep).await.unwrap().len(), 1);
}

/// Given a catalogue item with "unexcused absence consumes" = no, nothing is
/// consumed for the absence; the student who came still uses a credit.
#[renox::test]
async fn edu_15_the_policy_of_the_enrolment_decides() {
    let t = boot().await;
    let batch = batch(&t).await;
    let first = t.past_session(&batch, 1, 3).await;
    let second = t.past_session(&batch, 2, 2).await;
    let p = t.person(1, "Sara Ali");
    let ep = t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await;
    t.set_policy(&ep, false, 24).await;
    let desk = t.desk().await;
    let tariq = t.teacher().await;
    for session in [&first, &second] {
        roster::add_student(t.state(), &desk, session.id, ep.id)
            .await
            .unwrap();
    }

    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        first.id,
        &[(ep.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    assert!(
        t.packages.asked().is_empty(),
        "the policy says an absence is free"
    );
    assert_eq!(t.packages.left(PartyId(p), ITEM), 10);
    assert!(!row(&t, &first, &ep).await.credit_due);

    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        second.id,
        &[(ep.id, Mark::Present)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(t.packages.left(PartyId(p), ITEM), 9);
}

/// A cancelled session consumes nothing (it has no marks); a membership and a
/// course fee have no credits, an excused absence still gives their students a
/// make-up right.
#[renox::test]
async fn edu_15_a_cancelled_session_a_membership_and_a_course_fee_consume_nothing() {
    let t = boot().await;
    let batch = batch(&t).await;
    let cancelled = t.past_session(&batch, 1, 3).await;
    let held = t.past_session(&batch, 2, 2).await;
    let p = t.person(1, "Sara Ali");
    let (member, learner) = (t.person(2, "Mona Member"), t.person(3, "Cara Course"));
    let package = t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await;
    let membership = t
        .package_in(member, "monthly", 0, EnrolmentStatus::Active)
        .await;
    Enrolment::query()
        .where_(Enrolment::ID.eq(membership.id))
        .update(t.db(), &[("kind", &EnrolmentKind::Membership)])
        .await
        .unwrap();
    let course = t
        .enrolment_in(&batch, learner, EnrolmentStatus::Active)
        .await;
    let desk = t.desk().await;
    let amal = t.coordinator().await;
    let tariq = t.teacher().await;
    roster::add_student(t.state(), &desk, cancelled.id, package.id)
        .await
        .unwrap();
    sessions::cancel(t.state(), &amal, cancelled.id, "The room is closed", None)
        .await
        .unwrap();
    assert!(
        t.packages.asked().is_empty(),
        "a cancelled session consumes nothing"
    );
    assert_eq!(t.packages.left(PartyId(p), ITEM), 10);

    // The membership attends the held session; the course student is excused.
    roster::add_student(t.state(), &desk, held.id, membership.id)
        .await
        .unwrap();
    roster::record_notice(
        t.state(),
        &desk,
        held.id,
        course.id,
        at(held.session_date - TimeDelta::days(2), 9, 0),
    )
    .await
    .unwrap();
    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        held.id,
        &[(membership.id, Mark::Present)],
        false,
    )
    .await
    .unwrap();

    assert!(
        t.packages.asked().is_empty(),
        "no credit for a membership or a course fee"
    );
    assert!(!row(&t, &held, &membership).await.credit_due);
    let learner_row = row(&t, &held, &course).await;
    assert_eq!(learner_row.mark, Mark::Excused);
    assert_eq!(
        MakeUpRight::of_enrolment(t.db(), course.id)
            .await
            .unwrap()
            .len(),
        1,
        "an excused absence of a course fee gives a make-up right"
    );
}

/// The ledger could not serve the credit (the package has none left): the
/// attendance stays saved, the row says why, the counsellor gets one task to sell
/// credits, and the job takes the credit once a package is sold and closes the task.
#[renox::test]
async fn edu_15_with_no_credit_left_the_counsellor_gets_a_task() {
    let t = boot().await;
    let batch = batch(&t).await;
    let first = t.past_session(&batch, 1, 3).await;
    let second = t.past_session(&batch, 2, 2).await;
    let p = t.person(1, "Sara Ali");
    let ep = t.package_in(p, ITEM, 0, EnrolmentStatus::Active).await;
    let desk = t.desk().await;
    let tariq = t.teacher().await;
    for session in [&first, &second] {
        roster::add_student(t.state(), &desk, session.id, ep.id)
            .await
            .unwrap();
    }

    for session in [&first, &second] {
        let held = sessions::submit_attendance_marked(
            t.state(),
            &tariq,
            session.id,
            &[(ep.id, Mark::Present)],
            false,
        )
        .await
        .expect("the attendance is saved");
        assert_eq!(held.status, SessionStatus::Held);
    }

    let waiting = row(&t, &first, &ep).await;
    assert!(waiting.credit_due && !waiting.consumed);
    assert_eq!(waiting.credit_note.as_deref(), Some("no_credit"));
    let tasks = t.tasks.with_key("edtech.credits");
    assert_eq!(tasks.len(), 1, "one task however many sessions");
    assert!(
        tasks[0].ticket.title.contains("Sara Ali"),
        "{}",
        tasks[0].ticket.title
    );
    assert!(tasks[0].is_open());

    // A package is sold: the job takes both credits and the task closes.
    t.packages.give(PartyId(p), ITEM, 5);
    let done = credits::settle_pending(t.state()).await.unwrap();
    assert_eq!(done, 2);
    assert_eq!(t.packages.left(PartyId(p), ITEM), 3);
    assert!(row(&t, &first, &ep).await.consumed);
    assert!(!t.tasks.with_key("edtech.credits")[0].is_open());
}

/// A tutor-sick booking cancelled by the centre consumes nothing: the centre
/// cancels it with the reason `tutor_unavailable` (business-initiated) and no
/// credit is asked of the package ledger.
#[renox::test]
async fn edu_15_a_booking_the_centre_cancels_consumes_nothing() {
    let t = boot().await;
    let p = t.person(1, "Sara Ali");
    t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await;
    let tutor = t.tutor("Layla").await;
    let mut lesson = ServiceType::new("one_to_one_maths", "One-to-one maths", 60);
    lesson.save(t.db()).await.unwrap();
    resource::offer(t.db(), lesson.id, tutor.id).await.unwrap();
    for weekday in 1..=7 {
        AvailabilityRule::new(tutor.id, weekday, 0, 1440)
            .save(t.db())
            .await
            .unwrap();
    }
    let booking = service::book(
        t.state(),
        NewBooking::new(PartyId(p), lesson.id, &[tutor.id], at(day(10), 10, 0)),
        Actor::staff(1),
        renox::db::now(),
    )
    .await
    .expect("the lesson is booked");

    let cancelled = service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("tutor_unavailable"),
        Actor::staff(1),
        renox::db::now(),
    )
    .await
    .expect("the centre cancels it");

    assert_eq!(cancelled.status, wl_scheduling::BookingStatus::Cancelled);
    assert!(t.packages.asked().is_empty());
    assert_eq!(t.packages.left(PartyId(p), ITEM), 10);
}

/// A student absent two sessions in a row: the coordinator and the guardian are
/// told once for that run; a third absence adds nothing; being there starts the
/// count again.
#[renox::test]
async fn edu_15_two_absences_in_a_row_tell_the_coordinator_and_the_guardian() {
    let t = boot().await;
    let batch = batch(&t).await;
    let coordinator = t
        .holder(
            "Cora Coordinator",
            edtech::permissions::COORDINATOR_ROLE,
            None,
        )
        .await;
    let child = t.person(1, "Omar Child");
    let mother = t.person(2, "Mother");
    t.people.guardian(child, mother);
    let e = t.package_in(child, ITEM, 10, EnrolmentStatus::Active).await;
    Enrolment::query()
        .where_(Enrolment::ID.eq(e.id))
        .update(t.db(), &[("guardian_party_id", &Some(mother))])
        .await
        .unwrap();
    let desk = t.desk().await;
    let tariq = t.teacher().await;
    let mut all = Vec::new();
    for (number, ago) in [(1, 5), (2, 4), (3, 3), (4, 2)] {
        let session = t.past_session(&batch, number, ago).await;
        roster::add_student(t.state(), &desk, session.id, e.id)
            .await
            .unwrap();
        all.push(session);
    }

    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        all[0].id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    assert!(
        t.messages.of_type(FAMILY_NOTICE).is_empty(),
        "one absence is not a run"
    );
    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        all[1].id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();

    let family = t.messages.of_type(FAMILY_NOTICE);
    assert_eq!(family.len(), 1);
    assert!(Messages::told_party(&family[0], mother));
    let staff = t.messages.of_type(NOTICE);
    assert_eq!(staff.len(), 1);
    assert!(Messages::told_user(&staff[0], coordinator.id));
    assert!(Messages::text(&family[0]).contains("2 sessions in a row"));
    assert!(row(&t, &all[1], &e).await.alert_sent_at.is_some());

    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        all[2].id,
        &[(e.id, Mark::Absent)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        t.messages.of_type(FAMILY_NOTICE).len(),
        1,
        "the same run is told once"
    );
    assert_eq!(
        edtech::absences::absence_streak(t.db(), e.id)
            .await
            .unwrap(),
        3
    );
    sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        all[3].id,
        &[(e.id, Mark::Present)],
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        edtech::absences::absence_streak(t.db(), e.id)
            .await
            .unwrap(),
        0
    );
}

/// Every student must be marked when the tutor marks them one by one; the page
/// starts with everybody present and posts a mark per student.
#[renox::test]
async fn edu_15_every_student_is_marked_and_the_page_posts_the_marks() {
    let t = boot().await;
    let batch = batch(&t).await;
    let session = t.past_session(&batch, 1, 2).await;
    let (p, q) = (t.person(1, "Sara Ali"), t.person(2, "Omar Khan"));
    let (ep, eq) = (
        t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await,
        t.package_in(q, ITEM, 10, EnrolmentStatus::Active).await,
    );
    let desk = t.desk().await;
    roster::add_student(t.state(), &desk, session.id, ep.id)
        .await
        .unwrap();
    roster::add_student(t.state(), &desk, session.id, eq.id)
        .await
        .unwrap();
    let tariq = t.teacher().await;

    let error = sessions::submit_attendance_marked(
        t.state(),
        &tariq,
        session.id,
        &[(ep.id, Mark::Present)],
        false,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Omar Khan"), "{error}");
    assert_eq!(
        Session::find_or_404(t.db(), session.id)
            .await
            .unwrap()
            .status,
        SessionStatus::Scheduled
    );

    let teacher = t.teacher_user().await;
    t.app.acting_as(&teacher);
    t.app
        .get(&format!("/edtech/sessions/{}", session.id))
        .await
        .assert_ok()
        .assert_see("Submit attendance")
        .assert_see("Sara Ali")
        .assert_see("Omar Khan");
    t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/attendance", session.id),
            &[
                (format!("mark_{}", ep.id).as_str(), "present"),
                (format!("mark_{}", eq.id).as_str(), "absent"),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(row(&t, &session, &ep).await.mark, Mark::Present);
    assert_eq!(row(&t, &session, &eq).await.mark, Mark::Absent);
    assert_eq!(
        t.packages.left(PartyId(p), ITEM) + t.packages.left(PartyId(q), ITEM),
        18
    );
}

/// The counsellor records a notice from the session page and the coordinator
/// excuses a late one; the page shows the notice.
#[renox::test]
async fn edu_15_the_notice_is_recorded_from_the_session_page() {
    let t = boot().await;
    let batch = batch(&t).await;
    let session = t.past_session(&batch, 1, 2).await;
    let p = t.person(1, "Sara Ali");
    let ep = t.package_in(p, ITEM, 10, EnrolmentStatus::Active).await;
    let desk_user = t.desk_user().await;
    let desk = t.auth(&desk_user).await;
    roster::add_student(t.state(), &desk, session.id, ep.id)
        .await
        .unwrap();
    t.app.acting_as(&desk_user);
    let given = format!("{} 09:00", session.session_date - TimeDelta::days(1));

    t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/notice", session.id),
            &[
                ("enrolment", ep.id.to_string().as_str()),
                ("given_at", given.as_str()),
            ],
        )
        .await
        .assert_ok();

    assert_eq!(row(&t, &session, &ep).await.mark, Mark::Excused);
    t.app
        .get(&format!("/edtech/sessions/{}", session.id))
        .await
        .assert_ok()
        .assert_see("Record a notice of absence")
        .assert_see(&given);
    // A bad time is refused in words.
    t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/notice", session.id),
            &[
                ("enrolment", ep.id.to_string().as_str()),
                ("given_at", "yesterday"),
            ],
        )
        .await
        .assert_status(422);
}
