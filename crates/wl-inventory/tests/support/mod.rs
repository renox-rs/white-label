//! Booting wl-inventory in a `TestApp` with auth and the kernel: the roles
//! of a storekeeper and of procurement, the kernel events recorded, and
//! helpers for the fixtures (an item, a location, an issued purchase order).
//! A test cannot build an `AuthUser`, so posting a receipt goes through the
//! module's own route as the user.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::auth::{Permissions, permissions};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_inventory::item::value_of;
use wl_inventory::movement::StockMovement;
use wl_inventory::permissions as perm;
use wl_inventory::supplier_return::{NewReturn, NewReturnLine};
use wl_inventory::{
    DraftLine, DraftReceipt, GoodsReceipt, GoodsReceiptLine, Item, Location, Lot, LotStatus,
    PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus, SupplierReturn, WlInventory,
};
use wl_kernel::inventory::{GoodsReceived, SupplierReturnDispatched};
use wl_kernel::{PartyId, Subject};

/// A client with the inventory module on.
const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "property"

[modules]
enabled = ["inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

/// The supplier of the orders.
pub const SUPPLIER: PartyId = PartyId(7);

/// An event log.
#[derive(Clone)]
pub struct Recorded<E>(pub Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// What was recorded.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

/// Records `event`s of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Arc<Mutex<Vec<E>>>) {
    let log = log.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

/// Records the events.
struct Fakes {
    received: Arc<Mutex<Vec<GoodsReceived>>>,
    dispatched: Arc<Mutex<Vec<SupplierReturnDispatched>>>,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        record(app, &self.received);
        record(app, &self.dispatched);
    }
}

/// The app under test with the events.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// `GoodsReceived`.
    pub received: Recorded<GoodsReceived>,
    /// `SupplierReturnDispatched`.
    pub dispatched: Recorded<SupplierReturnDispatched>,
}

impl Booted {
    /// The database.
    pub fn db(&self) -> &Db {
        self.app.db()
    }

    /// The app state.
    pub fn state(&self) -> &AppState {
        self.app.state()
    }

    /// A user holding the role `role` everywhere.
    pub async fn user_with(&self, name: &str, role: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(self.db(), role).await.unwrap();
        user
    }

    /// A storekeeper (`goods_receipts.post`) everywhere.
    pub async fn storekeeper(&self, name: &str) -> User {
        self.user_with(name, "storekeeper").await
    }

    /// A storekeeper at the locations in `scope` only.
    pub async fn storekeeper_in(&self, name: &str, scope: &Subject) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(self.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role_in(
            self.db(),
            "storekeeper",
            &renox::auth::permissions::Scope::new(&scope.subject_type, scope.subject_id),
        )
        .await
        .unwrap();
        user
    }

    /// A stores manager (`goods_receipts.reverse`) everywhere.
    pub async fn reverser(&self, name: &str) -> User {
        self.user_with(name, "stores_manager").await
    }

    /// A procurement officer (`purchase_orders.issue`) everywhere.
    pub async fn procurement(&self, name: &str) -> User {
        self.user_with(name, "procurement").await
    }

    /// An item `code` in `kg`.
    pub async fn item(&self, code: &str) -> Item {
        let mut item = Item::new(code, code, "kg");
        item.save(self.db()).await.unwrap();
        item
    }

    /// A lot-tracked item `code` in `kg`.
    pub async fn tracked_item(&self, code: &str) -> Item {
        let mut item = Item::new(code, code, "kg").lot_tracked();
        item.save(self.db()).await.unwrap();
        item
    }

    /// A location `code`, in the scope `scope` when given.
    pub async fn location(&self, code: &str, scope: Option<&Subject>) -> Location {
        let mut location = Location::new(code, code);
        if let Some(scope) = scope {
            location = location.in_scope(scope);
        }
        location.save(self.db()).await.unwrap();
        location
    }

    /// An `issued` purchase order to the supplier for delivery at `location`
    /// with one line of `quantity_milli` of `item` at `unit_price`.
    pub async fn order(
        &self,
        location: &Location,
        item: &Item,
        quantity_milli: i64,
        unit_price: i64,
    ) -> (PurchaseOrder, PurchaseOrderLine) {
        let mut order = PurchaseOrder::new(SUPPLIER, location.id);
        order.status = PurchaseOrderStatus::Issued;
        let mut tx = self.db().begin().await.unwrap();
        order
            .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut line = PurchaseOrderLine::new(order.id, 1, item.id, quantity_milli, unit_price);
        line.save(self.db()).await.unwrap();
        (order, line)
    }

    /// A draft receipt of the order at the location with `lines`.
    pub async fn receipt(
        &self,
        order: &PurchaseOrder,
        location: &Location,
        lines: Vec<DraftLine>,
    ) -> GoodsReceipt {
        wl_inventory::goods_receipt::create_draft(
            self.db(),
            None,
            DraftReceipt {
                purchase_order_id: order.id,
                location_id: location.id,
                delivery_note: Some("DN-1".into()),
                lines,
            },
        )
        .await
        .unwrap()
    }

