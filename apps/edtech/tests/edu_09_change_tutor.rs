//! BL-EDU-09 (#444): the coordinator changes the tutor of a batch from a date,
//! or of one session. The new tutor must be free and eligible on every date, all
//! or nothing; the sessions of the batch's tutor are reassigned under a lock on
//! the new tutor's calendar; each student or guardian gets one message with no
//! reason; the flags of `EDU.RUL.12` on the sessions moved are cleared; a session
//! already moved to a third tutor keeps theirs; `held` and `cancelled` sessions
//! are not touched.

mod support;

use edtech::batches;
use edtech::notices::FAMILY_NOTICE;
use edtech::sessions;
use edtech::tutors;
use edtech::enrolments::{self, NewEnrolment};
use edtech::{Batch, BatchStatus, Session, SessionStatus, TutorAssignment};
use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use support::{Centre, Messages, PRICE, boot, day, next_weekday};
use wl_scheduling::Resource;

/// The first Monday of the batches.
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

/// A tutor who is an employee and holds a police clearance for a long time.
async fn cleared_tutor(t: &Centre, name: &str) -> (Resource, i64) {
    let (resource, employee) = t.employed_tutor(name, false).await;
    t.documents.set(
        &Centre::employee_subject(employee),
        "police_clearance",
        Some(day(400)),
    );
    (resource, employee)
}

/// An open batch of the adult course on Monday, Wednesday and Friday, taught
/// by `tutor`, running (the first session date is today or earlier is not
/// needed: the status is what the page of a running batch shows).
async fn open_batch(t: &Centre, tutor: &(Resource, i64), room: &str) -> Batch {
    let room = t.room(room, 14).await;
    let course = t.course().await;
    let batch = t
        .batch_with(&course, (&tutor.0, Some(tutor.1)), &room, monday(), &[1, 3, 5])
        .await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, batch.id).await.unwrap()
}

/// Flags every `scheduled` session of `batch` as `EDU.RUL.12` does.
async fn flag_all(t: &Centre, batch: &Batch) {
    Session::query()
        .where_(Session::BATCH_ID.eq(batch.id))
        .update(
            t.db(),
            &[
                ("flag_reason", &Some("tutor_document".to_owned())),
                ("flagged_at", &Some(renox::db::now())),
            ],
        )
        .await
        .unwrap();
}

/// The change notices sent so far.
fn change_notices(t: &Centre) -> Vec<wl_kernel::notify::NotifyRequest> {
    t.messages
        .of_type(FAMILY_NOTICE)
        .into_iter()
        .filter(|request| Messages::text(request).contains("another tutor"))
        .collect()
}

/// Given running batch B (tutor T) with its remaining sessions from the second
/// Monday, when the coordinator changes to tutor U from then and U is free and
/// eligible, all of them are U's, each student or guardian gets one message with
/// no reason, and the flags of the sessions are cleared.
#[renox::test]
async fn edu_09_the_remaining_sessions_move_to_the_new_tutor_and_everyone_is_told_once() {
    let t = boot().await;
    let layla = cleared_tutor(&t, "Layla Hassan").await;
    let nour = cleared_tutor(&t, "Nour Saleh").await;
    let batch = open_batch(&t, &layla, "Room 1").await;
    batches::start_due(t.state(), monday()).await.unwrap();
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id).await.unwrap().status,
        BatchStatus::Running
    );
    let (sara, child, mother) = (
        t.person(1, "Sara Ali"),
        t.person(2, "Omar Child"),
        t.person(3, "Mother"),
    );
    t.people.guardian(child, mother);
    t.enrol(sara, &batch).await;
    let desk = t.desk().await;
    enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(child, batch.id, PRICE).guardian(mother).terms_accepted(),
    )
    .await
    .unwrap();
    flag_all(&t, &batch).await;
    let from = monday() + TimeDelta::days(7);
    let amal = t.coordinator().await;

    let changed = tutors::change_batch_tutor(
        t.state(),
        &amal,
        batch.id,
        nour.0.id,
        Some(nour.1),
        from,
    )
    .await
    .unwrap();

    assert_eq!(changed.status, BatchStatus::Running);
    assert_eq!(changed.tutor_resource_id, Some(nour.0.id));
    let sessions = t.sessions_of(&batch).await;
    let moved: Vec<&Session> = sessions.iter().filter(|s| s.session_date >= from).collect();
    assert!(moved.len() >= 6, "a good number of remaining sessions");
    for session in &sessions {
        if session.session_date >= from {
            assert_eq!(session.tutor_resource_id, Some(nour.0.id), "{}", session.session_date);
            assert_eq!(session.tutor_employee_id, Some(nour.1));
            assert!(!session.is_flagged(), "T's flag on {} is cleared", session.session_date);
        } else {
            assert_eq!(session.tutor_resource_id, Some(layla.0.id), "{}", session.session_date);
            assert!(session.is_flagged(), "the sessions before the date keep T and their flag");
        }
    }
    let told = change_notices(&t);
    assert_eq!(told.len(), 2, "one message per student or guardian");
    assert!(told.iter().any(|n| Messages::told_party(n, sara)));
    assert!(told.iter().any(|n| Messages::told_party(n, mother)));
    for notice in &told {
        let text = Messages::text(notice);
        assert!(text.contains(&from.to_string()), "{text}");
        assert!(!text.to_lowercase().contains("clearance"), "no reason: {text}");
        assert!(!text.to_lowercase().contains("expired"), "no reason: {text}");
        assert!(!text.to_lowercase().contains("sick"), "no reason: {text}");
    }
    let history = TutorAssignment::of_batch(t.db(), batch.id).await.unwrap();
    assert_eq!(history.len(), 2);
}

