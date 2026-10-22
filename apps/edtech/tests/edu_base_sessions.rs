//! The base of the 10 EdTech session (base for #443 .. #450): the sessions
//! of a batch are generated from its weekly plan around the holidays when it
//! opens, a tutor and a room are never in two places at once, the sessions tell
//! wl-scheduling when they are taken, and a session is rescheduled, taught,
//! corrected in its reason and cancelled through its machine.

mod support;

use edtech::batch_machine::{BatchInput, CHANGE_TUTOR};
use edtech::batches;
use edtech::attendance::Mark;
use edtech::corrections;
use edtech::session_machine::{CANCEL, CORRECT_ATTENDANCE};
use edtech::sessions;
use edtech::{Batch, BatchStatus, Session, SessionStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use support::{Centre, at, boot, boot_with_holidays, day, next_weekday};
use wl_kernel::scheduling::availability_source;

/// The Monday a week from now.
fn monday() -> renox::chrono::NaiveDate {
    next_weekday(day(7), 1)
}

/// A running batch (a fixture: it is inserted `running`) with the Dubai
/// tutor and room of the tests.
async fn running(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let planned = t.batch_of(&tutor, &room, monday(), &[1, 3]).await;
    let mut row = Batch {
        id: 0,
        status: BatchStatus::Running,
        ..planned
    };
    let mut tx = t.db().begin().await.unwrap();
    row.insert_in_state(&mut tx, BatchStatus::Running, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Batch::find_or_404(t.db(), row.id).await.unwrap()
}

/// Opening a batch generates a session for each date of its pattern, in date
/// order, with the batch's tutor and room, at 18:00 Dubai time for 90 minutes.
#[renox::test]
async fn edu_base_opening_a_batch_generates_its_sessions() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    assert_eq!(batch.status, BatchStatus::Open);

    let planned = t.sessions_of(&batch).await;
    // Four Mondays and four Wednesdays.
    assert_eq!(planned.len(), 8);
    let numbers: Vec<i64> = planned.iter().map(|s| s.number).collect();
    assert_eq!(numbers, (1..=8).collect::<Vec<i64>>());
    assert_eq!(planned[0].session_date, monday());
    assert_eq!(planned[1].session_date, monday() + TimeDelta::days(2));
    assert_eq!(planned[0].starts_at, at(monday(), 18, 0));
    assert_eq!(planned[0].ends_at, at(monday(), 19, 30));
    assert!(planned.iter().all(|s| s.status == SessionStatus::Scheduled));
    assert!(planned.iter().all(|s| s.tutor_resource_id == Some(tutor.id)));
    assert!(planned.iter().all(|s| s.room_resource_id == Some(room.id)));
}

/// A public holiday of the calendar is skipped and nothing is added after the
/// end date.
#[renox::test]
async fn edu_base_a_public_holiday_gets_no_session() {
    let holiday = monday() + TimeDelta::days(2);
    let t = boot_with_holidays(&[holiday]).await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;

    let planned = t.sessions_of(&batch).await;
    assert_eq!(planned.len(), 7);
    assert!(planned.iter().all(|s| s.session_date != holiday));
    assert!(planned.iter().all(|s| s.session_date <= batch.end_date));
}

/// A batch whose tutor or room is already in a session at one of its dates
/// does not open, and the refusal names the days; it stays planned with no
/// session.
#[renox::test]
async fn edu_base_a_tutor_or_room_cannot_be_in_two_places_at_once() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let other_tutor = t.tutor("Omar").await;
    let room = t.room("Room 1", 14).await;
    let other_room = t.room("Room 2", 10).await;
    t.open_batch(&tutor, &room, monday()).await;
    let amal = t.coordinator().await;

    // The same tutor, another room, the same Mondays.
    let same_tutor = t.batch_of(&tutor, &other_room, monday(), &[1]).await;
    let refused = batches::open(t.state(), &amal, same_tutor.id).await;
    match refused {
        Err(Error::Status(status, message)) => {
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
            assert!(message.contains(&monday().to_string()), "{message}");
        }
        other => panic!("the batch opened or failed otherwise: {other:?}"),
    }
    // The same room, another tutor.
    let same_room = t.batch_of(&other_tutor, &room, monday(), &[3]).await;
    assert!(batches::open(t.state(), &amal, same_room.id).await.is_err());
    for batch in [&same_tutor, &same_room] {
        let still = Batch::find_or_404(t.db(), batch.id).await.unwrap();
        assert_eq!(still.status, BatchStatus::Planned);
        assert!(t.sessions_of(batch).await.is_empty());
    }
    // Another tutor in another room opens.
    let free = t.batch_of(&other_tutor, &other_room, monday(), &[1, 3]).await;
    batches::open(t.state(), &amal, free.id).await.expect("it opens");
}

