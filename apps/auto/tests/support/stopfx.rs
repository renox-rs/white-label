//! Fixtures of the claims that end before the repair is paid (BL-AUT-08 ..
//! BL-AUT-10): a job card in progress under an approved claim with its labour
//! done, a vehicle booked and checked in, and a job card waiting for the
//! estimate with an inspection fee agreed.

#![allow(dead_code)]

use auto::claims::{self, Answer};
use auto::job_cards::{self, Intake, NewJobCard, NewLabour, Promise};
use auto::{InsuranceClaim, JobCard, LabourLine, WorkStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_kernel::{PartyId, Subject};
use wl_scheduling::service::{self, Actor, NewBooking};
use wl_scheduling::{AvailabilityRule, Booking, Resource, ResourceType, ServiceType, resource};

use super::Garage;
use super::claimfx::{carla, registered};

/// The workshop of the bookings.
pub fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// The Hilux booked and checked in: the check-in opened the job card, in
/// `checked_in`, without its intake.
pub async fn checked_in(t: &Garage) -> (JobCard, Booking) {
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
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let start = renox::db::now() - TimeDelta::hours(2);
    let booking = service::book(
        t.state(),
        NewBooking::new(PartyId(7), service.id, &[bay.id], start)
            .for_subject(vehicle.subject())
            .at(workshop()),
        Actor::staff(sami.id),
        start - TimeDelta::hours(1),
    )
    .await
    .expect("the booking is made");
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
        .expect("the check-in opened the job card");
    (card, booking)
}

/// The Hilux booked, checked in, its intake recorded with the inspection fee
/// `fee` agreed, inspected and, when `estimate`, a brake-pad estimate sent.
pub async fn waiting(t: &Garage, fee: i64, estimate: bool) -> (JobCard, Booking) {
    let (card, booking) = checked_in(t).await;
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;
    job_cards::record_intake(
        t.state(),
        &sami,
        card.id,
        Intake {
            complaint: "Brake noise".to_owned(),
            odometer_in: 52_100,
            fuel_level: Some("half".to_owned()),
            condition: "Scratch on the left door".to_owned(),
            acknowledged_by: Some("Mona Hassan".to_owned()),
            signature: None,
            accident_damage: false,
            police_report_no: None,
            inspection_fee: fee,
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
    if estimate {
        job_cards::add_labour(
            t.state(),
            &tareq,
            card.id,
            NewLabour::hourly("Brake pads", 1_000, 20_000),
        )
        .await
        .unwrap();
        job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
            .await
            .unwrap();
    }
    (t.reload(card.id).await, booking)
}

/// A job card under an approved claim, in progress.
pub struct Running {
    /// The job card, `in_progress`.
    pub card: JobCard,
    /// The claim, `approved`.
    pub claim: InsuranceClaim,
    /// The labour lines, approved by the insurer at their price.
    pub lines: Vec<LabourLine>,
}

/// The Hilux's job card in progress under an approved claim: the labour lines
/// `(description, hours in thousandths, hourly rate in fils)`, all approved by
/// the insurer at their price, and the lines at the positions `done` marked
/// done by the technician.
pub async fn running(t: &Garage, labour: &[(&str, i64, i64)], done: &[usize]) -> Running {
    let vehicle = t.hilux().await;
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(vehicle.id, "Right side hit in a parking lot")
            .odometer(52_400)
            .condition("Right door dented, bumper cracked")
            .acknowledged_by("Mona Hassan")
            .technician(tareq_user.id)
            .workshop(workshop()),
    )
    .await
    .unwrap();
    let tareq = t.technician("Tareq").await;
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let mut lines = Vec::new();
    for (description, hours, rate) in labour {
        lines.push(
            job_cards::add_labour(
                t.state(),
                &tareq,
                card.id,
                NewLabour::hourly(description, *hours, *rate),
            )
            .await
            .unwrap(),
        );
    }
    job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    let claim = registered(t, card.id).await;
    let carla = carla(t).await;
    claims::submit_estimate(t.state(), &carla, claim.id)
        .await
        .unwrap();
    let mut answer = Answer::new("LPO-80", "orient-approval-email-80");
    for line in &lines {
        answer = answer.labour(line.id, line.price());
    }
    claims::approve(t.state(), &carla, claim.id, answer)
        .await
        .unwrap();
    for index in done {
        job_cards::record_work(t.state(), &tareq, lines[*index].id, WorkStatus::Done, None)
            .await
            .unwrap();
    }
    Running {
        card: t.reload(card.id).await,
        claim: InsuranceClaim::find_or_404(t.db(), claim.id).await.unwrap(),
        lines,
    }
}
