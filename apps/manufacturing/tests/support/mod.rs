//! Booting the Manufacturing vertical in a `TestApp` with auth and the kernel,
//! and fakes for what the vertical reaches through the kernel contracts: the
//! stock (consumption and the movements of a work order), the bills of
//! materials, the lines of the sales orders, the tickets of the role tasks and
//! the notifier. A test cannot build an `AuthUser`, so a request is made
//! through the vertical's own routes as the user.

#![allow(dead_code)]

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use manufacturing::permissions as perm;
use manufacturing::{
    Line, Manufacturing, Plant, WorkOrder, WorkOrderAborted, WorkOrderCancelled,
    WorkOrderCompleted, WorkOrderStatus, WorkOrderUnlinked,
};
use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::{TestApp, TestResponse};
use wl_kernel::inventory::{
    ConsumeRequest, Consumed, ConsumedLot, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE,
    LOT_SUBJECT_TYPE, PENDING_ISSUE_SUBJECT_TYPE, StockChoice, StockFuture, StockLot,
    StockNegative, StockService, provide_stock_service,
};
use wl_kernel::lot_disposal::{
    LotDisposal, LotFacts, LotFuture, ReturnLot, ScrapLot, Scrapped, provide_lot_disposal,
};
use wl_kernel::notify::{
    MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
};
use wl_kernel::production::{
    BillsOfMaterials, BomComponent, BomVersion, OrderLineInfo, OrderLines, ProduceOutput,
    ProductionFuture, ProductionStock, ReturnMaterial, VoidOutput, WasteMaterial, provide_boms,
    provide_order_lines, provide_production_stock,
};
use wl_kernel::sales::SalesOrderId;
use wl_kernel::tickets::{
    OpenTicket, TaskComment, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
};
use wl_kernel::voids::RecordVoided;
use wl_kernel::{BusinessCalendar, Subject};

/// A manufacturer with no shared module besides the kernel.
const CLIENT: &str = r#"
[client]
name = "Gulf Plastics LLC"
app = "manufacturing"

[modules]
enabled = ["parties"]

[[legal_entities]]
code = "GPL"
name_en = "Gulf Plastics LLC"
default = true
"#;

/// The finished item the tests make (a panel), and the resin it is made of.
pub const PANEL: i64 = 9;
/// The resin.
pub const RESIN: i64 = 11;
/// A second material, a pigment.
pub const PIGMENT: i64 = 12;
/// The released BOM version of the panel.
pub const BOM: i64 = 3;
/// The store of the line.
pub const STORE: i64 = 21;
/// The finished goods location of the line.
pub const FINISHED: i64 = 22;

/// The role of a line operator: reports on the work orders of the line.
pub const OPERATOR_ROLE: &str = "line_operator";

/// A date.
pub fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// An event log.
#[derive(Clone)]
pub struct Recorded<E>(pub Arc<Mutex<Vec<E>>>);

impl<E: Clone> Recorded<E> {
    /// What was recorded, oldest first.
    pub fn all(&self) -> Vec<E> {
        self.0.lock().unwrap().clone()
    }
}

impl<E> Default for Recorded<E> {
    fn default() -> Self {
        Recorded(Arc::new(Mutex::new(Vec::new())))
    }
}

/// Records `event`s of type `E` into `log`.
fn record<E: Event + Clone + Send + Sync + 'static>(app: &mut Registry, log: &Recorded<E>) {
    let log = log.0.clone();
    app.listen(move |event: E, _state: AppState| {
        let log = log.clone();
        async move {
            log.lock().unwrap().push(event);
            Ok(())
        }
    });
}

/// A shelf of the fake store: a quantity of a lot of a material at a cost.
#[derive(Clone, Debug)]
struct Shelf {
    item: i64,
    lot: Option<i64>,
    quantity_milli: i64,
    unit_cost: i64,
}

/// The stock of wl-inventory, as the vertical reaches it: the consumption of
/// the work order, and its return, wastage and output movements.
#[derive(Clone, Default)]
pub struct Stock {
    shelves: Arc<Mutex<Vec<Shelf>>>,
    /// The returns posted.
    pub returned: Arc<Mutex<Vec<ReturnMaterial>>>,
    /// The wastages posted.
    pub wasted: Arc<Mutex<Vec<WasteMaterial>>>,
    /// The outputs posted.
    pub produced: Arc<Mutex<Vec<ProduceOutput>>>,
    /// The reservations released, `(lot, order, line)`.
    pub released: Arc<Mutex<Vec<(Subject, i64, i64)>>>,
    /// The outputs taken out again by a void.
    pub output_voided: Arc<Mutex<Vec<VoidOutput>>>,
    /// The wastages reversed by a void.
    pub waste_voided: Arc<Mutex<Vec<WasteMaterial>>>,
    /// Thousandths of the output somebody else moved, issued or picked.
    used: Arc<AtomicI64>,
    /// Thousandths consumed from the shelves so far.
    taken: Arc<AtomicI64>,
    /// The items that are lot-tracked.
    tracked: Arc<Mutex<Vec<i64>>>,
    pending: Arc<AtomicI64>,
}

