//! Shared by the host tests of the Manufacturing vertical: a manufacturer
//! booted with the real wl-inventory, a person of the plant by role, a GET the
//! way a browser sends it, a form post the way htmx sends it, and a plant with
//! one line whose locations are real.

#![allow(dead_code)]

use manufacturing::{Line, Plant, WorkOrder, WorkOrderStatus};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_inventory::{Item, Location};
use wl_kernel::ClientConfig;

/// A manufacturer: the vertical on, with the shared modules it reaches.
pub const CLIENT: &str = r#"
[client]
name = "Gulf Plastics LLC"
app = "manufacturing"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "inventory", "portal"]

[[legal_entities]]
code = "GPL"
name_en = "Gulf Plastics LLC"
default = true
"#;

pub async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    app
}

pub async fn person(app: &TestApp, name: &str, role: &str, scope: Option<Scope>) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    match scope {
        Some(plant) => user.assign_role_in(app.db(), role, &plant).await.unwrap(),
        None => user.assign_role(app.db(), role).await.unwrap(),
    }
    user
}

pub async fn browse(app: &TestApp, by: &User, path: &str) -> TestResponse {
    app.acting_as(by);
    app.request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await
}

pub async fn post(app: &TestApp, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
    app.acting_as(by);
    app.htmx().post(path, form).await
}

/// A plant with one line whose store and finished goods are real locations of
/// wl-inventory, and a released work order for 100 panels.
pub struct Floor {
    pub plant: Plant,
    pub line: Line,
    pub order: WorkOrder,
    pub panel: Item,
    pub resin: Item,
    pub store: Location,
    pub finished: Location,
}

pub async fn floor(app: &TestApp) -> Floor {
    let plant = Plant::create(
        app.db(),
        Plant {
            name: "Plant 1".into(),
            active: true,
            ..Plant::default()
        },
    )
    .await
    .unwrap();
    let mut store = Location::new("RM-STORE", "Raw material store");
    store.save(app.db()).await.unwrap();
    let mut finished = Location::new("FG-STORE", "Finished goods");
    finished.save(app.db()).await.unwrap();
    let mut panel = Item::new("PANEL", "Panel", "pcs").lot_tracked();
    panel.save(app.db()).await.unwrap();
    let mut resin = Item::new("RESIN", "Resin", "kg").lot_tracked();
    resin.average_cost = 800;
    resin.save(app.db()).await.unwrap();
    let line = Line::create(
        app.db(),
        Line {
            plant_id: plant.id,
            code: "L1".into(),
            name: "Line 1".into(),
            hourly_rate: 12_000,
            store_location_id: Some(store.id),
            finished_location_id: Some(finished.id),
            active: true,
            ..Line::default()
        },
    )
    .await
    .unwrap();
    let mut order = WorkOrder {
        number: "WO-HOST-1".into(),
        plant_id: plant.id,
        line_id: Some(line.id),
        item_id: panel.id,
        bom_id: Some(1),
        quantity_milli: 100_000,
        status: WorkOrderStatus::Released,
        std_material_cost: 1_800,
        std_unit_cost: 2_400,
        ..WorkOrder::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, WorkOrderStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Floor {
        plant,
        line,
        order,
        panel,
        resin,
        store,
        finished,
    }
}