/// The sessions are blocks of their tutor and room for wl-scheduling, with
/// the session as the reference; another resource and a cancelled session
/// make none.
#[renox::test]
async fn edu_base_the_sessions_block_their_tutor_and_room_in_scheduling() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let elsewhere = t.room("Room 3", 8).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let planned = t.sessions_of(&batch).await;
    let source = availability_source(t.state());
    let from = at(monday(), 0, 0);
    let until = at(monday() + TimeDelta::days(1), 0, 0);

    for resource in [tutor.id, room.id] {
        let blocks = source
            .blocks(t.state(), resource, None, from, until)
            .await
            .unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].starts_at, planned[0].starts_at);
        assert_eq!(blocks[0].ends_at, planned[0].ends_at);
        assert_eq!(blocks[0].source, "session");
        assert_eq!(blocks[0].reference, format!("edtech.session:{}", planned[0].id));
    }
    let none = source
        .blocks(t.state(), elsewhere.id, None, from, until)
        .await
        .unwrap();
    assert!(none.is_empty());

    let amal = t.coordinator().await;
    sessions::cancel(t.state(), &amal, planned[0].id, "tutor sick", None)
        .await
        .unwrap();
    let after = source
        .blocks(t.state(), tutor.id, None, from, until)
        .await
        .unwrap();
    assert!(after.is_empty());
}

/// A session moves to another day or time and keeps its length; a time taken
/// by the same tutor in another session, and a time that is not `HH:MM`, are
/// refused.
#[renox::test]
async fn edu_base_a_session_is_rescheduled_when_the_tutor_and_room_are_free() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let planned = t.sessions_of(&batch).await;
    let amal = t.coordinator().await;

    let day_after = monday() + TimeDelta::days(1);
    let moved = sessions::reschedule(t.state(), &amal, planned[0].id, day_after, "17:00")
        .await
        .expect("it moves");
    assert_eq!(moved.session_date, day_after);
    assert_eq!(moved.starts_at, at(day_after, 17, 0));
    assert_eq!(moved.ends_at, at(day_after, 18, 30));
    assert_eq!(moved.status, SessionStatus::Scheduled);

    // The Wednesday session at 18:00 holds the tutor.
    let wednesday = monday() + TimeDelta::days(2);
    let taken = sessions::reschedule(t.state(), &amal, planned[0].id, wednesday, "18:30").await;
    assert!(matches!(taken, Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))));
    let bad = sessions::reschedule(t.state(), &amal, planned[0].id, wednesday, "evening").await;
    assert!(bad.is_err());
    let unchanged = Session::find_or_404(t.db(), planned[0].id).await.unwrap();
    assert_eq!(unchanged.session_date, day_after);
}

/// One session is taught by another tutor if that tutor is free; the batch's
/// other sessions keep theirs.
#[renox::test]
async fn edu_base_one_session_changes_tutor() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let omar = t.tutor("Omar").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let planned = t.sessions_of(&batch).await;
    let amal = t.coordinator().await;

    let moved = sessions::change_tutor(t.state(), &amal, planned[0].id, omar.id, None)
        .await
        .expect("Omar is free");
    assert_eq!(moved.tutor_resource_id, Some(omar.id));
    let rest = t.sessions_of(&batch).await;
    assert!(rest[1..].iter().all(|s| s.tutor_resource_id == Some(tutor.id)));

    // Omar teaches elsewhere on the first Wednesday.
    let other_room = t.room("Room 2", 10).await;
    let elsewhere = t.batch_of(&omar, &other_room, monday() + TimeDelta::days(2), &[3]).await;
    batches::open(t.state(), &amal, elsewhere.id).await.unwrap();
    let clash = sessions::change_tutor(t.state(), &amal, planned[1].id, omar.id, None).await;
    assert!(clash.is_err());
}

