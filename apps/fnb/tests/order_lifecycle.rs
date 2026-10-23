//! The base of the F&B order (06 §7.1 *Order* and *Kitchen ticket*): a direct
//! order is placed, accepted, cooked, put on a run, delivered and completed; an
//! aggregator or phone order starts accepted; the kitchen's tickets move the
//! order; the sales order follows (confirmed, then fulfilled). The kitchen
//! tells the sales module which lines it started, and the stock module what a
//! line is made of. The base for BL-FNB-01 to BL-FNB-03.

mod support;

use fnb::{FnbOrderStatus, KitchenTicket, TicketStatus};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::sales::{SalesOrderId, fulfilment_progress};
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{World, boot_own_riders};

const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "2", "30.00")];

async fn customer(w: &World) -> PartyId {
    w.customer("Mariam Al Hosani").await
}

/// A direct order paid online waits `placed`; the kitchen manager accepts it
/// through the sales order; tickets are made; cooking them makes it ready; the
/// delivery is requested within the brand's promise; the run leaves and the
/// order is dispatched; the rider delivers and it is completed, and the sales
/// order is fulfilled.
#[renox::test]
async fn a_direct_order_goes_from_placed_to_completed_with_its_sales_order() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;

    let placed = w.place(c, "online", &PLATE).await;
    assert_eq!(placed.status, FnbOrderStatus::Placed);
    assert_eq!(placed.total, 6_000);
    assert_eq!(placed.paid_amount, 6_000);
    assert_eq!(
        SalesOrder::find_or_404(w.db(), placed.sales_order_id)
            .await
            .unwrap()
            .status,
        SalesOrderStatus::PendingAcceptance
    );

    let accepted = w.accept(&placed).await;
    assert!(accepted.accepted_at.is_some());
    assert_eq!(
        SalesOrder::find_or_404(w.db(), placed.sales_order_id)
            .await
            .unwrap()
            .status,
        SalesOrderStatus::Confirmed
    );
    let tickets = KitchenTicket::of_order(w.db(), accepted.id).await.unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].status, TicketStatus::Queued);
    assert_eq!(tickets[0].quantity_milli, 2_000);

    // The first ticket started starts the order.
    w.post(
        &w.cook,
        &format!("/tickets/{}/start", tickets[0].id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&accepted).await.status, FnbOrderStatus::Preparing);
    // The last one bumped makes it ready, and asks for the delivery.
    w.post(
        &w.cook,
        &format!("/tickets/{}/bump", tickets[0].id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let ready = w.reload(&accepted).await;
    assert_eq!(ready.status, FnbOrderStatus::Ready);
    let delivery = w.delivery_of(&ready).await;
    assert!(delivery.perishable, "cooked food is perishable");
    assert_eq!(delivery.max_attempts, 1);
    assert_eq!(delivery.promise_minutes, Some(45));
    assert_eq!(delivery.amount_to_collect, 0, "prepaid: nothing to collect");

    let run = w.run_of(&w.rider).await;
    let out = w.send_out(&ready, &run).await;
    assert_eq!(out.rider_user_id, Some(w.rider.id));
    assert_eq!(out.run_id, Some(run.id));

    w.deliver_stop(&delivery, "0").await.assert_status(303);

    let done = w.reload(&out).await;
    assert_eq!(done.status, FnbOrderStatus::Completed);
    assert!(done.completed_at.is_some());
    assert_eq!(
        SalesOrder::find_or_404(w.db(), done.sales_order_id)
            .await
            .unwrap()
            .status,
        SalesOrderStatus::Fulfilled
    );
}

/// An order from a channel that needs no acceptance (the phone, an
/// aggregator's tablet) starts `accepted`, with its tickets.
#[renox::test]
async fn a_phone_order_starts_accepted_with_its_kitchen_tickets() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;

    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;

    assert_eq!(order.status, FnbOrderStatus::Accepted);
    assert_eq!(order.payment_mode, "cash_on_delivery");
    assert_eq!(order.paid_amount, 0);
    assert_eq!(
        KitchenTicket::of_order(w.db(), order.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Bumping straight from the queue starts the order and makes it ready.
#[renox::test]
async fn bumping_a_queued_ticket_starts_and_finishes_the_order() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;
    let ticket = KitchenTicket::of_order(w.db(), order.id)
        .await
        .unwrap()
        .remove(0);

    w.post(
        &w.cook,
        &format!("/tickets/{}/bump", ticket.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Ready);
}

/// Accepting needs `orders.manage` and the sales permission; a cook cannot,
/// and an order is accepted once.
#[renox::test]
async fn only_a_manager_accepts_an_order_and_only_once() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "online", &PLATE).await;
    let path = format!("/orders/{}/accept", placed.id);

    w.post(&w.cook, &path, &[("x", "")])
        .await
        .assert_forbidden();
    w.post(&w.cashier, &path, &[("x", "")])
        .await
        .assert_forbidden();
    assert_eq!(w.reload(&placed).await.status, FnbOrderStatus::Placed);

    w.post(&w.manager, &path, &[("x", "")])
        .await
        .assert_status(303);
    w.post(&w.manager, &path, &[("x", "")])
        .await
        .assert_status(422);
    assert_eq!(w.reload(&placed).await.status, FnbOrderStatus::Accepted);
}

/// A rejected order is `rejected`, with its sales order.
#[renox::test]
async fn rejecting_a_placed_order_needs_a_reason() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let placed = w.place(c, "cash_on_delivery", &PLATE).await;
    let path = format!("/orders/{}/reject", placed.id);

    w.post(&w.manager, &path, &[("reason", " ")])
        .await
        .assert_status(422);
    w.post(&w.manager, &path, &[("reason", "The kitchen is closed")])
        .await
        .assert_status(303);

    let after = w.reload(&placed).await;
    assert_eq!(after.status, FnbOrderStatus::Rejected);
    assert_eq!(after.close_reason.as_deref(), Some("The kitchen is closed"));
}

