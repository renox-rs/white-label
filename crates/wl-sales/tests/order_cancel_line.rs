//! BL-SAL-08 (#127): cancel one line of an order (wl-sales §4.2 row
//! `cancel_line`, *Cancelling after fulfilment started*, `SAL.ACL.05`). The
//! open quantity of a line is cancelled with a reason and the order goes on;
//! a line whose preparation started needs an outcome (return to stock, finish
//! to stock, scrap) and a manager's approval, the requester never approving;
//! the prepaid share is refunded less any fee, an issued invoice is credited,
//! and `SalesOrderLineCancelled` carries the outcome and fee. Every check
//! drives the real pages, machine, kernel approvals and listeners; payments
//! and invoicing are the fakes of `support::changes`. Lines are zero-rated so
//! the figures are the stories'.

mod support;

use wl_kernel::approvals;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::payments::RefundSource;
use wl_kernel::sales::LineOutcomeKind;
use wl_sales::order_change::{
    OrderSettlement, SETTLE_CREDIT_NOTE, SETTLE_FEE_INVOICE, SETTLE_REFUND,
};
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use renox::prelude::*;
use support::Booted;
use support::changes::{
    Changes, boot_changes, boot_changes_with, clerk, hand_over, lines, manager, place, reload,
    waiting,
};

/// A `confirmed` order of party 7 with line 1 (5 × AED 20) and line 2 (1 ×
/// AED 10), entered by Sam. `mode` is `on_account` or `online` (paid).
async fn world(mode: &str) -> (Booted, Changes, SalesOrder, Vec<SaleLine>, User) {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let order = place(
        &t,
        &c,
        "phone",
        mode,
        &[("tee", "5", "20.00"), ("cap", "1", "10.00")],
    )
    .await;
    let items = lines(&t, &order).await;
    (t, c, order, items, sam)
}

/// Posts the cancel-line form of `order` for the line at `position`.
async fn cancel(
    t: &Booted,
    order: &SalesOrder,
    position: &str,
    quantity: &str,
    extra: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let mut form = vec![
        ("line_no", position),
        ("quantity", quantity),
        ("reason", "out of stock"),
    ];
    form.extend_from_slice(extra);
    t.app
        .post(&format!("/sales-orders/{}/cancel-line", order.id), &form)
        .await
}

/// AC1: Given a `confirmed` order with line A (5 × AED 20, not started) and
/// line B open, when A is cancelled for 2 units reason "out of stock", then
/// line A cancelled qty = 2, 2 units' reservation released,
/// `SalesOrderLineCancelled` emitted, state stays `confirmed`.
#[renox::test]
async fn two_units_of_an_unstarted_line_are_cancelled_and_the_order_goes_on() {
    let (t, c, order, items, _sam) = world("on_account").await;

    cancel(&t, &order, "1", "2", &[]).await.assert_status(303);

    let now_order = reload(&t, order.id).await;
    assert_eq!(now_order.status, SalesOrderStatus::Confirmed);
    let after = lines(&t, &now_order).await;
    assert_eq!(after[0].cancelled_milli, 2_000);
    assert_eq!(after[0].open_milli(), 3_000);
    assert_eq!(after[0].cancel_outcome, None);
    assert_eq!(after[0].cancel_reason.as_deref(), Some("out of stock"));
    let events = c.line_cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order.0, order.id);
    assert_eq!(
        (events[0].line, events[0].quantity_milli),
        (items[0].id, 2_000)
    );
    assert_eq!(events[0].reason, "out of stock");
    assert_eq!((events[0].outcome, events[0].fee), (None, 0));
    // An order on account: nothing to refund.
    assert!(c.gateway.refunds().is_empty());
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Cancel a line");
}

/// AC2: Given the order prepaid, AED 40 is refunded.
#[renox::test]
async fn the_prepaid_share_of_the_cancelled_quantity_is_refunded() {
    let (t, c, order, _items, _sam) = world("online").await;

    cancel(&t, &order, "1", "2", &[]).await.assert_status(303);

    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 4_000);
    assert_eq!(refunds[0].0.reason, "order line cancelled");
    assert_eq!(refunds[0].0.source, RefundSource::Order(order.subject()));
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(
        (steps[0].kind.as_str(), steps[0].amount),
        (SETTLE_REFUND, 4_000)
    );
    assert_eq!(c.mail.of_type("sales.order_lines_cancelled").len(), 1);
}

