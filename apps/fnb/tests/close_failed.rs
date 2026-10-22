//! BL-FNB-02 (#366), 06 §3.3 `FNB.ACL.16` and `FNB.ACL.17`, §4.2 row
//! *Own-rider delivery fails*, §7.1 *Order* row `close_failed`, open item L-33:
//! when the food of a failed delivery is back at the outlet or was discarded,
//! the order is closed as `cancelled` with the refund decision of the rule and
//! a responsible party. An outlet-side failure of a prepaid order is refunded
//! in full at once; a customer-side one is not, unless the kitchen manager
//! approves a refund the cashier asked for within 7 days (never the
//! requester); the food is logged as wastage once, never returned to
//! sellable stock. The modules are the real wl-delivery, wl-sales,
//! wl-payments and wl-inventory.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::FnbOrderStatus;
use fnb::order::{decision, party, reason};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_delivery::DeliveryStatus;
use wl_inventory::MovementType;
use wl_kernel::delivery::DeliveryReturned;
use wl_payments::refund::{Refund, RefundStatus};
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{World, boot_own_riders, boot_with, own_riders};

/// One shawarma plate at AED 85.00 (ORD-501).
const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "85.00")];

/// Two shawarma plates at AED 30.00: AED 60.00 (ORD-504).
const TWO_PLATES: [(&str, &str, &str); 1] = [("shawarma-plate", "2", "30.00")];

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// A prepaid order `dispatched` with `lines`, for a customer of its own.
async fn dispatched(
    w: &World,
    mode: &str,
    lines: &[(&str, &str, &str)],
) -> (fnb::FnbOrder, wl_delivery::Delivery, wl_delivery::Run) {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    let customer = w.customer(&format!("Mariam Al Hosani {n}")).await;
    w.dispatched(customer, mode, lines).await
}

/// The refunds asked for `order`'s sales order, oldest first.
async fn refunds_of(w: &World, order: &fnb::FnbOrder) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(order.sales_order_id))
        .where_(Refund::SOURCE_KIND.eq("order"))
        .order_by(Refund::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The wastage movements of `item`.
async fn wastage_of(w: &World, item: &wl_inventory::Item) -> Vec<wl_inventory::StockMovement> {
    w.movements_of(item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Wastage)
        .collect()
}

/// An order `delivery_failed` for `code` and returned to the outlet: closed.
async fn failed_and_returned(
    w: &World,
    mode: &str,
    lines: &[(&str, &str, &str)],
    code: &str,
) -> fnb::FnbOrder {
    let (order, delivery, run) = dispatched(w, mode, lines).await;
    w.fail_stop(&delivery, code).await.assert_status(303);
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::DeliveryFailed);
    w.bring_back(&delivery, &run).await;
    w.reload(&order).await
}

/// AC1: given prepaid ORD-501 (AED 85.00) `delivery_failed` with *run
/// aborted* and the food discarded, on `DeliveryDiscarded` the order is
/// `cancelled`, AED 85.00 is refunded at once, the responsible party is
/// *business*, and exactly one wastage movement exists (one per ingredient).
#[renox::test]
async fn a_discarded_run_aborted_order_is_closed_with_a_full_refund_and_one_wastage() {
    let w = boot_own_riders().await;
    let (order, delivery, run) = dispatched(&w, "online", &PLATE).await;
    w.abort(&run, "breakdown").await.assert_ok();
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::DeliveryFailed);

    w.discard(&delivery).await.assert_status(303);

    let closed = w.reload(&order).await;
    assert_eq!(closed.status, FnbOrderStatus::Cancelled);
    assert_eq!(closed.refund_decision.as_deref(), Some(decision::FULL_REFUND));
    assert_eq!(closed.refund_amount, 8_500);
    assert_eq!(closed.responsible_party.as_deref(), Some(party::BUSINESS));
    assert_eq!(closed.failure_reason.as_deref(), Some(reason::RUN_ABORTED));
    assert_eq!(w.delivery(&delivery).await.status, DeliveryStatus::Discarded);
    // AED 85.00 is asked of wl-payments at once, from the order.
    let refunds = refunds_of(&w, &closed).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 8_500);
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(refunds[0].requested_by, None, "the system asked");
    assert_eq!(closed.refund_id, Some(refunds[0].id));
    // The sales order is cancelled and the payment is the refund's alone.
    let sales = SalesOrder::find_or_404(w.db(), closed.sales_order_id).await.unwrap();
    assert_eq!(sales.status, SalesOrderStatus::Cancelled);
    assert_eq!(sales.reason.as_deref(), Some("failed delivery"));
    // The discard posted the wastage; the cancellation posted no second one.
    assert!(closed.wastage_posted);
    assert!(closed.wastage_ref.is_some());
    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 1);
    assert_eq!(wastage_of(&w, &w.rice).await.len(), 1);
}

