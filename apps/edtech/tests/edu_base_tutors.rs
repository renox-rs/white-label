//! The base of the 10 EdTech tutor assignment (base for #443, #444, #445):
//! who teaches a batch or one of its sessions is kept as history, the tutor of
//! a batch changes from a date for all the sessions of the old tutor or for
//! none, and the guard slot where the tutor's eligibility is checked exists.

mod support;

use edtech::batches;
use edtech::sessions;
use edtech::tutors::{self, Tutor};
use edtech::{Batch, BatchStatus, TutorAssignment};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Centre, boot, day, next_weekday};

/// The Monday a week from now.
fn monday() -> renox::chrono::NaiveDate {
    next_weekday(day(7), 1)
}

/// An open batch of tutor `layla` (Monday and Wednesday, four weeks).
async fn open_with(t: &Centre, tutor: &wl_scheduling::Resource, room_name: &str) -> Batch {
    let room = t.room(room_name, 14).await;
    t.open_batch(tutor, &room, monday()).await
}

/// Planning a batch records its tutor as the batch's assignment from the first
/// day, open-ended.
#[renox::test]
async fn edu_base_planning_a_batch_records_its_tutor_assignment() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.batch_of(&layla, &room, monday(), &[1, 3]).await;

    let rows = TutorAssignment::of_batch(t.db(), batch.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].tutor_resource_id, layla.id);
    assert!(rows[0].session_id.is_none());
    assert_eq!(rows[0].valid_from, monday());
    assert!(rows[0].valid_to.is_none());
    assert!(rows[0].covers(monday() + TimeDelta::days(20)));
    assert_eq!(rows[0].subject().to_string(), format!("edtech.tutor_assignment:{}", rows[0].id));
}

/// Changing the tutor of an open batch from the second Monday moves the
/// sessions from then on, keeps the earlier ones, closes the old assignment the
/// day before and opens the new one.
#[renox::test]
async fn edu_base_changing_the_tutor_of_a_batch_moves_the_remaining_sessions() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let nour = t.tutor("Nour").await;
    let batch = open_with(&t, &layla, "Room 1").await;
    let amal = t.coordinator().await;
    let from = monday() + TimeDelta::days(7);

    let changed = tutors::change_batch_tutor(t.state(), &amal, batch.id, nour.id, None, from)
        .await
        .unwrap();
    assert_eq!(changed.tutor_resource_id, Some(nour.id));
    assert_eq!(changed.status, BatchStatus::Open);
    for session in t.sessions_of(&batch).await {
        let expected = if session.session_date >= from {
            nour.id
        } else {
            layla.id
        };
        assert_eq!(session.tutor_resource_id, Some(expected), "{}", session.session_date);
    }
    let rows = TutorAssignment::of_batch(t.db(), batch.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].tutor_resource_id, layla.id);
    assert_eq!(rows[0].valid_to, Some(from - TimeDelta::days(1)));
    assert_eq!(rows[1].tutor_resource_id, nour.id);
    assert_eq!(rows[1].valid_from, from);
    assert!(rows[1].valid_to.is_none());
    assert!(rows[0].covers(from - TimeDelta::days(1)));
    assert!(!rows[0].covers(from));
    assert!(rows[1].covers(from));
}

/// A new tutor who is taken by another batch on one of the dates changes
/// nothing: not the sessions, not the batch, not the assignments.
#[renox::test]
async fn edu_base_a_tutor_who_is_taken_on_one_date_changes_nothing() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let nour = t.tutor("Nour").await;
    let batch = open_with(&t, &layla, "Room 1").await;
    // Nour already teaches the same slots in another room.
    let _other = open_with(&t, &nour, "Room 2").await;
    let amal = t.coordinator().await;

    let refused = tutors::change_batch_tutor(t.state(), &amal, batch.id, nour.id, None, monday())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("already taken"));
    let same = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(same.tutor_resource_id, Some(layla.id));
    assert!(t
        .sessions_of(&batch)
        .await
        .iter()
        .all(|s| s.tutor_resource_id == Some(layla.id)));
    assert_eq!(
        TutorAssignment::of_batch(t.db(), batch.id).await.unwrap().len(),
        1
    );
}