impl Stock {
    /// `quantity_milli` of `item` (lot `lot`) on the shelf at `unit_cost`.
    pub fn put(&self, item: i64, lot: Option<i64>, quantity_milli: i64, unit_cost: i64) {
        self.shelves.lock().unwrap().push(Shelf {
            item,
            lot,
            quantity_milli,
            unit_cost,
        });
    }

    /// `item` is lot-tracked.
    pub fn track(&self, item: i64) {
        self.tracked.lock().unwrap().push(item);
    }

    /// Thousandths consumed from the shelves so far.
    pub fn consumed(&self) -> i64 {
        self.taken.load(Ordering::SeqCst)
    }

    /// `quantity_milli` of the output lots were moved out by somebody else.
    pub fn use_output(&self, quantity_milli: i64) {
        self.used.fetch_add(quantity_milli, Ordering::SeqCst);
    }

    /// What of the output is still in the lots: made less used less voided.
    pub fn output_left(&self) -> i64 {
        let made: i64 = self
            .produced
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.quantity_milli)
            .sum();
        let voided: i64 = self
            .output_voided
            .lock()
            .unwrap()
            .iter()
            .map(|v| v.quantity_milli)
            .sum();
        made - voided - self.used.load(Ordering::SeqCst)
    }

    /// The movements posted by the abort: returns, wastages.
    pub fn movements(&self) -> usize {
        self.returned.lock().unwrap().len() + self.wasted.lock().unwrap().len()
    }
}

impl StockService for Stock {
    fn consume<'a>(
        &'a self,
        state: &'a AppState,
        request: ConsumeRequest,
    ) -> StockFuture<'a, Consumed> {
        Box::pin(async move {
            let consumed = self.take(&request);
            if let Some(pending) = consumed.pending_issue.clone() {
                state
                    .emit(StockNegative {
                        item: request.item.clone(),
                        location: request.location.clone(),
                        quantity_milli: consumed.short_milli,
                        source: Some(request.source.clone()),
                        pending_issue: Some(pending),
                    })
                    .await?;
            }
            Ok(consumed)
        })
    }

    fn choices_of_item<'a>(
        &'a self,
        _state: &'a AppState,
        item: &'a Subject,
        _location: &'a Subject,
    ) -> StockFuture<'a, Option<StockChoice>> {
        Box::pin(async move { Ok(self.choice(item)) })
    }
}

