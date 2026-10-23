//! Found by walking the enrolment screens: a student has one place in a batch.
//! Typing the same student twice at the desk must not hold a second seat; an
//! enrolment that ended (lapsed) does not count.

mod support;

use edtech::enrolments::{self, NewEnrolment};
use edtech::{Batch, EnrolmentStatus, seats};
use renox::prelude::*;
use support::{Centre, PRICE, boot, day, next_weekday};

async fn open_batch(t: &Centre) -> Batch {
    let tutor = t.tutor("Layla").await;
    let room = t.room("Room 1", 14).await;
    t.open_batch(&tutor, &room, next_weekday(day(7), 1)).await
}

#[renox::test]
async fn a_second_enrolment_of_the_same_student_in_the_batch_is_refused() {
    let t = boot().await;
    let batch = open_batch(&t).await;
    let student = t.person(1, "Lucas Meyer");
    let first = t.enrol(student, &batch).await;
    let free = seats::available(t.db(), batch.id).await.unwrap();

    let desk = t.desk().await;
    let again = enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE),
    )
    .await;
    let Err(error) = again else {
        panic!("a second place was given")
    };
    assert!(
        error.to_string().contains("already has a place"),
        "the refusal says why: {error}"
    );
    assert_eq!(
        seats::available(t.db(), batch.id).await.unwrap(),
        free,
        "no second seat is held"
    );

    // Another student is welcome; and the student again once the first
    // enrolment lapsed.
    t.enrol(t.person(2, "Sofia Garcia"), &batch).await;
    let lapsed = t.lapse(&first).await;
    assert_eq!(lapsed.status, EnrolmentStatus::Lapsed);
    enrolments::create(
        t.state(),
        &desk,
        NewEnrolment::course(student, batch.id, PRICE),
    )
    .await
    .expect("a lapsed enrolment does not count");
}
