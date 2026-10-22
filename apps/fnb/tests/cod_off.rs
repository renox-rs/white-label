//! BL-FNB-03 (#367), 06 §4.2 row *Own-rider delivery fails*, §7.2
//! `FNB.RUL.14`: after N = 2 customer-side failures of cash-on-delivery orders
//! within D = 90 days, cash on delivery is turned off for the customer (online
//! payment only) and the kitchen manager is told and can turn it back on;
//! failures before it was last turned back on are not counted; outlet-side
//! failures and prepaid orders never count. The failures are real: the rider
//! records them in the driver PWA.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::order::reason;
use fnb::{CustomerFlag, Fnb, FnbOrder, FnbOrderStatus};
use renox::chrono::TimeDelta;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;

use support::{World, boot_own_riders, boot_with, own_riders};

const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "85.00")];

static FIXTURES: AtomicI64 = AtomicI64::new(1);

/// A failed cash order of `customer` from `days_ago` days ago, on the
/// customer's side: an order the rule counts, put in `delivery_failed` the way
/// an import or an earlier month would have left it.
async fn past_failure(w: &World, customer: PartyId, days_ago: i64) {
    let n = FIXTURES.fetch_add(1, Ordering::SeqCst);
    let mut order = FnbOrder {
        number: format!("SO-FIX-{n}"),
        sales_order_id: 9_000 + n,
        party_id: customer.0,
        channel: "website".to_owned(),
        kind: "delivery".to_owned(),
        payment_mode: "cash_on_delivery".to_owned(),
        total: 8_500,
        failure_reason: Some(reason::CUSTOMER_UNREACHABLE.to_owned()),
        delivery_reason: Some("not_home".to_owned()),
        failure_side: Some("customer".to_owned()),
        failure_attempt: 1,
        failed_at: Some(renox::db::now() - TimeDelta::days(days_ago)),
        responsible_party: Some("customer".to_owned()),
        status: FnbOrderStatus::DeliveryFailed,
        ..FnbOrder::default()
    };
    let mut tx = w.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, FnbOrderStatus::DeliveryFailed, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

/// A cash order of `customer` through `channel`, dispatched and failed by the
/// rider with `code`.
async fn failure(w: &World, channel: &str, customer: PartyId, mode: &str, code: &str) -> FnbOrder {
    let (order, delivery, _run) = w.dispatched_via(channel, customer, mode, &PLATE).await;
    w.fail_stop(&delivery, code).await.assert_status(303);
    let after = w.reload(&order).await;
    assert_eq!(after.status, FnbOrderStatus::DeliveryFailed);
    after
}

async fn flag_of(w: &World, customer: PartyId) -> Option<CustomerFlag> {
    CustomerFlag::of(w.db(), customer).await.unwrap()
}

async fn cod_allowed(w: &World, customer: PartyId) -> bool {
    fnb::cod::payment_options(w.db(), customer)
        .await
        .unwrap()
        .cash_on_delivery
}

/// Whether the kitchen manager was told that cash on delivery went off.
fn manager_told(w: &World) -> bool {
    w.notices
        .of_type(fnb::notices::STAFF_NOTICE)
        .iter()
        .any(|r| {
            r.data["title"]
                .as_str()
                .is_some_and(|title| title.contains("Cash on delivery is off"))
        })
        && w.notices
            .users_of_type(fnb::notices::STAFF_NOTICE)
            .contains(&w.manager.id)
}

/// AC1: customer C with a failed cash order 70 days ago (unreachable) and
/// another now (refused, cause other): COD is off for C and the kitchen
/// manager is notified.
#[renox::test]
async fn a_second_customer_side_failure_within_90_days_turns_cash_on_delivery_off() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    past_failure(&w, c, 70).await;
    assert!(cod_allowed(&w, c).await);

    let order = failure(&w, "website", c, "cash_on_delivery", "refused").await;

    assert_eq!(order.failure_reason.as_deref(), Some(reason::REFUSED));
    let flag = flag_of(&w, c).await.expect("a flag was made");
    assert!(!flag.cod_allowed);
    assert_eq!(flag.cod_off_order_id, Some(order.id));
    assert!(!cod_allowed(&w, c).await);
    assert!(manager_told(&w), "{:?}", w.notices.all());
}

/// AC2: failures 100 days apart: COD stays on.
#[renox::test]
async fn failures_more_than_90_days_apart_leave_cash_on_delivery_on() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    past_failure(&w, c, 100).await;

    failure(&w, "website", c, "cash_on_delivery", "refused").await;

    assert!(cod_allowed(&w, c).await);
    assert!(flag_of(&w, c).await.is_none());
    assert!(!manager_told(&w));
}

/// AC3: after the manager turns COD on, one new failure does not turn it off;
/// the earlier failures are not counted any more, and the next failure does.
#[renox::test]
async fn after_the_manager_turns_it_on_one_new_failure_does_not_turn_it_off() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    past_failure(&w, c, 20).await;
    failure(&w, "website", c, "cash_on_delivery", "no_cash").await;
    assert!(!cod_allowed(&w, c).await);

    w.post(&w.manager, &format!("/customers/{}/cod", c.0), &[("x", "")])
        .await
        .assert_status(303);

    let flag = flag_of(&w, c).await.unwrap();
    assert!(flag.cod_allowed);
    assert_eq!(flag.cod_on_by, Some(w.manager.id));
    assert!(flag.cod_on_at.is_some());
    assert!(cod_allowed(&w, c).await);

    // One new failure: the two before are not counted.
    failure(&w, "website", c, "cash_on_delivery", "not_home").await;
    assert!(cod_allowed(&w, c).await, "one failure since it was turned on");

    // The second since it was turned on turns it off again.
    failure(&w, "website", c, "cash_on_delivery", "wrong_address").await;
    assert!(!cod_allowed(&w, c).await);
}

