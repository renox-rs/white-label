//! The fakes and helpers the order-change tests need (BL-SAL-05, BL-SAL-06,
//! BL-SAL-08, BL-SAL-09): payments, payment links, the task desk and the
//! notifier of the online tests, an invoicer that records the drafts, updates
//! and credit notes it was asked for, the vertical's "fulfilment started", the
//! credit position of the customer, and the order events recorded. They are
//! the other modules' contracts, answered by hand; the rules under test are
//! the real `wl-sales` machine, services, listeners and jobs.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::invoicing::{
    CreditLine, CreditNoteId, InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceStatus,
    InvoiceSummary, Invoicer,
};
use wl_kernel::sales::{
    CreditExposure, CreditPosition, FulfilmentProgress, SalesFuture, SalesOrderAmended,
    SalesOrderCancelled, SalesOrderId, SalesOrderLineCancelled, SalesOrderLinesReleased,
};
use wl_kernel::{PartyId, Subject};
use wl_sales::permissions as perm;
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use super::online::{Desk, Gateway};
use super::shop::Mail;
use super::{Booted, boot_configured};

#[derive(Default)]
struct Pages {
    next: i64,
    drafts: Vec<(i64, InvoiceDraft)>,
    updates: Vec<(i64, InvoiceDraft)>,
    credits: Vec<(i64, Vec<CreditLine>, String)>,
    status: BTreeMap<i64, InvoiceStatus>,
}

/// The invoicing module as the order tests see it: every draft, update and
/// credit note asked for is kept; an invoice is a draft unless the test
/// issued it.
#[derive(Clone, Default)]
pub struct Ledger(Arc<Mutex<Pages>>);

impl Ledger {
    /// Sets the status invoice `id` has.
    pub fn set_status(&self, id: i64, status: InvoiceStatus) {
        self.0.lock().unwrap().status.insert(id, status);
    }

    /// The invoices drafted, oldest first, with their ids.
    pub fn drafts(&self) -> Vec<(i64, InvoiceDraft)> {
        self.0.lock().unwrap().drafts.clone()
    }

    /// The draft invoices updated, oldest first.
    pub fn updates(&self) -> Vec<(i64, InvoiceDraft)> {
        self.0.lock().unwrap().updates.clone()
    }

    /// The credit notes asked for: invoice, lines, reason.
    pub fn credits(&self) -> Vec<(i64, Vec<CreditLine>, String)> {
        self.0.lock().unwrap().credits.clone()
    }
}

impl Invoicer for Ledger {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async move {
            let mut book = self.0.lock().unwrap();
            book.next += 1;
            let id = 100 + book.next;
            book.drafts.push((id, draft));
            Ok(Some(InvoiceId(id)))
        })
    }

    fn update_draft<'a>(
        &'a self,
        _db: &'a Db,
        id: InvoiceId,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, bool> {
        Box::pin(async move {
            let mut book = self.0.lock().unwrap();
            let status = book.status.get(&id.0).copied().unwrap_or(InvoiceStatus::Draft);
            if status != InvoiceStatus::Draft {
                return Ok(false);
            }
            book.updates.push((id.0, draft));
            Ok(true)
        })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async move {
            let status = self
                .0
                .lock()
                .unwrap()
                .status
                .get(&id.0)
                .copied()
                .unwrap_or(InvoiceStatus::Draft);
            Ok(Some(InvoiceSummary {
                id,
                number: None,
                status,
                party: PartyId(7),
                legal_entity_id: None,
                currency: "AED".to_owned(),
                gross_total: 0,
                balance: 0,
                due_date: None,
                disputed: false,
                chasing_paused_until: None,
            }))
        })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _id: InvoiceId,
        _source: Subject,
        _amount: i64,
    ) -> InvoiceFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn credit_partly<'a>(
        &'a self,
        _state: &'a AppState,
        invoice: InvoiceId,
        lines: Vec<CreditLine>,
        reason: String,
        _by: Option<i64>,
    ) -> InvoiceFuture<'a, CreditNoteId> {
        Box::pin(async move {
            let mut book = self.0.lock().unwrap();
            book.credits.push((invoice.0, lines, reason));
            Ok(CreditNoteId(500 + book.credits.len() as i64))
        })
    }
}

