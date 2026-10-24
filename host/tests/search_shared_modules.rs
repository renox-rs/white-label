//! The shared modules of F2–F10 in the search index and the Related panel,
//! on the demo clients (BL-AGT-11): the demo seeds end with a full index; in
//! the F7 client a lot number finds the lot and its Related panel shows its
//! holds and the dispatches that shipped it; in the F9 client an employee
//! number finds the employee and a passport number finds them only on the
//! exact value, shown masked; in the F3 client a tracking number finds the
//! delivery and its run.

mod support;

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::inventory::LOT_SUBJECT_TYPE;
use wl_kernel::related::{self, RelatedGroup};
use wl_kernel::scopes::Reach;
use wl_kernel::search::{self, Access, kinds};
use wl_kernel::workforce::EMPLOYEE_SUBJECT_TYPE;
use wl_kernel::{ClientConfig, Subject};

use support::noon_clock;

const MANUFACTURING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manufacturing/client.toml"
);
const MANPOWER: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manpower/client.toml"
);
const LOGISTICS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-logistics/client.toml"
);

/// The demo client at `path`, installed and seeded as the demo does.
async fn installed(path: &str) -> TestApp {
    let config = ClientConfig::load(path).expect("the demo client");
    let app = TestApp::new(
        host::app_for(config)
            .expect("the app")
            .provide(noon_clock()),
    )
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Someone holding `permissions` everywhere.
fn holding(permissions: &[&str]) -> Access {
    Access::with(
        1,
        permissions
            .iter()
            .map(|p| ((*p).to_owned(), Reach::everything())),
    )
}

fn count(groups: &[RelatedGroup], name: &str) -> Option<u64> {
    groups.iter().find(|g| g.name == name).map(|g| g.count)
}

async fn found(db: &Db, text: &str, subject: &Subject) -> bool {
    search::lookup_identifier(db, text)
        .await
        .unwrap()
        .iter()
        .any(|m| &m.subject == subject)
}

#[renox::test]
async fn f7_a_lot_number_finds_the_lot_and_related_shows_its_holds_and_dispatches() {
    let app = installed(MANUFACTURING).await;
    host::manufacturing_demo::seed(app.kernel())
        .await
        .expect("the demo company is seeded");

    // The finished lot of food boxes left on the demo's dispatches.
    let lot = wl_inventory::Lot::query()
        .where_eq("number", "F-BOX-0901")
        .first(app.db())
        .await
        .unwrap()
        .expect("the demo lot");
    let subject = Subject::new(LOT_SUBJECT_TYPE, lot.id);
    // The seed ended with the full index: the number, typed loosely, finds
    // the lot.
    assert!(found(app.db(), "f box 0901", &subject).await);

    // Quality holds what is left of it.
    wl_inventory::hold_service::place(
        app.state(),
        wl_inventory::hold_service::NewHold {
            reason: "quality".to_owned(),
            reference: Subject::new("ticketing.ticket", 1),
            scope: wl_inventory::hold_service::NewScope::Lots(vec![lot.id]),
            by: None,
        },
    )
    .await
    .expect("the hold is placed");

    let access = holding(&[
        wl_inventory::permissions::STOCK_VIEW,
        wl_inventory::permissions::ITEMS_VIEW,
        wl_inventory::permissions::RECALLS_VIEW,
    ]);
    let groups = related::related(app.db(), &access, &subject, 1)
        .await
        .unwrap();
    assert!(count(&groups, "holds").unwrap_or(0) >= 1, "{groups:?}");
    assert!(count(&groups, "dispatches").unwrap_or(0) >= 1, "{groups:?}");
    assert_eq!(count(&groups, "item"), Some(1));
}

#[renox::test]
async fn f9_an_employee_number_and_the_exact_passport_number_find_the_employee() {
    let app = installed(MANPOWER).await;
    host::manpower_demo::seed(app.kernel())
        .await
        .expect("the demo agency is seeded");
    let employee = wl_hr::Employee::query()
        .where_eq("employee_number", "DSM-0001")
        .first(app.db())
        .await
        .unwrap()
        .expect("the first demo worker");
    let subject = Subject::new(EMPLOYEE_SUBJECT_TYPE, employee.id);
    assert!(found(app.db(), "dsm 0001", &subject).await);

    // The worker's passport, kept by the documents module; the employee's
    // next write (here the rebuild) brings its number into the index.
    let kind = wl_documents::DocumentKind::by_key(app.db(), "passport")
        .await
        .unwrap()
        .expect("the passport kind of the client");
    wl_documents::Document::create(
        app.db(),
        wl_documents::Document {
            subject_type: EMPLOYEE_SUBJECT_TYPE.to_owned(),
            subject_id: employee.id,
            kind_id: kind.id,
            title: "Passport".to_owned(),
            number: Some("P7712345".to_owned()),
            current_version: 1,
            status: wl_documents::DocumentStatus::Accepted,
            channel: "back_office".to_owned(),
            ..wl_documents::Document::default()
        },
    )
    .await
    .unwrap();
    search::rebuild_type(app.db(), EMPLOYEE_SUBJECT_TYPE)
        .await
        .unwrap();

    // Stored as a hash, never in the text.
    let identifiers = search::identifiers_of(app.db(), &subject).await.unwrap();
    let passport = identifiers
        .iter()
        .find(|row| row.kind == kinds::PASSPORT)
        .expect("the passport is an identifier of the employee");
    assert!(passport.hashed);
    let document = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    for text in [&document.title, &document.secondary, &document.keywords] {
        assert!(!text.contains("7712345"));
    }
    // Found on the exact value only.
    assert!(found(app.db(), "P7712345", &subject).await);
    assert!(!found(app.db(), "7712345", &subject).await);
    assert!(!found(app.db(), "P771234", &subject).await);

    // The result shows it masked.
    let access = holding(&[wl_hr::permissions::EMPLOYEES_VIEW]);
    let results = search::search(app.db(), &access, "P7712345", 5)
        .await
        .unwrap();
    let hit = results
        .groups
        .iter()
        .flat_map(|g| g.hits.iter())
        .find(|hit| hit.subject == subject)
        .expect("the employee is a result");
    assert_eq!(hit.matched.as_deref(), Some("•••• 2345"));
}

#[renox::test]
async fn f3_a_tracking_number_finds_the_delivery_and_its_run() {
    let config = ClientConfig::load(LOGISTICS).expect("the demo client");
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call(host::logistics_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");
    // The demo's shipments asked wl-delivery for their deliveries.
    let delivery = wl_delivery::Delivery::query()
        .order_by("id")
        .first(app.db())
        .await
        .unwrap()
        .expect("a demo delivery");
    let delivery_subject = Subject::new(wl_delivery::search::DELIVERY_SUBJECT_TYPE, delivery.id);
    assert!(found(app.db(), &delivery.number, &delivery_subject).await);

    // A run of today carries it.
    let driver = User::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a demo user");
    let run = wl_delivery::plan::create_run(
        app.state(),
        wl_delivery::NewRun::new(wl_kernel::documents::today(), driver.id)
            .vehicle(wl_delivery::VehicleType::Van, Some(1)),
    )
    .await
    .unwrap();
    // Linked as `assign` links it (the transition and its guards are the
    // module's own tests); the index is then built again as the seeds end.
    let run_id = Some(run.id);
    let sequence = Some(1_i64);
    wl_delivery::Delivery::query()
        .where_eq("id", delivery.id)
        .update(
            app.db(),
            &[("run_id", &run_id), ("run_sequence", &sequence)],
        )
        .await
        .unwrap();
    search::rebuild(app.db()).await.unwrap();

    let run_subject = Subject::new(wl_delivery::search::RUN_SUBJECT_TYPE, run.id);
    let typed = delivery.number.to_lowercase().replace('-', " ");
    assert!(found(app.db(), &typed, &delivery_subject).await);
    assert!(found(app.db(), &typed, &run_subject).await);

    // The shipment's own number, once the logistics app indexes its
    // shipments, is a tracking number of the delivery and the run too.
    let shipment = logistics::Shipment::query()
        .where_eq("delivery_id", delivery.id)
        .first(app.db())
        .await
        .unwrap();
    if let Some(shipment) = shipment
        && search::searchable_type("logistics.shipment").is_some()
    {
        search::rebuild_type(app.db(), wl_delivery::search::DELIVERY_SUBJECT_TYPE)
            .await
            .unwrap();
        search::rebuild_type(app.db(), wl_delivery::search::RUN_SUBJECT_TYPE)
            .await
            .unwrap();
        assert!(found(app.db(), &shipment.number, &delivery_subject).await);
        assert!(found(app.db(), &shipment.number, &run_subject).await);
    }
}
