//! The base of the 05 visit (base for #349): the check-in opens it, the
//! booking's events move it through the machine, and a cancellation before
//! the check-in opens none.

mod support;

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::history;
use wl_scheduling::service::{self, Actor, CancelRequest};

use clinics::{Visit, VisitStatus};
use support::{PATIENT, at, boot, day};

async fn history_of(t: &support::Clinic, visit: &Visit) -> Vec<(String, String)> {
    history(t.db(), Visit::TABLE, visit.id)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.transition, row.to_state))
        .collect()
}

/// The check-in of a booking opens one visit in `checked_in` with the
/// patient, the service and the day; the history names the check-in.
#[renox::test]
async fn the_check_in_opens_the_visit() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    let booking = t.book(&service, &doctor, at(day(1), 9)).await;
    assert!(
        Visit::of_booking(t.db(), booking.id)
            .await
            .unwrap()
            .is_none()
    );

    service::check_in(t.state(), booking.id, Actor::staff(1), now())
        .await
        .expect("the patient is checked in");

    let visit = Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .expect("the visit is open");
    assert_eq!(visit.status, VisitStatus::CheckedIn);
    assert_eq!(visit.patient_party_id, Some(PATIENT.0));
    assert_eq!(visit.service_code, "consultation");
    assert_eq!(visit.visit_date, day(1));
    assert!(!visit.is_insured());
    assert_eq!(visit.number(), format!("VIS-{:04}", visit.id));
    assert_eq!(
        history_of(&t, &visit).await,
        [("insert".to_owned(), "checked_in".to_owned())]
    );
}

/// `start` and `complete` of the booking move the visit; its times are kept.
/// The service completes only once its start time has passed.
#[renox::test]
async fn the_visit_follows_start_and_complete() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    let booking = t.book(&service, &doctor, at(day(1), 9)).await;
    service::check_in(t.state(), booking.id, Actor::staff(1), now())
        .await
        .unwrap();

    service::start(t.state(), booking.id, Actor::staff(1), now())
        .await
        .expect("the service starts");
    let started = Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.status, VisitStatus::InProgress);
    assert!(started.started_at.is_some());

    service::complete(t.state(), booking.id, Actor::staff(1), at(day(1), 10))
        .await
        .expect("the service is complete");
    let done = Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.status, VisitStatus::Completed);
    assert!(done.ended_at.is_some());
    assert_eq!(
        history_of(&t, &done).await,
        [
            ("insert".to_owned(), "checked_in".to_owned()),
            ("start".to_owned(), "in_progress".to_owned()),
            ("complete".to_owned(), "completed".to_owned()),
        ]
    );
}

/// A staff cancellation after the check-in cancels the visit with the
/// booking's reason code; one before it opens no visit.
#[renox::test]
async fn a_cancellation_after_the_check_in_cancels_the_visit() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    let before = t.book(&service, &doctor, at(day(1), 9)).await;
    service::cancel(
        t.state(),
        before.id,
        CancelRequest::new("equipment_failure"),
        Actor::staff(1),
        now(),
    )
    .await
    .unwrap();
    assert!(
        Visit::of_booking(t.db(), before.id)
            .await
            .unwrap()
            .is_none()
    );

    let booking = t.book(&service, &doctor, at(day(1), 11)).await;
    service::check_in(t.state(), booking.id, Actor::staff(1), now())
        .await
        .unwrap();
    // The work done is captured or marked none before the cancel
    // (`SCH.ACL.06`).
    let open = Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    clinics::charges::mark_none(t.state(), open.id)
        .await
        .expect("nothing was done");
    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("equipment_failure"),
        Actor::staff(1),
        now(),
    )
    .await
    .expect("staff cancel after the check-in");

    let visit = Visit::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visit.status, VisitStatus::Cancelled);
    assert_eq!(visit.cancel_reason.as_deref(), Some("equipment_failure"));
    assert!(visit.ended_at.is_some());
}