/// The sale lines whose fulfilment started (picked, in the kitchen), as the
/// vertical would answer.
#[derive(Clone, Default)]
pub struct Started(Arc<Mutex<Vec<i64>>>);

impl Started {
    /// Marks the sale line `line` as started.
    pub fn mark(&self, line: i64) {
        self.0.lock().unwrap().push(line);
    }
}

impl FulfilmentProgress for Started {
    fn started_lines<'a>(
        &'a self,
        _state: &'a AppState,
        _order: SalesOrderId,
        lines: &'a [i64],
    ) -> SalesFuture<'a, Vec<i64>> {
        Box::pin(async move {
            let marked = self.0.lock().unwrap().clone();
            Ok(lines
                .iter()
                .copied()
                .filter(|line| marked.contains(line))
                .collect())
        })
    }
}

/// The credit position of the customer, as the credit module would answer.
#[derive(Clone, Default)]
pub struct Exposure(Arc<Mutex<Option<CreditPosition>>>);

impl Exposure {
    /// The customer has `exposure` fils without the order being checked and
    /// a limit of `limit` fils.
    pub fn set(&self, exposure: i64, limit: i64) {
        *self.0.lock().unwrap() = Some(CreditPosition {
            exposure,
            limit: Some(limit),
            group_exposure: None,
            group_limit: None,
        });
    }
}

impl CreditExposure for Exposure {
    fn position<'a>(
        &'a self,
        _state: &'a AppState,
        _party: PartyId,
        _currency: &'a str,
        _excluding: &'a Subject,
    ) -> SalesFuture<'a, Option<CreditPosition>> {
        Box::pin(async move { Ok(self.0.lock().unwrap().clone()) })
    }
}

/// Every fake and the order events recorded.
#[derive(Clone, Default)]
pub struct Changes {
    /// The notifier.
    pub mail: Mail,
    /// The payments module.
    pub gateway: Gateway,
    /// The task desk.
    pub desk: Desk,
    /// The invoicing module.
    pub ledger: Ledger,
    /// The vertical's "fulfilment started".
    pub started: Started,
    /// The credit position.
    pub exposure: Exposure,
    /// `SalesOrderAmended`.
    pub amended: Arc<Mutex<Vec<SalesOrderAmended>>>,
    /// `SalesOrderLineCancelled`.
    pub line_cancelled: Arc<Mutex<Vec<SalesOrderLineCancelled>>>,
    /// `SalesOrderLinesReleased`.
    pub released: Arc<Mutex<Vec<SalesOrderLinesReleased>>>,
    /// `SalesOrderCancelled`.
    pub cancelled: Arc<Mutex<Vec<SalesOrderCancelled>>>,
}

/// Records the events of type `E` into `log`.
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

impl Module for Changes {
    fn name(&self) -> &'static str {
        "changes"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::notify::provide_notifier(app, self.mail.clone());
        wl_kernel::payments::provide_payment_recorder(app, self.gateway.clone());
        wl_kernel::payments::provide_payment_links(app, self.gateway.clone());
        wl_kernel::tickets::provide_task_creator(app, self.desk.clone());
        wl_kernel::invoicing::provide_invoicer(app, self.ledger.clone());
        wl_kernel::sales::provide_fulfilment_progress(app, self.started.clone());
        wl_kernel::sales::provide_credit_exposure(app, self.exposure.clone());
        record(app, &self.amended);
        record(app, &self.line_cancelled);
        record(app, &self.released);
        record(app, &self.cancelled);
    }
}

/// The sales module with the fakes and the default approval policies stored.
pub async fn boot_changes() -> (Booted, Changes) {
    boot_changes_with(WlSales::new()).await
}

/// [`boot_changes`] with `sales` as the sales module.
pub async fn boot_changes_with(sales: WlSales) -> (Booted, Changes) {
    let changes = Changes::default();
    let t = boot_configured(sales, changes.clone()).await;
    approvals::install(t.db()).await.unwrap();
    (t, changes)
}

