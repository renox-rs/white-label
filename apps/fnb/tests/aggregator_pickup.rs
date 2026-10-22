//! BL-FNB-06 (#370), 06 §4.1 row *The aggregator's rider does not arrive*,
//! §7.2 `FNB.RUL.16`: an aggregator order that is `ready` and not collected 15
//! minutes later alerts the kitchen manager to chase the aggregator; at 45
//! minutes a task asks the manager to record what happened: picked up late
//! (the order is completed), or cancelled by the aggregator (the order is
//! cancelled with the party *aggregator*, the food is wasted, compensation is
//! expected, nobody is refunded and no second approval is asked). An order
//! collected earlier gets no task, and an open task closes. The modules are
//! the real wl-sales, wl-payments, wl-inventory and wl-delivery.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use fnb::order::{cancel_reason, party};
use fnb::pickup::{self, Done};
use fnb::{Fnb, FnbOrder, FnbOrderStatus, KitchenTicket, TicketStatus};
use renox::chrono::TimeDelta;
use renox::prelude::*;
use wl_inventory::MovementType;
use wl_kernel::Subject;
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE;
use wl_payments::refund::Refund;
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{World, boot_own_riders, boot_with, own_riders};

/// One shawarma plate at AED 60.00, an aggregator's ORD-801.
const PLATE: [(&str, &str, &str); 1] = [("shawarma-plate", "1", "60.00")];

const NOTICE: &str = "fnb.staff_notice";

static CUSTOMERS: AtomicI64 = AtomicI64::new(1);

async fn customer(w: &World) -> wl_kernel::PartyId {
    let n = CUSTOMERS.fetch_add(1, Ordering::SeqCst);
    w.customer(&format!("Aggregator Customer {n}")).await
}

/// An aggregator order cooked and waiting at the pass.
async fn ready(w: &World) -> FnbOrder {
    let c = customer(w).await;
    let order = w.ready_via("aggregator_a", c, "on_account", &PLATE).await;
    assert_eq!(order.status, FnbOrderStatus::Ready);
    assert!(order.ready_at.is_some(), "the ready time is stamped");
    order
}

/// Runs the rule `minutes` after `order` became ready.
async fn after(w: &World, order: &FnbOrder, minutes: i64) -> Done {
    let ready = order.ready_at.expect("ready_at");
    pickup::run(w.state(), ready + TimeDelta::minutes(minutes))
        .await
        .expect("the rule runs")
}

/// The wastage movements of `item`.
async fn wastage_of(w: &World, item: &wl_inventory::Item) -> Vec<wl_inventory::StockMovement> {
    w.movements_of(item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Wastage)
        .collect()
}

/// AC1: given aggregator ORD-801 `ready` at 19:00, at 19:15 the kitchen
/// manager is notified (not at 19:14, and once).
#[renox::test]
async fn the_kitchen_manager_is_told_to_chase_the_aggregator_after_fifteen_minutes() {
    let w = boot_own_riders().await;
    let order = ready(&w).await;

    assert_eq!(after(&w, &order, 14).await, Done::default());
    assert!(w.notices.of_type(NOTICE).is_empty());

    assert_eq!(after(&w, &order, 15).await, Done { alerts: 1, tasks: 0 });
    let told = w.notices.users_of_type(NOTICE);
    assert!(told.contains(&w.manager.id) && told.contains(&w.manager_b.id));
    assert!(!told.contains(&w.cook.id) && !told.contains(&w.cashier.id));
    let notice = w.notices.of_type(NOTICE).remove(0);
    assert!(notice.data["title"].as_str().unwrap().contains("Chase the aggregator"));
    assert!(notice.data["body"].as_str().unwrap().contains("15 minutes"));
    // Once; nothing is cancelled, nothing is refunded, no task yet.
    assert_eq!(after(&w, &order, 16).await, Done::default());
    assert_eq!(w.notices.of_type(NOTICE).len(), 1);
    assert_eq!(w.reload(&order).await.status, FnbOrderStatus::Ready);
    assert!(w.tickets.opened().is_empty());
}

