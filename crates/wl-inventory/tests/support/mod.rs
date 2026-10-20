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
use wl_kernel::block_guard::{
    ActionKind, BlockFuture, BlockGuard, BlockRefusal, GuardOutcome, provide_block_guard,
};
use wl_kernel::inventory::{
    GoodsReceived, PurchaseOrderRevised, SupplierReturnCredited, SupplierReturnDispatched,
};
use wl_kernel::invoicing::{
    CostPoster, DebitedLine, PostCost, SupplierBillFuture, SupplierBillId,
    SupplierDebitNoteCancelled, SupplierDebitNoteId, SupplierDebitNoteIssued,
};
use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, NotifyRequest};
use wl_kernel::tickets::{OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef};
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

/// A client with the inventory and invoicing modules on.
const CLIENT_WITH_INVOICING: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "property"

[modules]
enabled = ["parties", "inventory", "invoicing"]

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

/// A `TaskCreator` that records the tickets it opens; a ticket opened with
/// a source key that is still open is the same ticket.
#[derive(Clone, Default)]
pub struct TicketDesk {
    opened: Arc<Mutex<Vec<OpenTicket>>>,
    resolved: Arc<Mutex<Vec<(TicketId, String)>>>,
}

impl TicketDesk {
    /// The tickets resolved by their source, with the note.
    pub fn resolved(&self) -> Vec<(TicketId, String)> {
        self.resolved.lock().unwrap().clone()
    }

    /// The tickets opened, oldest first.
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.opened.lock().unwrap().clone()
    }
}

impl TaskCreator for TicketDesk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.opened.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn resolve_from_source<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            self.resolved
                .lock()
                .unwrap()
                .push((ticket, note.to_owned()));
            Ok(true)
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let resolved = self.resolved.lock().unwrap().clone();
            let opened = self.opened.lock().unwrap();
            Ok(opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .filter(|n| !resolved.iter().any(|(id, _)| id.0 == *n as i64 + 1))
                .map(|n| TicketRef {
                    id: TicketId(n as i64 + 1),
                    number: format!("TKT-{}", n + 1),
                }))
        })
    }
}

/// A notifier that records every request and queues nothing.
#[derive(Clone, Default)]
pub struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

impl Recording {
    /// Every request so far, oldest first.
    pub fn all(&self) -> Vec<NotifyRequest> {
        self.0.lock().unwrap().clone()
    }
}

impl Notifier for Recording {
    fn notify<'a>(
        &'a self,
        _state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        self.0.lock().unwrap().push(request);
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// A `CostPoster` that records what is posted.
#[derive(Clone, Default)]
pub struct CostDesk {
    posted: Arc<Mutex<Vec<PostCost>>>,
}

impl CostDesk {
    /// The cost posted, oldest first.
    pub fn posted(&self) -> Vec<PostCost> {
        self.posted.lock().unwrap().clone()
    }
}

impl CostPoster for CostDesk {
    fn post<'a>(&'a self, _state: &'a AppState, request: PostCost) -> SupplierBillFuture<'a, ()> {
        Box::pin(async move {
            self.posted.lock().unwrap().push(request);
            Ok(())
        })
    }
}

/// Records the events and stands in for ticketing and cost entries.
struct Fakes {
    received: Arc<Mutex<Vec<GoodsReceived>>>,
    dispatched: Arc<Mutex<Vec<SupplierReturnDispatched>>>,
    credited: Arc<Mutex<Vec<SupplierReturnCredited>>>,
    tickets: TicketDesk,
    costs: CostDesk,
    notices: Recording,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        record(app, &self.revised);
        provide_block_guard(app, self.blocked.clone());
        record(app, &self.received);
        record(app, &self.dispatched);
        record(app, &self.credited);
        wl_kernel::tickets::provide_task_creator(app, self.tickets.clone());
        wl_kernel::invoicing::provide_cost_poster(app, self.costs.clone());
        wl_kernel::notify::provide_notifier(app, self.notices.clone());
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
    /// Every `PurchaseOrderRevised`.
    pub revised: Recorded<PurchaseOrderRevised>,
    /// The suppliers the block guard refuses.
    pub blocked: Blocked,
    /// `SupplierReturnCredited`.
    pub credited: Recorded<SupplierReturnCredited>,
    /// The tickets opened for tasks.
    pub tickets: TicketDesk,
    /// The cost posted (wastage).
    pub costs: CostDesk,
    /// The notifications asked for.
    pub notices: Recording,
    /// The days the clock was moved forward.
    clock_days: std::sync::atomic::AtomicI64,
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

    /// Moves the clock to `days` days after the real today (days only go
    /// forward within a test).
    pub fn clock_at(&self, days: i64) {
        use std::sync::atomic::Ordering;
        let moved = self.clock_days.load(Ordering::SeqCst);
        assert!(days >= moved, "the clock cannot go back to day {days}");
        if days > moved {
            let seconds = u64::try_from(days - moved).unwrap() * 24 * 3600;
            self.app.travel(std::time::Duration::from_secs(seconds));
            self.clock_days.store(days, Ordering::SeqCst);
        }
    }

