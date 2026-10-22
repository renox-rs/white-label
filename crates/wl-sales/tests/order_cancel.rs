//! BL-SAL-07 (#126): cancel a whole order after fulfilment started (wl-sales
//! §4.2 row `cancel`, *Cancelling after fulfilment started*, `SAL.ACL.01`,
//! `SAL.ACL.05`). A started line gets an outcome (return to stock, finish to
//! stock, scrap) with or without a fee; a manager approves the whole
//! cancellation, never the requester; one refund of what was paid less the
//! fees is requested; `SalesOrderCancelled` carries the outcome and fee per
//! line. A paid order above the limit needs the approval too, one below does
//! not. Every check drives the real pages, machine, kernel approvals and
//! listeners; payments and invoicing are the fakes of `support::changes`.
//! Lines are zero-rated so the figures are the stories'.

mod support;

use wl_kernel::approvals;
use wl_kernel::sales::LineOutcomeKind;
use wl_sales::order_change::{OrderSettlement, SETTLE_FEE_INVOICE, SETTLE_REFUND};
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use renox::prelude::*;
use support::Booted;
use support::changes::{
    Changes, boot_changes, boot_changes_with, clerk, hand_over, lines, manager, place, reload,
    waiting,
};

/// A `confirmed` order of party 7, entered by Sam: line 1 (1 × AED 80) and
/// line 2 (1 × AED 40), AED 120 in all. `mode` is `on_account` or `online`
/// (paid in full).
async fn world(mode: &str) -> (Booted, Changes, SalesOrder, Vec<SaleLine>, User) {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let order = place(
        &t,
        &c,
        "phone",
        mode,
        &[("meal", "1", "80.00"), ("side", "1", "40.00")],
    )
    .await;
    let items = lines(&t, &order).await;
    (t, c, order, items, sam)
}

/// Posts the cancel form of `order`.
async fn cancel(
    t: &Booted,
    order: &SalesOrder,
    extra: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let mut form = vec![("reason", "the guests left")];
    form.extend_from_slice(extra);
    t.app
        .post(&format!("/sales-orders/{}/cancel", order.id), &form)
        .await
}

/// AC1: Given a `confirmed` prepaid F&B order AED 120 with line 1 (AED 80)
/// started in the kitchen and line 2 (AED 40) not started, when cashier A
/// cancels with outcome *scrap* for line 1 and fee 0, then an approval request
/// is created and the order stays `confirmed`.
#[renox::test]
async fn cancelling_a_started_order_waits_for_a_manager_and_changes_nothing() {
    let (t, c, order, items, _sam) = world("online").await;
    c.started.mark(items[0].id);
    let outcome = format!("line_{}_outcome", items[0].id);
    let fee = format!("line_{}_fee", items[0].id);

    // The page offers the button.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Cancel the order");

    cancel(
        &t,
        &order,
        &[(outcome.as_str(), "scrap"), (fee.as_str(), "0")],
    )
    .await
    .assert_status(303);

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    let requests = waiting(&t, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "sales.order_cancel");
    assert!(c.cancelled.lock().unwrap().is_empty());
    assert!(c.gateway.refunds().is_empty());
}