impl Stock {
    /// The lots of `item` that can be taken from, as `choices` would give.
    fn choice(&self, item: &Subject) -> Option<StockChoice> {
        let tracked = self.tracked.lock().unwrap().contains(&item.subject_id);
        let lots = if tracked {
            self.shelves
                .lock()
                .unwrap()
                .iter()
                .filter(|shelf| shelf.item == item.subject_id && shelf.quantity_milli > 0)
                .filter_map(|shelf| {
                    Some(StockLot {
                        lot: Subject::new(LOT_SUBJECT_TYPE, shelf.lot?),
                        number: format!("R{}", shelf.lot?),
                        expiry_date: None,
                        available_milli: shelf.quantity_milli,
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        Some(StockChoice {
            item: item.clone(),
            lot_tracked: tracked,
            lots,
        })
    }

    /// Takes `request` from the shelves: the lot asked for first.
    fn take(&self, request: &ConsumeRequest) -> Consumed {
        {
            let mut shelves = self.shelves.lock().unwrap();
            let mut left = request.quantity_milli;
            let mut lots = Vec::new();
            let mut order: Vec<usize> = (0..shelves.len())
                .filter(|&i| shelves[i].item == request.item.subject_id)
                .collect();
            if let Some(wanted) = request.lot.as_ref().map(|lot| lot.subject_id) {
                order.sort_by_key(|&i| shelves[i].lot != Some(wanted));
            }
            for index in order {
                if left == 0 {
                    break;
                }
                let shelf = &mut shelves[index];
                let part = left.min(shelf.quantity_milli);
                if part == 0 {
                    continue;
                }
                shelf.quantity_milli -= part;
                left -= part;
                lots.push(ConsumedLot {
                    lot: shelf.lot.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
                    quantity_milli: part,
                    unit_cost: shelf.unit_cost,
                });
            }
            let pending = (left > 0).then(|| {
                Subject::new(
                    PENDING_ISSUE_SUBJECT_TYPE,
                    self.pending.fetch_add(1, Ordering::SeqCst) + 1,
                )
            });
            self.taken
                .fetch_add(request.quantity_milli - left, Ordering::SeqCst);
            Consumed {
                posted_milli: request.quantity_milli - left,
                short_milli: left,
                pending_issue: pending,
                lots,
            }
        }
    }
}

impl ProductionStock for Stock {
    fn return_material<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        request: ReturnMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move {
            self.returned.lock().unwrap().push(request);
            Ok(())
        })
    }

    fn waste_material<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        request: WasteMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move {
            self.wasted.lock().unwrap().push(request);
            Ok(())
        })
    }

    fn produce<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        request: ProduceOutput,
    ) -> ProductionFuture<'a, Option<Subject>> {
        Box::pin(async move {
            let lot = request
                .lot
                .clone()
                .unwrap_or_else(|| Subject::new(LOT_SUBJECT_TYPE, 900 + request.source.subject_id));
            self.produced.lock().unwrap().push(request);
            Ok(Some(lot))
        })
    }

    fn output_available<'a>(
        &'a self,
        _state: &'a AppState,
        _item: &'a Subject,
        _lot: Option<&'a Subject>,
        _location: &'a Subject,
    ) -> ProductionFuture<'a, Option<i64>> {
        Box::pin(async move { Ok(Some(self.output_left())) })
    }

    fn void_output<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        request: VoidOutput,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move {
            if self.output_left() < request.quantity_milli {
                return Err(Error::Status(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "Output already used".to_owned(),
                ));
            }
            self.output_voided.lock().unwrap().push(request);
            Ok(())
        })
    }

    fn void_waste<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        request: WasteMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async move {
            self.waste_voided.lock().unwrap().push(request);
            Ok(())
        })
    }

    fn release_reservation<'a>(
        &'a self,
        _tx: &'a mut renox::db::Transaction,
        lot: &'a Subject,
        order: SalesOrderId,
        line: i64,
    ) -> ProductionFuture<'a, bool> {
        Box::pin(async move {
            self.released
                .lock()
                .unwrap()
                .push((lot.clone(), order.0, line));
            Ok(true)
        })
    }
}

/// A lot of the fake inventory.
#[derive(Clone, Debug)]
struct FakeLot {
    facts: LotFacts,
}

/// A Return to supplier of the fake inventory.
#[derive(Clone, Debug)]
pub struct FakeReturn {
    /// What was asked.
    pub request: ReturnLot,
    /// `draft`, `dispatched` or `cancelled`.
    pub status: String,
    /// Its subject.
    pub subject: Subject,
}

/// The lots of wl-inventory, as an NCR reaches them through the kernel's
/// `LotDisposal`: the facts the test sets, the draft returns made, the scrap
/// and the releases asked.
#[derive(Clone, Default)]
pub struct Lots {
    lots: Arc<Mutex<Vec<FakeLot>>>,
    /// The returns made.
    pub returns: Arc<Mutex<Vec<FakeReturn>>>,
    /// The scrap posted.
    pub scrapped: Arc<Mutex<Vec<ScrapLot>>>,
    /// The lots released.
    pub released: Arc<Mutex<Vec<Subject>>>,
    /// The lots closed out (rejected when emptied).
    pub closed_out: Arc<Mutex<Vec<Subject>>>,
}

impl Lots {
    /// Lot `id` of `item`, number `number`, from supplier `supplier` (none:
    /// made in-house when `own`, else opening stock), `quantity_milli` at
    /// `unit_cost` fils, in `status`.
    #[allow(clippy::too_many_arguments)]
    pub fn put(
        &self,
        id: i64,
        number: &str,
        supplier: Option<i64>,
        own: bool,
        quantity_milli: i64,
        unit_cost: i64,
        status: &str,
    ) {
        self.lots.lock().unwrap().push(FakeLot {
            facts: LotFacts {
                lot: Subject::new(LOT_SUBJECT_TYPE, id),
                item: Subject::new(ITEM_SUBJECT_TYPE, RESIN),
                number: number.to_owned(),
                supplier_lot: supplier.map(|_| number.to_owned()),
                status: status.to_owned(),
                supplier: supplier.map(wl_kernel::PartyId),
                receipt: supplier.map(|_| Subject::new("inventory.goods_receipt", 12)),
                produced_by: own.then(|| Subject::new("manufacturing.work_order", 99)),
                quantity_milli,
                location: Some(location(STORE)),
                location_milli: quantity_milli,
                unit_cost,
                average_cost: unit_cost,
                po_line: supplier.map(|_| 4),
            },
        });
    }

