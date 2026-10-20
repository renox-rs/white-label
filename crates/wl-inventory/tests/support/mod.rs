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
    DraftLine, DraftReceipt, GoodsReceipt, GoodsReceiptLine, Item, LandedCost,
    LandedCostAdjustment, Location, Lot, LotStatus, PurchaseOrder, PurchaseOrderLine,
    PurchaseOrderStatus, SupplierReturn, WlInventory,
};
use wl_kernel::block_guard::{
    ActionKind, BlockFuture, BlockGuard, BlockRefusal, GuardOutcome, provide_block_guard,
};
use wl_kernel::inventory::{
    GoodsReceived, LandedCostAdjusted, LandedCostAllocated, PurchaseOrderRevised,
    SupplierReturnCredited, SupplierReturnDispatched,
};
use wl_kernel::invoicing::{
    CostPoster, DebitedLine, PeriodLocks, PostCost, SupplierBillFuture, SupplierBillId,
    SupplierBillLine, SupplierBillLineId, SupplierBillStatus, SupplierBills,
    SupplierDebitNoteCancelled, SupplierDebitNoteId, SupplierDebitNoteIssued,
};
use wl_kernel::documents::{
    BlockEntry, CheckOutcome, DocumentFuture, DocumentGuard, FlagFailure, InvalidReason,
    NoDocuments, RecordFlag, Validity,
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

/// A block guard that refuses the suppliers set as blocked. A party blocked
/// with a reason shows it to the users set as allowed to see it, as
/// `parties.block_reason.view` does; everyone else gets *on hold*.
#[derive(Clone, Default)]
pub struct Blocked {
    parties: Arc<Mutex<Vec<(PartyId, Option<String>)>>>,
    seers: Arc<Mutex<Vec<i64>>>,
}

impl Blocked {
    /// `party` is blocked from now on.
    pub fn block(&self, party: PartyId) {
        self.parties.lock().unwrap().push((party, None));
    }

    /// `party` is blocked from now on for `reason` (such as `Sanctions`).
    pub fn block_for(&self, party: PartyId, reason: &str) {
        self.parties
            .lock()
            .unwrap()
            .push((party, Some(reason.to_owned())));
    }

    /// `user` may see the reason of a block.
    pub fn show_reasons_to(&self, user: &User) {
        self.seers.lock().unwrap().push(user.id);
    }

    /// `party` is not blocked any more.
    pub fn unblock(&self, party: PartyId) {
        self.parties.lock().unwrap().retain(|(p, _)| *p != party);
    }

    /// The suppliers blocked now.
    pub fn blocked(&self) -> Vec<PartyId> {
        self.parties.lock().unwrap().iter().map(|(p, _)| *p).collect()
    }
}

impl BlockGuard for Blocked {
    fn check_in<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        viewer: Option<&'a User>,
        party: PartyId,
        kind: ActionKind,
    ) -> BlockFuture<'a, GuardOutcome> {
        let found = self
            .parties
            .lock()
            .unwrap()
            .iter()
            .find(|(p, _)| *p == party)
            .map(|(_, reason)| reason.clone());
        let sees = viewer.is_some_and(|v| self.seers.lock().unwrap().contains(&v.id));
        let outcome = match found {
            Some(reason) if kind.is_guarded() => {
                let lines = match (sees, reason) {
                    (true, Some(label)) => vec![wl_kernel::block_guard::BlockReasonLine {
                        label,
                        link: format!("/parties/{}/blocks/1", party.0),
                    }],
                    _ => Vec::new(),
                };
                GuardOutcome::Blocked(BlockRefusal::detailed(
                    party,
                    kind,
                    lines,
                    "finance_manager",
                ))
            }
            _ => GuardOutcome::Allowed,
        };
        Box::pin(async move { Ok(outcome) })
    }
}

/// A document guard that knows which certificates have expired: every kind
/// is valid for a party except those set as expired.
#[derive(Clone, Default)]
pub struct Certificates(Arc<Mutex<Vec<(PartyId, String)>>>);

impl Certificates {
    /// `supplier`'s certificate of `kind` has expired.
    pub fn expire(&self, supplier: PartyId, kind: &str) {
        self.0.lock().unwrap().push((supplier, kind.to_owned()));
    }
}