/// A clerk who enters, submits, accepts, amends and cancels orders (Sam).
pub async fn clerk(t: &Booted) -> User {
    t.user(
        "Sam",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::SALES_ORDERS_CREATE,
            perm::SALES_ORDERS_ACCEPT,
            perm::SALES_ORDERS_AMEND,
            perm::SALES_ORDERS_CANCEL,
        ],
    )
    .await
}

/// A manager who approves changes after fulfilment started (Mia).
pub async fn manager(t: &Booted) -> User {
    t.user(
        "Mia",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::SALES_ORDERS_AMEND,
            perm::SALES_ORDERS_CANCEL,
            perm::SALES_ORDERS_APPROVE,
        ],
    )
    .await
}

static PAYMENTS: AtomicI64 = AtomicI64::new(900);

/// A new payment id for the fake gateway.
pub fn payment_id() -> i64 {
    PAYMENTS.fetch_add(1, Ordering::SeqCst)
}

/// An order of party 7 through the pages: `lines` are (item, quantity in
/// units, unit price in AED), zero-rated so the totals are the figures of the
/// stories. Submitted; an order paid online is paid in full by the gateway;
/// a website order is accepted. It ends `confirmed`.
pub async fn place(
    t: &Booted,
    c: &Changes,
    channel: &str,
    mode: &str,
    lines: &[(&str, &str, &str)],
) -> SalesOrder {
    t.app
        .post(
            "/sales-orders",
            &[("party_id", "7"), ("channel", channel), ("payment_mode", mode)],
        )
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the order was entered");
    for (code, quantity, price) in lines {
        t.app
            .post(
                &format!("/sales-orders/{}/lines", order.id),
                &[
                    ("item_code", code),
                    ("quantity", quantity),
                    ("unit_price", price),
                    ("vat", "zero_rated"),
                ],
            )
            .await
            .assert_status(303);
    }
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let mut order = reload(t, order.id).await;
    if mode == "online" {
        let paid = c.gateway.received(&order, payment_id(), order.gross_total);
        t.state().emit(paid).await.unwrap();
        order = reload(t, order.id).await;
    }
    if order.status == SalesOrderStatus::PendingAcceptance {
        t.app
            .post(
                &format!("/sales-orders/{}/transitions/accept", order.id),
                &[("x", "")],
            )
            .await
            .assert_status(303);
        order = reload(t, order.id).await;
    }
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    order
}

/// The order `id` as it is now.
pub async fn reload(t: &Booted, id: i64) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), id).await.unwrap()
}

/// The lines of `order`, in order.
pub async fn lines(t: &Booted, order: &SalesOrder) -> Vec<SaleLine> {
    SaleLine::of(t.db(), &order.subject()).await.unwrap()
}

/// Posts the amend form of `order` as it is now, with `fields` added to the
/// revision and the reason.
pub async fn amend(
    t: &Booted,
    order: &SalesOrder,
    reason: &str,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let revision = order.revision.to_string();
    let mut form = vec![("revision", revision.as_str()), ("reason", reason)];
    form.extend_from_slice(fields);
    t.app
        .post(&format!("/sales-orders/{}/amend", order.id), &form)
        .await
}

/// The approval requests waiting about `order`.
pub async fn waiting(t: &Booted, order: &SalesOrder) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &order.subject())
        .await
        .unwrap()
}

/// A dispatch note `n` hands over `quantity_milli` of `line` of `order`
/// through the kernel's `OrderFulfilment`.
pub async fn hand_over(
    t: &Booted,
    order: &SalesOrder,
    line: &SaleLine,
    quantity_milli: i64,
    n: i64,
) {
    wl_kernel::sales::order_fulfilment(t.state())
        .fulfil(
            t.state(),
            wl_kernel::sales::FulfilOrder {
                order: SalesOrderId(order.id),
                lines: vec![wl_kernel::sales::FulfilledLine {
                    line: line.id,
                    quantity_milli,
                }],
                source: Subject::new("inventory.dispatch", n),
            },
        )
        .await
        .unwrap();
}
