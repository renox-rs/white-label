//! Fixtures of the fleet contract tests: the fleet account manager, a fleet
//! with a vehicle under an active contract and a service plan, and the bay
//! and the service type the plans are booked for.

#![allow(dead_code)]

use auto::fleet_contracts::{self, NewContract};
use auto::fleet_services::{self, NewPlan};
use auto::job_cards::{self, Delivery, NewJobCard, NewLabour, Promise};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, FleetContract, FleetService, JobCard, Vehicle, WorkStatus};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_scheduling::{AvailabilityRule, Resource, ResourceType, ServiceType, resource};

use super::{Garage, day};

/// The fleet (party 20) and its contact (party 21).
pub const FLEET: i64 = 20;
/// The fleet's contact.
pub const CONTACT: i64 = 21;

/// The workshop of the bookings.
pub fn workshop() -> Subject {
    Subject::new("scheduling.location", 1)
}

/// The fleet account manager Fatima.
pub async fn fatima(t: &Garage) -> AuthUser {
    let user = t
        .user(
            "Fatima",
            &[
                auto::permissions::FLEET_CONTRACTS_MANAGE,
                auto::permissions::VEHICLES_VIEW,
                auto::permissions::VEHICLES_MANAGE,
                auto::permissions::JOB_CARDS_VIEW,
            ],
        )
        .await;
    t.auth(&user).await
}

