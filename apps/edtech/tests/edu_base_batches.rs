//! The base of the 10 EdTech course and batch (base for #437, #447, #448):
//! a course is defined, a batch is planned and opens, starts when its first
//! session date comes, and takes its seats up to the capacity. `cancel` and
//! `terminate` are declared with their reason.

mod support;

use edtech::batch_machine::{CANCEL, OPEN, START, TERMINATE};
use edtech::batches::{self, NewBatch};
use edtech::courses::{self, NewCourse};
use edtech::seats::{self, Limit};
use edtech::{Batch, BatchStatus, Course, EnrolmentStatus};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use support::{Centre, boot, day, next_weekday};

/// A course and a batch of it planned by the coordinator: Monday and Wednesday
/// at 18:00 for 90 minutes, 12 seats, over four weeks from next Monday.
async fn planned(t: &Centre) -> (Course, Batch) {
    let amal = t.coordinator().await;
    let course = match Course::by_code(t.db(), "IELTS-B2").await.unwrap() {
        Some(found) => found,
        None => courses::create(
            t.state(),
            &amal,
            NewCourse::new("IELTS-B2", "IELTS preparation", "English language", 40).level("B2"),
        )
        .await
        .expect("the course is defined"),
    };
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let first = next_weekday(day(7), 1);
    let batch = batches::create(
        t.state(),
        &amal,
        NewBatch::new(
            course.id,
            12,
            (first, first + renox::chrono::TimeDelta::days(27)),
            &[3, 1],
            "18:00",
            90,
        )
        .tutor(tutor.id, None)
        .room(room.id, Some(14)),
    )
    .await
    .expect("the batch is planned");
    (course, batch)
}

/// A batch in `status`, inserted the way a fixture is.
async fn batch_in(t: &Centre, status: BatchStatus) -> Batch {
    let (_course, batch) = planned(t).await;
    let mut row = Batch {
        id: 0,
        status,
        ..batch
    };
    let mut tx = t.db().begin().await.unwrap();
    row.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Batch::find_or_404(t.db(), row.id).await.unwrap()
}

/// A course is defined with its programme and hours; its code is unique and
/// only a holder of `courses.manage` defines one.
#[renox::test]
async fn edu_base_a_course_is_defined_once_by_the_coordinator() {
    let t = boot().await;
    let amal = t.coordinator().await;
    let course = courses::create(
        t.state(),
        &amal,
        NewCourse::new(" CMA-1 ", "Management accounting", "Finance", 60).for_minors(),
    )
    .await
    .expect("the course is defined");
    assert_eq!(course.code, "CMA-1");
    assert_eq!(course.hours, 60);
    assert!(course.for_minors && course.active);
    assert_eq!(
        course.subject().to_string(),
        format!("edtech.course:{}", course.id)
    );

    let again = courses::create(
        t.state(),
        &amal,
        NewCourse::new("CMA-1", "Another", "Finance", 10),
    )
    .await;
    assert!(matches!(
        again,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));

    let tariq = t.teacher().await;
    let refused = courses::create(
        t.state(),
        &tariq,
        NewCourse::new("X-1", "Not mine", "Finance", 10),
    )
    .await;
    assert!(matches!(refused, Err(Error::Forbidden)));
}

/// The regulator's approval number is recorded on the course.
#[renox::test]
async fn edu_base_the_regulator_approval_is_recorded_on_the_course() {
    let t = boot().await;
    let amal = t.coordinator().await;
    let course = courses::create(
        t.state(),
        &amal,
        NewCourse::new("KHDA-1", "Approved course", "Safety", 8).approval(None),
    )
    .await
    .unwrap();
    assert!(course.approval_required && course.approval_no.is_none());
    let course = courses::record_approval(t.state(), &amal, course.id, " KHDA/2026/117 ")
        .await
        .unwrap();
    assert_eq!(course.approval_no.as_deref(), Some("KHDA/2026/117"));
    assert!(
        courses::record_approval(t.state(), &amal, course.id, " ")
            .await
            .is_err()
    );
}

/// A batch is planned: it starts `planned` with no seat taken, and its weekly
/// pattern is kept sorted.
#[renox::test]
async fn edu_base_a_batch_is_planned_with_no_seat_taken() {
    let t = boot().await;
    let (course, batch) = planned(&t).await;
    assert_eq!(batch.status, BatchStatus::Planned);
    assert_eq!(batch.course_id, course.id);
    assert_eq!(batch.weekday_list(), [1, 3]);
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 12);
    assert_eq!(batch.number(), format!("BAT-{:04}", batch.id));
}