    /// The Return to supplier `n` (from 1) is now `status`.
    pub fn set_return(&self, n: usize, status: &str) {
        self.returns.lock().unwrap()[n - 1].status = status.to_owned();
    }

    /// The returns made so far.
    pub fn made(&self) -> Vec<FakeReturn> {
        self.returns.lock().unwrap().clone()
    }

    fn find(&self, lot: &Subject) -> Option<LotFacts> {
        self.lots
            .lock()
            .unwrap()
            .iter()
            .find(|found| found.facts.lot == *lot)
            .map(|found| found.facts.clone())
    }
}

fn unprocessable(message: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.to_owned())
}

impl LotDisposal for Lots {
    fn facts<'a>(
        &'a self,
        _state: &'a AppState,
        lot: &'a Subject,
    ) -> LotFuture<'a, Option<LotFacts>> {
        Box::pin(async move { Ok(self.find(lot)) })
    }

    fn create_return<'a>(
        &'a self,
        _state: &'a AppState,
        request: ReturnLot,
    ) -> LotFuture<'a, Option<Subject>> {
        Box::pin(async move {
            let facts = self.find(&request.lot).expect("a known lot");
            if facts.supplier_lot.is_none() {
                return Err(unprocessable("The lot has no supplier lot"));
            }
            let mut all = self.returns.lock().unwrap();
            let subject = Subject::new(
                wl_kernel::inventory::SUPPLIER_RETURN_SUBJECT_TYPE,
                i64::try_from(all.len()).unwrap() + 1,
            );
            all.push(FakeReturn {
                request,
                status: "draft".to_owned(),
                subject: subject.clone(),
            });
            Ok(Some(subject))
        })
    }

    fn return_status<'a>(
        &'a self,
        _state: &'a AppState,
        supplier_return: &'a Subject,
    ) -> LotFuture<'a, Option<String>> {
        Box::pin(async move {
            Ok(self
                .returns
                .lock()
                .unwrap()
                .iter()
                .find(|found| found.subject == *supplier_return)
                .map(|found| found.status.clone()))
        })
    }

    fn scrap<'a>(
        &'a self,
        _state: &'a AppState,
        request: ScrapLot,
    ) -> LotFuture<'a, Option<Scrapped>> {
        Box::pin(async move {
            let facts = self.find(&request.lot).expect("a known lot");
            let value = request.quantity_milli * facts.unit_cost / 1_000;
            self.scrapped.lock().unwrap().push(request);
            Ok(Some(Scrapped {
                value,
                discarded: false,
            }))
        })
    }

    fn release<'a>(&'a self, _state: &'a AppState, lot: &'a Subject) -> LotFuture<'a, bool> {
        Box::pin(async move {
            self.released.lock().unwrap().push(lot.clone());
            Ok(true)
        })
    }

    fn close_out<'a>(&'a self, _state: &'a AppState, lot: &'a Subject) -> LotFuture<'a, bool> {
        Box::pin(async move {
            self.closed_out.lock().unwrap().push(lot.clone());
            Ok(true)
        })
    }
}

/// The bills of materials of wl-inventory.
#[derive(Clone, Default)]
pub struct Boms(Arc<Mutex<Vec<BomVersion>>>);

impl Boms {
    /// A BOM version `id` for the panel: 2 kg of resin and 0.1 kg of pigment
    /// a unit with 5% scrap on the resin, AED 18 of material and 0.05 hours
    /// a unit.
    pub fn panel(&self, id: i64, released: bool) {
        self.0.lock().unwrap().push(BomVersion {
            bom: Subject::new(wl_kernel::production::BOM_SUBJECT_TYPE, id),
            item: Subject::new(ITEM_SUBJECT_TYPE, PANEL),
            version: 1,
            released,
            components: vec![
                BomComponent {
                    item: Subject::new(ITEM_SUBJECT_TYPE, RESIN),
                    quantity_milli: 2_000,
                    scrap_bps: 500,
                },
                BomComponent {
                    item: Subject::new(ITEM_SUBJECT_TYPE, PIGMENT),
                    quantity_milli: 100,
                    scrap_bps: 0,
                },
            ],
            material_cost_per_unit: 1_800,
            standard_milli_hours_per_unit: 50,
        });
    }
}

