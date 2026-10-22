//! BL-EDU-11 (#446): the attendance of a session is always accepted, and when
//! the session's tutor did not hold the documents their batch needs on that day
//! (`EDU.ACL.14`, the police clearance for minors, the permit or visa for a
//! freelance tutor; flag mode of §9) the session is flagged for the centre
//! manager, who closes the flag with a note. The flag changes neither the
//! credits nor the tutor's pay.
//!
//! The documents are the stand-in guard of the support, which answers the
//! app's own `BlockSubjects` provider as wl-documents does and keeps the flags.

mod support;

use edtech::sessions;
use edtech::{Batch, Session, SessionStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use support::{Centre, at, boot, day, next_weekday};
use wl_scheduling::Resource;

/// A `scheduled` session of `batch` that started on `date` at 10:00 for an
/// hour, taught by `tutor`.
async fn past_session(t: &Centre, batch: &Batch, date: NaiveDate, tutor: &(Resource, i64)) -> Session {
    let mut session = Session {
        batch_id: batch.id,
        number: 1,
        session_date: date,
        starts_at: at(date, 10, 0),
        ends_at: at(date, 11, 0),
        tutor_resource_id: Some(tutor.0.id),
        tutor_employee_id: Some(tutor.1),
        room_resource_id: batch.room_resource_id,
        ..Session::default()
    };
    session.save(t.db()).await.unwrap();
    session
}

/// A batch for minors (or adults) taught by `tutor`, planned: the session is a
/// fixture.
async fn batch_of(t: &Centre, tutor: &(Resource, i64), minors: bool) -> Batch {
    let room = t.room("Room 1", 14).await;
    let course = if minors {
        t.minors_course().await
    } else {
        t.course().await
    };
    t.batch_with(&course, (&tutor.0, Some(tutor.1)), &room, next_weekday(day(7), 1), &[1, 3])
        .await
}

/// Given T's clearance expired and T's session in a batch for minors on a later
/// day, when T submits the attendance the session is `held` and the session has
/// a flag naming the clearance; the centre manager sees it on the session page.
#[renox::test]
async fn edu_11_the_attendance_is_accepted_and_the_session_is_flagged() {
    let t = boot().await;
    t.documents.tutor_blocks();
    let tutor = t.employed_tutor("Layla Hassan", false).await;
    t.documents.set(
        &Centre::employee_subject(tutor.1),
        "police_clearance",
        Some(day(-5)),
    );
    let batch = batch_of(&t, &tutor, true).await;
    let session = past_session(&t, &batch, day(-3), &tutor).await;
    let tariq = t.teacher().await;

    let held = sessions::submit_attendance(t.state(), &tariq, session.id)
        .await
        .expect("the attendance is always accepted");

    assert_eq!(held.status, SessionStatus::Held);
    assert!(held.held_at.is_some());
    let flags = sessions::document_flags(t.state(), session.id).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert!(flags[0].label.contains("Police clearance expired on"), "{}", flags[0].label);
    assert!(flags[0].label.contains(&day(-5).to_string()), "{}", flags[0].label);
    assert!(!flags[0].is_reviewed());
    assert_eq!(flags[0].on_date, day(-3));

    let manager = t.manager_user().await;
    t.app.acting_as(&manager);
    t.app
        .get(&format!("/edtech/sessions/{}", session.id))
        .await
        .assert_ok()
        .assert_see("Documents flagged")
        .assert_see("Police clearance expired on")
        .assert_see("Close the flag");
}

/// The centre manager closes the flag with a note; the history keeps it with the
/// note and who closed it; it cannot be closed without a note, twice, or by a
/// user who may not.
#[renox::test]
async fn edu_11_the_centre_manager_closes_the_flag_with_a_note() {
    let t = boot().await;
    t.documents.tutor_blocks();
    let tutor = t.employed_tutor("Layla Hassan", false).await;
    t.documents.set(
        &Centre::employee_subject(tutor.1),
        "police_clearance",
        Some(day(-5)),
    );
    let batch = batch_of(&t, &tutor, true).await;
    let session = past_session(&t, &batch, day(-3), &tutor).await;
    let tariq = t.teacher().await;
    sessions::submit_attendance(t.state(), &tariq, session.id)
        .await
        .unwrap();
    let flag = sessions::document_flags(t.state(), session.id).await.unwrap().remove(0);
    let url = format!("/edtech/sessions/{}/flag", session.id);
    let flag_id = flag.id.to_string();

    // Not without a note.
    let manager = t.manager_user().await;
    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(&url, &[("flag", flag_id.as_str()), ("note", " ")])
        .await
        .assert_status(422);
    // Not by the coordinator, who plans but does not approve.
    let amal = t.coordinator_user().await;
    t.app.acting_as(&amal);
    t.app
        .htmx()
        .post(&url, &[("flag", flag_id.as_str()), ("note", "Seen")])
        .await
        .assert_forbidden();

    t.app.acting_as(&manager);
    t.app
        .htmx()
        .post(
            &url,
            &[
                ("flag", flag_id.as_str()),
                ("note", "Renewal filed on 12 Dec; seen"),
            ],
        )
        .await
        .assert_ok();

    let closed = sessions::document_flags(t.state(), session.id).await.unwrap();
    assert_eq!(closed.len(), 1, "the flag is kept, never removed");
    assert!(closed[0].is_reviewed());
    assert_eq!(closed[0].review_note.as_deref(), Some("Renewal filed on 12 Dec; seen"));
    assert_eq!(closed[0].reviewed_by, Some(manager.id));
    // The page shows it in the history.
    t.app
        .get(&format!("/edtech/sessions/{}", session.id))
        .await
        .assert_ok()
        .assert_see("Renewal filed on 12 Dec; seen")
        .assert_see("Closed");
    // Closing it again is refused.
    t.app
        .htmx()
        .post(&url, &[("flag", flag_id.as_str()), ("note", "Again")])
        .await
        .assert_status(422);
}

/// Given T valid that day, there is no flag; a batch of adults needs no clearance
/// whatever the tutor holds.
#[renox::test]
async fn edu_11_a_tutor_who_holds_the_documents_gives_no_flag() {
    let t = boot().await;
    t.documents.tutor_blocks();
    let valid = t.employed_tutor("Layla Hassan", false).await;
    t.documents.set(
        &Centre::employee_subject(valid.1),
        "police_clearance",
        Some(day(-1)),
    );
    let kids = batch_of(&t, &valid, true).await;
    let covered = past_session(&t, &kids, day(-3), &valid).await;
    let tariq = t.teacher().await;

    sessions::submit_attendance(t.state(), &tariq, covered.id)
        .await
        .unwrap();
    assert!(sessions::document_flags(t.state(), covered.id).await.unwrap().is_empty());

    // An adult batch of a tutor with no clearance at all.
    let bare = t.employed_tutor("Nour Saleh", false).await;
    let room = t.room("Room 2", 14).await;
    let course = t.course().await;
    let adults = t
        .batch_with(&course, (&bare.0, Some(bare.1)), &room, next_weekday(day(7), 1), &[1, 3])
        .await;
    let session = past_session(&t, &adults, day(-3), &bare).await;
    sessions::submit_attendance(t.state(), &tariq, session.id)
        .await
        .unwrap();
    assert!(sessions::document_flags(t.state(), session.id).await.unwrap().is_empty());
}

/// A freelance tutor without a valid permit or visa flags the session too (the
/// same entries as the police clearance), and the check is on the session's tutor
/// whoever submits: here the coordinator does.
#[renox::test]
async fn edu_11_a_freelance_permit_flags_too_whoever_submits() {
    let t = boot().await;
    t.documents.tutor_blocks();
    let freelance = t.employed_tutor("Fiona Freelance", true).await;
    t.documents.set(
        &Centre::employee_subject(freelance.1),
        "tutor_permit",
        Some(day(-10)),
    );
    let batch = batch_of(&t, &freelance, false).await;
    let session = past_session(&t, &batch, day(-3), &freelance).await;
    let cora = t
        .user(
            "Cora",
            &[
                edtech::permissions::SESSIONS_VIEW,
                edtech::permissions::SESSIONS_TEACH,
                edtech::permissions::BATCHES_MANAGE,
            ],
        )
        .await;
    let cora = t.auth(&cora).await;

    let held = sessions::submit_attendance(t.state(), &cora, session.id)
        .await
        .unwrap();

    assert_eq!(held.status, SessionStatus::Held);
    let flags = sessions::document_flags(t.state(), session.id).await.unwrap();
    assert_eq!(flags.len(), 1);
    assert!(flags[0].label.contains("Tutor permit expired on"), "{}", flags[0].label);
}

/// The tutor's pay does not depend on the flag: the session counts as taught by
/// its tutor (`sessions::taught_in`) whether or not it is flagged.
#[renox::test]
async fn edu_11_the_flag_does_not_change_the_tutor_pay() {
    let t = boot().await;
    t.documents.tutor_blocks();
    let tutor = t.employed_tutor("Layla Hassan", false).await;
    t.documents.set(
        &Centre::employee_subject(tutor.1),
        "police_clearance",
        Some(day(-5)),
    );
    let batch = batch_of(&t, &tutor, true).await;
    let session = past_session(&t, &batch, day(-3), &tutor).await;
    let tariq = t.teacher().await;

    sessions::submit_attendance(t.state(), &tariq, session.id)
        .await
        .unwrap();

    let taught = sessions::taught_in(t.db(), tutor.0.id, day(-10), day(0)).await.unwrap();
    assert_eq!(taught.len(), 1, "the flagged session is paid like any other");
    assert_eq!(taught[0].id, session.id);
}
