//! BL-SAL-02 (#121): an order paid online waits in `awaiting_payment`, invisible
//! to fulfilment, until the gateway confirms the full total; then it goes on to
//! acceptance or confirmation without a credit check. Rules: wl-sales §4.2
//! *Online payment*, rows `submit` and `confirm_payment`, §5.3 steps 1-3.
//! The orders are AED 105.00 (AED 100 before VAT) instead of the story's AED 85.

mod support;

use wl_kernel::payments::PaymentRecorder;
use wl_sales::{OrderPayment, OrderPaymentStatus, SalesOrder, SalesOrderStatus};

use support::online::{TOTAL, boot_online, draft_order, reload, seller, submit};

/// AC1: Given a website order paid by card, when submitted, it is
/// `awaiting_payment` and absent from the outlet's list.
#[renox::test]
async fn an_online_order_waits_for_payment_and_is_not_in_the_outlets_list() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "website", "online", &[]).await;
    let order = submit(&t, draft.id).await;

    assert_eq!(order.status, SalesOrderStatus::AwaitingPayment);
    assert_eq!(order.payment_status, OrderPaymentStatus::Pending);
    assert_eq!(order.gross_total, TOTAL);
    // Nothing reached fulfilment: no confirmation event.
    assert!(online.confirmed.lock().unwrap().is_empty());
    // The gateway was asked for a link of the total that ends with the window.
    let links = online.gateway.links();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].amount, TOTAL);
    assert_eq!(links[0].prepaid, order.subject());
    assert_eq!(Some(links[0].expires_at), order.payment_due_at);
    assert_eq!(order.payment_link.as_deref(), Some("/pay/link-1"));
    // The lists of fulfilment leave it out; the awaiting tab shows it.
    let visible = SalesOrder::visible_to_fulfilment(t.db(), 50).await.unwrap();
    assert!(visible.iter().all(|o| o.id != order.id));
    t.app
        .get("/sales-orders")
        .await
        .assert_ok()
        .assert_dont_see(&order.number);
    t.app
        .get("/sales-orders?awaiting=1")
        .await
        .assert_ok()
        .assert_see(&order.number);
    // The order page says why it is not placed yet.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Waiting for the gateway");
}

/// AC2: When `PaymentReceived` for the full total arrives, it becomes
/// `pending_acceptance` (website needs acceptance).
#[renox::test]
async fn the_full_payment_moves_a_website_order_to_acceptance() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "website", "online", &[]).await;
    let order = submit(&t, draft.id).await;

    t.state()
        .emit(online.gateway.received(&order, 41, TOTAL))
        .await
        .unwrap();

    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PendingAcceptance);
    assert_eq!(order.payment_status, OrderPaymentStatus::Received);
    assert!(online.confirmed.lock().unwrap().is_empty());
    // It now shows in the lists.
    let visible = SalesOrder::visible_to_fulfilment(t.db(), 50).await.unwrap();
    assert!(visible.iter().any(|o| o.id == order.id));
    let handled = OrderPayment::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(handled.len(), 1);
    assert_eq!(handled[0].outcome, "confirmed");
    assert!(online.gateway.refunds().is_empty());
}

/// AC3: Given a channel without acceptance, it becomes `confirmed` and
/// `SalesOrderConfirmed` is emitted (stock is reserved by the inventory module
/// that listens to it), without a credit check.
#[renox::test]
async fn the_full_payment_confirms_an_order_of_a_channel_without_acceptance() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "phone", "online", &[]).await;
    let order = submit(&t, draft.id).await;
    assert_eq!(order.status, SalesOrderStatus::AwaitingPayment);

    t.state()
        .emit(online.gateway.received(&order, 42, TOTAL))
        .await
        .unwrap();

    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    assert!(order.confirmed_at.is_some());
    let events = online.confirmed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order.0, order.id);
}

/// AC4: Given `PaymentReceived` of a smaller amount for the order, the order
/// stays `awaiting_payment` (and the payment is refunded, BL-SAL-04).
#[renox::test]
async fn a_payment_of_another_amount_does_not_confirm_the_order() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "phone", "online", &[]).await;
    let order = submit(&t, draft.id).await;

    t.state()
        .emit(online.gateway.received(&order, 43, TOTAL - 500))
        .await
        .unwrap();

    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::AwaitingPayment);
    assert_eq!(order.payment_status, OrderPaymentStatus::Pending);
    assert!(online.confirmed.lock().unwrap().is_empty());
}

/// AC5: A second `PaymentReceived` with the same payment id changes nothing.
#[renox::test]
async fn the_same_payment_delivered_twice_confirms_once() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "phone", "online", &[]).await;
    let order = submit(&t, draft.id).await;

    let event = online.gateway.received(&order, 44, TOTAL);
    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    assert_eq!(reload(&t, order.id).await.status, SalesOrderStatus::Confirmed);
    assert_eq!(online.confirmed.lock().unwrap().len(), 1);
    assert_eq!(OrderPayment::of_order(t.db(), order.id).await.unwrap().len(), 1);
    assert!(online.gateway.refunds().is_empty());
}

/// Edge: an order whose payment was confirmed before it was placed skips
/// `awaiting_payment`.
#[renox::test]
async fn an_order_paid_before_it_is_placed_is_not_held() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "phone", "online", &[]).await;
    // The gateway answered synchronously: the payment is already known.
    let _ = online.gateway.received(&draft, 45, TOTAL);
    assert_eq!(
        online
            .gateway
            .payments_for(t.db(), &draft.subject())
            .await
            .unwrap()
            .len(),
        1
    );

    let order = submit(&t, draft.id).await;

    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    assert_eq!(order.payment_status, OrderPaymentStatus::Received);
    assert!(online.gateway.links().is_empty());
}