/// Given U busy on one date, nothing changes and the date is shown: not the
/// sessions, not the batch, not the history, and nobody is told.
#[renox::test]
async fn edu_09_a_tutor_who_is_busy_on_a_date_changes_nothing_and_the_date_is_shown() {
    let t = boot().await;
    let layla = cleared_tutor(&t, "Layla Hassan").await;
    let nour = cleared_tutor(&t, "Nour Saleh").await;
    let batch = open_batch(&t, &layla, "Room 1").await;
    t.enrol(t.person(1, "Sara Ali"), &batch).await;
    // Nour already teaches on the Monday of the second week, in another room.
    let room2 = t.room("Room 2", 14).await;
    let course = t.course().await;
    let other = t
        .batch_with(&course, (&nour.0, Some(nour.1)), &room2, monday() + TimeDelta::days(7), &[1])
        .await;
    let amal = t.coordinator().await;
    batches::open(t.state(), &amal, other.id).await.unwrap();

    let error = tutors::change_batch_tutor(
        t.state(),
        &amal,
        batch.id,
        nour.0.id,
        Some(nour.1),
        monday(),
    )
    .await
    .unwrap_err();

    let message = refusal(&error);
    assert!(message.contains(&(monday() + TimeDelta::days(7)).to_string()), "{message}");
    assert!(t
        .sessions_of(&batch)
        .await
        .iter()
        .all(|s| s.tutor_resource_id == Some(layla.0.id)));
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id).await.unwrap().tutor_resource_id,
        Some(layla.0.id)
    );
    assert_eq!(TutorAssignment::of_batch(t.db(), batch.id).await.unwrap().len(), 1);
    assert!(change_notices(&t).is_empty(), "nobody is told of a change that did not happen");
}

/// Changing one session to tutor V affects only that session, the students are
/// told of that session without a reason, and the batch's tutor stays.
#[renox::test]
async fn edu_09_one_session_moves_alone() {
    let t = boot().await;
    let layla = cleared_tutor(&t, "Layla Hassan").await;
    let vera = cleared_tutor(&t, "Vera Younis").await;
    let batch = open_batch(&t, &layla, "Room 1").await;
    let sara = t.person(1, "Sara Ali");
    t.enrol(sara, &batch).await;
    flag_all(&t, &batch).await;
    let amal = t.coordinator().await;
    let sessions = t.sessions_of(&batch).await;
    let one = sessions[4].clone();

    let moved = sessions::change_tutor(t.state(), &amal, one.id, vera.0.id, Some(vera.1))
        .await
        .unwrap();

    assert_eq!(moved.tutor_resource_id, Some(vera.0.id));
    assert!(!moved.is_flagged(), "this session's flag is cleared");
    for other in t.sessions_of(&batch).await {
        if other.id != one.id {
            assert_eq!(other.tutor_resource_id, Some(layla.0.id), "{}", other.session_date);
            assert!(other.is_flagged(), "the others keep their flag");
        }
    }
    assert_eq!(
        Batch::find_or_404(t.db(), batch.id).await.unwrap().tutor_resource_id,
        Some(layla.0.id)
    );
    let told = change_notices(&t);
    assert_eq!(told.len(), 1);
    assert!(Messages::told_party(&told[0], sara));
    assert!(Messages::text(&told[0]).contains(&one.session_date.to_string()));
}

