//! The workshop booking and the job card (base for #397 .. #403, 08 §7.1
//! *Booking*): the check-in of a booking opens the job card of the vehicle,
//! billed to its owner, in the workshop of the booking, and linked back to the
//! booking; delivering the job card completes the booking. Through the real
//! wl-scheduling.

mod support;

use auto::job_cards::{self, Delivery, Intake, NewLabour, Promise};
use auto::{JobCard, JobCardStatus, WorkStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use support::{Garage, boot};
use wl_kernel::scheduling::{BookingCheckedIn, BookingId};
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{
    AvailabilityRule, Booking, BookingStatus, Resource, ResourceType, ServiceType, resource,
};

/// The workshop of the bookings.
fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// A bay that is open every day and a periodic service that needs check-in and
/// a bay.
async fn bay_and_service(t: &Garage) -> (Resource, ServiceType) {
    let mut bay = Resource::new("Bay 1", ResourceType::Bay).at(&workshop());
    bay.save(t.db()).await.unwrap();
    for weekday in 1..=7 {
        let mut rule = AvailabilityRule::new(bay.id, weekday, 0, 1440);
        rule.save(t.db()).await.unwrap();
    }
    let mut service = ServiceType::new("periodic_service", "Periodic service", 120);
    service.required_types = "bay".to_owned();
    service.check_in_required = true;
    service.save(t.db()).await.unwrap();
    resource::offer(t.db(), service.id, bay.id).await.unwrap();
    (bay, service)
}

/// A booking of the Hilux that started two hours ago, made an hour before.
async fn booked(t: &Garage) -> Booking {
    let (bay, service) = bay_and_service(t).await;
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let start = renox::db::now() - TimeDelta::hours(2);
    service::book(
        t.state(),
        NewBooking::new(PartyId(7), service.id, &[bay.id], start)
            .for_subject(vehicle.subject())
            .at(workshop()),
        Actor::staff(sami.id),
        start - TimeDelta::hours(1),
    )
    .await
    .expect("the booking is made")
}

/// The check-in of the booking opens the job card of the vehicle: billed to
/// its owner, in the workshop of the booking, without its intake yet, and the
/// booking asks it what was done.
#[renox::test]
async fn aut_base_the_check_in_of_a_booking_opens_the_job_card() {
    let t = boot().await;
    let booking = booked(&t).await;
    let sami = t.advisor().await;

    service::check_in(
        t.state(),
        booking.id,
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .expect("the vehicle is checked in");

    let card = JobCard::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .expect("the job card is open");
    let vehicle = t.hilux().await;
    assert_eq!(card.status, JobCardStatus::CheckedIn);
    assert_eq!(card.vehicle_id, vehicle.id);
    assert_eq!(
        card.customer_party_id, 7,
        "billed to the owner of the vehicle"
    );
    assert_eq!(card.workshop_id, Some(1));
    assert!(!card.intake_done(), "the advisor records the intake");
    let linked = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(linked.status, BookingStatus::CheckedIn);
    assert_eq!(linked.work_done(), Some(card.subject()));
}

/// A second event for the booking returns the job card that exists, and a
/// booking that is not for a vehicle of the register opens nothing.
#[renox::test]
async fn aut_base_a_booking_opens_one_job_card_and_only_for_a_vehicle() {
    let t = boot().await;
    let booking = booked(&t).await;
    let sami = t.advisor().await;
    service::check_in(
        t.state(),
        booking.id,
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    let card = JobCard::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    let vehicle = t.hilux().await;

    let again = auto::bookings::on_checked_in(
        t.state(),
        &BookingCheckedIn {
            booking: BookingId(booking.id),
            subject: Some(vehicle.subject()),
            at: renox::db::now(),
        },
    )
    .await
    .unwrap()
    .expect("the job card that exists");
    assert_eq!(again.id, card.id);
    assert_eq!(JobCard::query().count(t.db()).await.unwrap(), 1);

    let not_a_vehicle = auto::bookings::on_checked_in(
        t.state(),
        &BookingCheckedIn {
            booking: BookingId(9_999),
            subject: Some(Subject::new("parties.party", 7)),
            at: renox::db::now(),
        },
    )
    .await
    .unwrap();
    assert!(not_a_vehicle.is_none());
    let nobody = auto::bookings::on_checked_in(
        t.state(),
        &BookingCheckedIn {
            booking: BookingId(9_998),
            subject: None,
            at: renox::db::now(),
        },
    )
    .await
    .unwrap();
    assert!(nobody.is_none());
}

/// Delivering the job card completes its booking as the system.
#[renox::test]
async fn aut_base_delivering_the_job_card_completes_the_booking() {
    let t = boot().await;
    let booking = booked(&t).await;
    let sami = t.advisor().await;
    service::check_in(
        t.state(),
        booking.id,
        Actor::staff(sami.id),
        renox::db::now(),
    )
    .await
    .unwrap();
    let card = JobCard::of_booking(t.db(), booking.id)
        .await
        .unwrap()
        .unwrap();
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;

    job_cards::record_intake(
        t.state(),
        &sami,
        card.id,
        Intake {
            complaint: "Periodic service".to_owned(),
            odometer_in: 52_100,
            fuel_level: Some("quarter".to_owned()),
            condition: "Clean".to_owned(),
            acknowledged_by: Some("Mona Hassan".to_owned()),
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: 0,
            storage_fee_daily: 0,
        },
    )
    .await
    .unwrap();
    job_cards::assign_technician(t.state(), &sami, card.id, tareq_user.id)
        .await
        .unwrap();
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let line = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::flat("Service", 2_000, 35_000),
    )
    .await
    .unwrap();
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let round = job_cards::pending_round(t.db(), card.id)
        .await
        .unwrap()
        .unwrap();
    job_cards::approve_by_link(t.state(), &round.token, "Mona Hassan", None, true)
        .await
        .unwrap();
    job_cards::record_work(t.state(), &tareq, line.id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let waleed = t.qc().await;
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(t.state(), &waleed, card.id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(t.state(), &waleed, card.id)
        .await
        .unwrap();

    job_cards::deliver(
        t.state(),
        &sami,
        card.id,
        Delivery {
            delivered_to: "Mona Hassan".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .unwrap();

    let done = Booking::find_or_404(t.db(), booking.id).await.unwrap();
    assert_eq!(done.status, BookingStatus::Completed);
    assert_eq!(t.books.about(&card.subject()).len(), 1);
}
