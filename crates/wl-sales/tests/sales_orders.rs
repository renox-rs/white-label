//! The sales order base (wl-sales §4.2, §5.2, §5.3, §6 `OrderFulfilment`;
//! base for BL-SAL-22): an order is entered, submitted, accepted or
//! rejected, paid online, fulfilled and reversed through the kernel's
//! `OrderFulfilment`, cancelled and closed; the kernel events follow the
//! commit.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::invoicing::{
    InvoiceDraft, InvoiceFuture, InvoiceId, InvoiceSummary, Invoicer, provide_invoicer,
};
use wl_kernel::sales::{
    FulfilOrder, FulfilledLine, FulfilmentOutcome, ReverseFulfilment, SalesOrderCancelled,
    SalesOrderClosed, SalesOrderConfirmed, SalesOrderFulfilled, SalesOrderFulfilmentReversed,
    SalesOrderId, SalesOrderRejected, order_fulfilment,
};
use wl_sales::permissions as perm;
use wl_sales::sales_orders;
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use support::{Booted, boot_configured};

/// Records the drafts it was asked for.
struct Books(Arc<Mutex<Vec<InvoiceDraft>>>);

impl Invoicer for Books {
    fn draft<'a>(
        &'a self,
        _db: &'a Db,
        draft: InvoiceDraft,
    ) -> InvoiceFuture<'a, Option<InvoiceId>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(draft);
            Ok(Some(InvoiceId(9)))
        })
    }

    fn find<'a>(
        &'a self,
        _db: &'a Db,
        _id: InvoiceId,
    ) -> InvoiceFuture<'a, Option<InvoiceSummary>> {
        Box::pin(async { Ok(None) })
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
}

/// The events and drafts a test saw.
#[derive(Clone, Default)]
struct Seen {
    confirmed: Arc<Mutex<Vec<SalesOrderConfirmed>>>,
    fulfilled: Arc<Mutex<Vec<SalesOrderFulfilled>>>,
    reversed: Arc<Mutex<Vec<SalesOrderFulfilmentReversed>>>,
    cancelled: Arc<Mutex<Vec<SalesOrderCancelled>>>,
    rejected: Arc<Mutex<Vec<SalesOrderRejected>>>,
    closed: Arc<Mutex<Vec<SalesOrderClosed>>>,
    drafts: Arc<Mutex<Vec<InvoiceDraft>>>,
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

impl Module for Seen {
    fn name(&self) -> &'static str {
        "order_events"
    }

    fn register(&self, app: &mut Registry) {
        record(app, &self.confirmed);
        record(app, &self.fulfilled);
        record(app, &self.reversed);
        record(app, &self.cancelled);
        record(app, &self.rejected);
        record(app, &self.closed);
        provide_invoicer(app, Books(self.drafts.clone()));
    }
}

async fn boot() -> (Booted, Seen) {
    let seen = Seen::default();
    let t = boot_configured(WlSales::new(), seen.clone()).await;
    (t, seen)
}

/// A user who enters and submits orders, accepts and cancels them.
async fn seller(t: &Booted) -> User {
    t.user(
        "Sam",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::SALES_ORDERS_CREATE,
            perm::SALES_ORDERS_ACCEPT,
            perm::SALES_ORDERS_CANCEL,
        ],
    )
    .await
}

/// A draft order for party 7 through the page, with two lines of AED 50
/// (quantity 2) and AED 100 (quantity 1) before VAT.
async fn draft(t: &Booted, channel: &str, mode: &str) -> SalesOrder {
    t.app
        .post(
            "/sales-orders",
            &[
                ("party_id", "7"),
                ("channel", channel),
                ("payment_mode", mode),
            ],
        )
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("the order was entered");
    for (code, price, quantity) in [("tee", "50.00", "2"), ("cap", "100.00", "1")] {
        t.app
            .post(
                &format!("/sales-orders/{}/lines", order.id),
                &[
                    ("item_code", code),
                    ("unit_price", price),
                    ("quantity", quantity),
                ],
            )
            .await
            .assert_status(303);
    }
    reload(t, order.id).await
}

