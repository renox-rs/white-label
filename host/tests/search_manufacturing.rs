//! The manufacturing records in the global search and the Related panel
//! (BL-AGT-13): on the demo plant, a production manager finds a work order
//! by its number and the customer's PO number, an NCR by its number, a
//! return inspection by the dispatch's number and the customer's PO number,
//! each with its related records; the work order page loads its panel.

use host::roles::{self, PRODUCTION_MANAGER};
use manufacturing::returns::ReturnInspection;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manufacturing/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the manufacturing client");
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

#[renox::test]
async fn a_production_manager_finds_work_orders_ncrs_and_return_inspections() {
    let app = booted().await;
    // A work order made for a sales order, whose customer PO is known.
    let order = scalar_i64(
        &app,
        "select id from manufacturing_work_orders where sales_order_id is not null order by id limit 1",
    )
    .await;
    let sales_order = scalar_i64(
        &app,
        &format!("select sales_order_id from manufacturing_work_orders where id = {order}"),
    )
    .await;
    renox::db::sql(format!(
        "update wl_sales_sales_orders set customer_po = 'PO-ALF-7781' where id = {sales_order}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    // A return of that order waiting for its inspection.
    let item = scalar_i64(
        &app,
        &format!("select item_id from manufacturing_work_orders where id = {order}"),
    )
    .await;
    let inspection = ReturnInspection::create(
        app.db(),
        ReturnInspection {
            event_key: "search-test".to_owned(),
            dispatch_id: 1,
            dispatch_number: "DSP-2026-00931".to_owned(),
            order_id: sales_order,
            item_id: item,
            quantity_milli: 2_000,
            status: "open".to_owned(),
            ..ReturnInspection::default()
        },
    )
    .await
    .unwrap();
    search::rebuild(app.db()).await.unwrap();

    let manager = person(&app, "Majed", PRODUCTION_MANAGER).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();

    // The work order by its number and by the customer's PO number.
    let order_subject = Subject::new("manufacturing.work_order", order);
    let number = scalar_text(
        &app,
        &format!("select number from manufacturing_work_orders where id = {order}"),
    )
    .await;
    assert!(finds(&app, &access, &number.to_lowercase(), &order_subject).await);
    assert!(finds(&app, &access, "po alf 7781", &order_subject).await);

    // An NCR by its number, with its related records.
    let ncr = scalar_i64(
        &app,
        "select id from manufacturing_ncrs order by id limit 1",
    )
    .await;
    let ncr_subject = Subject::new("manufacturing.ncr", ncr);
    let ncr_number = scalar_text(
        &app,
        &format!("select number from manufacturing_ncrs where id = {ncr}"),
    )
    .await;
    assert!(finds(&app, &access, &ncr_number, &ncr_subject).await);
    let groups = related::related(app.db(), &access, &ncr_subject, PANEL_DEPTH)
        .await
        .unwrap();
    assert!(groups.iter().any(|group| group.name == "inspections"));

    // The return inspection by the dispatch's number and the customer's PO.
    let inspection_subject = Subject::new("manufacturing.return_inspection", inspection.id);
    assert!(finds(&app, &access, "dsp 2026 00931", &inspection_subject).await);
    assert!(finds(&app, &access, "PO-ALF-7781", &inspection_subject).await);
    assert_eq!(
        related_count(&app, &access, &inspection_subject, "ncr").await,
        0
    );

    // A work order with an NCR raised on it shows it.
    let raised = renox::db::sql(
        "select source_id from manufacturing_ncrs where source_type = 'manufacturing.work_order' order by id limit 1"
            .to_owned(),
    )
    .scalar_optional::<i64>(app.db())
    .await
    .unwrap();
    if let Some(raised) = raised {
        let subject = Subject::new("manufacturing.work_order", raised);
        assert!(related_count(&app, &access, &subject, "ncrs").await >= 1);
    }

    // The work order page loads its Related panel.
    app.acting_as(&manager);
    app.get(&format!("/manufacturing/work-orders/{order}"))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/manufacturing.work_order:{order}/panel\""
        ));
    app.get(&format!("/related/manufacturing.work_order:{order}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"ncrs\"");
}