/// AC4: with COD off, C's checkout shows only online payment, with the line
/// "Online payment only for this order"; with it on, both.
#[renox::test]
async fn with_cash_on_delivery_off_the_checkout_offers_only_online_payment() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    let other = w.customer("Customer D").await;
    past_failure(&w, c, 30).await;
    failure(&w, "website", c, "cash_on_delivery", "refused").await;
    let path = format!("/checkout/payment?party={}", c.0);

    w.get(&w.manager, &path)
        .await
        .assert_ok()
        .assert_see("Pay online")
        .assert_see("Online payment only for this order")
        .assert_dont_see("Pay cash on delivery");
    // Another customer still sees both.
    w.get(&w.manager, &format!("/checkout/payment?party={}", other.0))
        .await
        .assert_ok()
        .assert_see("Pay online")
        .assert_see("Pay cash on delivery")
        .assert_dont_see("Online payment only for this order");
}

/// The customer is never blocked from ordering: a new order is still placed,
/// and it is placed prepaid.
#[renox::test]
async fn the_customer_can_still_order_with_cash_on_delivery_off() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    past_failure(&w, c, 30).await;
    failure(&w, "website", c, "cash_on_delivery", "refused").await;
    assert!(!cod_allowed(&w, c).await);

    let again = w.place(c, "online", &PLATE).await;

    assert_eq!(again.status, FnbOrderStatus::Placed);
    assert_eq!(again.paid_amount, 8_500);
}

/// AC5: a D2C failure (unreachable) and a phone order failure (no cash) for
/// the same customer within 90 days turn COD off for that customer.
#[renox::test]
async fn a_direct_order_and_a_phone_order_of_the_same_customer_count_together() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;

    failure(&w, "website", c, "cash_on_delivery", "not_home").await;
    assert!(cod_allowed(&w, c).await, "one failure so far");
    let phone = failure(&w, "phone", c, "cash_on_delivery", "no_cash").await;

    assert_eq!(phone.channel, "phone");
    assert!(!cod_allowed(&w, c).await);
    assert!(manager_told(&w));
}

/// Outlet-side failures (refused because late, a rider issue) and prepaid
/// orders never count.
#[renox::test]
async fn outlet_side_failures_and_prepaid_orders_do_not_count() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;

    failure(&w, "website", c, "cash_on_delivery", "refused_wrong_item").await;
    failure(&w, "website", c, "cash_on_delivery", "rider_issue").await;
    failure(&w, "website", c, "online", "not_home").await;
    failure(&w, "website", c, "online", "no_cash").await;

    assert!(cod_allowed(&w, c).await);
    assert!(flag_of(&w, c).await.is_none());
    assert!(!manager_told(&w));
    // One customer-side cash failure on top of that is still only the first.
    failure(&w, "website", c, "cash_on_delivery", "not_home").await;
    assert!(cod_allowed(&w, c).await);
}

/// The count is per customer: another customer's failures do not add up.
#[renox::test]
async fn failures_of_different_customers_are_counted_apart() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    let d = w.customer("Customer D").await;

    failure(&w, "website", c, "cash_on_delivery", "not_home").await;
    failure(&w, "website", d, "cash_on_delivery", "not_home").await;

    assert!(cod_allowed(&w, c).await);
    assert!(cod_allowed(&w, d).await);
}

/// N and D are settings (`FNB.RUL.14`), and the rule can be switched off.
#[renox::test]
async fn the_failures_and_the_days_are_settings_and_the_rule_can_be_off() {
    let three = boot_with(own_riders(), Fnb::new().cod_failures(3, 90)).await;
    let c = three.customer("Customer C").await;
    past_failure(&three, c, 5).await;
    failure(&three, "website", c, "cash_on_delivery", "refused").await;
    assert!(cod_allowed(&three, c).await, "two of three");

    let off = boot_with(own_riders(), Fnb::new().cod_rule(false)).await;
    let c = off.customer("Customer C").await;
    past_failure(&off, c, 5).await;
    failure(&off, "website", c, "cash_on_delivery", "refused").await;
    assert!(cod_allowed(&off, c).await);
}

/// The kitchen manager sees who is without COD and turns it on; others cannot,
/// and it cannot be turned on twice.
#[renox::test]
async fn only_the_kitchen_manager_turns_cash_on_delivery_back_on() {
    let w = boot_own_riders().await;
    let c = w.customer("Customer C").await;
    past_failure(&w, c, 5).await;
    failure(&w, "website", c, "cash_on_delivery", "refused").await;
    let path = format!("/customers/{}/cod", c.0);

    w.get(&w.manager, "/customers/cod")
        .await
        .assert_ok()
        .assert_see("Customer C")
        .assert_see("Turn cash on delivery on");
    w.get(&w.cashier, "/customers/cod").await.assert_forbidden();
    w.post(&w.cashier, &path, &[("x", "")]).await.assert_forbidden();
    assert!(!cod_allowed(&w, c).await);

    w.post(&w.manager, &path, &[("x", "")])
        .await
        .assert_status(303);
    assert!(cod_allowed(&w, c).await);
    w.post(&w.manager, &path, &[("x", "")])
        .await
        .assert_status(422);
}
