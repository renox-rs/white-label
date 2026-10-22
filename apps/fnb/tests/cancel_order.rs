//! BL-FNB-04 (#368), 06 §3.3 `SAL.ACL.01` and `SAL.ACL.05`, §4.2 row
//! *Customer cancels before the outlet accepts*, §7.1 *Order* row `cancel`
//! and *Kitchen ticket* row `void`: a customer cancels a `placed` direct order
//! from the order page (full refund, nothing in the kitchen); after acceptance
//! only staff cancel, with a reason and a responsible party; a paid order
//! above AED 50, or one the kitchen started, waits for a manager, and each
//! started line needs an outcome; an order entered by mistake is the
//! business's; an aggregator order cancelled with the aggregator responsible
//! issues no refund and expects compensation. The modules are the real
//! wl-sales, wl-payments, wl-inventory and wl-delivery.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::order::{cancel_reason, decision, party};
use fnb::{FnbOrder, FnbOrderStatus, KitchenTicket, TicketStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::MovementType;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE;
use wl_payments::refund::Refund;
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{World, boot_own_riders};

/// One falafel wrap at AED 40.00 (ORD-601).
const WRAP: [(&str, &str, &str); 1] = [("falafel-wrap", "1", "40.00")];

/// One shawarma plate at AED 120.00 (ORD-602).
const BIG_PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "120.00")];

/// One shawarma plate at AED 45.00 (ORD-603), below the limit.
const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "45.00")];

/// One hummus bowl at AED 30.00, below the AED 50.00 limit.
const BOWL: [(&str, &str, &str); 1] = [("hummus-bowl", "1", "30.00")];

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

/// A customer of its own.
async fn customer(w: &World) -> wl_kernel::PartyId {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    w.customer(&format!("Mariam Al Hosani {n}")).await
}

/// The refunds asked for `order`'s sales order, oldest first.
async fn refunds_of(w: &World, order: &FnbOrder) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(order.sales_order_id))
        .where_(Refund::SOURCE_KIND.eq("order"))
        .order_by(Refund::ID)
        .get(w.db())
        .await
        .unwrap()
}

/// The requests that wait about the sales order of `order`.
async fn waiting_for(w: &World, order: &FnbOrder) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(
        w.db(),
        &Subject::new(SALES_ORDER_SUBJECT_TYPE, order.sales_order_id),
    )
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

/// The cook starts every ticket of `order`: it is `preparing`.
async fn start_cooking(w: &World, order: &FnbOrder) -> FnbOrder {
    for ticket in KitchenTicket::of_order(w.db(), order.id).await.unwrap() {
        w.post(
            &w.cook,
            &format!("/tickets/{}/start", ticket.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    }
    let after = w.reload(order).await;
    assert_eq!(after.status, FnbOrderStatus::Preparing);
    after
}

/// The cancel form of the cashier.
fn ask<'a>(reason: &'a str, party: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![("reason", reason), ("party", party), ("note", "")]
}

/// AC1: given prepaid ORD-601 (AED 40.00) `placed`, when the customer cancels
/// from the order page, it is `cancelled`, AED 40.00 is refunded, and no
/// kitchen ticket exists.
#[renox::test]
async fn a_customer_cancels_a_placed_prepaid_order_and_is_refunded_in_full() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let user = w.customer_user("Mariam", c).await;
    let placed = w.place(c, "online", &WRAP).await;

    // The page offers the button.
    let page = w.get(&user, &format!("/my/orders/{}", placed.id)).await;
    page.assert_ok();
    assert!(page.text().contains("Cancel my order"));

    w.post(
        &user,
        &format!("/my/orders/{}/cancel", placed.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    let cancelled = w.reload(&placed).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert!(cancelled.cancelled_by_customer);
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::CUSTOMER));
    assert_eq!(
        cancelled.cancel_reason_code.as_deref(),
        Some(cancel_reason::CUSTOMER_REQUEST)
    );
    assert_eq!(
        cancelled.refund_decision.as_deref(),
        Some(decision::FULL_REFUND)
    );
    assert_eq!(cancelled.refund_amount, 4_000);
    assert!(!cancelled.compensation_expected);
    // AED 40.00 is asked of wl-payments from the order, for the customer.
    let refunds = refunds_of(&w, &cancelled).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 4_000);
    assert_eq!(refunds[0].requested_by, Some(user.id));
    // Nothing reached the kitchen.
    assert!(
        KitchenTicket::of_order(w.db(), cancelled.id)
            .await
            .unwrap()
            .is_empty()
    );
    let sales = SalesOrder::find_or_404(w.db(), cancelled.sales_order_id)
        .await
        .unwrap();
    assert_eq!(sales.status, SalesOrderStatus::Cancelled);
}

