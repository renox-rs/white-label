//! BL-SAL-03 (#122): an order whose online payment failed, or did not arrive
//! within the payment window (15 minutes), is cancelled as *payment failed*;
//! its promo code use is given back and the customer is told to place a new
//! order. Rules: wl-sales §4.2 row `payment_failed`, §5.3 step 2, §7 *Online
//! payment window*; the job is `sales:online-payment-window`.

mod support;

use renox::chrono::TimeDelta;
use renox::prelude::Model;
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyRecipient;
use wl_sales::notices::ORDER_PAYMENT_FAILED;
use wl_sales::online_payment::lapse_due;
use wl_sales::sales_orders;
use wl_sales::{
    Campaign, MarketingChannel, OrderPaymentStatus, PromoCode, Purpose, SalesOrder,
    SalesOrderStatus,
};

use support::Booted;
use support::online::{TOTAL, boot_online, draft_order, reload, seller, submit};

/// A website order paid online, submitted and waiting.
async fn waiting(t: &Booted, extra: &[(&str, &str)]) -> SalesOrder {
    let draft = draft_order(t, "website", "online", extra).await;
    submit(t, draft.id).await
}

/// AC1: Given an order submitted at 12:00:00 with no confirmation, at 12:15:00
/// it is `cancelled` with reason *payment failed* and `SalesOrderCancelled` is
/// emitted.
#[renox::test]
async fn an_order_without_confirmation_is_cancelled_when_the_window_ends() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t, &[]).await;
    let due = order.payment_due_at.expect("the window was stamped");

    // A second before the window ends nothing happens.
    let early = lapse_due(t.state(), due - TimeDelta::seconds(1))
        .await
        .unwrap();
    assert_eq!(early, 0);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::AwaitingPayment
    );
    assert!(online.cancelled.lock().unwrap().is_empty());

    // The window ends.
    let lapsed = lapse_due(t.state(), due).await.unwrap();
    assert_eq!(lapsed, 1);
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Cancelled);
    assert_eq!(order.payment_status, OrderPaymentStatus::Failed);
    assert!(
        order
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("payment failed"),
        "{:?}",
        order.reason
    );
    let events = online.cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order.0, order.id);
    // The link is withdrawn so the customer cannot pay the dead order.
    assert_eq!(online.gateway.closed_links().len(), 1);
    // Nothing reached fulfilment.
    assert!(online.confirmed.lock().unwrap().is_empty());
}

/// AC2: Given `PaymentFailed` at 12:02, it is cancelled at 12:02; the timer
/// later does nothing.
#[renox::test]
async fn a_failed_payment_cancels_at_once_and_the_timer_later_does_nothing() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t, &[]).await;
    let due = order.payment_due_at.unwrap();

    t.state()
        .emit(online.gateway.failed(&order, 51, TOTAL))
        .await
        .unwrap();
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Cancelled
    );
    assert_eq!(online.cancelled.lock().unwrap().len(), 1);

    let later = lapse_due(t.state(), due + TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(later, 0);
    assert_eq!(online.cancelled.lock().unwrap().len(), 1);
    assert_eq!(online.mail.of_type(ORDER_PAYMENT_FAILED).len(), 1);
}

/// Edge: the payment confirmed first wins; the timer finds the order placed.
#[renox::test]
async fn a_confirmed_order_is_not_cancelled_by_the_timer() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let draft = draft_order(&t, "phone", "online", &[]).await;
    let order = submit(&t, draft.id).await;
    let due = order.payment_due_at.unwrap();
    t.state()
        .emit(online.gateway.received(&order, 52, TOTAL))
        .await
        .unwrap();

    let lapsed = lapse_due(t.state(), due + TimeDelta::hours(1))
        .await
        .unwrap();

    assert_eq!(lapsed, 0);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert!(online.cancelled.lock().unwrap().is_empty());
}

/// A promo code SUMMER that may be used once, made through a campaign.
async fn summer(t: &Booted) {
    let campaign = Campaign {
        name: "Summer".into(),
        code: "SUMMER".into(),
        purpose: Some(Purpose::Acquisition),
        marketing_channel: Some(MarketingChannel::Meta),
        promo_code: Some("SUMMER".into()),
        ..Campaign::default()
    };
    assert!(campaign.store(t.db()).await.unwrap().is_ok());
    PromoCode::query()
        .where_(PromoCode::CODE.eq("SUMMER"))
        .update(t.db(), &[("max_uses", &Some(1_i64))])
        .await
        .unwrap();
}

async fn uses(t: &Booted) -> i64 {
    PromoCode::query()
        .where_(PromoCode::CODE.eq("SUMMER"))
        .first(t.db())
        .await
        .unwrap()
        .unwrap()
        .uses
}

/// AC3: Given promo code SUMMER (max 1 use) used on the failed order, the same
/// party can use it on a new order.
#[renox::test]
async fn the_promo_code_use_of_a_failed_order_is_available_again() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    summer(&t).await;

    let first = waiting(&t, &[("promo_code", "summer")]).await;
    assert_eq!(
        uses(&t).await,
        1,
        "the use is taken when the order is placed"
    );
    // While the order waits the code is used up for another order.
    let blocked = draft_order(&t, "website", "online", &[("promo_code", "SUMMER")]).await;
    t.app
        .post(
            &format!("/sales-orders/{}/submit", blocked.id),
            &[("x", "")],
        )
        .await
        .assert_status(422);

    t.state()
        .emit(online.gateway.failed(&first, 53, TOTAL))
        .await
        .unwrap();
    assert_eq!(
        uses(&t).await,
        0,
        "the failed order does not consume the code"
    );

    let again = submit(&t, blocked.id).await;
    assert_eq!(again.status, SalesOrderStatus::AwaitingPayment);
    assert_eq!(uses(&t).await, 1);
}

/// AC4: The customer receives a "payment failed, please try again" message.
#[renox::test]
async fn the_customer_is_told_the_payment_failed() {
    let (t, online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t, &[]).await;
    lapse_due(t.state(), order.payment_due_at.unwrap())
        .await
        .unwrap();

    let sent = online.mail.of_type(ORDER_PAYMENT_FAILED);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, order.subject());
    assert!(
        sent[0]
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == PartyId(7)))
    );
    assert_eq!(sent[0].data["number"], order.number.as_str());
}

/// Edge: the cancelled order is terminal; a retry is a new order.
#[renox::test]
async fn a_failed_order_cannot_be_paid_again() {
    let (t, _online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t, &[]).await;
    lapse_due(t.state(), order.payment_due_at.unwrap())
        .await
        .unwrap();

    assert!(
        sales_orders::confirm_payment(t.state(), order.id, TOTAL)
            .await
            .is_err()
    );
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Cancelled
    );
}

/// Edge: the job's own screen: the order page shows it cancelled with the reason.
#[renox::test]
async fn the_order_page_shows_the_failed_payment() {
    let (t, _online) = boot_online().await;
    t.app.acting_as(&seller(&t).await);
    let order = waiting(&t, &[]).await;
    lapse_due(t.state(), order.payment_due_at.unwrap())
        .await
        .unwrap();
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Cancelled")
        .assert_see("Failed");
}