/// A session moved to V earlier keeps V after a batch change to U; a cancelled
/// session keeps its tutor too.
#[renox::test]
async fn edu_09_a_session_moved_earlier_and_a_cancelled_one_keep_their_tutor() {
    let t = boot().await;
    let layla = cleared_tutor(&t, "Layla Hassan").await;
    let vera = cleared_tutor(&t, "Vera Younis").await;
    let uma = cleared_tutor(&t, "Uma Nasser").await;
    let batch = open_batch(&t, &layla, "Room 1").await;
    let amal = t.coordinator().await;
    let sessions = t.sessions_of(&batch).await;
    let (own, cancelled) = (sessions[3].clone(), sessions[5].clone());
    sessions::change_tutor(t.state(), &amal, own.id, vera.0.id, Some(vera.1))
        .await
        .unwrap();
    sessions::cancel(t.state(), &amal, cancelled.id, "Room flooded", None)
        .await
        .unwrap();

    tutors::change_batch_tutor(t.state(), &amal, batch.id, uma.0.id, Some(uma.1), monday())
        .await
        .unwrap();

    for session in t.sessions_of(&batch).await {
        let expected = if session.id == own.id {
            vera.0.id
        } else if session.id == cancelled.id {
            layla.0.id
        } else {
            uma.0.id
        };
        assert_eq!(session.tutor_resource_id, Some(expected), "{}", session.session_date);
    }
    let still = Session::find_or_404(t.db(), cancelled.id).await.unwrap();
    assert_eq!(still.status, SessionStatus::Cancelled);
}

/// Two coordinators give the same new tutor two batches that meet at the same
/// time: the change checks the tutor's calendar under a lock, so exactly one
/// goes through and the other is refused naming the days.
#[renox::test]
async fn edu_09_two_changes_to_one_tutor_at_the_same_time_only_one_wins() {
    let t = boot().await;
    let layla = cleared_tutor(&t, "Layla Hassan").await;
    let hana = cleared_tutor(&t, "Hana Omar").await;
    let nour = cleared_tutor(&t, "Nour Saleh").await;
    let first = open_batch(&t, &layla, "Room 1").await;
    let second = open_batch(&t, &hana, "Room 2").await;
    let amal = t.coordinator().await;
    let other = {
        let user = t
            .user(
                "Aya",
                &[
                    edtech::permissions::BATCHES_VIEW,
                    edtech::permissions::BATCHES_MANAGE,
                ],
            )
            .await;
        t.auth(&user).await
    };

    let (a, b) = renox::tokio::join!(
        tutors::change_batch_tutor(t.state(), &amal, first.id, nour.0.id, Some(nour.1), monday()),
        tutors::change_batch_tutor(t.state(), &other, second.id, nour.0.id, Some(nour.1), monday()),
    );

    assert_eq!(
        [a.is_ok(), b.is_ok()].iter().filter(|ok| **ok).count(),
        1,
        "exactly one change goes through"
    );
    let refused = a.err().or_else(|| b.err()).expect("one change is refused");
    if let Error::Status(_, message) = &refused {
        assert!(message.contains(&monday().to_string()), "{message}");
    }
    let taught: Vec<Session> = Session::query()
        .where_(Session::TUTOR_RESOURCE_ID.eq(nour.0.id))
        .get(t.db())
        .await
        .unwrap();
    let mut slots: Vec<_> = taught.iter().map(|s| s.starts_at).collect();
    slots.sort();
    let count = slots.len();
    slots.dedup();
    assert_eq!(slots.len(), count, "the tutor is never in two places at once");
}

/// A planned batch has no sessions and nobody enrolled: its tutor is replaced in
/// place and nobody is told.
#[renox::test]
async fn edu_09_a_planned_batch_changes_its_tutor_in_silence() {
    let t = boot().await;
    let layla = cleared_tutor(&t, "Layla Hassan").await;
    let nour = cleared_tutor(&t, "Nour Saleh").await;
    let room = t.room("Room 1", 14).await;
    let course = t.course().await;
    let batch = t
        .batch_with(&course, (&layla.0, Some(layla.1)), &room, monday(), &[1, 3])
        .await;
    let amal = t.coordinator().await;

    let changed = tutors::change_batch_tutor(
        t.state(),
        &amal,
        batch.id,
        nour.0.id,
        Some(nour.1),
        monday(),
    )
    .await
    .unwrap();

    assert_eq!(changed.status, BatchStatus::Planned);
    assert_eq!(changed.tutor_resource_id, Some(nour.0.id));
    assert!(change_notices(&t).is_empty());
}