/// A customer sees and cancels only their own order.
#[renox::test]
async fn a_customer_cannot_see_or_cancel_the_order_of_another() {
    let w = boot_own_riders().await;
    let mine = customer(&w).await;
    let theirs = customer(&w).await;
    let stranger = w.customer_user("Stranger", theirs).await;
    let placed = w.place(mine, "online", &WRAP).await;

    w.get(&stranger, &format!("/my/orders/{}", placed.id))
        .await
        .assert_status(404);
    w.post(
        &stranger,
        &format!("/my/orders/{}/cancel", placed.id),
        &[("x", "")],
    )
    .await
    .assert_status(404);
    assert_eq!(w.reload(&placed).await.status, FnbOrderStatus::Placed);
    // Someone who acts for nobody is turned away too.
    w.post(
        &w.nobody,
        &format!("/my/orders/{}/cancel", placed.id),
        &[("x", "")],
    )
    .await
    .assert_status(404);
}

/// AC2: given ORD-601 accepted a second earlier, the customer cancel is
/// refused with "order already accepted", and the page says so.
#[renox::test]
async fn once_the_outlet_accepted_the_order_the_customer_cannot_cancel_it() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let user = w.customer_user("Mariam", c).await;
    let placed = w.place(c, "online", &WRAP).await;
    w.accept(&placed).await;

    let answer = w
        .post(
            &user,
            &format!("/my/orders/{}/cancel", placed.id),
            &[("x", "")],
        )
        .await;
    answer.assert_status(422);
    assert!(answer.text().contains("already accepted"));

    let after = w.reload(&placed).await;
    assert_eq!(after.status, FnbOrderStatus::Accepted);
    assert!(refunds_of(&w, &after).await.is_empty());
    // The page refreshed shows no button and says why.
    let page = w.get(&user, &format!("/my/orders/{}", placed.id)).await;
    page.assert_ok();
    assert!(!page.text().contains("Cancel my order"));
    assert!(page.text().contains("can no longer be cancelled"));
}

/// The race the other way: the customer was quicker, so accepting is refused
/// and the order stays cancelled (whichever runs first wins).
#[renox::test]
async fn a_cancelled_order_cannot_be_accepted() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let user = w.customer_user("Mariam", c).await;
    let placed = w.place(c, "online", &WRAP).await;
    w.post(
        &user,
        &format!("/my/orders/{}/cancel", placed.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    w.post(
        &w.manager,
        &format!("/orders/{}/accept", placed.id),
        &[("x", "")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&placed).await.status, FnbOrderStatus::Cancelled);
    // A second cancel says it is cancelled already.
    let again = w
        .post(
            &user,
            &format!("/my/orders/{}/cancel", placed.id),
            &[("x", "")],
        )
        .await;
    again.assert_status(422);
    assert!(again.text().contains("already cancelled"));
    assert_eq!(refunds_of(&w, &placed).await.len(), 1, "refunded once");
}

/// AC3: given accepted ORD-602 (AED 120.00) not started, cashier A's cancel
/// creates an approval request (above AED 50); the order stays `accepted`
/// until a manager who is not the requester approves, then it is cancelled
/// with the refund, and its tickets are voided.
#[renox::test]
async fn a_paid_order_above_fifty_dirhams_waits_for_a_manager_and_then_is_refunded() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "online", &BIG_PLATE).await;
    let accepted = w.accept(&placed).await;
    let path = format!("/orders/{}/cancel", accepted.id);

    w.post(&w.cashier, &path, &ask("customer_request", "customer"))
        .await
        .assert_status(303);

    // Nothing changed yet.
    let waiting = w.reload(&accepted).await;
    assert_eq!(waiting.status, FnbOrderStatus::Accepted);
    let requests = waiting_for(&w, &accepted).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "sales.order_cancel");
    assert_eq!(requests[0].requester_id, w.cashier.id);
    assert_eq!(waiting.cancel_request_id, Some(requests[0].id));
    assert!(refunds_of(&w, &accepted).await.is_empty());
    // The page says a request waits instead of offering the button again.
    let page = w.get(&w.cashier, &format!("/orders/{}", accepted.id)).await;
    page.assert_ok();
    assert!(page.text().contains("already waits"));
    w.post(&w.cashier, &path, &ask("customer_request", "customer"))
        .await
        .assert_status(422);

    // The requester cannot approve; a manager does.
    assert!(
        approvals::approve(w.state(), requests[0].id, &w.cashier, "")
            .await
            .is_err()
    );
    approvals::approve(w.state(), requests[0].id, &w.manager, "")
        .await
        .unwrap();

    let cancelled = w.reload(&accepted).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::CUSTOMER));
    assert_eq!(cancelled.cancelled_by, Some(w.cashier.id));
    assert!(!cancelled.cancelled_by_customer);
    assert_eq!(cancelled.refund_amount, 12_000);
    let refunds = refunds_of(&w, &cancelled).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 12_000);
    for ticket in KitchenTicket::of_order(w.db(), cancelled.id).await.unwrap() {
        assert_eq!(ticket.status, TicketStatus::Voided);
    }
}

