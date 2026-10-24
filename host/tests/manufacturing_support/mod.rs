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
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;

use wl_inventory::{Item, Location};
use wl_kernel::ClientConfig;
use wl_kernel::PartyId;
use wl_kernel::production::{OrderLineInfo, OrderLines, ProductionFuture, provide_order_lines};
use wl_kernel::sales::{
    DeliveryDestination, PromisedLine, PromisedLines, SalesFuture, SalesOrderId,
    provide_delivery_destination, provide_promised_lines,
};

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

/// What wl-sales would tell, which no module provides to the host yet: the
/// lines of the orders, the lines promised for a day, and whether an order is
/// delivered in the UAE.
#[derive(Clone, Default)]
pub struct Orders(Arc<Mutex<Sales>>);

#[derive(Default)]
struct Sales {
    lines: Vec<OrderLineInfo>,
    promised: Vec<PromisedLine>,
    export: std::collections::BTreeMap<i64, bool>,
}

impl Orders {
    /// Line `line` of order `order` is open for `open_milli`, customer `party`.
    pub fn open(&self, order: i64, line: i64, open_milli: i64, party: i64) {
        self.0.lock().unwrap().lines.push(OrderLineInfo {
            order: SalesOrderId(order),
            line,
            number: Some(format!("SO-{order}")),
            owner: None,
            open_milli,
            cancelled: false,
            customer: Some(PartyId(party)),
        });
    }

    /// Line `line` of order `order` asks for `item` on `promised`.
    pub fn promise(&self, order: i64, line: i64, item: i64, promised: NaiveDate) {
        self.0.lock().unwrap().promised.push(PromisedLine {
            order: SalesOrderId(order),
            line,
            number: Some(format!("SO-{order}")),
            item: wl_kernel::Subject::new("inventory.item", item),
            promised,
        });
    }

    /// Order `order` is delivered abroad (`true`) or in the UAE (`false`).
    pub fn export(&self, order: i64, abroad: bool) {
        self.0.lock().unwrap().export.insert(order, abroad);
    }
}

impl OrderLines for Orders {
    fn line<'a>(
        &'a self,
        _state: &'a AppState,
        order: SalesOrderId,
        line: i64,
    ) -> ProductionFuture<'a, Option<OrderLineInfo>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .lines
                .iter()
                .find(|info| info.order == order && info.line == line)
                .cloned())
        })
    }
}

impl PromisedLines for Orders {
    fn of_item<'a>(
        &'a self,
        _state: &'a AppState,
        item: &'a wl_kernel::Subject,
        from: NaiveDate,
    ) -> SalesFuture<'a, Vec<PromisedLine>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .promised
                .iter()
                .filter(|line| &line.item == item && line.promised >= from)
                .cloned()
                .collect())
        })
    }
}

impl DeliveryDestination for Orders {
    fn is_export<'a>(
        &'a self,
        _state: &'a AppState,
        order: &'a wl_kernel::Subject,
    ) -> SalesFuture<'a, Option<bool>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .export
                .get(&order.subject_id)
                .copied())
        })
    }
}

/// Provides the sales order lines after the host's modules.
struct OrdersModule(Orders);

impl Module for OrdersModule {
    fn name(&self) -> &'static str {
        "test_orders"
    }

    fn register(&self, app: &mut Registry) {
        provide_order_lines(app, self.0.clone());
        provide_promised_lines(app, self.0.clone());
        provide_delivery_destination(app, self.0.clone());
    }
}

/// [`boot`] with the sales order lines of a test.
pub async fn boot_with_orders() -> (TestApp, Orders) {
    boot_tuned(host::Tuning::default()).await
}

/// [`boot_with_orders`] with the shared modules tuned (a mail provider that
/// bounces, say).
pub async fn boot_tuned(tuning: host::Tuning) -> (TestApp, Orders) {
    let orders = Orders::default();
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(
        host::app_for_tuned(config, tuning)
            .unwrap()
            .module(OrdersModule(orders.clone())),
    )
    .await;
    app.kernel().seed().await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    (app, orders)
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

/// The messages of type `type_key` that went to `user`, oldest first.
pub async fn messages_to(app: &TestApp, type_key: &str, user: &User) -> Vec<wl_notify::Message> {
    wl_notify::Message::query()
        .where_(wl_notify::Message::TYPE_KEY.eq(type_key))
        .where_(wl_notify::Message::USER_ID.eq(user.id))
        .order_by(wl_notify::Message::ID)
        .get(app.db())
        .await
        .unwrap()
}

/// An item `code` that is not lot-tracked, with `quantity` pieces on hand at
/// `location` (a fixture of what a goods receipt posted).
pub async fn stocked(app: &TestApp, code: &str, location: &Location, quantity: i64) -> Item {
    let mut item = Item::new(code, code, "pcs");
    item.average_cost = 500;
    item.save(app.db()).await.unwrap();
    wl_inventory::StockMovement::receive(
        item.id,
        None,
        location.id,
        quantity * 1_000,
        500,
        quantity * 500,
        &wl_kernel::Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    item
}

/// A draft dispatch of `quantity` pieces of `item` from `location` for sales
/// order `order`, to customer `party`.
pub async fn dispatch_of(
    app: &TestApp,
    item: &Item,
    location: &Location,
    order: i64,
    party: i64,
    quantity: i64,
) -> wl_inventory::Dispatch {
    wl_inventory::create_dispatch(
        app.db(),
        None,
        wl_inventory::NewDispatch {
            source: wl_kernel::Subject::new("sales.sales_order", order),
            recipient: Some(PartyId(party)),
            recipient_name: "Customer".into(),
            address: "Dubai".into(),
            location_id: location.id,
            lines: vec![wl_inventory::NewDispatchLine::of(item.id, quantity * 1_000)],
        },
    )
    .await
    .unwrap()
}

/// A document of `kind` of `subject`, valid through `expires`, in `status`.
pub async fn document(
    app: &TestApp,
    subject: &wl_kernel::Subject,
    kind: &str,
    expires: Option<NaiveDate>,
    status: wl_documents::DocumentStatus,
) -> wl_documents::Document {
    let kind = wl_documents::kinds::DocumentKind::by_key(app.db(), kind)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the kind {kind} is seeded"));
    wl_documents::Document::create(
        app.db(),
        wl_documents::Document {
            subject_type: subject.subject_type.clone(),
            subject_id: subject.subject_id,
            kind_id: kind.id,
            title: kind.name_en.clone(),
            expiry_date: expires,
            current_version: 1,
            status,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// The visible words of an HTML answer, one line per block, for a message.
pub fn words(html: &str) -> String {
    let mut plain = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => plain.push(c),
            _ => {}
        }
    }
    plain
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ")
}