    /// `POST /goods-receipts/{id}/post` as `by`.
    pub async fn post_as(&self, by: &User, receipt: &GoodsReceipt) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/goods-receipts/{}/post", receipt.id), &[])
            .await
    }

    /// `POST /goods-receipts/{id}/reverse` as `by`.
    pub async fn reverse_as(&self, by: &User, receipt: &GoodsReceipt) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/goods-receipts/{}/reverse", receipt.id), &[])
            .await
    }

    /// `POST /supplier-returns/{id}/dispatch` as `by`, with a collection
    /// proof of `kind` (`driver_signature`, `courier`, `goods_receipt`).
    pub async fn dispatch_as(
        &self,
        by: &User,
        ret: &SupplierReturn,
        kind: &str,
        reference: &str,
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/supplier-returns/{}/dispatch", ret.id),
                &[("proof_kind", kind), ("proof_ref", reference)],
            )
            .await
    }

    /// `POST /supplier-returns/{id}/cancel` as `by`, for `reason`; `kept` is
    /// `receive_again` or `discard` for goods rejected at receipt.
    pub async fn cancel_as(
        &self,
        by: &User,
        ret: &SupplierReturn,
        reason: &str,
        kept: Option<&str>,
    ) -> TestResponse {
        self.app.acting_as(by);
        let mut form = vec![("reason", reason)];
        if let Some(kept) = kept {
            form.push(("kept", kept));
        }
        self.app
            .htmx()
            .post(&format!("/supplier-returns/{}/cancel", ret.id), &form)
            .await
    }

    /// `quantity_milli` of `item` in `location` (and `lot`) at `unit_cost`
    /// fils a unit, as a `receive` movement of an earlier receipt.
    pub async fn stock_in(
        &self,
        item: &Item,
        location: &Location,
        lot: Option<&Lot>,
        quantity_milli: i64,
        unit_cost: i64,
    ) {
        let source = Subject::new("inventory.goods_receipt", 900);
        let mut moved = StockMovement::receive(
            item.id,
            lot.map(|l| l.id),
            location.id,
            quantity_milli,
            unit_cost,
            value_of(quantity_milli, unit_cost),
            &source,
            None,
        );
        moved.save(self.db()).await.unwrap();
    }

    /// A lot `number` of `item` in `status`.
    pub async fn lot(&self, item: &Item, number: &str, status: LotStatus) -> Lot {
        let mut lot = Lot {
            item_id: item.id,
            number: number.to_owned(),
            status,
            ..Lot::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        lot.insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        lot
    }

    /// A draft return of `lines` taken from stock at `location`.
    pub async fn stock_return(
        &self,
        location: &Location,
        lines: Vec<NewReturnLine>,
    ) -> SupplierReturn {
        wl_inventory::supplier_return::create_from_stock(
            self.db(),
            None,
            NewReturn {
                supplier: SUPPLIER,
                source: Subject::new("inventory.stock_hold", 1),
                location_id: location.id,
                lines,
            },
        )
        .await
        .unwrap()
    }

    /// The return as stored now.
    pub async fn return_now(&self, ret: &SupplierReturn) -> SupplierReturn {
        SupplierReturn::find_or_404(self.db(), ret.id)
            .await
            .unwrap()
    }

    /// The receipt as stored now.
    pub async fn reload(&self, receipt: &GoodsReceipt) -> GoodsReceipt {
        GoodsReceipt::find_or_404(self.db(), receipt.id)
            .await
            .unwrap()
    }

    /// The lines of the receipt as stored now.
    pub async fn lines(&self, receipt: &GoodsReceipt) -> Vec<GoodsReceiptLine> {
        wl_inventory::goods_receipt::lines_of(self.db(), receipt.id)
            .await
            .unwrap()
    }

    /// The order as stored now.
    pub async fn order_now(&self, order: &PurchaseOrder) -> PurchaseOrder {
        PurchaseOrder::find_or_404(self.db(), order.id)
            .await
            .unwrap()
    }

    /// The order's line as stored now.
    pub async fn line_now(&self, line: &PurchaseOrderLine) -> PurchaseOrderLine {
        PurchaseOrderLine::find_or_404(self.db(), line.id)
            .await
            .unwrap()
    }
}

/// wl-inventory with the §7 defaults, booted.
pub async fn boot() -> Booted {
    boot_with(WlInventory::new()).await
}

/// `module` booted in an app with auth and the kernel, the roles of a
/// storekeeper and of procurement defined.
pub async fn boot_with(module: WlInventory) -> Booted {
    let received = Arc::new(Mutex::new(Vec::new()));
    let dispatched = Arc::new(Mutex::new(Vec::new()));
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(module)
        .module(Fakes {
            received: received.clone(),
            dispatched: dispatched.clone(),
        });
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let app = TestApp::new(app).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    permissions::define_role(app.db(), "storekeeper", &[perm::GOODS_RECEIPTS_POST])
        .await
        .unwrap();
    permissions::define_role(app.db(), "stores_manager", &[perm::GOODS_RECEIPTS_REVERSE])
        .await
        .unwrap();
    permissions::define_role(
        app.db(),
        "procurement",
        &[perm::PURCHASE_ORDERS_ISSUE, perm::SUPPLIER_RETURNS_MANAGE],
    )
    .await
    .unwrap();
    Booted {
        app,
        received: Recorded(received),
        dispatched: Recorded(dispatched),
    }
}