/// A small order nobody paid for is cancelled by the cashier at once: no
/// approval, nothing to refund.
#[renox::test]
async fn a_small_unpaid_order_is_cancelled_by_the_cashier_at_once() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "cash_on_delivery", &BOWL).await;
    let accepted = w.accept(&placed).await;

    w.post(
        &w.cashier,
        &format!("/orders/{}/cancel", accepted.id),
        &ask("customer_request", "customer"),
    )
    .await
    .assert_status(303);

    let cancelled = w.reload(&accepted).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert!(waiting_for(&w, &cancelled).await.is_empty());
    assert!(refunds_of(&w, &cancelled).await.is_empty());
    assert_eq!(
        cancelled.refund_decision.as_deref(),
        Some(decision::NOT_APPLICABLE)
    );
    for ticket in KitchenTicket::of_order(w.db(), cancelled.id).await.unwrap() {
        assert_eq!(ticket.status, TicketStatus::Voided);
    }
}

/// AC4: given ORD-603 `preparing` with a ticket `in_prep`, cancel requires an
/// outcome per started line and manager approval; with *scrap*, the food is
/// posted as wastage and the tickets are `voided`.
#[renox::test]
async fn cancelling_a_started_order_needs_an_outcome_and_a_manager_and_scraps_the_food() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "online", &PLATE).await;
    let accepted = w.accept(&placed).await;
    let preparing = start_cooking(&w, &accepted).await;
    let ticket = KitchenTicket::of_order(w.db(), preparing.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(ticket.status, TicketStatus::InPrep);
    let path = format!("/orders/{}/cancel", preparing.id);

    // The page asks for the outcome of the started line.
    let page = w
        .get(&w.cashier, &format!("/orders/{}", preparing.id))
        .await;
    page.assert_ok();
    assert!(page.text().contains("outcome_"));

    // No outcome: refused, nothing waits.
    let missing = w
        .post(&w.cashier, &path, &ask("kitchen_issue", "business"))
        .await;
    missing.assert_status(422);
    assert!(missing.text().contains("started"));
    assert!(waiting_for(&w, &preparing).await.is_empty());
    // A wrong outcome is refused too.
    let outcome = format!("outcome_{}", ticket.sale_line_id);
    w.post(
        &w.cashier,
        &path,
        &[
            ("reason", "kitchen_issue"),
            ("party", "business"),
            ("note", ""),
            (outcome.as_str(), "finish_to_stock"),
        ],
    )
    .await
    .assert_status(422);

    // With scrap it waits for a manager.
    w.post(
        &w.cashier,
        &path,
        &[
            ("reason", "kitchen_issue"),
            ("party", "business"),
            ("note", ""),
            (outcome.as_str(), "scrap"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&preparing).await.status, FnbOrderStatus::Preparing);
    let requests = waiting_for(&w, &preparing).await;
    assert_eq!(requests.len(), 1);
    assert!(wastage_of(&w, &w.chicken).await.is_empty());

    approvals::approve(w.state(), requests[0].id, &w.manager, "")
        .await
        .unwrap();

    let cancelled = w.reload(&preparing).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::BUSINESS));
    assert!(cancelled.wastage_posted);
    // The food is posted as wastage, once per ingredient.
    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 1);
    assert_eq!(wastage_of(&w, &w.rice).await.len(), 1);
    // The ticket is voided; the customer is refunded.
    let after = KitchenTicket::of_order(w.db(), cancelled.id).await.unwrap();
    assert!(after.iter().all(|t| t.status == TicketStatus::Voided));
    assert_eq!(refunds_of(&w, &cancelled).await[0].amount, 4_500);
}