/// A bay that is open every day and the periodic service that needs it.
pub async fn bay_and_service(t: &Garage) -> (Resource, ServiceType) {
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

/// A draft contract of the fleet for the term `start` .. `end` days from today,
/// at AED 120 an hour, with an early termination fee of AED 2,000.
pub async fn draft_contract(t: &Garage, start: i64, end: i64) -> FleetContract {
    t.people.add(FLEET, "Al Noor Logistics");
    t.people.add(CONTACT, "Hamdan Saeed");
    let fatima = fatima(t).await;
    fleet_contracts::create(
        t.state(),
        &fatima,
        NewContract::new(FLEET, "Fleet service 2026", day(start), day(end))
            .contact(CONTACT)
            .labour_rate(12_000)
            .tier("fleet")
            .inclusions("Periodic service and brake checks")
            .fee(200_000),
    )
    .await
    .expect("the contract is agreed")
}

/// A van of the fleet (a Nissan Urvan, plate B 555) at `odometer` kilometres.
pub async fn van(t: &Garage, odometer: i64) -> Vehicle {
    t.people.add(FLEET, "Al Noor Logistics");
    let fatima = fatima(t).await;
    if let Some(found) = Vehicle::by_plate(t.db(), Emirate::Dubai, "B", "555")
        .await
        .unwrap()
    {
        return found;
    }
    vehicles::create(
        t.state(),
        &fatima,
        NewVehicle::new(Emirate::Dubai, "B", "555", "Nissan", "Urvan", FLEET).odometer(odometer),
    )
    .await
    .expect("the van is registered")
}

/// An active contract (from 40 days ago to 300 days ahead) with the van under
/// it and a plan that books `periodic_service` every 10,000 km, the last
/// service at 50,000 km; the van reads `odometer` kilometres.
pub async fn active_contract(t: &Garage, odometer: i64) -> (FleetContract, Vehicle, FleetService) {
    let fatima = fatima(t).await;
    let contract = draft_contract(t, -40, 300).await;
    let van = van(t, odometer).await;
    fleet_contracts::add_vehicle(t.state(), &fatima, contract.id, van.id)
        .await
        .unwrap();
    let plan = fleet_services::add_plan(
        t.state(),
        &fatima,
        contract.id,
        NewPlan::new(van.id, "periodic_service")
            .every_km(10_000)
            .last_service(day(-100), 50_000),
    )
    .await
    .unwrap();
    fleet_contracts::attach_document(t.state(), &fatima, contract.id, "signed-fleet-2026")
        .await
        .unwrap();
    let active = fleet_contracts::activate(t.state(), &fatima, contract.id)
        .await
        .expect("the contract is activated");
    let van = Vehicle::find_or_404(t.db(), van.id).await.unwrap();
    let plan = FleetService::find_or_404(t.db(), plan.id).await.unwrap();
    (active, van, plan)
}

/// The user who approves a termination: the owner (`fleet_contracts.approve`),
/// who does not manage the contracts.
pub async fn omar(t: &Garage) -> User {
    t.user(
        "Omar",
        &[
            auto::permissions::FLEET_CONTRACTS_APPROVE,
            auto::permissions::VEHICLES_VIEW,
            auto::permissions::JOB_CARDS_VIEW,
        ],
    )
    .await
}

/// A job card of `van` under its contract, checked in, with one hourly labour
/// line of `hours_milli` at `rate` fils that the customer approved: in
/// progress. Answers the job card and the labour line.
pub async fn job_in_progress(
    t: &Garage,
    van: &Vehicle,
    hours_milli: i64,
    rate: i64,
) -> (JobCard, i64) {
    let sami = t.advisor().await;
    let tareq_user = t.technician_user("Tareq").await;
    let tareq = t.technician("Tareq").await;
    let card = job_cards::check_in(
        t.state(),
        &sami,
        NewJobCard::new(van.id, "Periodic service")
            .odometer(van.odometer_km + 10)
            .condition("Clean")
            .acknowledged_by("Hamdan Saeed")
            .technician(tareq_user.id),
    )
    .await
    .expect("the van is checked in");
    job_cards::start_inspection(t.state(), &tareq, card.id)
        .await
        .unwrap();
    let line = job_cards::add_labour(
        t.state(),
        &tareq,
        card.id,
        NewLabour::hourly("Periodic service", hours_milli, rate),
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
    job_cards::approve_by_link(t.state(), &round.token, "Hamdan Saeed", None, true)
        .await
        .unwrap();
    (t.reload(card.id).await, line.id)
}

/// The work of `line_id` is done, the job card passes its quality control and
/// is delivered to the customer: delivered.
pub async fn finish_and_deliver(t: &Garage, card_id: i64, line_id: i64) -> JobCard {
    let sami = t.advisor().await;
    let tareq = t.technician("Tareq").await;
    job_cards::record_work(t.state(), &tareq, line_id, WorkStatus::Done, None)
        .await
        .unwrap();
    job_cards::finish_work(t.state(), &tareq, card_id)
        .await
        .unwrap();
    let waleed = t.qc().await;
    let all: Vec<String> = auto::job_card::QC_KEYS
        .iter()
        .map(|k| (*k).to_owned())
        .collect();
    job_cards::save_qc(t.state(), &waleed, card_id, &all)
        .await
        .unwrap();
    job_cards::pass_qc(t.state(), &waleed, card_id)
        .await
        .unwrap();
    job_cards::deliver(
        t.state(),
        &sami,
        card_id,
        Delivery {
            delivered_to: "Hamdan Saeed".to_owned(),
            ..Delivery::default()
        },
    )
    .await
    .expect("the job card is delivered")
}

/// A second van of the fleet (a Nissan Urvan, plate B 556) at `odometer`
/// kilometres, under `contract_id`, with a plan that is not due (the last
/// service at `odometer` less 1,000 km, every 10,000 km).
pub async fn second_van(t: &Garage, contract_id: i64, odometer: i64) -> (Vehicle, FleetService) {
    let fatima = fatima(t).await;
    let van = vehicles::create(
        t.state(),
        &fatima,
        NewVehicle::new(Emirate::Dubai, "B", "556", "Nissan", "Urvan", FLEET).odometer(odometer),
    )
    .await
    .expect("the second van is registered");
    fleet_contracts::add_vehicle(t.state(), &fatima, contract_id, van.id)
        .await
        .unwrap();
    let plan = fleet_services::add_plan(
        t.state(),
        &fatima,
        contract_id,
        NewPlan::new(van.id, "periodic_service")
            .every_km(10_000)
            .last_service(day(-100), odometer - 1_000),
    )
    .await
    .unwrap();
    (van, plan)
}

/// A job card of `van` delivered `days_ago` days ago: one hour at AED 120.
pub async fn delivered_job(t: &Garage, van: &Vehicle, days_ago: i64) -> JobCard {
    let (card, line) = job_in_progress(t, van, 1_000, 12_000).await;
    finish_and_deliver(t, card.id, line).await;
    let at = renox::db::now() - renox::chrono::TimeDelta::days(days_ago);
    JobCard::query()
        .where_(JobCard::ID.eq(card.id))
        .update(t.db(), &[("delivered_at", &Some(at))])
        .await
        .unwrap();
    t.reload(card.id).await
}

/// Gives the plan `plan_id` the booking `booking_id`, for the day `date`, as
/// the daily run would have.
pub async fn book_plan(t: &Garage, plan_id: i64, booking_id: i64, date: renox::chrono::NaiveDate) {
    FleetService::query()
        .where_(FleetService::ID.eq(plan_id))
        .update(
            t.db(),
            &[
                ("booking_id", &Some(booking_id)),
                ("requested_for_date", &Some(date)),
            ],
        )
        .await
        .unwrap();
}