/// Dispatch is offered to the manager of a ready delivery order; without a
/// rider on a run the page says why it is not available and refuses the
/// action.
#[renox::test]
async fn dispatch_waits_for_a_rider_and_the_page_says_so() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let ready = w.ready(c, "online", &PLATE).await;

    w.get(&w.manager, &format!("/orders/{}", ready.id))
        .await
        .assert_ok()
        .assert_see("put the delivery of the order on a run")
        .assert_dont_see("Dispatch the order");
    w.post(
        &w.manager,
        &format!("/orders/{}/dispatch", ready.id),
        &[("x", "")],
    )
    .await
    .assert_status(422);
    assert_eq!(w.reload(&ready).await.status, FnbOrderStatus::Ready);

    // With the delivery assigned to a run, the manager may dispatch by hand.
    let delivery = w.delivery_of(&ready).await;
    let run = w.run_of(&w.rider).await;
    let run_id = run.id.to_string();
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/assign", delivery.id),
        &[("run_id", run_id.as_str())],
    )
    .await
    .assert_ok();
    w.get(&w.manager, &format!("/orders/{}", ready.id))
        .await
        .assert_ok()
        .assert_see("Dispatch the order");
    w.post(
        &w.manager,
        &format!("/orders/{}/dispatch", ready.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    assert_eq!(w.reload(&ready).await.status, FnbOrderStatus::Dispatched);
}

/// A pickup order is completed by hand, and the sales order is fulfilled.
#[renox::test]
async fn a_pickup_order_is_completed_by_the_manager() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;
    w.post(
        &w.manager,
        &format!("/orders/{}/details", order.id),
        &[
            ("kind", "pickup"),
            ("recipient_name", "Mariam"),
            ("address", ""),
        ],
    )
    .await
    .assert_status(303);
    let ready = w.cook(&w.reload(&order).await).await;
    assert!(ready.delivery_id.is_none(), "a pickup asks for no delivery");

    w.post(
        &w.cook,
        &format!("/orders/{}/complete", ready.id),
        &[("x", "")],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.manager,
        &format!("/orders/{}/complete", ready.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    assert_eq!(w.reload(&ready).await.status, FnbOrderStatus::Completed);
    assert_eq!(
        SalesOrder::find_or_404(w.db(), ready.sales_order_id)
            .await
            .unwrap()
            .status,
        SalesOrderStatus::Fulfilled
    );
}

/// A delivery needs an address.
#[renox::test]
async fn a_delivery_needs_an_address() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;

    w.post(
        &w.manager,
        &format!("/orders/{}/details", order.id),
        &[
            ("kind", "delivery"),
            ("recipient_name", "Mariam"),
            ("address", " "),
        ],
    )
    .await
    .assert_status(422);
    w.post(
        &w.cashier,
        &format!("/orders/{}/details", order.id),
        &[("kind", "pickup")],
    )
    .await
    .assert_forbidden();
}

/// The kitchen board lists the open tickets without customer details; a
/// person without `kds.work` cannot open it.
#[renox::test]
async fn the_kitchen_board_shows_open_tickets_and_no_customer_details() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;

    let page = w.get(&w.cook, "/kitchen").await;
    page.assert_ok()
        .assert_see("shawarma-plate")
        .assert_see(&order.number)
        .assert_dont_see("Mariam Al Hosani")
        .assert_dont_see("Al Barsha");
    w.get(&w.nobody, "/kitchen").await.assert_forbidden();
    w.get(&w.nobody, "/orders").await.assert_forbidden();
    w.get(&w.manager, "/orders")
        .await
        .assert_ok()
        .assert_see(&order.number);
}