/// A planned batch has no sessions yet: its tutor is replaced in place.
#[renox::test]
async fn edu_base_the_tutor_of_a_planned_batch_is_replaced_in_place() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let nour = t.tutor("Nour").await;
    let room = t.room("Room 1", 14).await;
    let batch = t.batch_of(&layla, &room, monday(), &[1, 3]).await;
    let amal = t.coordinator().await;

    let changed =
        tutors::change_batch_tutor(t.state(), &amal, batch.id, nour.id, None, monday())
            .await
            .unwrap();
    assert_eq!(changed.status, BatchStatus::Planned);
    assert_eq!(changed.tutor_resource_id, Some(nour.id));
    let rows = TutorAssignment::of_batch(t.db(), batch.id).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].tutor_resource_id, nour.id);
    assert!(rows[1].valid_to.is_none());
    // Opening it now gives its sessions to the new tutor.
    let opened = batches::open(t.state(), &amal, batch.id).await.unwrap();
    assert_eq!(opened.status, BatchStatus::Open);
    assert!(t
        .sessions_of(&batch)
        .await
        .iter()
        .all(|s| s.tutor_resource_id == Some(nour.id)));
}

/// A session with its own tutor is recorded as that session's assignment, for
/// its day.
#[renox::test]
async fn edu_base_a_session_with_its_own_tutor_is_recorded() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let nour = t.tutor("Nour").await;
    let batch = open_with(&t, &layla, "Room 1").await;
    let amal = t.coordinator().await;
    let session = t.sessions_of(&batch).await.remove(1);

    let moved = sessions::change_tutor(t.state(), &amal, session.id, nour.id, None)
        .await
        .unwrap();
    assert_eq!(moved.tutor_resource_id, Some(nour.id));
    let rows = TutorAssignment::of_batch(t.db(), batch.id).await.unwrap();
    let own: Vec<_> = rows.iter().filter(|r| r.session_id == Some(session.id)).collect();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].tutor_resource_id, nour.id);
    assert_eq!(own[0].valid_from, session.session_date);
    assert_eq!(own[0].valid_to, Some(session.session_date));
    // The batch's own assignment is untouched.
    assert!(rows
        .iter()
        .any(|r| r.session_id.is_none() && r.tutor_resource_id == layla.id && r.valid_to.is_none()));
}

/// The slot where BL-EDU-08 checks the tutor's documents passes today, and
/// only `batches.manage` changes a tutor.
#[renox::test]
async fn edu_base_the_eligibility_slot_passes_and_only_the_coordinator_changes_a_tutor() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let nour = t.tutor("Nour").await;
    let batch = open_with(&t, &layla, "Room 1").await;
    let tutor = Tutor {
        resource_id: nour.id,
        employee_id: None,
    };
    tutors::tutor_eligible(t.state(), &tutor, &[monday(), monday() + TimeDelta::days(30)])
        .await
        .unwrap();

    let cara = t.counsellor().await;
    assert!(
        tutors::change_batch_tutor(t.state(), &cara, batch.id, nour.id, None, monday())
            .await
            .is_err()
    );
}

/// The batch page lists the tutors and offers the change to the coordinator,
/// who sends it from the page.
#[renox::test]
async fn edu_base_the_batch_page_changes_the_tutor() {
    let t = boot().await;
    let layla = t.tutor("Layla").await;
    let nour = t.tutor("Nour").await;
    let batch = open_with(&t, &layla, "Room 1").await;
    let amal = t.coordinator_user().await;
    t.app.acting_as(&amal);
    t.app
        .get(&format!("/edtech/batches/{}", batch.id))
        .await
        .assert_ok()
        .assert_see("Change tutor")
        .assert_see("Whole batch");

    let from = (monday() + TimeDelta::days(7)).to_string();
    t.app
        .htmx()
        .post(
            &format!("/edtech/batches/{}/tutor", batch.id),
            &[
                ("tutor", nour.id.to_string().as_str()),
                ("from", from.as_str()),
            ],
        )
        .await
        .assert_ok();
    let changed = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(changed.tutor_resource_id, Some(nour.id));

    let cara = t.counsellor_user().await;
    t.app.acting_as(&cara);
    t.app
        .htmx()
        .post(
            &format!("/edtech/batches/{}/tutor", batch.id),
            &[("tutor", layla.id.to_string().as_str()), ("from", from.as_str())],
        )
        .await
        .assert_forbidden();
}