impl BillsOfMaterials for Boms {
    fn version<'a>(
        &'a self,
        _state: &'a AppState,
        bom: &'a Subject,
    ) -> ProductionFuture<'a, Option<BomVersion>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|version| version.bom == *bom)
                .cloned())
        })
    }
}

/// The sales order lines of wl-sales.
#[derive(Clone, Default)]
pub struct Lines(Arc<Mutex<Vec<OrderLineInfo>>>);

impl Lines {
    /// Line `line` of order `order` is open for `open_milli`, owned by `owner`.
    pub fn open(&self, order: i64, line: i64, open_milli: i64, owner: Option<i64>) {
        self.0.lock().unwrap().push(OrderLineInfo {
            order: SalesOrderId(order),
            line,
            number: Some(format!("SO-{order}")),
            owner,
            open_milli,
            cancelled: false,
        });
    }

    /// Line `line` of order `order` was cancelled.
    pub fn cancel(&self, order: i64, line: i64) {
        for info in self.0.lock().unwrap().iter_mut() {
            if info.order.0 == order && info.line == line {
                info.cancelled = true;
                info.open_milli = 0;
            }
        }
    }
}

impl OrderLines for Lines {
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
                .iter()
                .find(|info| info.order == order && info.line == line)
                .cloned())
        })
    }
}

/// A ticket another module opened, as the tickets fake keeps it.
#[derive(Debug, Clone)]
pub struct Opened {
    /// The ticket.
    pub ticket: OpenTicket,
    /// Its id.
    pub id: i64,
    /// Whether it was completed.
    pub done: bool,
}

/// The tickets of wl-ticketing, as the role tasks open them.
#[derive(Clone, Default)]
pub struct Tickets(Arc<Mutex<Vec<Opened>>>);

impl Tickets {
    /// Every ticket opened, oldest first.
    pub fn all(&self) -> Vec<Opened> {
        self.0.lock().unwrap().clone()
    }
}

impl TaskCreator for Tickets {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            let id = i64::try_from(all.len()).unwrap() + 1;
            all.push(Opened {
                ticket,
                id,
                done: false,
            });
            Ok(Some(TicketRef {
                id: TicketId(id),
                number: format!("TKT-2027-{id:04}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .iter()
                .find(|opened| {
                    !opened.done && opened.ticket.source_key.as_deref() == Some(source_key)
                })
                .map(|opened| TicketRef {
                    id: TicketId(opened.id),
                    number: format!("TKT-2027-{:04}", opened.id),
                }))
        })
    }