impl DocumentGuard for Certificates {
    fn blocks_for<'a>(
        &'a self,
        db: &'a Db,
        guarded: &'a str,
    ) -> DocumentFuture<'a, Vec<BlockEntry>> {
        NoDocuments.blocks_for(db, guarded)
    }

    fn is_valid<'a>(
        &'a self,
        _db: &'a Db,
        subject: &'a Subject,
        kind: &'a str,
        _on_date: renox::chrono::NaiveDate,
    ) -> DocumentFuture<'a, Validity> {
        let expired = self
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(party, k)| party.subject() == *subject && k == kind);
        Box::pin(async move {
            Ok(if expired {
                Validity::Invalid {
                    reason: InvalidReason::Expired,
                    document: None,
                    expired_on: None,
                    pending: None,
                }
            } else {
                Validity::Valid {
                    document: None,
                    expires_on: None,
                }
            })
        })
    }

    fn check_as<'a>(
        &'a self,
        state: &'a AppState,
        guarded: &'a str,
        record_id: i64,
        on_date: renox::chrono::NaiveDate,
        actor: Option<i64>,
    ) -> DocumentFuture<'a, std::result::Result<CheckOutcome, wl_kernel::documents::BlockRefusal>>
    {
        NoDocuments.check_as(state, guarded, record_id, on_date, actor)
    }

    fn record_flags<'a>(
        &'a self,
        db: &'a Db,
        guarded: &'a str,
        record_id: i64,
        flags: &'a [FlagFailure],
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        NoDocuments.record_flags(db, guarded, record_id, flags)
    }

    fn flags_for<'a>(
        &'a self,
        db: &'a Db,
        record_type: &'a str,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<RecordFlag>> {
        NoDocuments.flags_for(db, record_type, record_id)
    }

    fn open_flag_counts<'a>(
        &'a self,
        db: &'a Db,
        record_type: &'a str,
        ids: &'a [i64],
    ) -> DocumentFuture<'a, std::collections::BTreeMap<i64, u64>> {
        NoDocuments.open_flag_counts(db, record_type, ids)
    }

    fn review_flag<'a>(
        &'a self,
        db: &'a Db,
        flag_id: i64,
        reviewer: i64,
        note: &'a str,
    ) -> DocumentFuture<'a, RecordFlag> {
        NoDocuments.review_flag(db, flag_id, reviewer, note)
    }
}

/// The dates in a filed accounting period, as wl-invoicing would answer.
#[derive(Clone, Default)]
pub struct PeriodDesk(Arc<Mutex<Vec<renox::chrono::NaiveDate>>>);

impl PeriodDesk {
    /// `date` is in a filed period from now on.
    pub fn lock(&self, date: renox::chrono::NaiveDate) {
        self.0.lock().unwrap().push(date);
    }
}

impl PeriodLocks for PeriodDesk {
    fn is_locked<'a>(
        &'a self,
        _db: &'a Db,
        _entity: i64,
        date: renox::chrono::NaiveDate,
    ) -> SupplierBillFuture<'a, bool> {
        let locked = self.0.lock().unwrap().contains(&date);
        Box::pin(async move { Ok(locked) })
    }
}

/// The supplier bill lines wl-invoicing would know, set by the test.
#[derive(Clone, Default)]
pub struct BillDesk(Arc<Mutex<Vec<SupplierBillLine>>>);

impl BillDesk {
    /// Line `line` of bill `bill` (invoice number `number`) of `amount_aed`
    /// fils, described as `description`.
    pub fn add(&self, bill: i64, line: i64, number: &str, description: &str, amount_aed: i64) {
        self.0.lock().unwrap().push(SupplierBillLine {
            bill: SupplierBillId(bill),
            line: SupplierBillLineId(line),
            bill_number: number.to_owned(),
            supplier: SUPPLIER,
            legal_entity_id: None,
            description: description.to_owned(),
            amount_aed,
            bill_status: SupplierBillStatus::Approved,
        });
    }
}

impl SupplierBills for BillDesk {
    fn line<'a>(
        &'a self,
        _db: &'a Db,
        id: SupplierBillLineId,
    ) -> SupplierBillFuture<'a, Option<SupplierBillLine>> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|l| l.line == id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }
}