/// A pattern that gives no session, or a batch without a seat, is refused
/// when the batch is planned.
#[renox::test]
async fn edu_base_a_batch_without_a_session_is_not_planned() {
    let t = boot().await;
    let (course, _batch) = planned(&t).await;
    let amal = t.coordinator().await;
    let first = next_weekday(day(7), 1);
    // Tuesday only, in a window that holds one Monday.
    let none = batches::create(
        t.state(),
        &amal,
        NewBatch::new(course.id, 5, (first, first), &[2], "18:00", 60),
    )
    .await;
    assert!(none.is_err());
    let seatless = batches::create(
        t.state(),
        &amal,
        NewBatch::new(
            course.id,
            0,
            (first, first + renox::chrono::TimeDelta::days(7)),
            &[1],
            "18:00",
            60,
        ),
    )
    .await;
    assert!(seatless.is_err());
}

/// `open`: the coordinator opens a planned batch; it is then `open` with a
/// history row, and a second open is refused.
#[renox::test]
async fn edu_base_the_coordinator_opens_a_planned_batch() {
    let t = boot().await;
    let (_course, batch) = planned(&t).await;
    let amal = t.coordinator().await;

    let opened = batches::open(t.state(), &amal, batch.id)
        .await
        .expect("it opens");
    assert_eq!(opened.status, BatchStatus::Open);
    let history = renox::state_machine::history(t.db(), "edtech_batches", batch.id)
        .await
        .unwrap();
    assert_eq!(
        history.last().map(|row| row.transition.as_str()),
        Some(OPEN)
    );

    assert!(batches::open(t.state(), &amal, batch.id).await.is_err());
}

/// A batch without a tutor does not open: the plan is incomplete.
#[renox::test]
async fn edu_base_a_batch_without_a_tutor_does_not_open() {
    let t = boot().await;
    let (course, _batch) = planned(&t).await;
    let amal = t.coordinator().await;
    let first = next_weekday(day(7), 1);
    let batch = batches::create(
        t.state(),
        &amal,
        NewBatch::new(
            course.id,
            8,
            (first, first + renox::chrono::TimeDelta::days(13)),
            &[1],
            "10:00",
            60,
        ),
    )
    .await
    .unwrap();
    let refused = batches::open(t.state(), &amal, batch.id).await;
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    let still = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(still.status, BatchStatus::Planned);
}

/// A tutor holds `sessions.teach`, not `batches.manage`: they cannot open a
/// batch.
#[renox::test]
async fn edu_base_a_tutor_cannot_open_a_batch() {
    let t = boot().await;
    let (_course, batch) = planned(&t).await;
    let tariq = t.teacher().await;
    let refused = batches::open(t.state(), &tariq, batch.id).await;
    assert!(matches!(refused, Err(Error::Forbidden)));
}

/// `start`: an open batch is running from its first session date, not before.
#[renox::test]
async fn edu_base_an_open_batch_starts_on_its_first_session_date() {
    let t = boot().await;
    let (_course, batch) = planned(&t).await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, batch.id).await.unwrap();

    let before = batches::start_due(
        t.state(),
        batch.start_date - renox::chrono::TimeDelta::days(1),
    )
    .await
    .unwrap();
    assert!(before.is_empty());
    let started = batches::start_due(t.state(), batch.start_date)
        .await
        .unwrap();
    assert_eq!(started, [batch.id]);
    let running = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(running.status, BatchStatus::Running);
    // A second run changes nothing.
    assert!(
        batches::start_due(t.state(), batch.start_date)
            .await
            .unwrap()
            .is_empty()
    );
    let history = renox::state_machine::history(t.db(), "edtech_batches", batch.id)
        .await
        .unwrap();
    assert_eq!(
        history.last().map(|row| row.transition.as_str()),
        Some(START)
    );
}

/// `cancel` is for a planned or open batch and needs a reason and the choice
/// per enrolment the service checks; `terminate` only leaves a running batch and
/// needs the centre manager's approval (BL-EDU-13 and BL-EDU-12 test the bodies).
#[renox::test]
async fn edu_base_cancel_and_terminate_are_declared_with_their_reason() {
    let t = boot().await;
    let amal = t.coordinator().await;

    let planned_batch = batch_in(&t, BatchStatus::Planned).await;
    // The transition alone is refused: nobody checked the choices.
    let unchecked = planned_batch
        .transition(CANCEL)
        .by(&amal)
        .reason("too few students")
        .run(t.state())
        .await;
    assert!(matches!(unchecked, Err(TransitionError::Guard(_))));
    let missing = batches::cancel(t.state(), &amal, planned_batch.id, " ", Vec::new()).await;
    assert!(missing.is_err());
    batches::cancel(
        t.state(),
        &amal,
        planned_batch.id,
        "too few students",
        Vec::new(),
    )
    .await
    .expect("a planned batch is cancelled");
    let cancelled = Batch::find_or_404(t.db(), planned_batch.id).await.unwrap();
    assert_eq!(cancelled.status, BatchStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("too few students"));

    let open_batch = batch_in(&t, BatchStatus::Open).await;
    let early = open_batch
        .transition(TERMINATE)
        .by(&amal)
        .reason("venue")
        .run(t.state())
        .await;
    assert!(matches!(early, Err(TransitionError::NotFrom(_))));

    let running = batch_in(&t, BatchStatus::Running).await;
    let unapproved = running
        .transition(TERMINATE)
        .by(&amal)
        .reason("tutor unavailable")
        .run(t.state())
        .await;
    assert!(matches!(unapproved, Err(TransitionError::Guard(_))));
    assert_eq!(
        Batch::find_or_404(t.db(), running.id).await.unwrap().status,
        BatchStatus::Running
    );
}

