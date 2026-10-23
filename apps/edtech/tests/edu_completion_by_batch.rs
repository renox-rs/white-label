//! The completion of the enrolments follows the batch (10 §7.1 *Batch*
//! `complete`, *Enrolment* `complete`, `EDU.RUL.06`, `EDU.ACL.06`): when the last
//! session is closed the active students complete the course and get the draft
//! of their certificate; a student who attended less than 80% completes but has
//! no draft; a frozen or unpaid enrolment is left alone.

mod support;

use edtech::attendance::Mark;
use edtech::certificate::Certificate;
use edtech::sessions;
use edtech::{Batch, BatchStatus, Enrolment, EnrolmentStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{Centre, boot, day, next_weekday};

/// A running batch (a fixture) with the tutor and room of the tests.
async fn running(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    let planned = t
        .batch_of(&tutor, &room, next_weekday(day(7), 1), &[1, 3])
        .await;
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

#[renox::test]
async fn completing_the_batch_completes_its_active_enrolments_and_drafts_certificates() {
    let t = boot().await;
    let batch = running(&t).await;
    let active = t
        .enrolment_in(&batch, t.person(1, "Sara Ali"), EnrolmentStatus::Active)
        .await;
    let frozen = t
        .enrolment_in(&batch, t.person(2, "Omar Nasser"), EnrolmentStatus::Frozen)
        .await;
    let unpaid = t
        .enrolment_in(
            &batch,
            t.person(3, "Mona Said"),
            EnrolmentStatus::PendingPayment,
        )
        .await;
    let first = t.past_session(&batch, 1, 2).await;
    let last = t.past_session(&batch, 2, 1).await;
    let tariq = t.teacher().await;

    sessions::submit_attendance(t.state(), &tariq, first.id)
        .await
        .unwrap();
    let still = Enrolment::find_or_404(t.db(), active.id).await.unwrap();
    assert_eq!(
        still.status,
        EnrolmentStatus::Active,
        "the batch still runs"
    );

    sessions::submit_attendance(t.state(), &tariq, last.id)
        .await
        .unwrap();
    let done = Batch::find_or_404(t.db(), batch.id).await.unwrap();
    assert_eq!(done.status, BatchStatus::Completed);

    let completed = Enrolment::find_or_404(t.db(), active.id).await.unwrap();
    assert_eq!(completed.status, EnrolmentStatus::Completed);
    assert_eq!(completed.completed_on, Some(day(0)));
    let drafts = Certificate::of_enrolment(t.db(), active.id).await.unwrap();
    assert_eq!(drafts.len(), 1, "a draft certificate for the student");
    assert_eq!(drafts[0].student_name, "Sara Ali");

    for left in [&frozen, &unpaid] {
        let same = Enrolment::find_or_404(t.db(), left.id).await.unwrap();
        assert_eq!(same.status, left.status, "left as it was");
        assert!(
            Certificate::of_enrolment(t.db(), left.id)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[renox::test]
async fn a_student_below_the_attendance_threshold_completes_without_a_draft() {
    let t = boot().await;
    let batch = running(&t).await;
    let present = t
        .enrolment_in(&batch, t.person(1, "Sara Ali"), EnrolmentStatus::Active)
        .await;
    let absent = t
        .enrolment_in(&batch, t.person(2, "Omar Nasser"), EnrolmentStatus::Active)
        .await;
    let session = t.past_session(&batch, 1, 1).await;
    let tariq = t.teacher().await;
    sessions::submit_attendance(t.state(), &tariq, session.id)
        .await
        .unwrap();
    // The fixture's marks are present: the student with no attendance is
    // marked absent by a correction of the same session.
    let amal = t.coordinator().await;
    edtech::corrections::correct_attendance(
        t.state(),
        &amal,
        session.id,
        &[(absent.id, Mark::Absent)],
        "never came",
    )
    .await
    .unwrap();

    let a = Enrolment::find_or_404(t.db(), absent.id).await.unwrap();
    assert_eq!(a.status, EnrolmentStatus::Completed);
    assert!(
        Certificate::of_enrolment(t.db(), absent.id)
            .await
            .unwrap()
            .is_empty(),
        "attendance below 80% gives no certificate"
    );
    assert_eq!(
        Certificate::of_enrolment(t.db(), present.id)
            .await
            .unwrap()
            .len(),
        1
    );
}