/// AC2: given prepaid ORD-504 (AED 60.00) failed with customer unreachable and
/// returned, on `DeliveryReturned` the order is `cancelled`, no refund is made,
/// the responsible party is *customer*, and the food is posted as wastage with
/// the reason *failed delivery*.
#[renox::test]
async fn a_returned_customer_side_order_is_closed_with_no_refund_and_the_food_as_wastage() {
    let w = boot_own_riders().await;
    let before = (w.on_hand(&w.chicken).await, w.on_hand(&w.rice).await);

    let closed = failed_and_returned(&w, "online", &TWO_PLATES, "not_home").await;

    assert_eq!(closed.status, FnbOrderStatus::Cancelled);
    assert_eq!(closed.total, 6_000);
    assert_eq!(closed.refund_decision.as_deref(), Some(decision::NO_REFUND));
    assert_eq!(closed.refund_amount, 0);
    assert_eq!(closed.refund_id, None);
    assert_eq!(closed.responsible_party.as_deref(), Some(party::CUSTOMER));
    assert!(refunds_of(&w, &closed).await.is_empty());
    let sales = SalesOrder::find_or_404(w.db(), closed.sales_order_id).await.unwrap();
    assert_eq!(sales.status, SalesOrderStatus::Cancelled);
    // The food is wastage, reason *failed delivery*, with the order as
    // reference: 2 plates use 0.5 kg of chicken and 0.3 kg of rice.
    let chicken = wastage_of(&w, &w.chicken).await;
    assert_eq!(chicken.len(), 1);
    assert_eq!(chicken[0].reason.as_deref(), Some("failed delivery"));
    assert_eq!(chicken[0].quantity_milli, 500);
    assert_eq!(chicken[0].source_type, "sales.sales_order");
    assert_eq!(chicken[0].source_id, closed.sales_order_id);
    let rice = wastage_of(&w, &w.rice).await;
    assert_eq!(rice.len(), 1);
    assert_eq!(rice[0].quantity_milli, 300);
    // Nothing was taken from the outlet other than the wastage.
    assert_eq!(w.on_hand(&w.chicken).await, before.0 - 500);
    assert_eq!(w.on_hand(&w.rice).await, before.1 - 300);
}

/// AC5: for ORD-504 the only stock movement of its items is the wastage:
/// no receipt or return-to-stock movement exists, so returned food is never
/// sellable stock again (`FNB.ACL.16`).
#[renox::test]
async fn returned_food_never_comes_back_as_stock_the_only_new_movement_is_the_wastage() {
    let w = boot_own_riders().await;
    let chicken_before = w.movements_of(&w.chicken).await.len();
    let rice_before = w.movements_of(&w.rice).await.len();

    failed_and_returned(&w, "online", &TWO_PLATES, "not_home").await;

    for (item, before) in [(&w.chicken, chicken_before), (&w.rice, rice_before)] {
        let after = w.movements_of(item).await;
        assert_eq!(after.len(), before + 1, "one new movement of {}", item.code);
        let added = &after[before..];
        assert!(
            added.iter().all(|m| m.movement_type == MovementType::Wastage),
            "{added:?}"
        );
    }
}

/// The wastage is posted once: the same event again changes nothing.
#[renox::test]
async fn a_repeated_return_event_posts_no_second_wastage_and_no_second_refund() {
    let w = boot_own_riders().await;
    let (order, delivery, run) = dispatched(&w, "online", &PLATE).await;
    w.fail_stop(&delivery, "rider_issue").await.assert_status(303);
    w.bring_back(&delivery, &run).await;
    let closed = w.reload(&order).await;
    assert_eq!(closed.status, FnbOrderStatus::Cancelled);

    w.state()
        .emit(DeliveryReturned {
            delivery: delivery.delivery_id(),
            source: delivery.source(),
            reason: "rider_issue".to_owned(),
        })
        .await
        .unwrap();

    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 1);
    assert_eq!(refunds_of(&w, &closed).await.len(), 1);
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Cancelled);
}

