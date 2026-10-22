//! BL-SAL-04 (#123): `SAL.RUL.06`. A payment confirmed for an order that cannot
//! take it (`cancelled`, `rejected`, already paid, or `awaiting_payment` with
//! another amount) is refunded in full through `wl-payments` with the reason
//! *late payment*, once per payment id, the customer and the manager role are
//! told, and the order never changes state (wl-sales §4.2, §6).

mod support;

use renox::chrono::TimeDelta;
use wl_kernel::payments::{PaymentId, RefundSource};
use wl_sales::notices::ORDER_LATE_PAYMENT;
use wl_sales::online_payment::lapse_due;
use wl_sales::{OrderPayment, SalesOrder, SalesOrderStatus};

use support::Booted;
use support::online::{Online, TOTAL, boot_online, draft_order, reload, seller, submit};

/// A website order waiting for its online payment.
async fn waiting(t: &Booted) -> SalesOrder {
    let draft = draft_order(t, "website", "online", &[]).await;
    submit(t, draft.id).await
}

/// What the module told wl-payments and the people, checked for one late
/// payment `id` of `amount`.
fn assert_refunded(online: &Online, order: &SalesOrder, id: i64, amount: i64) {
    let refunds = online.gateway.refunds();
    assert_eq!(refunds.len(), 1, "one refund request");
    let (request, by) = &refunds[0];
    assert_eq!(request.amount, amount);
    assert_eq!(request.reason, "late payment");
    assert_eq!(request.source, RefundSource::Payment(PaymentId(id)));
    assert_eq!(request.party.0, 7);
    assert_eq!(request.currency, "AED");
    assert_eq!(*by, None, "asked by the system, not a person");
    // The customer is told, and the manager role gets a task.
    let told = online.mail.of_type(ORDER_LATE_PAYMENT);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].subject, order.subject());
    let tasks = online.desk.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(format!("sales.late_payment:{id}").as_str())
    );
    assert!(tasks[0].title.contains(&order.number), "{}", tasks[0].title);
}

/// AC1: Given an order `cancelled` by timeout, when `PaymentReceived` arrives,
/// then a refund request of the amount, reason *late payment*, exists, the
/// customer and outlet manager are notified, and the order stays `cancelled`.
#[renox::test]
async fn a_payment_after_the_timeout_is_refunded_and_the_order_stays_cancelled() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t).await;
    lapse_due(t.state(), order.payment_due_at.unwrap() + TimeDelta::seconds(1))
        .await
        .unwrap();
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Cancelled);

    t.state()
        .emit(online.gateway.received(&order, 61, TOTAL))
        .await
        .unwrap();

    assert_refunded(&online, &order, 61, TOTAL);
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Cancelled);
    assert!(online.confirmed.lock().unwrap().is_empty());
    let handled = OrderPayment::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(handled.len(), 1);
    assert_eq!(handled[0].outcome, "refund_requested");
    assert_eq!(handled[0].refund_id, Some(1));
    // The order page lists the payment and its refund.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Refund requested");
}

/// AC2: Given an order `rejected`, the same.
#[renox::test]
async fn a_payment_for_a_rejected_order_is_refunded() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t).await;
    t.state()
        .emit(online.gateway.received(&order, 62, TOTAL))
        .await
        .unwrap();
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PendingAcceptance
    );
    t.app
        .post(
            &format!("/sales-orders/{}/transitions/reject", order.id),
            &[("reason", "Kitchen closed")],
        )
        .await
        .assert_status(303);
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Rejected);
    assert!(online.gateway.refunds().is_empty());

    // The payment that arrives after the rejection goes back.
    t.state()
        .emit(online.gateway.received(&order, 63, TOTAL))
        .await
        .unwrap();

    assert_refunded(&online, &order, 63, TOTAL);
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Rejected);
}

/// AC3: Given an order already paid, a second payment of the same amount is
/// refunded in full.
#[renox::test]
async fn a_second_payment_of_a_paid_order_is_refunded_in_full() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "phone", "online", &[]).await;
    let order = submit(&t, draft.id).await;
    t.state()
        .emit(online.gateway.received(&order, 64, TOTAL))
        .await
        .unwrap();
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Confirmed);

    t.state()
        .emit(online.gateway.received(&order, 65, TOTAL))
        .await
        .unwrap();

    assert_refunded(&online, &order, 65, TOTAL);
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Confirmed);
    assert_eq!(online.confirmed.lock().unwrap().len(), 1);
}

/// AC4: Given an order `awaiting_payment`, when `PaymentReceived` of a smaller
/// amount arrives, a refund request of that amount exists at once and the
/// order stays `awaiting_payment` (it times out at the window end).
#[renox::test]
async fn a_payment_of_another_amount_is_refunded_and_the_order_still_times_out() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t).await;

    t.state()
        .emit(online.gateway.received(&order, 66, TOTAL - 500))
        .await
        .unwrap();

    assert_refunded(&online, &order, 66, TOTAL - 500);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::AwaitingPayment
    );
    // Nothing needs to run again after the timeout.
    lapse_due(t.state(), order.payment_due_at.unwrap()).await.unwrap();
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Cancelled);
    assert_eq!(online.gateway.refunds().len(), 1);
}

/// AC5: The same `PaymentReceived` delivered twice creates one refund request.
#[renox::test]
async fn the_same_late_payment_delivered_twice_is_refunded_once() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t).await;
    lapse_due(t.state(), order.payment_due_at.unwrap()).await.unwrap();

    let event = online.gateway.received(&order, 67, TOTAL);
    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    assert_refunded(&online, &order, 67, TOTAL);
    assert_eq!(OrderPayment::of_order(t.db(), order.id).await.unwrap().len(), 1);
}

/// Edge: a refund wl-payments refuses is kept as such, and the manager is told
/// to refund it by hand; the order is untouched.
#[renox::test]
async fn a_refused_refund_is_left_to_the_manager() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t).await;
    lapse_due(t.state(), order.payment_due_at.unwrap()).await.unwrap();
    online.gateway.refuse_refunds(true);

    t.state()
        .emit(online.gateway.received(&order, 68, TOTAL))
        .await
        .unwrap();

    assert!(online.gateway.refunds().is_empty());
    let handled = OrderPayment::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(handled[0].outcome, "refund_refused");
    let tasks = online.desk.opened();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].description.contains("could not be requested"));
    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Cancelled);
}

/// Edge: a payment that is not for a sales order of this module changes
/// nothing and is not refunded by it.
#[renox::test]
async fn a_payment_for_no_order_is_left_alone() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t).await;
    let mut stray = online.gateway.received(&order, 69, TOTAL);
    // Made for something else: not among the payments made for the order.
    stray.payment = PaymentId(999);

    t.state().emit(stray).await.unwrap();

    assert!(online.gateway.refunds().is_empty());
    assert!(OrderPayment::of_order(t.db(), order.id).await.unwrap().is_empty());
}
