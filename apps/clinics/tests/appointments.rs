//! The base of the 05 appointment (base for #349, #360, #350): the clinic's
//! record follows its wl-scheduling booking through the kernel's events,
//! without a dependency on the scheduling crate.

mod support;

use renox::db::now;
use wl_kernel::documents::dubai_date;
use wl_scheduling::service::{self, Actor, CancelRequest};

use clinics::Appointment;
use support::{PATIENT, at, boot, day};

/// A booking made for a patient gives the clinic an appointment with the
/// patient, the service code and the Dubai day of the visit.
#[renox::test]
async fn a_booking_gives_the_clinic_an_appointment() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;

    let booking = t.book(&service, &doctor, at(day(1), 9)).await;

    let appointment = Appointment::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .expect("the appointment exists");
    assert_eq!(appointment.patient_party_id, Some(PATIENT.0));
    assert_eq!(appointment.booker_party_id, PATIENT.0);
    assert_eq!(appointment.service_code, "consultation");
    assert_eq!(appointment.visit_date, day(1));
    assert_eq!(appointment.starts_at, booking.starts_at);
    assert_eq!(appointment.policy_id, None);
    assert!(!appointment.is_insured());
    assert!(!appointment.is_cancelled());
    assert_eq!(appointment.subject(), booking.subject());
}

/// A reschedule moves the appointment to the new day.
#[renox::test]
async fn a_reschedule_moves_the_appointment() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    let booking = t.book(&service, &doctor, at(day(1), 9)).await;

    service::reschedule(
        t.state(),
        booking.id,
        at(day(3), 10),
        None,
        Actor::staff(1),
        now(),
    )
    .await
    .expect("the booking moves");

    let appointment = Appointment::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(appointment.visit_date, day(3));
    assert_eq!(appointment.starts_at, at(day(3), 10));
    assert_eq!(dubai_date(appointment.ends_at), day(3));
}

/// A cancellation closes the appointment; it is not deleted.
#[renox::test]
async fn a_cancellation_closes_the_appointment() {
    let t = boot().await;
    let doctor = t.practitioner("Dr A", 61, None).await;
    let service = t.consultation(&[&doctor]).await;
    let booking = t.book(&service, &doctor, at(day(2), 9)).await;

    service::cancel(
        t.state(),
        booking.id,
        CancelRequest::new("equipment_failure"),
        Actor::staff(1),
        now(),
    )
    .await
    .expect("the booking is cancelled");

    let appointment = Appointment::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    assert!(appointment.is_cancelled());
}