/// A bumped ticket stays `done`: its food follows the line's outcome.
#[renox::test]
async fn a_done_ticket_stays_done_when_the_order_is_cancelled() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "cash_on_delivery", &BOWL).await;
    let ready = w.cook(&w.accept(&placed).await).await;
    let ticket = KitchenTicket::of_order(w.db(), ready.id)
        .await
        .unwrap()
        .remove(0);
    let outcome = format!("outcome_{}", ticket.sale_line_id);

    // The kitchen manager's own cancel of a started line waits for the other
    // kitchen manager: the requester never approves (`SAL.ACL.05`).
    w.post(
        &w.manager,
        &format!("/orders/{}/cancel", ready.id),
        &[
            ("reason", "kitchen_issue"),
            ("party", "business"),
            ("note", ""),
            (outcome.as_str(), "scrap"),
        ],
    )
    .await
    .assert_status(303);
    let request = waiting_for(&w, &ready).await.remove(0);
    approvals::approve(w.state(), request.id, &w.manager_b, "")
        .await
        .unwrap();

    assert_eq!(w.reload(&ready).await.status, FnbOrderStatus::Cancelled);
    let after = KitchenTicket::of_order(w.db(), ready.id).await.unwrap();
    assert!(after.iter().all(|t| t.status == TicketStatus::Done));
}

/// A ticket that starts while a cancellation waits withdraws the request: it
/// was asked on a kitchen that had not started the line (`SAL.ACL.05`).
#[renox::test]
async fn a_ticket_that_starts_withdraws_a_waiting_cancellation() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "online", &BIG_PLATE).await;
    let accepted = w.accept(&placed).await;
    w.post(
        &w.cashier,
        &format!("/orders/{}/cancel", accepted.id),
        &ask("customer_request", "customer"),
    )
    .await
    .assert_status(303);
    let request = waiting_for(&w, &accepted).await.remove(0);

    start_cooking(&w, &accepted).await;

    assert!(waiting_for(&w, &accepted).await.is_empty());
    let ended = ApprovalRequest::find_or_404(w.db(), request.id)
        .await
        .unwrap();
    assert_eq!(ended.status, ApprovalStatus::Withdrawn);
    let now = w.reload(&accepted).await;
    assert_eq!(now.status, FnbOrderStatus::Preparing);
    assert_eq!(now.cancel_request_id, None);
    // Asked again, the started line needs its outcome.
    w.post(
        &w.cashier,
        &format!("/orders/{}/cancel", accepted.id),
        &ask("customer_request", "customer"),
    )
    .await
    .assert_status(422);
}

/// AC5: cancelling with reason *entered in error* records the responsible
/// party *business* (not editable for that reason); no `void` action exists
/// on orders.
#[renox::test]
async fn an_order_entered_in_error_is_the_businesss_and_orders_are_never_voided() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "cash_on_delivery", &BOWL).await;
    let accepted = w.accept(&placed).await;

    w.post(
        &w.cashier,
        &format!("/orders/{}/cancel", accepted.id),
        &ask("entered_in_error", "customer"),
    )
    .await
    .assert_status(303);

    let cancelled = w.reload(&accepted).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason_code.as_deref(),
        Some(cancel_reason::ENTERED_IN_ERROR)
    );
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::BUSINESS));
    // There is no `void` on the order's machine and none is offered.
    assert!(FnbOrder::machine().find("void").is_err());
    let other = w.place(c, "cash_on_delivery", &BOWL).await;
    assert!(
        w.try_run(&w.manager, &other, "void")
            .await
            .starts_with("error")
    );
    assert!(
        !w.offered(&w.manager, &other)
            .await
            .iter()
            .any(|name| name == "void")
    );
}