/// The batch's `change_tutor` gives the new tutor the sessions of the old one
/// from the date, not the sessions before it, and not a session already moved
/// to another tutor.
#[renox::test]
async fn edu_base_the_batch_changes_tutor_from_a_date() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let omar = t.tutor("Omar").await;
    let sami = t.tutor("Sami").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let amal = t.coordinator().await;
    let planned = t.sessions_of(&batch).await;
    // The last session already belongs to Sami.
    sessions::change_tutor(t.state(), &amal, planned[7].id, sami.id, None)
        .await
        .unwrap();

    let from = monday() + TimeDelta::days(7);
    let input = BatchInput {
        from: Some(from),
        tutor_resource_id: Some(omar.id),
        tutor_employee_id: Some(5),
        ..BatchInput::default()
    };
    batch
        .transition(CHANGE_TUTOR)
        .by(&amal)
        .input(&input)
        .run(t.state())
        .await
        .expect("the tutor changes");

    let changed = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(changed.tutor_resource_id, Some(omar.id));
    assert_eq!(changed.status, BatchStatus::Open);
    let rest = t.sessions_of(&batch).await;
    for session in &rest {
        let expected = if session.number == 8 {
            sami.id
        } else if session.session_date >= from {
            omar.id
        } else {
            tutor.id
        };
        assert_eq!(session.tutor_resource_id, Some(expected), "session {}", session.number);
    }
}

/// A tutor submits the attendance of a session that has started: it is held,
/// with the time and the user; before it started, or without `sessions.teach`,
/// it is refused.
#[renox::test]
async fn edu_base_a_tutor_submits_the_attendance_of_a_session_that_started() {
    let t = boot().await;
    let batch = running(&t).await;
    let past = t.past_session(&batch, 1, 1).await;
    let tariq = t.teacher().await;

    let held = sessions::submit_attendance(t.state(), &tariq, past.id)
        .await
        .expect("it is held");
    assert_eq!(held.status, SessionStatus::Held);
    assert!(held.held_at.is_some());
    assert_eq!(held.attendance_by, Some(tariq.id));

    // A session in the future has not started.
    let mut later = Session {
        number: 2,
        session_date: day(3),
        starts_at: at(day(3), 10, 0),
        ends_at: at(day(3), 11, 0),
        ..past.clone()
    };
    later.id = 0;
    later.status = SessionStatus::Scheduled;
    later.held_at = None;
    later.attendance_by = None;
    later.save(t.db()).await.unwrap();
    let early = sessions::submit_attendance(t.state(), &tariq, later.id).await;
    assert!(matches!(early, Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _))));

    // A counsellor does not teach.
    let cara = t.counsellor().await;
    let other = t.past_session(&batch, 3, 2).await;
    let refused = sessions::submit_attendance(t.state(), &cara, other.id).await;
    assert!(matches!(refused, Err(Error::Forbidden)));
}

/// Correcting the attendance of a held session needs a reason, and it leaves a
/// held session held (BL-EDU-14 gives the correction its body).
#[renox::test]
async fn edu_base_correcting_attendance_needs_a_reason() {
    let t = boot().await;
    let batch = running(&t).await;
    let past = t.past_session(&batch, 1, 1).await;
    let student = t.person(1, "Sara Ali");
    let enrolment = t.enrolment_in(&batch, student, edtech::EnrolmentStatus::Active).await;
    let tariq = t.teacher().await;
    let amal = t.coordinator().await;
    sessions::submit_attendance(t.state(), &tariq, past.id).await.unwrap();

    let bare = corrections::correct_attendance(
        t.state(),
        &amal,
        past.id,
        &[(enrolment.id, Mark::Absent)],
        " ",
    )
    .await;
    assert!(bare.is_err(), "a reason is needed");
    let bare_run = Session::find_or_404(t.db(), past.id)
        .await
        .unwrap()
        .transition(CORRECT_ATTENDANCE)
        .by(&tariq)
        .run(t.state())
        .await;
    assert!(bare_run.is_err(), "the machine refuses a bare run too");
    corrections::correct_attendance(
        t.state(),
        &amal,
        past.id,
        &[(enrolment.id, Mark::Absent)],
        "marked Sara absent by mistake",
    )
    .await
    .expect("it is corrected");
    let again = Session::find_or_404(t.db(), past.id).await.unwrap();
    assert_eq!(again.status, SessionStatus::Held);
}