/// AC2: at 19:45 the manager has a task; choosing *cancelled by aggregator*
/// cancels the order with party *aggregator*, posts the food as wastage and
/// sets "compensation expected"; nobody is refunded and no approval is asked.
#[renox::test]
async fn the_task_at_forty_five_minutes_cancels_the_order_with_the_aggregator_responsible() {
    let w = boot_own_riders().await;
    let order = ready(&w).await;

    assert_eq!(after(&w, &order, 45).await, Done { alerts: 1, tasks: 1 });
    let opened = w.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert!(opened[0].title.contains("Record what happened"));
    assert_eq!(opened[0].subject, order.subject());
    assert_eq!(
        opened[0].source_key.as_deref(),
        Some(pickup::task_key(&order).as_str())
    );
    assert_eq!(w.reload(&order).await.pickup_task_id, Some(1));
    // Not twice.
    assert_eq!(after(&w, &order, 50).await, Done::default());
    assert_eq!(w.tickets.opened().len(), 1);

    // The page offers the two choices to the manager only.
    let page = w.get(&w.manager, &format!("/orders/{}", order.id)).await;
    page.assert_ok();
    assert!(page.text().contains("Cancelled by the aggregator"));
    assert!(page.text().contains("Picked up late"));
    let none = w.get(&w.cashier, &format!("/orders/{}", order.id)).await;
    assert!(!none.text().contains("Picked up late"));

    let path = format!("/orders/{}/pickup/cancel", order.id);
    w.post(&w.cashier, &path, &[("x", "")]).await.assert_forbidden();
    w.post(&w.cook, &path, &[("x", "")]).await.assert_forbidden();
    w.post(&w.manager, &path, &[("x", "")]).await.assert_status(303);

    let cancelled = w.reload(&order).await;
    assert_eq!(cancelled.status, FnbOrderStatus::Cancelled);
    assert_eq!(cancelled.cancel_party.as_deref(), Some(party::AGGREGATOR));
    assert_eq!(
        cancelled.cancel_reason_code.as_deref(),
        Some(cancel_reason::AGGREGATOR_CANCELLED)
    );
    assert!(cancelled.compensation_expected);
    assert_eq!(cancelled.cancelled_by, Some(w.manager.id));
    // The food is wasted, once per ingredient; the cooked tickets stay done.
    assert!(cancelled.wastage_posted);
    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 1);
    assert_eq!(wastage_of(&w, &w.rice).await.len(), 1);
    for ticket in KitchenTicket::of_order(w.db(), cancelled.id).await.unwrap() {
        assert_eq!(ticket.status, TicketStatus::Done);
    }
    // Nobody is refunded and nothing waited for an approval.
    let refunds = Refund::query()
        .where_(Refund::SOURCE_ID.eq(cancelled.sales_order_id))
        .get(w.db())
        .await
        .unwrap();
    assert!(refunds.is_empty());
    let waiting = ApprovalRequest::pending_about(
        w.db(),
        &Subject::new(SALES_ORDER_SUBJECT_TYPE, cancelled.sales_order_id),
    )
    .await
    .unwrap();
    assert!(waiting.is_empty());
    let sales = SalesOrder::find_or_404(w.db(), cancelled.sales_order_id).await.unwrap();
    assert_eq!(sales.status, SalesOrderStatus::Cancelled);
    // The task closed itself.
    assert_eq!(w.tickets.closed(), vec![1]);
}

/// AC3: given pickup at 19:30, no task is created; a task that is open when
/// the order is collected closes.
#[renox::test]
async fn an_order_collected_before_forty_five_minutes_gets_no_task_and_an_open_one_closes() {
    let w = boot_own_riders().await;
    let early = ready(&w).await;
    assert_eq!(after(&w, &early, 15).await.alerts, 1);
    w.post(&w.manager, &format!("/orders/{}/complete", early.id), &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(w.reload(&early).await.status, FnbOrderStatus::Completed);
    assert_eq!(after(&w, &early, 45).await, Done::default());
    assert!(w.tickets.opened().is_empty());

    // A task already open closes when the order is collected.
    let late = ready(&w).await;
    assert_eq!(after(&w, &late, 45).await.tasks, 1);
    assert!(w.tickets.closed().is_empty());
    w.post(&w.manager, &format!("/orders/{}/complete", late.id), &[("x", "")])
        .await
        .assert_status(303);
    assert_eq!(w.tickets.closed(), vec![1]);
}

/// AC4: choosing *picked up late* completes the order.
#[renox::test]
async fn picked_up_late_completes_the_order_and_closes_the_task() {
    let w = boot_own_riders().await;
    let order = ready(&w).await;
    after(&w, &order, 45).await;
    let path = format!("/orders/{}/pickup/late", order.id);

    w.post(&w.cashier, &path, &[("x", "")]).await.assert_forbidden();
    w.post(&w.manager, &path, &[("x", "")]).await.assert_status(303);

    let done = w.reload(&order).await;
    assert_eq!(done.status, FnbOrderStatus::Completed);
    assert!(!done.compensation_expected);
    assert_eq!(wastage_of(&w, &w.chicken).await.len(), 0);
    assert_eq!(w.tickets.closed(), vec![1]);
    // A second choice finds nothing waiting.
    w.post(&w.manager, &path, &[("x", "")]).await.assert_status(422);
}

/// A direct order that waits for its own rider is not an aggregator's: the
/// rule leaves it alone, and the aggregator choices are refused.
#[renox::test]
async fn a_direct_order_is_not_watched_for_an_aggregators_rider() {
    let w = boot_own_riders().await;
    let c = customer(&w).await;
    let direct = w.ready(c, "cash_on_delivery", &PLATE).await;
    assert_eq!(after(&w, &direct, 60).await, Done::default());
    assert!(w.notices.of_type(NOTICE).is_empty());
    assert!(w.tickets.opened().is_empty());
    for tail in ["late", "cancel"] {
        w.post(&w.manager, &format!("/orders/{}/pickup/{tail}", direct.id), &[("x", "")])
            .await
            .assert_status(422);
    }
    assert_eq!(w.reload(&direct).await.status, FnbOrderStatus::Ready);
}

/// The minutes are settings (N = 15, M = 45 by default) and the rule can be
/// switched off.
#[renox::test]
async fn the_minutes_and_the_rule_are_settings() {
    let quick = boot_with(own_riders(), Fnb::new().aggregator_pickup(5, 10)).await;
    let order = ready(&quick).await;
    assert_eq!(after(&quick, &order, 4).await, Done::default());
    assert_eq!(after(&quick, &order, 5).await, Done { alerts: 1, tasks: 0 });
    assert_eq!(after(&quick, &order, 10).await, Done { alerts: 0, tasks: 1 });

    let off = boot_with(own_riders(), Fnb::new().aggregator_pickup_rule(false)).await;
    let order = ready(&off).await;
    assert_eq!(after(&off, &order, 120).await, Done::default());
    assert!(off.notices.of_type(NOTICE).is_empty());
}
