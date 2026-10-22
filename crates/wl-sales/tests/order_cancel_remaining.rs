//! BL-SAL-09 (#128): cancel the remaining lines with outcomes (wl-sales §4.2
//! row `cancel_remaining`, *Cancelling after fulfilment started*,
//! `SAL.ACL.05`). A `partially_fulfilled` order whose rest will never be
//! delivered is closed off: every open line is cancelled, one
//! `SalesOrderLineCancelled` is emitted per line, the prepaid shares are
//! refunded less the fees, and lines whose preparation started need an outcome
//! and a manager's approval. Every check drives the real pages, machine,
//! kernel approvals and listeners; payments and invoicing are the fakes of
//! `support::changes`. Lines are zero-rated so the figures are the stories'.

mod support;

use wl_kernel::approvals;
use wl_kernel::sales::LineOutcomeKind;
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use renox::prelude::*;
use support::Booted;
use support::changes::{
    Changes, boot_changes, boot_changes_with, clerk, hand_over, lines, manager, place, reload,
    waiting,
};

/// A `partially_fulfilled` order of party 7: line 1 (2 × AED 20) handed over,
/// line 2 (3 × AED 10) and line 3 (1 × AED 5) open. `mode` is `on_account` or
/// `online` (paid in full: AED 75).
async fn world(t: &Booted, c: &Changes, mode: &str) -> (SalesOrder, Vec<SaleLine>) {
    let order = place(
        t,
        c,
        "phone",
        mode,
        &[
            ("tee", "2", "20.00"),
            ("cap", "3", "10.00"),
            ("pin", "1", "5.00"),
        ],
    )
    .await;
    let items = lines(t, &order).await;
    hand_over(t, &order, &items[0], 2_000, 1).await;
    let order = reload(t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PartiallyFulfilled);
    (order, items)
}

/// Posts the cancel-remaining form of `order`.
async fn cancel_rest(
    t: &Booted,
    order: &SalesOrder,
    extra: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let mut form = vec![("reason", "the guests left")];
    form.extend_from_slice(extra);
    t.app
        .post(
            &format!("/sales-orders/{}/cancel-remaining", order.id),
            &form,
        )
        .await
}

/// AC1: Given a `partially_fulfilled` order with line A fulfilled and line B
/// (3 open, not started), `cancel_remaining` with a reason makes it
/// `fulfilled`, B cancelled qty 3, reservation released, one
/// `SalesOrderLineCancelled` emitted.
#[renox::test]
async fn the_remaining_lines_are_cancelled_and_the_order_is_fulfilled() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(
        &t,
        &c,
        "phone",
        "on_account",
        &[("tee", "2", "20.00"), ("cap", "3", "10.00")],
    )
    .await;
    let items = lines(&t, &order).await;
    hand_over(&t, &order, &items[0], 2_000, 1).await;
    let partly = reload(&t, order.id).await;
    // The page offers the button on a partly fulfilled order.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("cancel-order-remaining");

    cancel_rest(&t, &partly, &[]).await.assert_status(303);

    let done = reload(&t, order.id).await;
    assert_eq!(done.status, SalesOrderStatus::Fulfilled);
    assert!(done.fulfilled_at.is_some());
    let after = lines(&t, &done).await;
    assert_eq!(after[0].cancelled_milli, 0, "what was handed over stays");
    assert_eq!(after[1].cancelled_milli, 3_000);
    assert_eq!(after[1].open_milli(), 0);
    let events = c.line_cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].line, events[0].quantity_milli),
        (items[1].id, 3_000)
    );
    assert_eq!(events[0].reason, "the guests left");
    assert_eq!((events[0].outcome, events[0].fee), (None, 0));
    // The invoice bills what was handed over only.
    let drafts = c.ledger.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].1.lines.len(), 1);
    assert_eq!(drafts[0].1.lines[0].quantity_milli, 2_000);
}

/// Edge: one event per cancelled line, so inventory reacts as for
/// `cancel_line`; a prepaid order is refunded the shares once.
#[renox::test]
async fn one_event_per_line_and_one_refund_of_the_prepaid_shares() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let (order, items) = world(&t, &c, "online").await;

    cancel_rest(&t, &order, &[]).await.assert_status(303);

    let events = c.line_cancelled.lock().unwrap().clone();
    let cancelled: Vec<(i64, i64)> = events.iter().map(|e| (e.line, e.quantity_milli)).collect();
    assert_eq!(cancelled, [(items[1].id, 3_000), (items[2].id, 1_000)]);
    // 3 × AED 10 + 1 × AED 5 in one refund.
    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 3_500);
    assert_eq!(refunds[0].0.reason, "order line cancelled");
}