/// The batch completes when its last open session is held or cancelled, and
/// not before.
#[renox::test]
async fn edu_base_the_batch_completes_after_its_last_session() {
    let t = boot().await;
    let batch = running(&t).await;
    let first = t.past_session(&batch, 1, 2).await;
    let second = t.past_session(&batch, 2, 1).await;
    let tariq = t.teacher().await;
    let amal = t.coordinator().await;

    sessions::submit_attendance(t.state(), &tariq, first.id).await.unwrap();
    let still = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(still.status, BatchStatus::Running);
    // The coordinator cannot complete it by hand while a session is open.
    assert!(batches::complete(t.state(), Some(&amal), batch.id).await.is_err());

    sessions::cancel(t.state(), &amal, second.id, "tutor sick", None)
        .await
        .unwrap();
    let done = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(done.status, BatchStatus::Completed);
}

/// Only a holder of `batches.manage` completes a batch by hand, and only when
/// every session is closed; the daily run completes a running batch whose
/// sessions are all closed.
#[renox::test]
async fn edu_base_a_batch_is_completed_by_hand_or_by_the_daily_run() {
    let t = boot().await;
    let batch = running(&t).await;
    let held = t.past_session(&batch, 1, 1).await;
    let tariq = t.teacher().await;
    let amal = t.coordinator().await;
    // Two sessions: the second keeps the batch running after the first is held.
    let open = t.past_session(&batch, 2, 1).await;
    sessions::submit_attendance(t.state(), &tariq, held.id).await.unwrap();
    assert!(matches!(
        batches::complete(t.state(), Some(&tariq), batch.id).await,
        Err(Error::Forbidden)
    ));
    assert!(batches::complete(t.state(), Some(&amal), batch.id).await.is_err());

    sessions::submit_attendance(t.state(), &tariq, open.id).await.unwrap();
    let done = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(done.status, BatchStatus::Completed);

    // A second running batch with every session closed already.
    let other = running(&t).await;
    let closed = t.past_session(&other, 1, 3).await;
    // The status column is the machine's; the fixture closes the session the
    // way an import would.
    renox::db::sql("UPDATE edtech_sessions SET status = 'held' WHERE id = ?")
        .bind(closed.id)
        .execute(t.db())
        .await
        .unwrap();
    batches::run_daily(t.state()).await.unwrap();
    let swept = Batch::find_or_404(t.db(), other.id).await.unwrap();
    assert_eq!(swept.status, BatchStatus::Completed);
}

/// Cancelling a session needs a reason; with a make-up day, a session is
/// added to the batch on that day with the next number, the same time, tutor
/// and room, and a make-up day with the tutor taken is refused with the
/// session left as it was.
#[renox::test]
async fn edu_base_a_cancelled_session_can_have_a_make_up() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let planned = t.sessions_of(&batch).await;
    let amal = t.coordinator().await;

    let blank = sessions::cancel(t.state(), &amal, planned[0].id, " ", None).await;
    assert!(blank.is_err());
    let untouched = Session::find_or_404(t.db(), planned[0].id).await.unwrap();
    assert_eq!(untouched.status, SessionStatus::Scheduled);

    // The Wednesday session holds the tutor.
    let taken = sessions::cancel(
        t.state(),
        &amal,
        planned[0].id,
        "room flooded",
        Some(monday() + TimeDelta::days(2)),
    )
    .await;
    assert!(taken.is_err());
    let untouched = Session::find_or_404(t.db(), planned[0].id).await.unwrap();
    assert_eq!(untouched.status, SessionStatus::Scheduled);

    // A Friday is free.
    let friday = monday() + TimeDelta::days(4);
    let cancelled = sessions::cancel(t.state(), &amal, planned[0].id, "room flooded", Some(friday))
        .await
        .expect("it is cancelled");
    assert_eq!(cancelled.status, SessionStatus::Cancelled);
    assert_eq!(cancelled.cancel_reason.as_deref(), Some("room flooded"));
    let all = t.sessions_of(&batch).await;
    assert_eq!(all.len(), 9);
    let added = all.iter().find(|s| s.make_up).expect("a make-up session");
    assert_eq!(added.number, 9);
    assert_eq!(added.session_date, friday);
    assert_eq!(added.starts_at, at(friday, 18, 0));
    assert_eq!(added.ends_at, at(friday, 19, 30));
    assert_eq!(added.tutor_resource_id, Some(tutor.id));
    assert_eq!(added.room_resource_id, Some(room.id));
    assert_eq!(added.status, SessionStatus::Scheduled);
}

