//! The base of the 08 Auto vehicle register (base for #397 .. #403,
//! `AUT.VEH.01`): a vehicle is registered with its UAE plate, VIN, make,
//! model, year, odometer and owner; the plate and the VIN are registered once;
//! the odometer only goes up; the pages list, search and show it.

mod support;

use auto::vehicles::{self, NewVehicle};
use auto::{Emirate, Vehicle, permissions};
use renox::prelude::*;
use support::{Garage, boot};

/// A service advisor who registers vehicles and reads them.
async fn advisor(t: &Garage) -> renox::AuthUser {
    let user = t
        .user(
            "Sami",
            &[
                permissions::VEHICLES_VIEW,
                permissions::VEHICLES_MANAGE,
                permissions::JOB_CARDS_ADVISE,
            ],
        )
        .await;
    t.auth(&user).await
}

/// A Toyota Hilux, Dubai A 12345, owned by party 7.
fn hilux() -> NewVehicle {
    NewVehicle::new(Emirate::Dubai, "a", "12345", "Toyota", "Hilux", 7)
        .vin("JTHBK262582012345")
        .year(2022)
        .engine("2.7 L petrol")
        .odometer(52_000)
}

/// A vehicle is registered with its plate, VIN, make, model, year, odometer
/// and owner; the plate reads as people write it.
#[renox::test]
async fn aut_base_a_vehicle_is_registered_with_its_plate_vin_and_owner() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = advisor(&t).await;

    let vehicle = vehicles::create(t.state(), &sami, hilux()).await.unwrap();
    assert_eq!(vehicle.plate(), "Dubai A 12345");
    assert_eq!(vehicle.vin.as_deref(), Some("JTHBK262582012345"));
    assert_eq!(
        (vehicle.make.as_str(), vehicle.model_name.as_str()),
        ("Toyota", "Hilux")
    );
    assert_eq!(vehicle.model_year, Some(2022));
    assert_eq!(vehicle.owner_party_id, 7);
    assert_eq!(vehicle.fleet_contract_id, None);
    assert_eq!(vehicle.odometer_km, 52_000);
    assert!(vehicle.odometer_at.is_some(), "the reading has a time");
    assert!(vehicle.active);
    assert_eq!(vehicle.created_by, Some(sami.id));
    assert_eq!(
        vehicle.subject().to_string(),
        format!("auto.vehicle:{}", vehicle.id)
    );
}

/// Only a holder of `vehicles.manage` registers a vehicle, and the owner must
/// be a party.
#[renox::test]
async fn aut_base_only_the_manager_registers_and_the_owner_is_a_party() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let reader = t.user("Rana", &[permissions::VEHICLES_VIEW]).await;
    let reader = t.auth(&reader).await;
    let refused = vehicles::create(t.state(), &reader, hilux()).await;
    assert!(matches!(refused, Err(Error::Forbidden)));

    let sami = advisor(&t).await;
    let stranger = NewVehicle::new(Emirate::Dubai, "B", "1", "Nissan", "Patrol", 99);
    let error = vehicles::create(t.state(), &sami, stranger)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Pick the owner"), "{error}");
}

/// The plate is an emirate, a code and a number registered once; the VIN too.
#[renox::test]
async fn aut_base_the_plate_and_the_vin_are_registered_once() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = advisor(&t).await;
    vehicles::create(t.state(), &sami, hilux()).await.unwrap();

    let same_plate = vehicles::create(
        t.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "A", "12345", "Nissan", "Patrol", 7),
    )
    .await
    .unwrap_err();
    assert!(
        same_plate
            .to_string()
            .contains("Dubai A 12345 is registered already"),
        "{same_plate}"
    );

    // The same code and number in another emirate is another vehicle.
    vehicles::create(
        t.state(),
        &sami,
        NewVehicle::new(Emirate::Sharjah, "A", "12345", "Nissan", "Patrol", 7),
    )
    .await
    .unwrap();

    let same_vin = vehicles::create(
        t.state(),
        &sami,
        NewVehicle::new(Emirate::Dubai, "C", "777", "Toyota", "Hilux", 7).vin("jthbk262582012345"),
    )
    .await
    .unwrap_err();
    assert!(
        same_vin.to_string().contains("already on Dubai A 12345"),
        "{same_vin}"
    );
}

/// A plate code, a plate number and a VIN that are wrong are refused with the
/// field named.
#[renox::test]
async fn aut_base_a_wrong_plate_or_vin_is_refused() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = advisor(&t).await;
    for (new, said) in [
        (
            NewVehicle::new(Emirate::Dubai, "ABCD", "1", "Toyota", "Hilux", 7),
            "plate code",
        ),
        (
            NewVehicle::new(Emirate::Dubai, "A", "12a", "Toyota", "Hilux", 7),
            "plate number",
        ),
        (
            NewVehicle::new(Emirate::Dubai, "A", "1", "Toyota", "Hilux", 7).vin("SHORT"),
            "17",
        ),
        (
            NewVehicle::new(Emirate::Dubai, "A", "1", " ", "Hilux", 7),
            "make",
        ),
        (
            NewVehicle::new(Emirate::Dubai, "A", "1", "Toyota", "Hilux", 7).year(1900),
            "model year",
        ),
    ] {
        let error = vehicles::create(t.state(), &sami, new).await.unwrap_err();
        assert!(error.to_string().contains(said), "{said}: {error}");
    }
}