/// Seats are counted from the enrolments: twelve holders fill a batch of
/// twelve, the next taker is refused, and the seat of a lapsed hold can be
/// taken again.
#[renox::test]
async fn edu_base_seats_are_taken_up_to_the_capacity() {
    let t = boot().await;
    let (_course, batch) = planned(&t).await;
    for n in 0..11 {
        t.enrolment_in(&batch, 100 + n, EnrolmentStatus::Active)
            .await;
    }
    let held = t
        .enrolment_in(&batch, 200, EnrolmentStatus::PendingPayment)
        .await;
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 0);

    let mut tx = t.db().begin_immediate().await.unwrap();
    let full = seats::take(&mut tx, batch.id, 1, Limit::Capacity).await;
    assert!(matches!(
        full,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))
    ));
    tx.rollback().await.unwrap();

    t.lapse(&held).await;
    assert_eq!(seats::available(t.db(), batch.id).await.unwrap(), 1);
    let mut tx = t.db().begin_immediate().await.unwrap();
    seats::take(&mut tx, batch.id, 1, Limit::Capacity)
        .await
        .expect("the freed seat is free");
    tx.commit().await.unwrap();
}

/// Only the room limit goes over the capacity (14 here), never beyond it.
#[renox::test]
async fn edu_base_the_room_limit_is_the_only_way_past_the_capacity() {
    let t = boot().await;
    let (_course, batch) = planned(&t).await;
    for n in 0..12 {
        t.enrolment_in(&batch, 100 + n, EnrolmentStatus::Active)
            .await;
    }

    let mut tx = t.db().begin_immediate().await.unwrap();
    assert!(
        seats::take(&mut tx, batch.id, 1, Limit::Capacity)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    for n in 0..2 {
        t.enrolment_in(&batch, 300 + n, EnrolmentStatus::Active)
            .await;
    }
    let mut tx = t.db().begin_immediate().await.unwrap();
    assert!(
        seats::take(&mut tx, batch.id, 1, Limit::Room)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
}

/// The batch pages: the list names the batch with its seats and status in
/// words, the page offers `open` to the coordinator, and a counsellor only
/// looks.
#[renox::test]
async fn edu_base_the_batch_pages_show_the_batch_and_its_actions() {
    let t = boot().await;
    let (_course, batch) = planned(&t).await;
    let amal = t.coordinator_user().await;
    t.app.acting_as(&amal);
    t.app
        .get("/edtech/batches")
        .await
        .assert_ok()
        .assert_see("BAT-")
        .assert_see("0 of 12")
        .assert_see("Planned");
    t.app
        .get(&format!("/edtech/batches/{}", batch.id))
        .await
        .assert_ok()
        .assert_see("Open for enrolment")
        .assert_see("Mon, Wed 18:00");

    t.app
        .htmx()
        .post(&format!("/edtech/batches/{}/open", batch.id), &[("x", "y")])
        .await
        .assert_ok();
    let opened = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(opened.status, BatchStatus::Open);

    let cara = t.counsellor_user().await;
    t.app.acting_as(&cara);
    t.app
        .get("/edtech/batches")
        .await
        .assert_ok()
        .assert_see("Open");
    t.app
        .htmx()
        .post(&format!("/edtech/batches/{}/open", batch.id), &[("x", "y")])
        .await
        .assert_forbidden();
}

/// The course page lists the course with its approval, and the form defines
/// one.
#[renox::test]
async fn edu_base_the_course_page_lists_and_defines_courses() {
    let t = boot().await;
    let amal = t.coordinator_user().await;
    t.app.acting_as(&amal);
    t.app
        .htmx()
        .post(
            "/edtech/courses",
            &[
                ("code", "ARB-1"),
                ("name_en", "Arabic for beginners"),
                ("programme", "Languages"),
                ("hours", "30"),
                ("delivery", "hybrid"),
                ("approval_no", "KHDA/9"),
            ],
        )
        .await
        .assert_ok();
    t.app
        .get("/edtech/courses")
        .await
        .assert_ok()
        .assert_see("ARB-1")
        .assert_see("Arabic for beginners")
        .assert_see("KHDA/9");
    assert!(Course::by_code(t.db(), "ARB-1").await.unwrap().is_some());
}
