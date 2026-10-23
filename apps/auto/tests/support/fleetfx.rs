//! Fixtures of the fleet contract tests: the fleet account manager, a fleet
//! with a vehicle under an active contract and a service plan, and the bay
//! and the service type the plans are booked for.

#![allow(dead_code)]

use auto::fleet_contracts::{self, NewContract};
use auto::fleet_services::{self, NewPlan};
use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, FleetContract, FleetService, Vehicle};
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