/// Records the events and stands in for ticketing and cost entries.
struct Fakes {
    revised: Arc<Mutex<Vec<PurchaseOrderRevised>>>,
    allocated: Arc<Mutex<Vec<LandedCostAllocated>>>,
    adjusted: Arc<Mutex<Vec<LandedCostAdjusted>>>,
    blocked: Blocked,
    certificates: Certificates,
    locks: Option<PeriodDesk>,
    bills: Option<BillDesk>,
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
        record(app, &self.allocated);
        record(app, &self.adjusted);
        if let Some(bills) = &self.bills {
            wl_kernel::invoicing::provide_supplier_bills(app, bills.clone());
        }
        if let Some(locks) = &self.locks {
            wl_kernel::invoicing::provide_period_locks(app, locks.clone());
        }
        provide_block_guard(app, self.blocked.clone());
        wl_kernel::documents::provide_guard(app, self.certificates.clone());
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
    /// Every `LandedCostAllocated`.
    pub allocated: Recorded<LandedCostAllocated>,
    /// Every `LandedCostAdjusted`.
    pub adjusted: Recorded<LandedCostAdjusted>,
    /// The dates in a filed period.
    pub locks: PeriodDesk,
    /// The supplier bill lines the test sets.
    pub bills: BillDesk,
    /// The suppliers the block guard refuses.
    pub blocked: Blocked,
    /// The certificates that have expired.
    pub certificates: Certificates,
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
        self.order_from(SUPPLIER, location, item, quantity_milli, unit_price)
            .await
    }

    /// Like [`order`](Self::order), to `supplier`.
    pub async fn order_from(
        &self,
        supplier: PartyId,
        location: &Location,
        item: &Item,
        quantity_milli: i64,
        unit_price: i64,
    ) -> (PurchaseOrder, PurchaseOrderLine) {
        let mut order = PurchaseOrder::new(supplier, location.id);
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

    /// A `draft` purchase order to the supplier for delivery at `location`
    /// with one line of `quantity_milli` of `item` at `unit_price`.
    pub async fn draft_order(
        &self,
        location: &Location,
        item: &Item,
        quantity_milli: i64,
        unit_price: i64,
    ) -> (PurchaseOrder, PurchaseOrderLine) {
        let mut order = PurchaseOrder::new(SUPPLIER, location.id);
        let mut tx = self.db().begin().await.unwrap();
        order
            .insert_in_state(&mut tx, PurchaseOrderStatus::Draft, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut line = PurchaseOrderLine::new(order.id, 1, item.id, quantity_milli, unit_price);
        line.save(self.db()).await.unwrap();
        (order, line)
    }

    /// `POST /purchase-orders/{id}/issue` as `by`; `extra` is more form
    /// fields (`block_exception=1` asks for the exception).
    pub async fn issue_as(
        &self,
        by: &User,
        order: &PurchaseOrder,
        extra: &[(&str, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(&format!("/purchase-orders/{}/issue", order.id), extra)
            .await
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

    /// `POST /landed-costs/{id}/allocate` as `by`, with the weights given as
    /// `(receipt line, weight)`.
    pub async fn allocate_as(
        &self,
        by: &User,
        landed: &LandedCost,
        weights: &[(i64, &str)],
    ) -> TestResponse {
        self.app.acting_as(by);
        let keys: Vec<String> = weights
            .iter()
            .map(|(line, _)| format!("weight_{line}"))
            .collect();
        let form: Vec<(&str, &str)> = keys
            .iter()
            .zip(weights)
            .map(|(key, (_, weight))| (key.as_str(), *weight))
            .collect();
        self.app
            .htmx()
            .post(&format!("/landed-costs/{}/allocate", landed.id), &form)
            .await
    }

    /// `POST /landed-costs/{id}/adjustments/{adjustment}/post` as `by`.
    pub async fn post_adjustment_as(
        &self,
        by: &User,
        landed: &LandedCost,
        adjustment: &LandedCostAdjustment,
    ) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .htmx()
            .post(
                &format!(
                    "/landed-costs/{}/adjustments/{}/post",
                    landed.id, adjustment.id
                ),
                &[],
            )
            .await
    }

    /// The landed cost as stored now.
    pub async fn landed_now(&self, landed: &LandedCost) -> LandedCost {
        LandedCost::find_or_404(self.db(), landed.id).await.unwrap()
    }

    /// The adjustment as stored now.
    pub async fn adjustment_now(&self, adjustment: &LandedCostAdjustment) -> LandedCostAdjustment {
        LandedCostAdjustment::find_or_404(self.db(), adjustment.id)
            .await
            .unwrap()
    }

    /// The item as stored now.
    pub async fn item_now(&self, item: &Item) -> Item {
        Item::find_or_404(self.db(), item.id).await.unwrap()
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
    let allocated = Arc::new(Mutex::new(Vec::new()));
    let adjusted = Arc::new(Mutex::new(Vec::new()));
    let locks = PeriodDesk::default();
    let bills = BillDesk::default();
    let blocked = Blocked::default();
    let certificates = Certificates::default();
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
        revised: revised.clone(),
        allocated: allocated.clone(),
        adjusted: adjusted.clone(),
        blocked: blocked.clone(),
        certificates: certificates.clone(),
        locks: (!with_invoicing).then(|| locks.clone()),
        bills: (!with_invoicing).then(|| bills.clone()),
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
    // What wl-parties registers: an exception to a block is decided by the
    // finance manager (screening off).
    wl_kernel::PolicyDefault::new(wl_kernel::BLOCK_EXCEPTION_ACTION)
        .with_step(
            wl_kernel::approvals::Step::one_of(["finance_manager"])
                .requiring(wl_kernel::BLOCK_EXCEPTION_ACTION),
        )
        .register();
    permissions::define_role(
        app.db(),
        "finance_manager",
        &[wl_kernel::BLOCK_EXCEPTION_ACTION],
    )
    .await
    .unwrap();
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
        &[
            perm::PURCHASE_ORDERS_ISSUE,
            perm::SUPPLIER_RETURNS_MANAGE,
            perm::LANDED_COSTS_POST,
            perm::ITEM_SUPPLIERS_MANAGE,
        ],
    )
    .await
    .unwrap();
    permissions::define_role(app.db(), "cost_viewer", &[perm::STOCK_COST_VIEW])
        .await
        .unwrap();
    Booted {
        app,
        received: Recorded(received),
        dispatched: Recorded(dispatched),
        revised: Recorded(revised),
        allocated: Recorded(allocated),
        adjusted: Recorded(adjusted),
        locks,
        bills,
        blocked,
        certificates,
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