/// AC3: Given line A picked, cancelling needs an outcome and approval by
/// someone other than the requester.
#[renox::test]
async fn a_started_line_needs_an_outcome_and_a_manager_other_than_the_requester() {
    let (t, c, order, items, sam) = world("on_account").await;
    c.started.mark(items[0].id);

    // No outcome: refused, nothing waits.
    cancel(&t, &order, "1", "2", &[]).await.assert_status(422);
    assert!(waiting(&t, &order).await.is_empty());
    // An outcome: the request waits and nothing changes yet.
    cancel(&t, &order, "1", "2", &[("outcome", "scrap")])
        .await
        .assert_status(303);
    let after = lines(&t, &order).await;
    assert_eq!(after[0].cancelled_milli, 0);
    assert!(c.line_cancelled.lock().unwrap().is_empty());
    let requests = waiting(&t, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "sales.order_cancel_line");
    assert_eq!(requests[0].requester_id, sam.id);
    // A second request of the same kind waits behind the first.
    cancel(&t, &order, "1", "1", &[("outcome", "scrap")])
        .await
        .assert_status(422);

    // The requester cannot approve; a manager can.
    assert!(
        approvals::approve(t.state(), requests[0].id, &sam, "")
            .await
            .is_err()
    );
    let mia = manager(&t).await;
    approvals::approve(t.state(), requests[0].id, &mia, "")
        .await
        .unwrap();

    let done = reload(&t, order.id).await;
    assert_eq!(done.status, SalesOrderStatus::Confirmed);
    let after = lines(&t, &done).await;
    assert_eq!(after[0].cancelled_milli, 2_000);
    assert_eq!(after[0].cancel_outcome.as_deref(), Some("scrap"));
    let events = c.line_cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, Some(LineOutcomeKind::Scrap));
    assert_eq!(events[0].fee, 0);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Scrap");
}

/// Edge: a rejected request cancels nothing and refunds nothing.
#[renox::test]
async fn a_rejected_cancellation_changes_nothing() {
    let (t, c, order, items, _sam) = world("online").await;
    c.started.mark(items[0].id);
    cancel(&t, &order, "1", "2", &[("outcome", "return_to_stock")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);

    let mia = manager(&t).await;
    approvals::reject(t.state(), request.id, &mia, "Keep it")
        .await
        .unwrap();

    assert_eq!(lines(&t, &order).await[0].cancelled_milli, 0);
    assert!(c.gateway.refunds().is_empty());
    assert!(c.line_cancelled.lock().unwrap().is_empty());
}

/// Edge: an outcome the vertical does not offer is refused (06: return to
/// stock and scrap only; *finish to stock* needs a work order).
#[renox::test]
async fn an_outcome_the_vertical_does_not_offer_is_refused() {
    let (t, c) =
        boot_changes_with(WlSales::new().cancel_outcomes(&["return_to_stock", "scrap"])).await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(
        &t,
        &c,
        "phone",
        "online",
        &[("meal", "1", "80.00"), ("side", "1", "40.00")],
    )
    .await;
    let items = lines(&t, &order).await;
    c.started.mark(items[0].id);

    cancel(&t, &order, "1", "1", &[("outcome", "finish_to_stock")])
        .await
        .assert_status(422);

    assert!(waiting(&t, &order).await.is_empty());
}

/// Edge: the fee is a share of the line, kept from the prepayment with a fee
/// document of the configured VAT category; the refund is the prepaid share
/// less the fee (a prepaid order of AED 120, line 1 AED 80 started, fee 10%:
/// refund AED 72, a fee document of AED 8).
#[renox::test]
async fn ten_percent_of_a_started_line_is_the_fee() {
    let (t, c) = boot_changes_with(
        WlSales::new()
            .cancel_outcomes(&["return_to_stock", "scrap"])
            .cancellation_fee_percent(10)
            .cancellation_fee_vat("exempt"),
    )
    .await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let order = place(
        &t,
        &c,
        "phone",
        "online",
        &[("meal", "1", "80.00"), ("side", "1", "40.00")],
    )
    .await;
    let items = lines(&t, &order).await;
    c.started.mark(items[0].id);

    cancel(&t, &order, "1", "1", &[("outcome", "scrap")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);
    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 7_200);
    let drafts = c.ledger.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].1.lines.len(), 1);
    assert_eq!(drafts[0].1.lines[0].gross(), 800);
    assert_eq!(drafts[0].1.source, order.subject());
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    let kinds: Vec<&str> = steps.iter().map(|s| s.kind.as_str()).collect();
    assert!(kinds.contains(&SETTLE_REFUND) && kinds.contains(&SETTLE_FEE_INVOICE));
    let events = c.line_cancelled.lock().unwrap().clone();
    assert_eq!(
        (events[0].outcome, events[0].fee),
        (Some(LineOutcomeKind::Scrap), 800)
    );
    assert!(!events[0].fee_waived);
    assert_eq!(lines(&t, &order).await[0].cancel_fee, 800);
}

