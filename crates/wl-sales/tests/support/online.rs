//! The fakes the online-payment tests need (BL-SAL-01 to BL-SAL-04): a payments
//! recorder that knows the payments made for an order and records the refunds
//! asked, payment links that are recorded and closed, a notifier, a task
//! desk, and the order events. They are the other modules' contracts,
//! answered by hand; the rules under test are the real `wl-sales` listeners,
//! services and job.

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::payments::{
    AllocationTarget, Direction, PAYMENT_LINK_SUBJECT_TYPE, PaymentFailed, PaymentFailureKind,
    PaymentFuture, PaymentId, PaymentLinkRef, PaymentLinkRequest, PaymentLinks, PaymentMethod,
    PaymentReceived, PaymentRecorder, PaymentStatus, PaymentSummary, RecordPayment, RefundId,
    RefundRequest, ReversalReason,
};
use wl_kernel::sales::{SalesOrderCancelled, SalesOrderConfirmed};
use wl_kernel::tickets::{OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef};
use wl_kernel::{PartyId, Subject};
use wl_sales::{SalesOrder, WlSales};

use super::shop::Mail;
use super::{Booted, boot_configured};

#[derive(Default)]
struct Book {
    payments: Vec<(Subject, PaymentSummary)>,
    refunds: Vec<(RefundRequest, Option<i64>)>,
    links: Vec<PaymentLinkRequest>,
    closed: Vec<Subject>,
    refuse_refunds: bool,
}

/// The payments module as the sales tests see it.
#[derive(Clone, Default)]
pub struct Gateway(Arc<Mutex<Book>>);

fn off() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "payments is not enabled".to_owned(),
    )
}

impl Gateway {
    /// The gateway confirms `amount` fils of payment `id` for `order`: the
    /// payment is kept as made in advance for the order, and the event that
    /// `wl-payments` would emit is returned.
    pub fn received(&self, order: &SalesOrder, id: i64, amount: i64) -> PaymentReceived {
        let summary = PaymentSummary {
            id: PaymentId(id),
            number: Some(format!("RCT-{id}")),
            party: order.party(),
            legal_entity_id: 1,
            direction: Direction::In,
            method: PaymentMethod::Gateway,
            amount,
            allocated: 0,
            unallocated: amount,
            currency: "AED".to_owned(),
            status: PaymentStatus::Received,
            date: today(),
        };
        self.0
            .lock()
            .unwrap()
            .payments
            .push((order.subject(), summary));
        PaymentReceived {
            payment: PaymentId(id),
            party: order.party(),
            amount,
            method: PaymentMethod::Gateway,
            allocations: Vec::new(),
        }
    }

    /// The `PaymentFailed` event for `order`.
    pub fn failed(&self, order: &SalesOrder, id: i64, amount: i64) -> PaymentFailed {
        PaymentFailed {
            payment: PaymentId(id),
            party: order.party(),
            amount,
            kind: PaymentFailureKind::Declined,
            reason: "card declined".to_owned(),
            subscription: None,
            payment_link: None,
            prepaid: Some(order.subject()),
        }
    }

    /// The refunds asked for, with the user who asked, oldest first.
    pub fn refunds(&self) -> Vec<(RefundRequest, Option<i64>)> {
        self.0.lock().unwrap().refunds.clone()
    }

    /// The payment links asked for, oldest first.
    pub fn links(&self) -> Vec<PaymentLinkRequest> {
        self.0.lock().unwrap().links.clone()
    }

    /// The links withdrawn, oldest first.
    pub fn closed_links(&self) -> Vec<Subject> {
        self.0.lock().unwrap().closed.clone()
    }

    /// Makes `request_refund` refuse (or work again).
    pub fn refuse_refunds(&self, on: bool) {
        self.0.lock().unwrap().refuse_refunds = on;
    }
}

impl PaymentRecorder for Gateway {
    fn record<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: RecordPayment,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, PaymentSummary> {
        Box::pin(async { Err(off()) })
    }

    fn allocate<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _target: AllocationTarget,
        _amount: i64,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(off()) })
    }

    fn reverse<'a>(
        &'a self,
        _state: &'a AppState,
        _payment: PaymentId,
        _reason: ReversalReason,
        _note: Option<String>,
        _by: Option<i64>,
    ) -> PaymentFuture<'a, ()> {
        Box::pin(async { Err(off()) })
    }

    fn find<'a>(&'a self, _db: &'a Db, id: PaymentId) -> PaymentFuture<'a, Option<PaymentSummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .payments
                .iter()
                .find(|(_, p)| p.id == id)
                .map(|(_, p)| p.clone()))
        })
    }

    fn credit_balance<'a>(
        &'a self,
        _db: &'a Db,
        _party: PartyId,
        _currency: &'a str,
    ) -> PaymentFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }

    fn payments_for<'a>(
        &'a self,
        _db: &'a Db,
        prepaid: &'a Subject,
    ) -> PaymentFuture<'a, Vec<PaymentSummary>> {
        Box::pin(async move {
            Ok(self
                .0
                .lock()
                .unwrap()
                .payments
                .iter()
                .filter(|(subject, _)| subject == prepaid)
                .map(|(_, p)| p.clone())
                .collect())
        })
    }

    fn request_refund<'a>(
        &'a self,
        _state: &'a AppState,
        request: RefundRequest,
        by: Option<i64>,
    ) -> PaymentFuture<'a, RefundId> {
        Box::pin(async move {
            let mut book = self.0.lock().unwrap();
            if book.refuse_refunds {
                return Err(off());
            }
            book.refunds.push((request, by));
            Ok(RefundId(
                i64::try_from(book.refunds.len()).unwrap_or(i64::MAX),
            ))
        })
    }
}