/// `cancel` of a session leaves a cancelled session cancelled: it is terminal.
#[renox::test]
async fn edu_base_a_cancelled_session_cannot_be_cancelled_again() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let planned = t.sessions_of(&batch).await;
    let amal = t.coordinator().await;
    sessions::cancel(t.state(), &amal, planned[0].id, "holiday", None)
        .await
        .unwrap();
    let cancelled = Session::find_or_404(t.db(), planned[0].id).await.unwrap();
    let again = cancelled.transition(CANCEL).by(&amal).reason("again").run(t.state()).await;
    assert!(matches!(again, Err(TransitionError::NotFrom(_))));
}

/// The session pages: the list names the next sessions, the page of a session
/// offers the coordinator reschedule, change of tutor and cancel, and the
/// tutor submits attendance when it has started.
#[renox::test]
async fn edu_base_the_session_pages_show_the_sessions_and_their_actions() {
    let t = boot().await;
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.open_batch(&tutor, &room, monday()).await;
    let planned = t.sessions_of(&batch).await;
    let amal = t.coordinator_user().await;
    t.app.acting_as(&amal);

    t.app
        .get("/edtech/sessions")
        .await
        .assert_ok()
        .assert_see(&monday().to_string())
        .assert_see("18:00 – 19:30")
        .assert_see("Scheduled");
    t.app
        .get(&format!("/edtech/batches/{}", batch.id))
        .await
        .assert_ok()
        .assert_see("18:00 – 19:30");
    t.app
        .get(&format!("/edtech/sessions/{}", planned[0].id))
        .await
        .assert_ok()
        .assert_see("Reschedule")
        .assert_see("Cancel session");

    let tuesday = (monday() + TimeDelta::days(1)).to_string();
    t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/reschedule", planned[0].id),
            &[("date", tuesday.as_str()), ("start_time", "16:00")],
        )
        .await
        .assert_ok();
    let moved = Session::find_or_404(t.db(), planned[0].id).await.unwrap();
    assert_eq!(moved.starts_at, at(monday() + TimeDelta::days(1), 16, 0));

    t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/cancel", planned[1].id),
            &[("reason", "public holiday"), ("make_up_on", "")],
        )
        .await
        .assert_ok();
    let cancelled = Session::find_or_404(t.db(), planned[1].id).await.unwrap();
    assert_eq!(cancelled.status, SessionStatus::Cancelled);

    // A tutor sees the page but not the coordinator's actions, and cannot
    // cancel.
    let tariq = t.teacher_user().await;
    t.app.acting_as(&tariq);
    t.app
        .get(&format!("/edtech/sessions/{}", planned[2].id))
        .await
        .assert_ok()
        .assert_dont_see("Cancel session");
    t.app
        .htmx()
        .post(
            &format!("/edtech/sessions/{}/cancel", planned[2].id),
            &[("reason", "no"), ("make_up_on", "")],
        )
        .await
        .assert_forbidden();
}

/// A tutor submits attendance from the page of a session that has started.
#[renox::test]
async fn edu_base_the_tutor_submits_attendance_from_the_session_page() {
    let t = boot().await;
    let batch = running(&t).await;
    let past = t.past_session(&batch, 1, 1).await;
    let _later = t.past_session(&batch, 2, 2).await;
    let tariq = t.teacher_user().await;
    t.app.acting_as(&tariq);
    t.app
        .get(&format!("/edtech/sessions/{}", past.id))
        .await
        .assert_ok()
        .assert_see("Submit attendance");
    t.app
        .htmx()
        .post(&format!("/edtech/sessions/{}/attendance", past.id), &[("x", "y")])
        .await
        .assert_ok();
    let held = Session::find_or_404(t.db(), past.id).await.unwrap();
    assert_eq!(held.status, SessionStatus::Held);
}