/// AC3: cashier A requests an AED 30.00 refund on ORD-504; A cannot approve
/// it; kitchen manager B approves and AED 30.00 is refunded (the refund of
/// wl-payments is approved for AED 30.00 and the order records the decision).
#[renox::test]
async fn the_cashier_asks_for_a_refund_and_a_kitchen_manager_approves_it_never_the_requester() {
    let w = boot_own_riders().await;
    let closed = failed_and_returned(&w, "online", &TWO_PLATES, "not_home").await;
    let path = format!("/orders/{}/refund", closed.id);

    w.post(&w.cashier, &path, &[("amount", "30.00"), ("reason", "The customer called later")])
        .await
        .assert_status(303);

    let asked = refunds_of(&w, &closed).await;
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].amount, 3_000);
    assert_eq!(asked[0].status, RefundStatus::Requested);
    assert_eq!(asked[0].requested_by, Some(w.cashier.id));
    let approve = format!("/refunds/{}/transitions/approve", asked[0].id);
    // A cashier does not hold the approval at all.
    w.post(&w.cashier, &approve, &[("x", "")])
        .await
        .assert_forbidden();
    assert_eq!(refunds_of(&w, &closed).await[0].status, RefundStatus::Requested);

    // The kitchen manager approves.
    w.post(&w.manager_b, &approve, &[("x", "")])
        .await
        .assert_status(303);

    let approved = refunds_of(&w, &closed).await;
    assert_eq!(approved[0].status, RefundStatus::Approved);
    assert_eq!(approved[0].approved_by, Some(w.manager_b.id));
    assert_eq!(approved[0].amount, 3_000);
    let order = w.reload(&closed).await;
    assert_eq!(order.refund_decision.as_deref(), Some(decision::MANAGER_APPROVED));
    assert_eq!(order.refund_amount, 3_000);
    let ask = fnb::RefundAsk::of_order(w.db(), order.id).await.unwrap().remove(0);
    assert_eq!(ask.approved_by, Some(w.manager_b.id));
    assert!(ask.approved_at.is_some());
    // The order page lists it.
    w.get(&w.manager, &format!("/orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Manager-approved refund");
}

/// Whoever requested a refund cannot approve it, even holding both permissions
/// (`FIN.ACL.04`).
#[renox::test]
async fn a_user_who_holds_both_permissions_still_cannot_approve_their_own_request() {
    let w = boot_own_riders().await;
    let closed = failed_and_returned(&w, "online", &TWO_PLATES, "wrong_address").await;

    w.post(
        &w.lead_cashier,
        &format!("/orders/{}/refund", closed.id),
        &[("amount", "60.00"), ("reason", "Goodwill")],
    )
    .await
    .assert_status(303);
    let ask = refunds_of(&w, &closed).await.remove(0);

    w.post(
        &w.lead_cashier,
        &format!("/refunds/{}/transitions/approve", ask.id),
        &[("x", "")],
    )
    .await
    .assert_status(422);
    assert_eq!(refunds_of(&w, &closed).await[0].status, RefundStatus::Requested);
    assert_eq!(w.reload(&closed).await.refund_decision.as_deref(), Some(decision::NO_REFUND));
}

/// AC3 (window): a refund is asked within 7 days of the cancellation, and one
/// after the window is refused with "refund request window closed". Never
/// more than what was paid.
#[renox::test]
async fn a_refund_is_asked_within_seven_days_and_refused_after_and_never_above_what_was_paid() {
    let w = boot_own_riders().await;
    let closed = failed_and_returned(&w, "online", &TWO_PLATES, "not_home").await;
    let path = format!("/orders/{}/refund", closed.id);

    // More than the AED 60.00 paid, nothing, or no reason is refused.
    w.post(&w.cashier, &path, &[("amount", "60.01"), ("reason", "x")])
        .await
        .assert_status(422);
    w.post(&w.cashier, &path, &[("amount", "0"), ("reason", "x")])
        .await
        .assert_status(422);
    w.post(&w.cashier, &path, &[("amount", "10.00"), ("reason", " ")])
        .await
        .assert_status(422);
    assert!(refunds_of(&w, &closed).await.is_empty());

    // 6 days 23 hours after the cancellation: still inside the window.
    w.travel(TimeDelta::days(6) + TimeDelta::hours(23));
    w.post(&w.cashier, &path, &[("amount", "10.00"), ("reason", "Late apology")])
        .await
        .assert_status(303);

    // 7 days 1 hour after: refused.
    w.travel(TimeDelta::hours(2));
    w.post(&w.cashier, &path, &[("amount", "10.00"), ("reason", "Too late")])
        .await
        .assert_status(422)
        .assert_see("refund request window closed");
    assert_eq!(refunds_of(&w, &closed).await.len(), 1);
}

/// Only a failure on the customer's side with nothing refunded is asked for;
/// the cashier who is not given the permission cannot ask.
#[renox::test]
async fn a_refund_is_not_asked_for_an_order_refunded_in_full_or_by_someone_without_the_permission() {
    let w = boot_own_riders().await;
    let refunded = failed_and_returned(&w, "online", &PLATE, "rider_issue").await;
    assert_eq!(refunded.refund_decision.as_deref(), Some(decision::FULL_REFUND));
    let path = format!("/orders/{}/refund", refunded.id);

    w.post(&w.cashier, &path, &[("amount", "10.00"), ("reason", "More")])
        .await
        .assert_status(422);
    w.post(&w.cook, &path, &[("amount", "10.00"), ("reason", "More")])
        .await
        .assert_forbidden();
    assert_eq!(refunds_of(&w, &refunded).await.len(), 1, "only the full refund");
}