/// AC2: When kitchen manager B approves, the order is `cancelled`, a refund of
/// AED 120 is requested, and `SalesOrderCancelled` carries line 1 *scrap*,
/// fee 0.
#[renox::test]
async fn the_approval_cancels_the_order_refunds_it_all_and_carries_the_outcome() {
    let (t, c, order, items, _sam) = world("online").await;
    c.started.mark(items[0].id);
    let outcome = format!("line_{}_outcome", items[0].id);
    let fee = format!("line_{}_fee", items[0].id);
    cancel(
        &t,
        &order,
        &[(outcome.as_str(), "scrap"), (fee.as_str(), "0")],
    )
    .await
    .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);

    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    let done = reload(&t, order.id).await;
    assert_eq!(done.status, SalesOrderStatus::Cancelled);
    assert_eq!(done.reason.as_deref(), Some("the guests left"));
    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 12_000);
    assert_eq!(refunds[0].0.reason, "order cancelled");
    let events = c.cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "the guests left");
    assert_eq!(events[0].outcomes.len(), 1);
    assert_eq!(events[0].outcomes[0].line, items[0].id);
    assert_eq!(events[0].outcomes[0].kind, LineOutcomeKind::Scrap);
    assert_eq!(events[0].outcomes[0].fee, 0);
    // The lines carry what happened to them.
    let after = lines(&t, &done).await;
    assert_eq!(after[0].cancel_outcome.as_deref(), Some("scrap"));
    assert_eq!(after[1].cancel_outcome, None);
    assert_eq!(
        (after[0].cancelled_milli, after[1].cancelled_milli),
        (1_000, 1_000)
    );
}

/// AC3: Given fee 10% on line 1, the refund is AED 112 and a fee document of
/// AED 8 is requested with the configured VAT category.
#[renox::test]
async fn a_ten_percent_fee_is_kept_from_the_refund_and_billed_on_a_fee_document() {
    let (t, c) = boot_changes_with(
        WlSales::new()
            .cancel_outcomes(&["return_to_stock", "scrap"])
            .cancellation_fee_percent(10)
            .cancellation_fee_vat("exempt"),
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
    let outcome = format!("line_{}_outcome", items[0].id);
    cancel(&t, &order, &[(outcome.as_str(), "scrap")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);
    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 11_200);
    let drafts = c.ledger.drafts();
    assert_eq!(
        drafts.len(),
        1,
        "only the fee is billed, not the cancelled order"
    );
    assert_eq!(drafts[0].1.lines[0].gross(), 800);
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert!(
        steps
            .iter()
            .any(|s| s.kind == SETTLE_REFUND && s.amount == 11_200)
    );
    assert!(
        steps
            .iter()
            .any(|s| s.kind == SETTLE_FEE_INVOICE && s.amount == 800)
    );
    let events = c.cancelled.lock().unwrap().clone();
    assert_eq!(events[0].outcomes[0].fee, 800);
}

/// AC4: Cashier A cannot approve.
#[renox::test]
async fn the_requester_cannot_approve_the_cancellation() {
    let (t, c, order, items, sam) = world("online").await;
    c.started.mark(items[0].id);
    let outcome = format!("line_{}_outcome", items[0].id);
    cancel(&t, &order, &[(outcome.as_str(), "return_to_stock")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);

    assert!(
        approvals::approve(t.state(), request.id, &sam, "")
            .await
            .is_err()
    );

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    // A second request of the same kind waits behind the first.
    cancel(&t, &order, &[(outcome.as_str(), "return_to_stock")])
        .await
        .assert_status(422);
}

/// AC5: Given a paid order AED 30 with nothing started (below AED 50), no
/// approval is needed.
#[renox::test]
async fn a_small_paid_order_with_nothing_started_is_cancelled_at_once_and_refunded() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "online", &[("snack", "1", "30.00")]).await;

    cancel(&t, &order, &[]).await.assert_status(303);

    assert!(waiting(&t, &order).await.is_empty());
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Cancelled
    );
    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 3_000);
    let events = c.cancelled.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert!(events[0].outcomes.is_empty());
}

/// Edge (`SAL.ACL.01`): a paid order above the limit needs a manager even
/// when nothing started; the limit is a setting.
#[renox::test]
async fn a_paid_order_above_the_limit_needs_a_manager_even_when_nothing_started() {
    let (t, c, order, _items, _sam) = world("online").await;

    cancel(&t, &order, &[]).await.assert_status(303);

    assert_eq!(waiting(&t, &order).await.len(), 1);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert!(c.cancelled.lock().unwrap().is_empty());
}

