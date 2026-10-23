//! BL-FNB-16 (#380), 06 §4.2 row *Customer refuses one item at the door*,
//! §7.2 *Partial acceptance*, `SAL.ACL.06`: the rider records a line the
//! customer refused through the driver route `POST /driver/stops/{id}/deliver`
//! (the delivery is `delivered`, the order `completed` for the accepted
//! lines); on a prepaid order the refused value up to AED 50.00 is refunded at
//! once by a *no goods* return of the business, above it the kitchen managers
//! are told and nothing is refunded; on cash on delivery the rider collects
//! the accepted lines only. The modules are the real wl-delivery (partial
//! acceptance on, as the 06 client sets it), wl-sales, wl-payments (with its
//! cash custody) and wl-inventory.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::FnbOrderStatus;
use fnb::notices::STAFF_NOTICE;
use fnb::order::payment;
use renox::prelude::*;
use renox::testing::TestResponse;
use wl_delivery::close::cash_expected;
use wl_delivery::{Delivery, DeliveryStatus, Run};
use wl_kernel::custody::custody;
use wl_payments::refund::Refund;
use wl_sales::{ReturnKind, ReturnResolution, ReturnStatus, SalesOrder, SalesReturn};

use support::{World, boot_full, own_riders};

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// The app with the 06 delivery settings and partial acceptance on.
async fn world() -> World {
    boot_full(
        own_riders().partial_acceptance("fnb.order", true),
        fnb::Fnb::new(),
        wl_payments::WlPayments::new(),
    )
    .await
}

/// An order of `lines`, `dispatched`.
async fn on_the_road(
    w: &World,
    mode: &str,
    lines: &[(&str, &str, &str)],
) -> (fnb::FnbOrder, Delivery, Run) {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Mariam Al Hosani {n}")).await;
    w.dispatched(customer, mode, lines).await
}

/// The rider delivers the stop, refusing the whole line whose description
/// starts with `refused` (reason *wrong item*) and collecting `cash`.
async fn deliver_refusing(
    w: &World,
    delivery: &Delivery,
    refused: &str,
    cash: &str,
) -> TestResponse {
    let mut fields: Vec<(String, String)> = vec![
        ("recipient_name".into(), "Mariam Al Hosani".into()),
        ("photo".into(), "photo-0001".into()),
        ("signature".into(), "signature-0001".into()),
        ("latitude".into(), "25.204849".into()),
        ("longitude".into(), "55.270783".into()),
        ("cash_collected".into(), cash.into()),
    ];
    for line in delivery.lines(w.db()).await.unwrap() {
        let whole = format!(
            "{}.{:03}",
            line.quantity_milli / 1000,
            line.quantity_milli % 1000
        );
        if line.description.starts_with(refused) {
            fields.push((format!("delivered_{}", line.id), "0".into()));
            fields.push((format!("refused_{}", line.id), whole));
            fields.push((format!("reason_{}", line.id), "wrong_item".into()));
        } else {
            fields.push((format!("delivered_{}", line.id), whole));
            fields.push((format!("refused_{}", line.id), "0".into()));
        }
    }
    let borrowed: Vec<(&str, &str)> = fields
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
        .collect();
    w.post(
        &w.rider,
        &format!("/driver/stops/{}/deliver", delivery.id),
        &borrowed,
    )
    .await
}