/// AC2: Given B picked, outcome and approval are required.
#[renox::test]
async fn a_started_line_needs_an_outcome_and_a_manager() {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let (order, items) = world(&t, &c, "on_account").await;
    c.started.mark(items[1].id);
    let outcome = format!("line_{}_outcome", items[1].id);

    // No outcome for the picked line: refused.
    cancel_rest(&t, &order, &[]).await.assert_status(422);
    assert!(waiting(&t, &order).await.is_empty());
    // With one: the request waits and nothing is cancelled.
    cancel_rest(&t, &order, &[(&outcome, "return_to_stock")])
        .await
        .assert_status(303);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PartiallyFulfilled
    );
    assert!(
        lines(&t, &order)
            .await
            .iter()
            .all(|l| l.cancelled_milli == 0)
    );
    let request = waiting(&t, &order).await.remove(0);
    assert_eq!(request.action, "sales.order_cancel_remaining");
    assert_eq!(request.requester_id, sam.id);
    assert!(
        approvals::approve(t.state(), request.id, &sam, "")
            .await
            .is_err()
    );

    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    let done = reload(&t, order.id).await;
    assert_eq!(done.status, SalesOrderStatus::Fulfilled);
    let after = lines(&t, &done).await;
    assert_eq!(
        (after[1].cancelled_milli, after[2].cancelled_milli),
        (3_000, 1_000)
    );
    assert_eq!(after[1].cancel_outcome.as_deref(), Some("return_to_stock"));
    assert_eq!(after[2].cancel_outcome, None);
    let events = c.line_cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 2);
    let started = events.iter().find(|e| e.line == items[1].id).unwrap();
    assert_eq!(started.outcome, Some(LineOutcomeKind::ReturnToStock));
    let plain = events.iter().find(|e| e.line == items[2].id).unwrap();
    assert_eq!(plain.outcome, None);
}

/// Edge: scrap with a fee on a prepaid order: the fee is kept from the refund
/// and billed; a waived fee is recorded.
#[renox::test]
async fn fees_are_kept_from_the_refund_and_a_waived_fee_is_not() {
    let (t, c) = boot_changes_with(
        WlSales::new()
            .cancellation_fee_percent(10)
            .cancellation_fee_vat("exempt"),
    )
    .await;
    t.app.acting_as(&clerk(&t).await);
    let (order, items) = world(&t, &c, "online").await;
    c.started.mark(items[1].id);
    c.started.mark(items[2].id);
    let (o1, o2, w2) = (
        format!("line_{}_outcome", items[1].id),
        format!("line_{}_outcome", items[2].id),
        format!("line_{}_waive", items[2].id),
    );

    cancel_rest(&t, &order, &[(&o1, "scrap"), (&o2, "scrap"), (&w2, "on")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);
    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    // Line 2: share AED 30, fee 10% = AED 3. Line 3: share AED 5, fee waived.
    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 3_500 - 300);
    // The order's own invoice (what was handed over) and the fee document.
    let drafts = c.ledger.drafts();
    assert_eq!(drafts.len(), 2);
    assert_eq!(drafts[0].1.lines.len(), 1);
    assert_eq!(drafts[1].1.lines[0].gross(), 300);
    let after = lines(&t, &order).await;
    assert_eq!((after[1].cancel_fee, after[1].fee_waived), (300, false));
    assert_eq!((after[2].cancel_fee, after[2].fee_waived), (0, true));
    let events = c.line_cancelled.lock().unwrap().clone();
    let waived = events.iter().find(|e| e.line == items[2].id).unwrap();
    assert!(waived.fee_waived);
}

/// Edge: a rejected request cancels nothing.
#[renox::test]
async fn a_rejected_request_leaves_the_order_as_it_was() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let (order, items) = world(&t, &c, "online").await;
    c.started.mark(items[1].id);
    let outcome = format!("line_{}_outcome", items[1].id);
    cancel_rest(&t, &order, &[(&outcome, "scrap")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);

    let mia = manager(&t).await;
    approvals::reject(t.state(), request.id, &mia, "Cook it anyway")
        .await
        .unwrap();

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PartiallyFulfilled
    );
    assert!(
        lines(&t, &order)
            .await
            .iter()
            .all(|l| l.cancelled_milli == 0)
    );
    assert!(c.gateway.refunds().is_empty());
    assert!(c.line_cancelled.lock().unwrap().is_empty());
}

/// Edge: if goods are handed over while the request waits, the final check
/// fails the request and nothing is cancelled.
#[renox::test]
async fn a_request_whose_order_moved_while_it_waited_fails() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let (order, items) = world(&t, &c, "on_account").await;
    c.started.mark(items[1].id);
    let outcome = format!("line_{}_outcome", items[1].id);
    cancel_rest(&t, &order, &[(&outcome, "scrap")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);
    // The pin is handed over meanwhile.
    hand_over(&t, &order, &items[2], 1_000, 2).await;

    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    let now_order = reload(&t, order.id).await;
    assert_eq!(now_order.status, SalesOrderStatus::PartiallyFulfilled);
    assert!(
        lines(&t, &now_order)
            .await
            .iter()
            .all(|l| l.cancelled_milli == 0)
    );
    assert!(c.line_cancelled.lock().unwrap().is_empty());
}

/// Edge: a `confirmed` order has nothing handed over: `cancel_remaining` is
/// not offered (the page says to cancel the order) and is refused.
#[renox::test]
async fn nothing_handed_over_means_the_whole_order_is_cancelled_instead() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("tee", "2", "20.00")]).await;

    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("cancel the order instead")
        .assert_dont_see("cancel-order-remaining");
    cancel_rest(&t, &order, &[]).await.assert_status(422);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
}

/// Edge: only a holder of `sales_orders.cancel` cancels the remaining lines.
#[renox::test]
async fn a_user_without_the_cancel_permission_cannot_cancel_the_rest() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let (order, _items) = world(&t, &c, "on_account").await;
    let viewer = t
        .user("Vic", &[wl_sales::permissions::SALES_ORDERS_VIEW])
        .await;
    t.app.acting_as(&viewer);

    cancel_rest(&t, &order, &[]).await.assert_status(403);

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PartiallyFulfilled
    );
}