async fn reload(t: &Booted, id: i64) -> SalesOrder {
    SalesOrder::find_or_404(t.db(), id).await.unwrap()
}

/// A phone order submitted and `confirmed`.
async fn confirmed(t: &Booted) -> SalesOrder {
    let order = draft(t, "phone", "on_account").await;
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    reload(t, order.id).await
}

async fn lines(t: &Booted, order: &SalesOrder) -> Vec<SaleLine> {
    SaleLine::of(t.db(), &order.subject()).await.unwrap()
}

fn handover(n: i64) -> Subject {
    Subject::new("inventory.dispatch", n)
}

fn take(line: &SaleLine, quantity_milli: i64) -> FulfilledLine {
    FulfilledLine {
        line: line.id,
        quantity_milli,
    }
}

#[renox::test]
async fn an_order_is_entered_priced_and_confirmed_on_submit() {
    let (t, seen) = boot().await;
    t.app.acting_as(&seller(&t).await);
    let order = draft(&t, "phone", "on_account").await;
    assert!(order.number.starts_with("SO-"), "{}", order.number);
    assert_eq!(order.status, SalesOrderStatus::Draft);

    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    assert_eq!(
        (order.net_total, order.vat_total, order.gross_total),
        (20_000, 1_000, 21_000)
    );
    assert!(order.confirmed_at.is_some());
    let events = seen.confirmed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order, SalesOrderId(order.id));
    assert_eq!(events[0].party.0, 7);
    assert_eq!(events[0].channel, "phone");

    // A placed order's lines do not change.
    t.app
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[("item_code", "tee"), ("unit_price", "5.00")],
        )
        .await
        .assert_status(422);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see(&order.number)
        .assert_see("AED 210.00");
}

#[renox::test]
async fn an_order_without_lines_is_not_submitted() {
    let (t, _) = boot().await;
    t.app.acting_as(&seller(&t).await);
    t.app
        .post("/sales-orders", &[("party_id", "7")])
        .await
        .assert_status(303);
    let order = SalesOrder::query().first(t.db()).await.unwrap().unwrap();
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(422);
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Draft);
}

