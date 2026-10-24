//! The logistics records in the global search and the Related panel
//! (BL-AGT-12): on the demo company, a shipment is found by its number, its
//! client reference and a parcel's barcode, a freight job by its number, its
//! AWB / B/L and its container number, a COD remittance by its number, each
//! with its related records; a container used on two jobs finds both, the
//! latest first; the shipment page loads its Related panel.

use host::roles::{self, OPERATIONS_MANAGER};
use logistics::freight::{self, NewJob};
use logistics::remittance::RemittanceLine;
use logistics::{Remittance, RemittanceStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access, SearchResults};
use wl_kernel::{ClientConfig, PartyId, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-logistics/client.toml"
);

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

async fn results(app: &TestApp, access: &Access, typed: &str) -> SearchResults {
    search::search(app.db(), access, typed, 50).await.unwrap()
}

async fn finds(app: &TestApp, access: &Access, typed: &str, subject: &Subject) -> bool {
    results(app, access, typed)
        .await
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

#[renox::test]
async fn an_operations_manager_finds_each_logistics_record_by_its_identifiers() {
    let config = ClientConfig::load(CLIENT).expect("the demo logistics client");
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel()
        .call(host::logistics_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");

    // A remittance of the first shipment, as the cashier prepares it.
    let shipment = scalar_i64(
        &app,
        "select id from logistics_shipments order by id limit 1",
    )
    .await;
    let client = scalar_i64(
        &app,
        &format!("select client_party_id from logistics_shipments where id = {shipment}"),
    )
    .await;
    let mut remittance = Remittance {
        number: "REM-NEC-2026-0007".to_owned(),
        client_party_id: client,
        period_end: renox::db::now(),
        status: RemittanceStatus::Approved,
        ..Remittance::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    remittance
        .insert_in_state(&mut tx, RemittanceStatus::Approved, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = RemittanceLine {
        remittance_id: remittance.id,
        shipment_id: shipment,
        ..RemittanceLine::default()
    };
    line.insert(app.db()).await.unwrap();
    // The demo job's bill of lading, recorded at departure.
    let job = scalar_i64(
        &app,
        "select id from logistics_freight_jobs order by id limit 1",
    )
    .await;
    renox::db::sql(format!(
        "update logistics_freight_jobs set bl_awb = 'MAEU 240 556 781' where id = {job}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    search::rebuild(app.db()).await.unwrap();

    let manager = person(&app, "Omar", OPERATIONS_MANAGER).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();

    // The shipment: its number, the client's reference, a parcel's barcode.
    let shipment_subject = Subject::new("logistics.shipment", shipment);
    let number = scalar_text(
        &app,
        &format!("select number from logistics_shipments where id = {shipment}"),
    )
    .await;
    assert!(finds(&app, &access, &number.to_lowercase(), &shipment_subject).await);
    let reference = scalar_text(
        &app,
        &format!("select client_reference from logistics_shipments where id = {shipment}"),
    )
    .await;
    assert!(finds(&app, &access, &reference, &shipment_subject).await);
    let barcode = scalar_text(
        &app,
        &format!("select barcode from logistics_parcels where shipment_id = {shipment} order by id limit 1"),
    )
    .await;
    assert!(finds(&app, &access, &barcode, &shipment_subject).await);
    assert!(related_count(&app, &access, &shipment_subject, "parcels").await >= 1);
    assert_eq!(
        related_count(&app, &access, &shipment_subject, "remittances").await,
        1
    );

    // The freight job: its number, its B/L typed with spaces, its container.
    let job_subject = Subject::new("logistics.freight_job", job);
    let job_number = scalar_text(
        &app,
        &format!("select number from logistics_freight_jobs where id = {job}"),
    )
    .await;
    assert!(finds(&app, &access, &job_number, &job_subject).await);
    assert!(finds(&app, &access, "maeu240556781", &job_subject).await);
    assert!(finds(&app, &access, "MSKU 123 4567", &job_subject).await);
    assert_eq!(
        related_count(&app, &access, &job_subject, "containers").await,
        1
    );

    // The remittance by its number, with its shipment.
    let remittance_subject = Subject::new("logistics.remittance", remittance.id);
    assert!(finds(&app, &access, "rem nec 2026 0007", &remittance_subject).await);
    assert_eq!(
        related_count(&app, &access, &remittance_subject, "shipments").await,
        1
    );

    // The same container on a later job: both match, the latest first.
    let new = NewJob {
        mode: "sea".to_owned(),
        origin: "Ningbo".to_owned(),
        destination: "Jebel Ali".to_owned(),
        eta: None,
        international: true,
    };
    let later = freight::create(app.state(), None, PartyId(client), "Second move", &new)
        .await
        .unwrap();
    freight::insert_container(
        app.state(),
        later.id,
        "MSKU1234567",
        "40HC",
        [None, None, None],
    )
    .await
    .unwrap();
    let found = results(&app, &access, "msku1234567").await;
    let jobs: Vec<i64> = found
        .groups
        .iter()
        .filter(|group| group.subject_type == "logistics.freight_job")
        .flat_map(|group| group.hits.iter().map(|hit| hit.subject.subject_id))
        .collect();
    assert_eq!(jobs, vec![later.id, job]);

    // The shipment page loads its Related panel.
    app.acting_as(&manager);
    app.get(&format!("/shipments/{shipment}"))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/logistics.shipment:{shipment}/panel\""
        ));
    app.get(&format!("/related/logistics.shipment:{shipment}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"remittances\" data-count=\"1\"");
}
