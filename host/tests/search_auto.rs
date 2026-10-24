//! The workshop's records in the global search and the Related panel
//! (BL-AGT-13): on the demo workshop, a job card is found by its number,
//! the car's plate and its VIN, an insurance claim by its number and the
//! insurer's, a supplier warranty claim and a fleet contract by their
//! numbers, each with its related records; a plate typed with spaces,
//! dashes or in lower case finds the same job cards; a plate moved to
//! another car finds the job cards of both cars, the latest first.

use host::roles::{self, ADMIN, SERVICE_ADVISOR};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the auto client");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@search.example", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

async fn scalar_i64(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn scalar_text(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn ids(app: &TestApp, query: &str) -> Vec<i64> {
    renox::db::sql(query.to_owned())
        .fetch_all(app.db())
        .await
        .unwrap()
        .iter()
        .map(|row| row.try_get::<i64>("id").unwrap())
        .collect()
}

/// The job cards a search for `typed` lists, in order.
async fn job_cards(app: &TestApp, access: &Access, typed: &str) -> Vec<i64> {
    search::search(app.db(), access, typed, 50)
        .await
        .unwrap()
        .groups
        .iter()
        .filter(|group| group.subject_type == "auto.job_card")
        .flat_map(|group| group.hits.iter().map(|hit| hit.subject.subject_id))
        .collect()
}

async fn finds(app: &TestApp, access: &Access, typed: &str, subject: &Subject) -> bool {
    search::search(app.db(), access, typed, 50)
        .await
        .unwrap()
        .groups
        .iter()
        .flat_map(|group| group.hits.iter())
        .any(|hit| &hit.subject == subject)
}

async fn related_count(app: &TestApp, access: &Access, subject: &Subject, name: &str) -> u64 {
    related::related(app.db(), access, subject, PANEL_DEPTH)
        .await
        .unwrap()
        .iter()
        .find(|group| group.name == name)
        .map_or(0, |group| group.count)
}

/// The plate of vehicle `id` as `(emirate, code, number)`.
async fn plate_of(app: &TestApp, id: i64) -> (String, String, String) {
    (
        scalar_text(
            app,
            &format!("select plate_emirate from auto_vehicles where id = {id}"),
        )
        .await,
        scalar_text(
            app,
            &format!("select plate_code from auto_vehicles where id = {id}"),
        )
        .await,
        scalar_text(
            app,
            &format!("select plate_number from auto_vehicles where id = {id}"),
        )
        .await,
    )
}

/// The short code of an emirate's stored name.
fn short(emirate: &str) -> &'static str {
    match emirate {
        "abu_dhabi" => "AUH",
        "dubai" => "DXB",
        "sharjah" => "SHJ",
        "ajman" => "AJM",
        "umm_al_quwain" => "UAQ",
        "ras_al_khaimah" => "RAK",
        _ => "FUJ",
    }
}

#[renox::test]
async fn each_auto_record_is_found_by_its_identifiers_with_its_related_records() {
    let app = booted().await;
    search::rebuild(app.db()).await.unwrap();
    let admin = person(&app, "Ali", ADMIN).await;
    let access = Access::of_user(app.db(), &admin).await.unwrap();

    // A job card by its number, its car's plate and VIN.
    let card = scalar_i64(
        &app,
        "select j.id from auto_job_cards j join auto_vehicles v on v.id = j.vehicle_id \
         where v.vin is not null and v.vin <> '' order by j.id limit 1",
    )
    .await;
    let card_subject = Subject::new("auto.job_card", card);
    assert!(finds(&app, &access, &format!("job {card:05}"), &card_subject).await);
    let vehicle = scalar_i64(
        &app,
        &format!("select vehicle_id from auto_job_cards where id = {card}"),
    )
    .await;
    let vin = scalar_text(
        &app,
        &format!("select vin from auto_vehicles where id = {vehicle}"),
    )
    .await;
    assert!(finds(&app, &access, &vin.to_lowercase(), &card_subject).await);
    let (emirate, code, number) = plate_of(&app, vehicle).await;
    assert!(
        finds(
            &app,
            &access,
            &format!("{} {code} {number}", short(&emirate)),
            &card_subject
        )
        .await
    );
    assert_eq!(
        related_count(&app, &access, &card_subject, "vehicle").await,
        1
    );

    // An insurance claim by its number and the insurer's claim number.
    let claim = scalar_i64(
        &app,
        "select id from auto_insurance_claims order by id limit 1",
    )
    .await;
    let claim_subject = Subject::new("auto.insurance_claim", claim);
    assert!(finds(&app, &access, &format!("CLM-{claim:05}"), &claim_subject).await);
    let claim_no = scalar_text(
        &app,
        &format!("select claim_no from auto_insurance_claims where id = {claim}"),
    )
    .await;
    assert!(finds(&app, &access, &claim_no, &claim_subject).await);
    assert_eq!(
        related_count(&app, &access, &claim_subject, "vehicle").await,
        1
    );

    // A supplier warranty claim by its number.
    let warranty = scalar_i64(
        &app,
        "select id from auto_supplier_claims order by id limit 1",
    )
    .await;
    let warranty_subject = Subject::new("auto.supplier_claim", warranty);
    assert!(
        finds(
            &app,
            &access,
            &format!("swc {warranty:04}"),
            &warranty_subject
        )
        .await
    );
    assert!(
        related::related(app.db(), &access, &warranty_subject, PANEL_DEPTH)
            .await
            .unwrap()
            .iter()
            .any(|group| group.name == "fitted_job_card")
    );

    // A fleet contract by its number, with its vehicles' job cards.
    let contract = scalar_i64(
        &app,
        "select fleet_contract_id as id from auto_vehicles where fleet_contract_id is not null order by id limit 1",
    )
    .await;
    let contract_subject = Subject::new("auto.fleet_contract", contract);
    assert!(
        finds(
            &app,
            &access,
            &format!("FC-{contract:04}"),
            &contract_subject
        )
        .await
    );
    assert!(related_count(&app, &access, &contract_subject, "vehicles").await >= 1);

    // The job card page loads its Related panel.
    let advisor = person(&app, "Sami", SERVICE_ADVISOR).await;
    app.acting_as(&advisor);
    app.get(&format!("/auto/job-cards/{card}"))
        .await
        .assert_ok()
        .assert_see(&format!("hx-get=\"/related/auto.job_card:{card}/panel\""));
    app.get(&format!("/related/auto.job_card:{card}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"vehicle\" data-count=\"1\"");
}

#[renox::test]
async fn a_plate_typed_any_way_finds_the_same_job_cards() {
    let app = booted().await;
    search::rebuild(app.db()).await.unwrap();
    let admin = person(&app, "Ali", ADMIN).await;
    let access = Access::of_user(app.db(), &admin).await.unwrap();
    let vehicle = scalar_i64(
        &app,
        "select vehicle_id from auto_job_cards group by vehicle_id order by count(*) desc, vehicle_id limit 1",
    )
    .await;
    let (emirate, code, number) = plate_of(&app, vehicle).await;
    let mut expected = ids(
        &app,
        &format!("select id from auto_job_cards where vehicle_id = {vehicle}"),
    )
    .await;
    expected.sort_unstable();
    let short = short(&emirate);
    let typed = [
        format!("{short} {code} {number}"),
        format!("{short}-{code}-{number}").to_lowercase(),
        format!("{short}{code}{number}").to_lowercase(),
        format!("{} {}", code.to_lowercase(), number),
    ];
    for plate in &typed {
        let mut found = job_cards(&app, &access, plate).await;
        found.retain(|id| expected.contains(id));
        found.sort_unstable();
        assert_eq!(found, expected, "`{plate}`");
    }
}

#[renox::test]
async fn a_plate_moved_to_another_car_finds_the_job_cards_of_both_cars_latest_first() {
    let app = booted().await;
    let cars = ids(
        &app,
        "select vehicle_id as id from auto_job_cards group by vehicle_id order by vehicle_id limit 2",
    )
    .await;
    let (first, second) = (cars[0], cars[1]);
    let (emirate, code, number) = plate_of(&app, first).await;
    // The first car's cards were opened with its plate.
    renox::db::sql(format!(
        "update auto_job_cards set intake_plate = '{emirate} {code} {number}' where vehicle_id = {first}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    search::rebuild(app.db()).await.unwrap();
    // The plate moves: the first car is re-registered, the second takes it.
    renox::db::sql(format!(
        "update auto_vehicles set plate_code = 'ZZ', plate_number = '99901' where id = {first}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    auto::search::index_vehicle(app.db(), first).await.unwrap();
    renox::db::sql(format!(
        "update auto_vehicles set plate_emirate = '{emirate}', plate_code = '{code}', plate_number = '{number}' where id = {second}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    auto::search::index_vehicle(app.db(), second).await.unwrap();

    let admin = person(&app, "Ali", ADMIN).await;
    let access = Access::of_user(app.db(), &admin).await.unwrap();
    let found = job_cards(
        &app,
        &access,
        &format!("{} {code} {number}", short(&emirate)),
    )
    .await;
    let of_first = ids(
        &app,
        &format!("select id from auto_job_cards where vehicle_id = {first}"),
    )
    .await;
    let of_second = ids(
        &app,
        &format!("select id from auto_job_cards where vehicle_id = {second}"),
    )
    .await;
    for id in of_first.iter().chain(of_second.iter()) {
        assert!(
            found.contains(id),
            "job card {id} of a car that carried the plate"
        );
    }
    // The second car's cards were indexed last: they come first.
    assert!(of_second.contains(&found[0]), "{found:?}");
}