/// The returns of `order`'s sales order.
async fn returns_of(w: &World, order: &fnb::FnbOrder) -> Vec<SalesReturn> {
    SalesReturn::query()
        .where_(SalesReturn::SALE_ID.eq(order.sales_order_id))
        .order_by(SalesReturn::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The refunds asked for `order`'s sales order.
async fn refunds_of(w: &World, order: &fnb::FnbOrder) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(order.sales_order_id))
        .where_(Refund::SOURCE_KIND.eq("order"))
        .order_by(Refund::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// AC (prepaid): given a prepaid order of a shawarma plate (AED 40.00) and a
/// falafel wrap (AED 12.00), when the customer refuses the plate at the door,
/// the delivery is `delivered`, the order `completed`, a *no goods* return of
/// AED 40.00 (party business, category wrong item) is resolved by a refund of
/// AED 40.00 without an approval, and the sales order shows AED 40.00 refunded.
#[renox::test]
async fn a_refused_line_of_a_prepaid_order_is_refunded_at_once() {
    let w = world().await;
    let lines = [
        ("shawarma-plate", "1", "40.00"),
        ("falafel-wrap", "1", "12.00"),
    ];
    let (order, delivery, _run) = on_the_road(&w, "online", &lines).await;

    deliver_refusing(&w, &delivery, "shawarma", "0")
        .await
        .assert_status(303);

    assert_eq!(w.delivery(&delivery).await.status, DeliveryStatus::Delivered);
    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Completed);
    let returns = returns_of(&w, &after).await;
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].kind, ReturnKind::NoGoods);
    assert_eq!(returns[0].status, ReturnStatus::Resolved);
    assert_eq!(returns[0].resolution, Some(ReturnResolution::Refund));
    assert_eq!(returns[0].gross_value, 4_000);
    assert_eq!(returns[0].responsible_party.as_deref(), Some("business"));
    assert_eq!(
        returns[0].complaint_category.as_deref(),
        Some(fnb::refused::CATEGORY)
    );
    let refunds = refunds_of(&w, &after).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 4_000);
    let sales = SalesOrder::find_or_404(w.db(), after.sales_order_id)
        .await
        .unwrap();
    assert_eq!(sales.refunded_amount, 4_000);
    // Nobody was asked to approve it.
    assert!(w.notices.users_of_type(STAFF_NOTICE).is_empty());
}

/// AC (cash on delivery): the same refusal on a cash order collects the
/// accepted line only: the rider's expected cash is AED 12.00, no return and
/// no refund exist.
#[renox::test]
async fn a_refused_line_of_a_cash_order_is_left_out_of_the_cash_the_rider_expects() {
    let w = world().await;
    let lines = [
        ("shawarma-plate", "1", "40.00"),
        ("falafel-wrap", "1", "12.00"),
    ];
    let (order, delivery, run) = on_the_road(&w, payment::CASH_ON_DELIVERY, &lines).await;

    deliver_refusing(&w, &delivery, "shawarma", "12.00")
        .await
        .assert_status(303);

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Completed);
    assert_eq!(cash_expected(w.db(), &run).await.unwrap(), 1_200);
    let handover = custody(w.state())
        .handover_of(w.db(), w.rider.id, &run.subject())
        .await
        .unwrap()
        .expect("the rider's handover of the run");
    assert_eq!(handover.expected, 1_200);
    assert!(returns_of(&w, &after).await.is_empty());
    assert!(refunds_of(&w, &after).await.is_empty());
}

/// AC (above the limit): a refused line of AED 60.00, above the AED 50.00 the
/// system refunds, is not refunded by the system: the order is `completed`,
/// no return or refund exists and the kitchen manager is told the amount that
/// waits.
#[renox::test]
async fn a_refused_line_above_fifty_dirhams_is_left_to_the_kitchen_manager() {
    let w = world().await;
    let lines = [
        ("shawarma-plate", "1", "60.00"),
        ("falafel-wrap", "1", "12.00"),
    ];
    let (order, delivery, _run) = on_the_road(&w, "online", &lines).await;

    deliver_refusing(&w, &delivery, "shawarma", "0")
        .await
        .assert_status(303);

    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::Completed);
    assert!(returns_of(&w, &after).await.is_empty());
    assert!(refunds_of(&w, &after).await.is_empty());
    let told = w.notices.of_type(STAFF_NOTICE);
    let waiting = told
        .iter()
        .find(|r| r.data["title"].as_str().is_some_and(|t| t.contains("refus")))
        .unwrap_or_else(|| panic!("the managers were not told: {told:?}"));
    assert!(
        waiting.data["body"].as_str().unwrap().contains("60.00"),
        "{:?}",
        waiting.data
    );
    assert!(w.notices.users_of_type(STAFF_NOTICE).contains(&w.manager.id));
}