/// The odometer only goes up: a change that reads lower is refused, a higher
/// one is recorded with its time.
#[renox::test]
async fn aut_base_the_odometer_only_goes_up() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = advisor(&t).await;
    let vehicle = vehicles::create(t.state(), &sami, hilux()).await.unwrap();

    let lower = vehicles::update(t.state(), &sami, vehicle.id, hilux().odometer(51_000))
        .await
        .unwrap_err();
    assert!(
        lower.to_string().contains("last reading of 52000 km"),
        "{lower}"
    );

    let higher = vehicles::update(
        t.state(),
        &sami,
        vehicle.id,
        hilux().odometer(60_500).colour("White"),
    )
    .await
    .unwrap();
    assert_eq!(higher.odometer_km, 60_500);
    assert_eq!(higher.colour.as_deref(), Some("White"));
}

/// A vehicle is found by plate, VIN, make or model, in any case.
#[renox::test]
async fn aut_base_a_vehicle_is_found_by_plate_vin_make_or_model() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = advisor(&t).await;
    let hilux = vehicles::create(t.state(), &sami, hilux()).await.unwrap();
    vehicles::create(
        t.state(),
        &sami,
        NewVehicle::new(Emirate::AbuDhabi, "12", "34567", "Nissan", "Patrol", 7),
    )
    .await
    .unwrap();

    for text in [
        "dubai a 12345",
        "12345",
        "jthbk",
        "toyota",
        "HILUX",
        "toyota hilux",
    ] {
        let found = vehicles::search(t.db(), text).await.unwrap();
        assert_eq!(found.len(), 1, "{text}");
        assert_eq!(found[0].id, hilux.id, "{text}");
    }
    assert_eq!(vehicles::search(t.db(), "").await.unwrap().len(), 2);
    assert!(
        vehicles::search(t.db(), "ferrari")
            .await
            .unwrap()
            .is_empty()
    );
    let by_plate = Vehicle::by_plate(t.db(), Emirate::Dubai, "a", "12345")
        .await
        .unwrap();
    assert_eq!(by_plate.map(|v| v.id), Some(hilux.id));
}

/// The list shows the vehicle with its plate and owner; the fleet tab keeps
/// only the fleet vehicles; the page of the vehicle shows its details; a
/// technician has no vehicle screens.
#[renox::test]
async fn aut_base_the_pages_list_search_and_show_the_vehicles() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let sami = advisor(&t).await;
    let private = vehicles::create(t.state(), &sami, hilux()).await.unwrap();
    vehicles::create(
        t.state(),
        &sami,
        NewVehicle::new(Emirate::Sharjah, "D", "9001", "Mitsubishi", "L200", 7).fleet(3),
    )
    .await
    .unwrap();

    let user = t
        .user(
            "Sami",
            &[permissions::VEHICLES_VIEW, permissions::VEHICLES_MANAGE],
        )
        .await;
    t.app.acting_as(&user);
    t.app
        .get("/auto/vehicles")
        .await
        .assert_ok()
        .assert_see("Dubai A 12345")
        .assert_see("Sharjah D 9001")
        .assert_see("Mona Hassan");
    let fleet = t.app.get("/auto/vehicles?kind=fleet").await;
    fleet.assert_ok().assert_see("Sharjah D 9001");
    assert!(!fleet.text().contains("Dubai A 12345"));
    let found = t.app.get("/auto/vehicles?q=hilux").await;
    found.assert_ok().assert_see("Dubai A 12345");
    assert!(!found.text().contains("Sharjah D 9001"));

    t.app
        .get(&format!("/auto/vehicles/{}", private.id))
        .await
        .assert_ok()
        .assert_see("JTHBK262582012345")
        .assert_see("52,000 km")
        .assert_see("Toyota Hilux 2022");

    let technician = t.user("Tareq", &[permissions::JOB_CARDS_WORK]).await;
    t.app.acting_as(&technician);
    t.app.get("/auto/vehicles").await.assert_status(403);
}

/// The form registers a vehicle from the owner's party code and refuses an
/// unknown emirate.
#[renox::test]
async fn aut_base_the_form_registers_a_vehicle_by_party_code() {
    let t = boot().await;
    t.people.add(7, "Mona Hassan");
    let user = t
        .user(
            "Sami",
            &[permissions::VEHICLES_VIEW, permissions::VEHICLES_MANAGE],
        )
        .await;
    t.app.acting_as(&user);
    let fields = [
        ("emirate", "dubai"),
        ("plate_code", "k"),
        ("plate_number", "4321"),
        ("make", "Kia"),
        ("model_name", "Sportage"),
        ("model_year", "2021"),
        ("owner", "P-00007"),
        ("odometer", "12,500"),
    ];
    t.app
        .htmx()
        .post("/auto/vehicles", &fields)
        .await
        .assert_ok();
    let stored = Vehicle::by_plate(t.db(), Emirate::Dubai, "K", "4321")
        .await
        .unwrap()
        .expect("the vehicle was registered");
    assert_eq!(stored.owner_party_id, 7);
    assert_eq!(stored.odometer_km, 12_500);

    let bad = [
        ("emirate", "riyadh"),
        ("plate_code", "k"),
        ("plate_number", "1"),
        ("make", "Kia"),
        ("model_name", "Rio"),
        ("owner", "7"),
    ];
    t.app
        .htmx()
        .post("/auto/vehicles", &bad)
        .await
        .assert_status(422);
}