/// AC4: given *refused* with cause *wrong item*, the refund is full and the
/// party is *business*.
#[renox::test]
async fn refused_for_a_wrong_item_is_refunded_in_full_and_the_business_is_responsible() {
    let w = boot_own_riders().await;

    let closed = failed_and_returned(&w, "online", &PLATE, "refused_wrong_item").await;

    assert_eq!(closed.status, FnbOrderStatus::Cancelled);
    assert_eq!(closed.failure_reason.as_deref(), Some(reason::REFUSED_WRONG_ITEM));
    assert_eq!(closed.refund_decision.as_deref(), Some(decision::FULL_REFUND));
    assert_eq!(closed.refund_amount, 8_500);
    assert_eq!(closed.responsible_party.as_deref(), Some(party::BUSINESS));
    assert_eq!(refunds_of(&w, &closed).await[0].amount, 8_500);
}

/// *Refused* with cause *late* is the outlet's too; with cause *other* it is
/// the customer's and nothing is refunded.
#[renox::test]
async fn refused_late_is_refunded_and_refused_for_another_cause_is_not() {
    let w = boot_own_riders().await;

    let late = failed_and_returned(&w, "online", &PLATE, "refused_late").await;
    let other = failed_and_returned(&w, "online", &PLATE, "refused").await;

    assert_eq!(late.refund_decision.as_deref(), Some(decision::FULL_REFUND));
    assert_eq!(other.refund_decision.as_deref(), Some(decision::NO_REFUND));
    assert_eq!(other.responsible_party.as_deref(), Some(party::CUSTOMER));
    assert!(refunds_of(&w, &other).await.is_empty());
}

/// Cash on delivery collected nothing, so the decision is *not applicable*
/// and no refund is made, whichever side failed; the food is still wastage.
#[renox::test]
async fn a_cash_order_is_closed_with_not_applicable_and_no_refund() {
    let w = boot_own_riders().await;

    let closed = failed_and_returned(&w, "cash_on_delivery", &PLATE, "rider_issue").await;

    assert_eq!(closed.status, FnbOrderStatus::Cancelled);
    assert_eq!(closed.refund_decision.as_deref(), Some(decision::NOT_APPLICABLE));
    assert_eq!(closed.refund_amount, 0);
    assert!(refunds_of(&w, &closed).await.is_empty());
    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 1);
}

/// L-33 is open: the customer-side default is *no refund*, and it is a setting.
/// With the setting on, a prepaid order failed on the customer's side is
/// refunded in full.
#[renox::test]
async fn the_customer_side_default_is_a_setting_not_a_decision_in_code() {
    let w = boot_with(own_riders(), fnb::Fnb::new().customer_side_refund(true)).await;

    let closed = failed_and_returned(&w, "online", &PLATE, "not_home").await;

    assert_eq!(closed.refund_decision.as_deref(), Some(decision::FULL_REFUND));
    assert_eq!(closed.refund_amount, 8_500);
    assert_eq!(refunds_of(&w, &closed).await.len(), 1);
    // The party is still the customer's.
    assert_eq!(closed.responsible_party.as_deref(), Some(party::CUSTOMER));
}

/// The kitchen manager may close a failed delivery by hand with the proposed
/// decision; a cashier may not; and `close_failed` asks for no `SAL.ACL.05`
/// approval.
#[renox::test]
async fn a_manager_closes_a_failed_delivery_by_hand_without_a_second_approval() {
    let w = boot_own_riders().await;
    let (order, delivery, _run) = dispatched(&w, "online", &PLATE).await;
    w.fail_stop(&delivery, "no_cash").await.assert_status(303);
    let path = format!("/orders/{}/close", order.id);

    w.post(&w.cashier, &path, &[("x", "")]).await.assert_forbidden();
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::DeliveryFailed);
    w.get(&w.manager, &format!("/orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Close the failed delivery");

    w.post(&w.manager, &path, &[("x", "")])
        .await
        .assert_status(303);

    let closed = w.reload(&order).await;
    assert_eq!(closed.status, FnbOrderStatus::Cancelled);
    assert_eq!(closed.refund_decision.as_deref(), Some(decision::NO_REFUND));
    assert_eq!(
        SalesOrder::find_or_404(w.db(), closed.sales_order_id)
            .await
            .unwrap()
            .status,
        SalesOrderStatus::Cancelled
    );
    assert!(
        wl_kernel::approvals::ApprovalRequest::pending_about(
            w.db(),
            &wl_kernel::sales::SalesOrderId(closed.sales_order_id).subject()
        )
        .await
        .unwrap()
        .is_empty(),
        "no approval was asked"
    );
}