/// AC6: an aggregator order cancelled with the party *aggregator* is flagged
/// "compensation expected" and issues no refund.
#[renox::test]
async fn an_aggregator_order_cancelled_by_the_aggregator_expects_compensation_and_no_refund() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w
        .place_via("aggregator_a", c, "on_account", &BIG_PLATE)
        .await;
    assert_eq!(order.status, FnbOrderStatus::Accepted);

    // Only an aggregator's order can blame the aggregator.
    let direct = w.place(c, "cash_on_delivery", &BOWL).await;
    let direct = w.accept(&direct).await;
    w.post(
        &w.cashier,
        &format!("/orders/{}/cancel", direct.id),
        &[
            ("reason", "other"),
            ("party", "aggregator"),
            ("note", "no rider"),
        ],
    )
    .await
    .assert_status(422);

    w.post(
        &w.cashier,
        &format!("/orders/{}/cancel", order.id),
        &[
            ("reason", "other"),
            ("party", "aggregator"),
            ("note", "The rider never came"),
        ],
    )
    .await
    .assert_status(303);

    let cancelled = w.reload(&order).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::AGGREGATOR));
    assert!(cancelled.compensation_expected);
    assert!(refunds_of(&w, &cancelled).await.is_empty());
    assert_eq!(cancelled.refund_amount, 0);
    for ticket in KitchenTicket::of_order(w.db(), cancelled.id).await.unwrap() {
        assert_eq!(ticket.status, TicketStatus::Voided);
    }
    // The page shows the flag.
    let page = w
        .get(&w.manager, &format!("/orders/{}", cancelled.id))
        .await;
    page.assert_ok();
    assert!(page.text().contains("Compensation expected"));
}

/// Who may cancel, and what: the cook and a user with no permission cannot; a
/// note is needed for the reason *other*; an order out of the door is too late;
/// an unknown reason or party is refused.
#[renox::test]
async fn only_the_holders_of_orders_cancel_cancel_and_the_form_is_checked() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "cash_on_delivery", &BOWL).await;
    let accepted = w.accept(&placed).await;
    let path = format!("/orders/{}/cancel", accepted.id);

    w.post(&w.cook, &path, &ask("customer_request", "customer"))
        .await
        .assert_forbidden();
    w.post(&w.nobody, &path, &ask("customer_request", "customer"))
        .await
        .assert_forbidden();
    w.post(
        &w.cashier,
        &path,
        &[("reason", "other"), ("party", "business"), ("note", " ")],
    )
    .await
    .assert_status(422);
    w.post(&w.cashier, &path, &ask("because", "business"))
        .await
        .assert_status(422);
    w.post(&w.cashier, &path, &ask("customer_request", "rider"))
        .await
        .assert_status(422);
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Accepted);

    // Out of the door it is too late.
    let (out, _delivery, _run) = w.dispatched(c, "cash_on_delivery", &BOWL).await;
    let late = w
        .post(
            &w.cashier,
            &format!("/orders/{}/cancel", out.id),
            &ask("customer_request", "customer"),
        )
        .await;
    late.assert_status(422);
    assert!(late.text().contains("out of the door"));
}

/// A cancellation the sales module's own screen makes is followed too: the
/// order is cancelled as the business's, with the sales order's reason.
#[renox::test]
async fn a_cancellation_made_in_the_sales_module_is_followed_by_the_order() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "cash_on_delivery", &BOWL).await;
    let accepted = w.accept(&placed).await;
    let path = format!("/sales-orders/{}/cancel", accepted.sales_order_id);

    w.post(&w.manager, &path, &[("reason", "The owner asked")])
        .await
        .assert_status(303);

    let cancelled = w.reload(&accepted).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::BUSINESS));
    assert_eq!(cancelled.close_reason.as_deref(), Some("The owner asked"));
}
