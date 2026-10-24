//! Fixtures of the insurance claim tests: the coordinator and the accountant,
//! an insurer with a rate card, an estimate sent on a job card, and a
//! registered claim.

#![allow(dead_code)]

use auto::claims::{self, NewClaim};
use auto::insurers::{self, NewInsurer};
use auto::job_cards::{self, NewJobCard, NewLabour, NewPart, Promise};
use auto::{InsuranceClaim, Insurer, JobCard, JobCardStatus, LabourLine, PartLine};
use renox::prelude::*;
use wl_kernel::Subject;

use super::{Garage, day};

/// The coordinator Carla: registers claims and records the insurer's answers.
pub async fn carla(t: &Garage) -> AuthUser {
    let user = t
        .user(
            "Carla",
            &[
                auto::permissions::INSURANCE_MANAGE,
                auto::permissions::JOB_CARDS_VIEW,
                auto::permissions::VEHICLES_VIEW,
            ],
        )
        .await;
    t.auth(&user).await
}

/// The accountant Aisha: issues the invoices of the claims.
pub async fn aisha(t: &Garage) -> AuthUser {
    let user = t
        .user(
            "Aisha",
            &[
                auto::permissions::INVOICES_ISSUE,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    t.auth(&user).await
}

/// Orient Insurance (party 9), billed under payer agreement 3, with a rate
/// card (version 2) in force since 30 days.
pub async fn orient(t: &Garage) -> Insurer {
    t.people.add(9, "Orient Insurance");
    let carla = carla(t).await;
    let insurer = insurers::register(
        t.state(),
        &carla,
        NewInsurer::new(9)
            .agreement(3)
            .email("claims@orient.example")
            .surveyors("Samir 050 000 0000")
            .labour_rate(12_000),
    )
    .await
    .expect("the insurer is recorded");
    t.cards.accept(insurer.subject(), 31, 2, day(-30));
    insurer
}

/// The lines of the estimate that was sent.
pub struct Estimate {
    /// The job card, awaiting the answer.
    pub card: JobCard,
    /// The door repair, AED 400.
    pub door: LabourLine,
    /// The bumper cover, AED 600.
    pub bumper: PartLine,
    /// The wheel alignment, AED 100.
    pub alignment: LabourLine,
}

/// The Hilux checked in, inspected, and its estimate sent: a door repair
/// (AED 400), a bumper cover (AED 600) and a wheel alignment (AED 100).
pub async fn sent_estimate(t: &Garage) -> Estimate {
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
            .workshop(Subject::new("scheduling.location", 1)),
    )
    .await
    .unwrap();
    let tareq = t.technician("Tareq").await;
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let door = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Door panel repair", 2_000, 20_000),
    )
    .await
    .unwrap();
    let bumper = job_cards::add_part(
        t.state(),
        &tareq,
        card.id,
        NewPart::new("Bumper cover", 1_000, 60_000),
    )
    .await
    .unwrap();
    let alignment = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Wheel alignment", 1_000, 10_000),
    )
    .await
    .unwrap();
    let card = job_cards::send_estimate(t.state(), &sami, card.id, Promise::default())
        .await
        .unwrap();
    assert_eq!(card.status, JobCardStatus::AwaitingApproval);
    Estimate {
        card,
        door,
        bumper,
        alignment,
    }
}

/// A claim of Orient for the Hilux, funding the job card, with an excess of
/// AED 200 and the police report.
pub async fn registered(t: &Garage, job_card: i64) -> InsuranceClaim {
    let insurer = orient(t).await;
    let carla = carla(t).await;
    let vehicle = t.hilux().await;
    claims::register(
        t.state(),
        &carla,
        NewClaim::new(insurer.id, vehicle.id, "POL-7788", "OR-2026-118", day(-3))
            .job_card(job_card)
            .police_report("DXB-PR-5521", Some(day(-2)))
            .excess(20_000),
    )
    .await
    .expect("the claim is registered")
}