/// The kitchen tells the sales module which lines it started: none while the
/// ticket is queued, the line once it is in preparation.
#[renox::test]
async fn the_kitchen_reports_the_lines_it_started_to_the_sales_module() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;
    let ticket = KitchenTicket::of_order(w.db(), order.id)
        .await
        .unwrap()
        .remove(0);
    let progress = fulfilment_progress(w.state());
    let lines = [ticket.sale_line_id];

    let before = progress
        .started_lines(w.state(), SalesOrderId(order.sales_order_id), &lines)
        .await
        .unwrap();
    assert!(before.is_empty());

    w.post(
        &w.cook,
        &format!("/tickets/{}/start", ticket.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);

    let after = progress
        .started_lines(w.state(), SalesOrderId(order.sales_order_id), &lines)
        .await
        .unwrap();
    assert_eq!(after, vec![ticket.sale_line_id]);
}

/// The stock side of a line is its recipe at the outlet's location: the
/// components per unit, the line's quantity, whether the consumption was
/// posted and the value of the line.
#[renox::test]
async fn the_stock_side_of_a_line_is_its_recipe_at_the_outlets_store() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let order = w.place_via("phone", c, "cash_on_delivery", &PLATE).await;
    let ticket = KitchenTicket::of_order(w.db(), order.id)
        .await
        .unwrap()
        .remove(0);

    // By the sales order, and by the F&B order a delivery names.
    for subject in [
        SalesOrderId(order.sales_order_id).subject(),
        order.subject(),
    ] {
        let stock = wl_kernel::inventory::sale_line_stock(w.state())
            .of_line(w.state(), &subject, ticket.sale_line_id)
            .await
            .unwrap()
            .expect("the line has a recipe");
        assert_eq!(stock.location.subject_id, w.store.id);
        assert_eq!(stock.quantity_milli, 2_000);
        assert!(!stock.consumed);
        assert_eq!(stock.value, 2_200, "2 plates at AED 11.00 cost");
        assert_eq!(stock.components.len(), 2);
        assert!(
            stock
                .components
                .iter()
                .any(|c| c.item.subject_id == w.chicken.id && c.per_unit_milli == 250)
        );
    }
}

/// BL-SAL-02 (#121) on the F&B deployment: an online order that is not paid
/// yet is awaiting payment; the outlet does not see it in the orders to
/// accept or on the kitchen board, no kitchen ticket exists, and it appears
/// as `placed` once the gateway confirms the full payment.
#[renox::test]
async fn an_order_awaiting_payment_is_not_in_the_acceptance_list_or_the_kitchen() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let party = c.0.to_string();
    w.post(
        &w.cashier,
        "/sales-orders",
        &[
            ("party_id", party.as_str()),
            ("channel", "website"),
            ("payment_mode", "online"),
        ],
    )
    .await
    .assert_status(303);
    let sales = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(w.db())
        .await
        .unwrap()
        .expect("the sales order was entered");
    w.post(
        &w.cashier,
        &format!("/sales-orders/{}/lines", sales.id),
        &[
            ("item_code", "shawarma-plate"),
            ("quantity", "2"),
            ("unit_price", "30.00"),
            ("vat", "zero_rated"),
        ],
    )
    .await
    .assert_status(303);
    w.post(
        &w.cashier,
        &format!("/sales-orders/{}/submit", sales.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let waiting = SalesOrder::find_or_404(w.db(), sales.id).await.unwrap();
    assert_eq!(waiting.status, SalesOrderStatus::AwaitingPayment);
    let number = waiting.number.clone();

    assert!(
        fnb::FnbOrder::of_sales_order(w.db(), sales.id)
            .await
            .unwrap()
            .is_none(),
        "no F&B order yet"
    );
    w.get(&w.manager, "/orders")
        .await
        .assert_ok()
        .assert_dont_see(&number);
    w.get(&w.cook, "/kitchen")
        .await
        .assert_ok()
        .assert_dont_see("shawarma-plate");

    wl_sales::sales_orders::confirm_payment(w.state(), sales.id, waiting.gross_total)
        .await
        .expect("the gateway confirmed the payment");

    let placed = fnb::FnbOrder::of_sales_order(w.db(), sales.id)
        .await
        .unwrap()
        .expect("the F&B order exists once paid");
    assert_eq!(placed.status, FnbOrderStatus::Placed);
    w.get(&w.manager, "/orders")
        .await
        .assert_ok()
        .assert_see(&placed.number);
}