impl PaymentLinks for Gateway {
    fn create<'a>(
        &'a self,
        _state: &'a AppState,
        request: PaymentLinkRequest,
    ) -> PaymentFuture<'a, Option<PaymentLinkRef>> {
        Box::pin(async move {
            let mut book = self.0.lock().unwrap();
            book.links.push(request);
            let n = i64::try_from(book.links.len()).unwrap_or(i64::MAX);
            Ok(Some(PaymentLinkRef {
                link: Subject::new(PAYMENT_LINK_SUBJECT_TYPE, n),
                path: format!("/pay/link-{n}"),
            }))
        })
    }

    fn cancel<'a>(&'a self, _state: &'a AppState, link: &'a Subject) -> PaymentFuture<'a, bool> {
        Box::pin(async move {
            self.0.lock().unwrap().closed.push(link.clone());
            Ok(true)
        })
    }
}

/// The tasks opened, as ticketing would keep them.
#[derive(Clone, Default)]
pub struct Desk(Arc<Mutex<Vec<OpenTicket>>>);

impl Desk {
    /// The tasks opened, oldest first.
    pub fn opened(&self) -> Vec<OpenTicket> {
        self.0.lock().unwrap().clone()
    }
}

impl TaskCreator for Desk {
    fn open<'a>(
        &'a self,
        _state: &'a AppState,
        ticket: OpenTicket,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let mut opened = self.0.lock().unwrap();
            opened.push(ticket);
            let n = opened.len() as i64;
            Ok(Some(TicketRef {
                id: TicketId(n),
                number: format!("TKT-{n}"),
            }))
        })
    }

    fn find_open<'a>(
        &'a self,
        _state: &'a AppState,
        source_key: &'a str,
    ) -> TicketFuture<'a, Option<TicketRef>> {
        Box::pin(async move {
            let opened = self.0.lock().unwrap();
            Ok(opened
                .iter()
                .position(|t| t.source_key.as_deref() == Some(source_key))
                .map(|n| TicketRef {
                    id: TicketId(n as i64 + 1),
                    number: format!("TKT-{}", n + 1),
                }))
        })
    }
}

/// The fakes and the order events recorded.
#[derive(Clone, Default)]
pub struct Online {
    /// The notifier.
    pub mail: Mail,
    /// The payments module.
    pub gateway: Gateway,
    /// The task desk.
    pub desk: Desk,
    /// `SalesOrderConfirmed`.
    pub confirmed: Arc<Mutex<Vec<SalesOrderConfirmed>>>,
    /// `SalesOrderCancelled`.
    pub cancelled: Arc<Mutex<Vec<SalesOrderCancelled>>>,
}

impl Module for Online {
    fn name(&self) -> &'static str {
        "online"
    }

    fn register(&self, app: &mut Registry) {
        wl_kernel::notify::provide_notifier(app, self.mail.clone());
        wl_kernel::payments::provide_payment_recorder(app, self.gateway.clone());
        wl_kernel::payments::provide_payment_links(app, self.gateway.clone());
        wl_kernel::tickets::provide_task_creator(app, self.desk.clone());
        let log = self.confirmed.clone();
        app.listen(move |event: SalesOrderConfirmed, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
        let log = self.cancelled.clone();
        app.listen(move |event: SalesOrderCancelled, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}

/// The sales module with the fakes.
pub async fn boot_online() -> (Booted, Online) {
    boot_online_with(WlSales::new()).await
}

/// [`boot_online`] with `sales` as the sales module.
pub async fn boot_online_with(sales: WlSales) -> (Booted, Online) {
    let online = Online::default();
    let t = boot_configured(sales, online.clone()).await;
    (t, online)
}

/// A user who enters, submits, accepts and cancels orders, and sees them.
pub async fn seller(t: &Booted) -> User {
    t.user(
        "Sam",
        &[
            wl_sales::permissions::SALES_ORDERS_VIEW,
            wl_sales::permissions::SALES_ORDERS_CREATE,
            wl_sales::permissions::SALES_ORDERS_ACCEPT,
            wl_sales::permissions::SALES_ORDERS_CANCEL,
        ],
    )
    .await
}

/// A draft order for party 7 through the page, with one line of AED 100
/// before VAT (AED 105.00 with VAT = 10,500 fils). `extra` are more form
/// fields (`external_number`, `promo_code`).
pub async fn draft_order(
    t: &Booted,
    channel: &str,
    mode: &str,
    extra: &[(&str, &str)],
) -> SalesOrder {
    let mut form = vec![
        ("party_id", "7"),
        ("channel", channel),
        ("payment_mode", mode),
    ];
    form.extend_from_slice(extra);
    t.app.post("/sales-orders", &form).await.assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the order was entered");
    t.app
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", "meal"),
                ("unit_price", "100.00"),
                ("quantity", "1"),
            ],
        )
        .await
        .assert_status(303);
    reload(t, order.id).await
}

/// The order `id` as it is now.
pub async fn reload(t: &Booted, id: i64) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), id).await.unwrap()
}

/// Submits the draft order `id` through its page.
pub async fn submit(t: &Booted, id: i64) -> SalesOrder {
    t.app
        .post(&format!("/sales-orders/{id}/submit"), &[("x", "")])
        .await
        .assert_status(303);
    reload(t, id).await
}

/// The total of an order made by [`draft_order`], in fils.
pub const TOTAL: i64 = 10_500;