    fn complete<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: TicketId,
        _note: &'a str,
    ) -> TicketFuture<'a, bool> {
        Box::pin(async move {
            let mut all = self.0.lock().unwrap();
            match all.iter_mut().find(|opened| opened.id == ticket.0) {
                Some(opened) if !opened.done => {
                    opened.done = true;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })
    }

    fn comment<'a>(
        &'a self,
        _state: &'a AppState,
        _ticket: TicketId,
        _comment: TaskComment,
    ) -> TicketFuture<'a, ()> {
        Box::pin(async { Ok(()) })
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

    /// The titles of the notices sent to `user`.
    pub fn titles_for(&self, user: &User) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|request| {
                request
                    .recipients
                    .iter()
                    .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user.id))
            })
            .map(|request| {
                request.data["title"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect()
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

/// Provides the fakes.
struct Fakes {
    stock: Stock,
    lots: Lots,
    boms: Boms,
    lines: Lines,
    tickets: Tickets,
    notices: Recording,
    aborted: Recorded<WorkOrderAborted>,
    unlinked: Recorded<WorkOrderUnlinked>,
    cancelled: Recorded<WorkOrderCancelled>,
    completed: Recorded<WorkOrderCompleted>,
    voided: Recorded<RecordVoided>,
}

impl Module for Fakes {
    fn name(&self) -> &'static str {
        "fakes"
    }

    fn register(&self, app: &mut Registry) {
        provide_stock_service(app, self.stock.clone());
        provide_production_stock(app, self.stock.clone());
        provide_lot_disposal(app, self.lots.clone());
        provide_boms(app, self.boms.clone());
        provide_order_lines(app, self.lines.clone());
        provide_task_creator(app, self.tickets.clone());
        provide_notifier(app, self.notices.clone());
        record(app, &self.aborted);
        record(app, &self.unlinked);
        record(app, &self.cancelled);
        record(app, &self.completed);
        record(app, &self.voided);
    }
}

/// The app under test with the fakes.
pub struct Booted {
    /// The app.
    pub app: TestApp,
    /// The stock.
    pub stock: Stock,
    /// The lots of the inventory.
    pub lots: Lots,
    /// The BOMs.
    pub boms: Boms,
    /// The sales order lines.
    pub lines: Lines,
    /// The tickets the role tasks opened.
    pub tickets: Tickets,
    /// What the notifier was asked for.
    pub notices: Recording,
    /// `WorkOrderAborted`.
    pub aborted: Recorded<WorkOrderAborted>,
    /// `WorkOrderUnlinked`.
    pub unlinked: Recorded<WorkOrderUnlinked>,
    /// `WorkOrderCancelled`.
    pub cancelled: Recorded<WorkOrderCancelled>,
    /// `WorkOrderCompleted`.
    pub completed: Recorded<WorkOrderCompleted>,
    /// `RecordVoided`.
    pub voided: Recorded<RecordVoided>,
    /// Seconds the clock was moved forward from the real one.
    clock_secs: AtomicI64,
    /// The users made, for their emails.
    made: AtomicI64,
    /// Keeps the views directory alive.
    _views: tempfile::TempDir,
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

    /// A user with no role.
    pub async fn user(&self, name: &str) -> User {
        let n = self.made.fetch_add(1, Ordering::SeqCst);
        let email = format!("{}{n}@example.com", name.to_lowercase());
        User::register(self.db(), name, &email, "password123")
            .await
            .unwrap()
    }

    /// A user holding `role` in `scope`.
    pub async fn user_in(&self, name: &str, role: &str, scope: &permissions::Scope) -> User {
        let user = self.user(name).await;
        user.assign_role_in(self.db(), role, scope).await.unwrap();
        user
    }

    /// A production planner of the plant.
    pub async fn planner(&self, name: &str, plant: &Plant) -> User {
        self.user_in(name, perm::PLANNER_ROLE, &plant.scope()).await
    }

    /// The production manager of the plant.
    pub async fn manager(&self, name: &str, plant: &Plant) -> User {
        self.user_in(name, perm::PRODUCTION_MANAGER_ROLE, &plant.scope())
            .await
    }

    /// A storekeeper of the plant.
    pub async fn keeper(&self, name: &str, plant: &Plant) -> User {
        self.user_in(name, perm::STOREKEEPER_ROLE, &plant.scope())
            .await
    }

    /// A line supervisor of `line` only.
    pub async fn supervisor(&self, name: &str, line: &Line) -> User {
        self.user_in(name, perm::SUPERVISOR_ROLE, &line.scope())
            .await
    }

    /// A line operator of `line` only.
    pub async fn operator(&self, name: &str, line: &Line) -> User {
        self.user_in(name, OPERATOR_ROLE, &line.scope()).await
    }

    /// A QC inspector of the plant.
    pub async fn inspector(&self, name: &str, plant: &Plant) -> User {
        self.user_in(name, perm::QC_ROLE, &plant.scope()).await
    }

    /// A holder of `stock.approve` everywhere (`STK.ACL.02`).
    pub async fn stock_approver(&self, name: &str) -> User {
        let user = self.user(name).await;
        user.assign_role(self.db(), "stock_approver").await.unwrap();
        user
    }

    /// Moves the clock to `hour:minute` (Asia/Dubai) on `day` and runs `work`
    /// there; the clock only goes forward within a test.
    pub async fn at<F: std::future::Future>(
        &self,
        day: NaiveDate,
        hour: u32,
        minute: u32,
        work: F,
    ) -> F::Output {
        use renox::chrono::Duration;
        let moved = self.clock_secs.load(Ordering::SeqCst);
        let now = renox::db::now() + Duration::seconds(moved);
        let target =
            BusinessCalendar::default().from_local(day.and_hms_opt(hour, minute, 0).unwrap());
        let need = (target - now).num_seconds();
        assert!(
            need >= 0,
            "the clock cannot go back to {day} {hour}:{minute}"
        );
        self.app
            .travel(std::time::Duration::from_secs(u64::try_from(need).unwrap()));
        self.clock_secs.store(moved + need, Ordering::SeqCst);
        self.app.at_travelled_time(work).await
    }

    /// A plant.
    pub async fn plant(&self, name: &str) -> Plant {
        Plant::create(
            self.db(),
            Plant {
                name: name.to_owned(),
                active: true,
                ..Plant::default()
            },
        )
        .await
        .unwrap()
    }

    /// A line of `plant` at AED 120 an hour, with a store and a finished
    /// goods location.
    pub async fn line(&self, plant: &Plant, code: &str) -> Line {
        Line::create(
            self.db(),
            Line {
                plant_id: plant.id,
                code: code.to_owned(),
                name: format!("Line {code}"),
                hourly_rate: 12_000,
                store_location_id: Some(STORE),
                finished_location_id: Some(FINISHED),
                active: true,
                ..Line::default()
            },
        )
        .await
        .unwrap()
    }

    /// A POST as `by`.
    pub async fn post(&self, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
        self.app.acting_as(by);
        self.app.htmx().post(path, form).await
    }

    /// A GET as `by`.
    pub async fn get(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app.get(path).await
    }

    /// A GET as `by` the way a browser sends it.
    pub async fn browse(&self, by: &User, path: &str) -> TestResponse {
        self.app.acting_as(by);
        self.app
            .request()
            .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
            .get(path)
            .await
    }

    /// A QC inspector made without the approval right: a role of NCR work
    /// only.
    pub async fn clerk(&self, name: &str, plant: &Plant) -> User {
        self.user_in(name, "ncr_clerk", &plant.scope()).await
    }

    /// The NCR `id` as it is now.
    pub async fn ncr_now(&self, id: i64) -> manufacturing::Ncr {
        manufacturing::Ncr::find_or_404(self.db(), id)
            .await
            .unwrap()
    }

    /// An NCR on lot `lot` of `kg` kilos with the defect "colour off", raised
    /// by `by` in `plant` through the page's route.
    pub async fn ncr(&self, by: &User, plant: &Plant, lot: i64, kg: &str) -> manufacturing::Ncr {
        let (plant_id, lot_id) = (plant.id.to_string(), lot.to_string());
        self.post(
            by,
            "/manufacturing/ncrs",
            &[
                ("plant_id", &plant_id),
                ("lot_id", &lot_id),
                ("quantity", kg),
                ("defect", "colour off"),
            ],
        )
        .await
        .assert_status(303);
        manufacturing::Ncr::query()
            .order_by_desc(manufacturing::Ncr::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("an NCR")
    }

    /// `POST /manufacturing/ncrs/{id}/{action}` as `by`.
    pub async fn ncr_act(
        &self,
        by: &User,
        ncr: &manufacturing::Ncr,
        action: &str,
        form: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            by,
            &format!("/manufacturing/ncrs/{}/{action}", ncr.id),
            form,
        )
        .await
    }

    /// The work order `id` as it is now.
    pub async fn reload(&self, id: i64) -> WorkOrder {
        WorkOrder::find_or_404(self.db(), id).await.unwrap()
    }

    /// A `planned` work order for 1,000 panels on `line`, made by `by`
    /// through the page's route; from the BOM version `BOM`.
    pub async fn order(&self, by: &User, plant: &Plant, line: &Line) -> WorkOrder {
        let (plant_id, line_id) = (plant.id.to_string(), line.id.to_string());
        let (item, bom) = (PANEL.to_string(), BOM.to_string());
        self.post(
            by,
            "/manufacturing/work-orders",
            &[
                ("plant_id", &plant_id),
                ("line_id", &line_id),
                ("item_id", &item),
                ("bom_id", &bom),
                ("quantity", "1000"),
            ],
        )
        .await
        .assert_status(303);
        WorkOrder::query()
            .order_by_desc(WorkOrder::ID)
            .first(self.db())
            .await
            .unwrap()
            .expect("a work order")
    }

    /// Runs `action` (`release`, `start`...) on the work order as `by`.
    pub async fn act(
        &self,
        by: &User,
        order: &WorkOrder,
        action: &str,
        form: &[(&str, &str)],
    ) -> TestResponse {
        self.post(
            by,
            &format!("/manufacturing/work-orders/{}/{action}", order.id),
            form,
        )
        .await
    }

    /// A work order made and released: the planner makes it, the BOM is
    /// released.
    pub async fn released(&self, planner: &User, plant: &Plant, line: &Line) -> WorkOrder {
        let order = self.order(planner, plant, line).await;
        self.act(planner, &order, "release", &[])
            .await
            .assert_status(303);
        self.reload(order.id).await
    }

    /// A work order in `status` (a fixture, with a line and a BOM).
    pub async fn fixture(&self, plant: &Plant, line: &Line, status: WorkOrderStatus) -> WorkOrder {
        let n = self.made.fetch_add(1, Ordering::SeqCst) + 1000;
        let mut order = WorkOrder {
            number: format!("WO-FIX-{n}"),
            plant_id: plant.id,
            line_id: Some(line.id),
            item_id: PANEL,
            bom_id: Some(BOM),
            quantity_milli: 1_000_000,
            status,
            ..WorkOrder::default()
        };
        let mut tx = self.db().begin().await.unwrap();
        order
            .insert_in_state(&mut tx, status, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        order
    }
}

/// Boots the app.
pub async fn boot() -> Booted {
    let stock = Stock::default();
    let lots = Lots::default();
    let boms = Boms::default();
    boms.panel(BOM, true);
    boms.panel(BOM + 1, false);
    let lines = Lines::default();
    let tickets = Tickets::default();
    let notices = Recording::default();
    let aborted = Recorded::default();
    let unlinked = Recorded::default();
    let cancelled = Recorded::default();
    let completed = Recorded::default();
    let voided = Recorded::default();
    let fakes = Fakes {
        stock: stock.clone(),
        lots: lots.clone(),
        boms: boms.clone(),
        lines: lines.clone(),
        tickets: tickets.clone(),
        notices: notices.clone(),
        aborted: aborted.clone(),
        unlinked: unlinked.clone(),
        cancelled: cancelled.clone(),
        completed: completed.clone(),
        voided: voided.clone(),
    };
    let app = App::new()
        .module(Auth::new())
        .module(Permissions)
        .module(renox::audit::Audit)
        .module(Manufacturing)
        .module(fakes);
    let config = wl_kernel::ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    // The pages extend the app's layout; a vertical's test has no host, so it
    // brings the smallest one.
    let views = tempfile::tempdir().expect("a temporary views directory");
    let layout = views.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().unwrap()).unwrap();
    std::fs::write(
        &layout,
        "<!doctype html>\n<html><body><rx-outlet name=\"content\"/></body></html>\n",
    )
    .unwrap();
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    let define = |role: &'static str, grants: &'static [&'static str]| {
        let db = app.db().clone();
        async move { permissions::define_role(&db, role, grants).await.unwrap() }
    };
    define(
        perm::PLANNER_ROLE,
        &[
            perm::WORK_ORDERS_VIEW,
            perm::WORK_ORDERS_CREATE,
            perm::WORK_ORDERS_RELEASE,
        ],
    )
    .await;
    define(
        perm::PRODUCTION_MANAGER_ROLE,
        &[
            perm::WORK_ORDERS_VIEW,
            perm::WORK_ORDERS_CREATE,
            perm::WORK_ORDERS_RELEASE,
            perm::WORK_ORDERS_REPORT,
            perm::WORK_ORDERS_CLOSE,
            perm::WORK_ORDERS_ABORT_APPROVE,
            perm::WORK_ORDERS_VOID_APPROVE,
            perm::WORK_ORDERS_COST_VIEW,
            perm::NCRS_VIEW,
            perm::NCRS_RAISE,
            perm::NCRS_DISPOSITION,
            perm::NCRS_APPROVE,
        ],
    )
    .await;
    define(
        perm::STOREKEEPER_ROLE,
        &[perm::WORK_ORDERS_VIEW, perm::WORK_ORDERS_ISSUE],
    )
    .await;
    define(
        perm::SUPERVISOR_ROLE,
        &[
            perm::WORK_ORDERS_VIEW,
            perm::WORK_ORDERS_REPORT,
            perm::WORK_ORDERS_RELEASE,
            perm::WORK_ORDERS_VOID_APPROVE,
        ],
    )
    .await;
    define(
        OPERATOR_ROLE,
        &[perm::WORK_ORDERS_VIEW, perm::WORK_ORDERS_REPORT],
    )
    .await;
    define(
        perm::QC_ROLE,
        &[
            perm::WORK_ORDERS_VIEW,
            perm::NCRS_VIEW,
            perm::NCRS_RAISE,
            perm::NCRS_DISPOSITION,
            perm::NCRS_APPROVE,
        ],
    )
    .await;
    define("stock_approver", &[perm::STOCK_APPROVE]).await;
    define("ncr_clerk", &[perm::NCRS_VIEW, perm::NCRS_RAISE]).await;
    Booted {
        app,
        stock,
        lots,
        boms,
        lines,
        tickets,
        notices,
        aborted,
        unlinked,
        cancelled,
        completed,
        voided,
        clock_secs: AtomicI64::new(0),
        made: AtomicI64::new(0),
        _views: views,
    }
}

/// The subject of the location `id`, for assertions.
pub fn location(id: i64) -> Subject {
    Subject::new(LOCATION_SUBJECT_TYPE, id)
}