#[renox::test]
async fn a_website_order_waits_for_the_seller_who_accepts_or_rejects_it() {
    let (t, seen) = boot().await;
    t.app.acting_as(&seller(&t).await);
    let accepted = draft(&t, "website", "on_account").await;
    t.app
        .post(
            &format!("/sales-orders/{}/submit", accepted.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        reload(&t, accepted.id).await.status,
        SalesOrderStatus::PendingAcceptance
    );
    assert!(seen.confirmed.lock().unwrap().is_empty());
    t.app
        .post(
            &format!("/sales-orders/{}/transitions/accept", accepted.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        reload(&t, accepted.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert_eq!(seen.confirmed.lock().unwrap().len(), 1);

    let refused = draft(&t, "portal", "on_account").await;
    t.app
        .post(
            &format!("/sales-orders/{}/submit", refused.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    let url = format!("/sales-orders/{}/transitions/reject", refused.id);
    // A reason is required.
    t.app.post(&url, &[("reason", "")]).await.assert_status(422);
    t.app
        .post(&url, &[("reason", "Out of stock")])
        .await
        .assert_status(303);
    let refused = reload(&t, refused.id).await;
    assert_eq!(refused.status, SalesOrderStatus::Rejected);
    assert_eq!(refused.reason.as_deref(), Some("Out of stock"));
    let events = seen.rejected.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "Out of stock");
}

#[renox::test]
async fn an_online_order_waits_for_the_exact_payment_or_is_cancelled() {
    let (t, seen) = boot().await;
    t.app.acting_as(&seller(&t).await);
    let order = draft(&t, "phone", "online").await;
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::AwaitingPayment
    );
    // A different amount cannot confirm it.
    assert!(
        sales_orders::confirm_payment(t.state(), order.id, 5_000)
            .await
            .is_err()
    );
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::AwaitingPayment
    );
    let paid = sales_orders::confirm_payment(t.state(), order.id, 21_000)
        .await
        .unwrap();
    assert_eq!(paid.status, SalesOrderStatus::Confirmed);
    assert_eq!(seen.confirmed.lock().unwrap().len(), 1);

    // A payment that fails cancels the order and nothing reaches fulfilment.
    let failed = draft(&t, "phone", "online").await;
    t.app
        .post(&format!("/sales-orders/{}/submit", failed.id), &[("x", "")])
        .await
        .assert_status(303);
    let failed = sales_orders::payment_failed(t.state(), failed.id)
        .await
        .unwrap();
    assert_eq!(failed.status, SalesOrderStatus::Cancelled);
    assert_eq!(failed.reason.as_deref(), Some("The online payment failed."));
    assert_eq!(seen.cancelled.lock().unwrap().len(), 1);
}

#[renox::test]
async fn cancelling_needs_a_reason_and_an_order_in_a_state_it_leaves() {
    let (t, seen) = boot().await;
    t.app.acting_as(&seller(&t).await);
    let order = confirmed(&t).await;
    let url = format!("/sales-orders/{}/cancel", order.id);
    t.app
        .htmx()
        .post(&url, &[("reason", "")])
        .await
        .assert_status(422);
    t.app
        .post(&url, &[("reason", "Customer changed his mind")])
        .await
        .assert_status(303);
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Cancelled);
    assert_eq!(seen.cancelled.lock().unwrap().len(), 1);
    // Cancelled is final.
    t.app
        .post(&url, &[("reason", "again")])
        .await
        .assert_status(422);
}

#[renox::test]
async fn fulfilment_moves_the_order_through_partly_and_fully_fulfilled() {
    let (t, seen) = boot().await;
    t.app.acting_as(&seller(&t).await);
    let order = confirmed(&t).await;
    let items = lines(&t, &order).await;
    let ledger = order_fulfilment(t.state());
    let id = SalesOrderId(order.id);

    // One of the two tees: partly fulfilled, no invoice yet.
    let first = ledger
        .fulfil(
            t.state(),
            FulfilOrder {
                order: id,
                lines: vec![take(&items[0], 1000)],
                source: handover(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        first,
        FulfilmentOutcome::Fulfilled {
            order: id,
            complete: false
        }
    );
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PartiallyFulfilled);
    assert_eq!(order.invoice_id, None);
    assert_eq!(lines(&t, &order).await[0].fulfilled_milli, 1000);

    // The same hand-over again changes nothing.
    let again = ledger
        .fulfil(
            t.state(),
            FulfilOrder {
                order: id,
                lines: vec![take(&items[0], 1000)],
                source: handover(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(again, FulfilmentOutcome::Already { order: id });
    assert_eq!(lines(&t, &order).await[0].fulfilled_milli, 1000);

    // More than is open is refused.
    let too_many = ledger
        .fulfil(
            t.state(),
            FulfilOrder {
                order: id,
                lines: vec![take(&items[0], 2000)],
                source: handover(2),
            },
        )
        .await;
    assert!(too_many.is_err());

    // The rest: fulfilled, invoice drafted for the order, event with the
    // hand-over.
    let rest = ledger
        .fulfil(
            t.state(),
            FulfilOrder {
                order: id,
                lines: vec![take(&items[0], 1000), take(&items[1], 1000)],
                source: handover(3),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        rest,
        FulfilmentOutcome::Fulfilled {
            order: id,
            complete: true
        }
    );
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Fulfilled);
    assert!(order.fulfilled_at.is_some());
    assert_eq!(order.invoice_id, Some(9));
    let drafts = seen.drafts.lock().unwrap().clone();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].source, order.subject());
    assert_eq!(drafts[0].lines.len(), 2);
    let events = seen.fulfilled.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    assert!(!events[0].complete);
    assert!(events[1].complete);
    assert_eq!(events[1].source, handover(3));
    assert_eq!(events[1].lines.len(), 2);

    // Closing: the invoice exists, so the order closes.
    assert!(sales_orders::close(t.state(), order.id).await.unwrap());
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Closed);
    assert_eq!(seen.closed.lock().unwrap().len(), 1);
}

#[renox::test]
async fn a_reversed_hand_over_takes_its_quantities_off_and_a_new_one_fulfils_again() {
    let (t, seen) = boot().await;
    t.app.acting_as(&seller(&t).await);
    let order = confirmed(&t).await;
    let items = lines(&t, &order).await;
    let ledger = order_fulfilment(t.state());
    let id = SalesOrderId(order.id);
    ledger
        .fulfil(
            t.state(),
            FulfilOrder {
                order: id,
                lines: vec![take(&items[0], 2000), take(&items[1], 1000)],
                source: handover(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Fulfilled
    );

    // A hand-over nobody made cannot be reversed.
    let unknown = ledger
        .reverse(
            t.state(),
            ReverseFulfilment {
                order: id,
                lines: vec![take(&items[0], 1000)],
                source: handover(99),
                cause: "reversed".into(),
            },
        )
        .await;
    assert!(unknown.is_err());

    // Reversing one tee leaves the order partly fulfilled.
    let outcome = ledger
        .reverse(
            t.state(),
            ReverseFulfilment {
                order: id,
                lines: vec![take(&items[0], 1000)],
                source: handover(1),
                cause: "reversed".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(outcome, FulfilmentOutcome::Reversed { order: id });
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PartiallyFulfilled);
    assert_eq!(lines(&t, &order).await[0].fulfilled_milli, 1000);
    let events = seen.reversed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].cause, "reversed");
    assert_eq!(events[0].source, handover(1));

    // A tee more than the hand-over still holds is refused.
    let over = ledger
        .reverse(
            t.state(),
            ReverseFulfilment {
                order: id,
                lines: vec![take(&items[0], 2000)],
                source: handover(1),
                cause: "reversed".into(),
            },
        )
        .await;
    assert!(over.is_err());

    // Reversing the rest leaves nothing fulfilled: back to confirmed.
    ledger
        .reverse(
            t.state(),
            ReverseFulfilment {
                order: id,
                lines: vec![take(&items[0], 1000), take(&items[1], 1000)],
                source: handover(1),
                cause: "lost".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );

    // A new hand-over fulfils the order again.
    let again = ledger
        .fulfil(
            t.state(),
            FulfilOrder {
                order: id,
                lines: vec![take(&items[0], 2000), take(&items[1], 1000)],
                source: handover(2),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        again,
        FulfilmentOutcome::Fulfilled {
            order: id,
            complete: true
        }
    );
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Fulfilled
    );
    assert_eq!(seen.fulfilled.lock().unwrap().len(), 2);
}

#[renox::test]
async fn the_order_pages_need_their_permissions() {
    let (t, _) = boot().await;
    let nobody = t.user("Nina", &[]).await;
    t.app.acting_as(&nobody);
    t.app.get("/sales-orders").await.assert_forbidden();
    t.app
        .post("/sales-orders", &[("party_id", "7")])
        .await
        .assert_forbidden();
    let sam = seller(&t).await;
    t.app.acting_as(&sam);
    t.app
        .get("/sales-orders")
        .await
        .assert_ok()
        .assert_see("Sales orders");
    // An order that is not fulfilled has nothing to close.
    let order = confirmed(&t).await;
    assert!(!sales_orders::close(t.state(), order.id).await.unwrap());
}