/// Edge: the manager may waive the fee (the approval is the yes); it is
/// recorded on the line and no fee document is made.
#[renox::test]
async fn a_waived_fee_is_recorded_and_not_billed() {
    let (t, c) = boot_changes_with(
        WlSales::new()
            .cancel_outcomes(&["return_to_stock", "scrap"])
            .cancellation_fee_percent(10),
    )
    .await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(
        &t,
        &c,
        "phone",
        "online",
        &[("meal", "1", "80.00"), ("side", "1", "40.00")],
    )
    .await;
    let items = lines(&t, &order).await;
    c.started.mark(items[0].id);

    cancel(
        &t,
        &order,
        "1",
        "1",
        &[("outcome", "scrap"), ("waive_fee", "on")],
    )
    .await
    .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);
    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    assert_eq!(c.gateway.refunds()[0].0.amount, 8_000);
    assert!(c.ledger.drafts().is_empty());
    let line = lines(&t, &order).await.remove(0);
    assert!(line.fee_waived);
    assert_eq!(line.cancel_fee, 0);
    let events = c.line_cancelled.lock().unwrap().clone();
    assert!(events[0].fee_waived);
    assert_eq!(events[0].fee, 0);
}

/// Edge: a fee above the prepaid share of the line is refused.
#[renox::test]
async fn a_fee_above_the_prepaid_share_is_refused() {
    let (t, c, order, items, _sam) = world("online").await;
    c.started.mark(items[0].id);

    cancel(
        &t,
        &order,
        "1",
        "1",
        &[("outcome", "scrap"), ("fee", "25.00")],
    )
    .await
    .assert_status(422);

    assert!(waiting(&t, &order).await.is_empty());
}

/// Edge: an invoice already issued for the line is corrected by a credit note
/// for the cancelled value.
#[renox::test]
async fn an_issued_invoice_gets_a_credit_note_for_the_cancelled_value() {
    let (t, c, order, _items, _sam) = world("on_account").await;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(t.db(), &[("invoice_id", &Some(101_i64))])
        .await
        .unwrap();
    c.ledger.set_status(101, InvoiceStatus::Issued);

    cancel(&t, &order, "1", "2", &[]).await.assert_status(303);

    let credits = c.ledger.credits();
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].0, 101);
    assert_eq!(
        (credits[0].1[0].invoice_line, credits[0].1[0].net),
        (1, 4_000)
    );
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(
        (steps[0].kind.as_str(), steps[0].amount),
        (SETTLE_CREDIT_NOTE, 4_000)
    );
}

/// AC4: Given a `partially_fulfilled` order whose only open line is cancelled,
/// it becomes `fulfilled`.
#[renox::test]
async fn cancelling_the_only_open_line_of_a_partly_fulfilled_order_completes_it() {
    let (t, c, order, items, _sam) = world("on_account").await;
    hand_over(&t, &order, &items[0], 5_000, 1).await;
    let partly = reload(&t, order.id).await;
    assert_eq!(partly.status, SalesOrderStatus::PartiallyFulfilled);

    cancel(&t, &partly, "2", "1", &[]).await.assert_status(303);

    let done = reload(&t, order.id).await;
    assert_eq!(done.status, SalesOrderStatus::Fulfilled);
    assert!(done.fulfilled_at.is_some());
    assert_eq!(lines(&t, &done).await[1].cancelled_milli, 1_000);
    assert_eq!(c.line_cancelled.lock().unwrap().len(), 1);
    // The invoice bills what was handed over, not what was cancelled.
    let drafts = c.ledger.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].1.lines.len(), 1);
    assert_eq!(drafts[0].1.lines[0].quantity_milli, 5_000);
    assert!(done.invoice_id.is_some());
}

/// AC5: Given a `confirmed` order with one line, `cancel_line` for all its
/// quantity is refused.
#[renox::test]
async fn cancelling_all_of_the_only_line_of_a_confirmed_order_is_refused() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("tee", "3", "20.00")]).await;

    cancel(&t, &order, "1", "3", &[]).await.assert_status(422);

    assert_eq!(lines(&t, &order).await[0].cancelled_milli, 0);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    // More than is open is refused too.
    cancel(&t, &order, "1", "4", &[]).await.assert_status(422);
}

/// Edge: only a holder of `sales_orders.cancel` cancels a line.
#[renox::test]
async fn a_user_without_the_cancel_permission_cannot_cancel_a_line() {
    let (t, _c, order, _items, _sam) = world("on_account").await;
    let viewer = t
        .user("Vic", &[wl_sales::permissions::SALES_ORDERS_VIEW])
        .await;
    t.app.acting_as(&viewer);

    cancel(&t, &order, "1", "2", &[]).await.assert_status(403);

    assert_eq!(lines(&t, &order).await[0].cancelled_milli, 0);
}