/// Edge (`SAL.ACL.01`): the same order under a client whose limit is AED 500
/// needs nobody.
#[renox::test]
async fn the_limit_is_a_setting_of_the_client() {
    let (t, c) = boot_changes_with(WlSales::new().cancel_approval_limit(50_000)).await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "online", &[("meal", "1", "120.00")]).await;
    cancel(&t, &order, &[]).await.assert_status(303);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Cancelled
    );
}

/// Edge: an order on account with nothing started is cancelled at once with
/// no refund, whatever its value.
#[renox::test]
async fn an_unpaid_order_with_nothing_started_is_cancelled_at_once() {
    let (t, c, order, _items, _sam) = world("on_account").await;

    cancel(&t, &order, &[]).await.assert_status(303);

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Cancelled
    );
    assert!(c.gateway.refunds().is_empty());
    assert_eq!(c.cancelled.lock().unwrap().len(), 1);
}

/// Edge: a blank reason is refused.
#[renox::test]
async fn a_blank_reason_is_refused() {
    let (t, _c, order, _items, _sam) = world("on_account").await;
    t.app
        .htmx()
        .post(
            &format!("/sales-orders/{}/cancel", order.id),
            &[("reason", "")],
        )
        .await
        .assert_status(422);
}

/// AC6: Given a line partially fulfilled, `cancel` is not offered.
#[renox::test]
async fn cancel_is_not_offered_once_something_was_handed_over() {
    let (t, c, order, items, _sam) = world("on_account").await;
    hand_over(&t, &order, &items[0], 1_000, 1).await;
    let partly = reload(&t, order.id).await;
    assert_eq!(partly.status, SalesOrderStatus::PartiallyFulfilled);

    cancel(&t, &partly, &[]).await.assert_status(422);

    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::PartiallyFulfilled
    );
    assert!(c.cancelled.lock().unwrap().is_empty());
    // The page says why and points to the remaining lines; the generic
    // transition route cannot run `cancel` either.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Cancel the remaining lines instead")
        .assert_dont_see("Cancel the order");
    t.app
        .post(
            &format!("/sales-orders/{}/transitions/cancel", order.id),
            &[("reason", "x")],
        )
        .await
        .assert_status(422);
}

/// Edge: a started line has no outcome → refused; an outcome the vertical
/// does not offer (06: no *finish to stock*) → refused; a rejected request
/// leaves the order alone.
#[renox::test]
async fn a_started_line_needs_an_offered_outcome_and_a_rejection_changes_nothing() {
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
    let outcome = format!("line_{}_outcome", items[0].id);

    cancel(&t, &order, &[]).await.assert_status(422);
    cancel(&t, &order, &[(outcome.as_str(), "finish_to_stock")])
        .await
        .assert_status(422);
    assert!(waiting(&t, &order).await.is_empty());

    cancel(&t, &order, &[(outcome.as_str(), "return_to_stock")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);
    let mia = manager(&t).await;
    approvals::reject(t.state(), request.id, &mia, "Keep it")
        .await
        .unwrap();
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
    assert!(c.gateway.refunds().is_empty());
    assert!(c.cancelled.lock().unwrap().is_empty());
}

/// Edge: goods handed over while the approval waits make the request fail at
/// the final check (the guards are re-read); the user asks again.
#[renox::test]
async fn goods_handed_over_while_the_request_waits_fail_the_request() {
    let (t, c, order, items, _sam) = world("online").await;
    c.started.mark(items[0].id);
    let outcome = format!("line_{}_outcome", items[0].id);
    cancel(&t, &order, &[(outcome.as_str(), "scrap")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);

    hand_over(&t, &order, &items[1], 1_000, 1).await;
    let mia = manager(&t).await;
    let _ = approvals::approve(t.state(), request.id, &mia, "").await;

    let after = reload(&t, order.id).await;
    assert_eq!(after.status, SalesOrderStatus::PartiallyFulfilled);
    assert!(c.cancelled.lock().unwrap().is_empty());
    assert!(c.gateway.refunds().is_empty());
}