    /// A finance user (`finance` role).
    pub async fn finance(&self, name: &str) -> User {
        self.user_with(name, "finance").await
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

    /// `POST /purchase-orders/{id}/revise` as `by`: each `(line, quantity,
    /// price)` is the order line, the quantity in the base unit and the price
    /// in AED, like `("1200.00")`; `extra` is more form fields.
    pub async fn revise_as(
        &self,
        by: &User,
        order: &PurchaseOrder,
        reason: &str,
        lines: &[(&PurchaseOrderLine, &str, &str)],
        extra: &[(&str, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        let mut keys = Vec::new();
        for (line, _, _) in lines {
            keys.push((format!("qty_{}", line.id), format!("price_{}", line.id)));
        }
        let mut form: Vec<(&str, &str)> = vec![("reason", reason)];
        for ((_, quantity, price), (quantity_key, price_key)) in lines.iter().zip(&keys) {
            form.push((quantity_key.as_str(), quantity));
            form.push((price_key.as_str(), price));
        }
        form.extend_from_slice(extra);
        self.app
            .htmx()
            .post(&format!("/purchase-orders/{}/revise", order.id), &form)
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

    /// `POST /supplier-returns/{id}/credit` as `by`: `basis` is
    /// `refund_received` or `supplier_refused`.
    pub async fn credit_as(
        &self,
        by: &User,
        ret: &SupplierReturn,
        basis: &str,
        reason: &str,
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!("/supplier-returns/{}/credit", ret.id),
                &[("basis", basis), ("reason", reason)],
            )
            .await
    }

    /// The supplier issues debit note `id` for `ret`, debiting `lines`
    /// (purchase order line, quantity in thousandths).
    pub async fn debit_note_issued(&self, ret: &SupplierReturn, id: i64, lines: &[(i64, i64)]) {
        self.state()
            .emit(SupplierDebitNoteIssued {
                debit_note: SupplierDebitNoteId(id),
                supplier: SUPPLIER,
                bill: SupplierBillId(1),
                return_to_supplier: Some(ret.subject()),
                amount: 0,
                vat: 0,
                lines: debited(lines),
            })
            .await
            .unwrap();
    }

    /// Debit note `id` for `ret` is cancelled; it debited `lines`.
    pub async fn debit_note_cancelled(&self, ret: &SupplierReturn, id: i64, lines: &[(i64, i64)]) {
        self.state()
            .emit(SupplierDebitNoteCancelled {
                debit_note: SupplierDebitNoteId(id),
                supplier: SUPPLIER,
                bill: Some(SupplierBillId(1)),
                return_to_supplier: Some(ret.subject()),
                amount: 0,
                reason: "issued in error".into(),
                lines: debited(lines),
            })
            .await
            .unwrap();
    }

    /// A dispatched return of credit lines, one per `(item, quantity_milli,
    /// order line)`, each at AED 100.00 a unit, from a store holding enough.
    pub async fn dispatched_credit_return(
        &self,
        store: &Location,
        lines: &[(&Item, i64, &PurchaseOrderLine)],
    ) -> SupplierReturn {
        let mut new = Vec::new();
        for (item, quantity_milli, po_line) in lines {
            self.stock_in(item, store, None, *quantity_milli, 10_000)
                .await;
            let mut line = NewReturnLine::of(item.id, *quantity_milli, "damaged")
                .credit()
                .at_cost(10_000);
            line.po_line_id = Some(po_line.id);
            new.push(line);
        }
        let ret = self.stock_return(store, new).await;
        let clerk = self.procurement("Dispatcher").await;
        self.dispatch_as(&clerk, &ret, "courier", "C-1")
            .await
            .assert_ok();
        self.return_now(&ret).await
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
    boot_modules(module, false).await
}

/// wl-inventory and the real wl-invoicing booted together, to follow an
/// event from one module into the other.
pub async fn boot_with_invoicing() -> Booted {
    boot_modules(WlInventory::new(), true).await
}

async fn boot_modules(module: WlInventory, with_invoicing: bool) -> Booted {
    let received = Arc::new(Mutex::new(Vec::new()));
    let dispatched = Arc::new(Mutex::new(Vec::new()));
    let revised = Arc::new(Mutex::new(Vec::new()));
    let blocked = Blocked::default();
    let credited = Arc::new(Mutex::new(Vec::new()));
    let tickets = TicketDesk::default();
    let costs = CostDesk::default();
    let notices = Recording::default();
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(module);
    let app = if with_invoicing {
        app.module(wl_invoicing::WlInvoicing::new())
    } else {
        app
    };
    let app = app.module(Fakes {
        received: received.clone(),
        dispatched: dispatched.clone(),
        credited: credited.clone(),
        tickets: tickets.clone(),
        costs: costs.clone(),
        notices: notices.clone(),
    });
    let client = if with_invoicing {
        CLIENT_WITH_INVOICING
    } else {
        CLIENT
    };
    let config = wl_kernel::ClientConfig::parse(client).expect("a valid client.toml");
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
    permissions::define_role(app.db(), "finance", &[perm::STOCK_VIEW])
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
        revised: Recorded(revised),
        blocked,
        credited: Recorded(credited),
        tickets,
        costs,
        notices,
        clock_days: std::sync::atomic::AtomicI64::new(0),
    }
}

/// The lines a debit note debits, as events carry them.
fn debited(lines: &[(i64, i64)]) -> Vec<DebitedLine> {
    lines
        .iter()
        .map(|(po_line, quantity_milli)| DebitedLine {
            po_line: Some(*po_line),
            quantity_milli: *quantity_milli,
        })
        .collect()
}
