//! BL-SAL-14 (#133): refund after completion with nothing returned (wl-sales
//! §4.5 rows `resolve` and `reverse`, *Refund after completion*, *Refunded by
//! the channel*, `SAL.ACL.06`). Staff raise a *no goods* return from a
//! completed order with a complaint category, a responsible party and a refund
//! per line; above the limit a manager who is not the recorder approves; a
//! refund the aggregator made itself is resolved by the system and reversed
//! when the channel takes it back. Every check drives the real pages,
//! machines, kernel approvals and listeners; payments and invoicing are the
//! fakes of `support::changes`. Lines are zero-rated so the figures are the
//! stories'.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::sales::{ReturnResolved, ReturnReversed};
use wl_sales::permissions as perm;
use wl_sales::sales_returns::{self, NewReturn};
use wl_sales::{
    ReturnResolution, ReturnStatus, SaleLine, SalesOrder, SalesOrderStatus, SalesReturn, WlSales,
};

use support::Booted;
use support::changes::{Changes, hand_over, lines, place, reload};

/// The return events a test saw.
#[derive(Clone, Default)]
struct Seen {
    resolved: Arc<Mutex<Vec<ReturnResolved>>>,
    reversed: Arc<Mutex<Vec<ReturnReversed>>>,
}

impl Module for Seen {
    fn name(&self) -> &'static str {
        "return_events"
    }

    fn register(&self, app: &mut Registry) {
        let resolved = self.resolved.clone();
        app.listen(move |event: ReturnResolved, _state: AppState| {
            let resolved = resolved.clone();
            async move {
                resolved.lock().unwrap().push(event);
                Ok(())
            }
        });
        let reversed = self.reversed.clone();
        app.listen(move |event: ReturnReversed, _state: AppState| {
            let reversed = reversed.clone();
            async move {
                reversed.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}

async fn boot() -> (Booted, Changes, Seen) {
    let changes = Changes::default();
    let seen = Seen::default();
    let t = support::boot_pair(WlSales::new(), changes.clone(), seen.clone()).await;
    approvals::install(t.db()).await.unwrap();
    (t, changes, seen)
}

/// Cashier A: enters orders and raises and resolves returns, cannot approve.
async fn cashier(t: &Booted) -> User {
    t.user(
        "Aya",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::SALES_ORDERS_CREATE,
            perm::SALES_ORDERS_ACCEPT,
            perm::RETURNS_REQUEST,
            perm::RETURNS_HANDLE,
        ],
    )
    .await
}

/// Manager B: approves a refund above the limit.
async fn manager(t: &Booted) -> User {
    t.user(
        "Bassem",
        &[
            perm::SALES_ORDERS_VIEW,
            perm::RETURNS_REQUEST,
            perm::RETURNS_HANDLE,
            perm::RETURNS_APPROVE,
        ],
    )
    .await
}

/// A completed (`fulfilled`) order of party 7 on `channel`: `items` are (code,
/// price in AED), one unit each; every line handed over.
async fn completed(
    t: &Booted,
    c: &Changes,
    channel: &str,
    items: &[(&str, &str)],
) -> (SalesOrder, Vec<SaleLine>) {
    let rows: Vec<(&str, &str, &str)> = items
        .iter()
        .map(|(code, price)| (*code, "1", *price))
        .collect();
    let order = place(t, c, channel, "on_account", &rows).await;
    let lines = lines(t, &order).await;
    for (n, line) in lines.iter().enumerate() {
        hand_over(t, &order, line, 1_000, n as i64 + 1).await;
    }
    let order = reload(t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Fulfilled);
    (order, lines)
}

/// Posts the refund form of `order` for `line` with `amount` (AED).
async fn refund(
    t: &Booted,
    order: &SalesOrder,
    line: &SaleLine,
    amount: &str,
    extra: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let key = format!("line_{}_refund", line.id);
    let mut form = vec![
        ("reason", "arrived cold"),
        ("category", "cold"),
        ("party", "driver_or_rider"),
        (key.as_str(), amount),
    ];
    form.extend_from_slice(extra);
    t.app
        .post(&format!("/sales-orders/{}/refund", order.id), &form)
        .await
}

/// The latest return.
async fn latest(t: &Booted) -> SalesReturn {
    SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(t.db())
        .await
        .unwrap()
        .expect("a return was raised")
}

/// The approval requests waiting about the return `r`.
async fn waiting(t: &Booted, r: &SalesReturn) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &r.subject())
        .await
        .unwrap()
}

/// AC1: Given a completed order AED 90 (line pizza AED 60), when cashier A
/// raises a *no goods* return for the pizza, category *cold*, responsible
/// *driver or rider*, refund AED 60, then an approval request is created; A
/// cannot approve.
#[renox::test]
async fn a_refund_above_the_limit_waits_for_a_manager_and_the_recorder_cannot_approve() {
    let (t, c, _seen) = boot().await;
    let aya = cashier(&t).await;
    t.app.acting_as(&aya);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00"), ("cola", "30.00")]).await;
    // The page offers the form on a completed order.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("refund-order");

    refund(&t, &order, &items[0], "60.00", &[])
        .await
        .assert_status(303);

    let raised = latest(&t).await;
    assert_eq!(raised.status, ReturnStatus::Requested);
    assert_eq!(raised.complaint_category.as_deref(), Some("cold"));
    assert_eq!(raised.responsible_party.as_deref(), Some("driver_or_rider"));
    assert_eq!(raised.gross_value, 6_000);
    let requests = waiting(&t, &raised).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "sales.return_refund");
    assert!(
        approvals::approve(t.state(), requests[0].id, &aya, "")
            .await
            .is_err()
    );
    assert!(c.gateway.refunds().is_empty());
    assert_eq!(reload(&t, order.id).await.refunded_amount, 0);
    // The return page says it waits and does not offer to resolve it again.
    t.app
        .get(&format!("/sales-returns/{}", raised.id))
        .await
        .assert_ok()
        .assert_see("A manager must approve this return")
        .assert_dont_see("resolve-sales-return");
}

/// AC2: When manager B approves, the return is `resolved`, a refund of AED 60
/// is requested, the order's amount refunded is AED 60, and `ReturnResolved`
/// carries the category and responsible party.
#[renox::test]
async fn the_approval_resolves_the_return_requests_the_refund_and_carries_the_complaint() {
    let (t, c, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00"), ("cola", "30.00")]).await;
    refund(&t, &order, &items[0], "60.00", &[])
        .await
        .assert_status(303);
    let raised = latest(&t).await;
    let request = waiting(&t, &raised).await.remove(0);

    let bassem = manager(&t).await;
    approvals::approve(t.state(), request.id, &bassem, "")
        .await
        .unwrap();

    let done = SalesReturn::find_or_404(t.db(), raised.id).await.unwrap();
    assert_eq!(done.status, ReturnStatus::Resolved);
    assert_eq!(done.resolution, Some(ReturnResolution::Refund));
    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 6_000);
    assert_eq!(reload(&t, order.id).await.refunded_amount, 6_000);
    let events = seen.resolved.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].kind, "no_goods");
    assert_eq!(events[0].resolution, "refund");
    assert_eq!(events[0].value, 6_000);
    assert_eq!(events[0].complaint_category.as_deref(), Some("cold"));
    assert_eq!(
        events[0].responsible_party.as_deref(),
        Some("driver_or_rider")
    );
    // The order stays as it was: fulfilled.
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Fulfilled
    );
    // The order page shows the amount refunded.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("AED 60.00");
}

/// AC1 (complaint ticket): a refund raised from complaint ticket 77 stores the
/// ticket on the return, and when it is resolved the ticket gets an internal
/// comment with the return number, the resolution and the value.
#[renox::test]
async fn a_refund_raised_from_a_complaint_ticket_comments_on_the_ticket() {
    let (t, c, _seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00"), ("cola", "30.00")]).await;

    refund(&t, &order, &items[1], "30.00", &[("ticket", "77")])
        .await
        .assert_status(303);

    let raised = latest(&t).await;
    assert_eq!(raised.ticket_id, Some(77));
    assert_eq!(raised.status, ReturnStatus::Resolved, "within the limit");
    let comments = c.desk.comments();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].0, 77);
    assert!(comments[0].1.contains(&raised.number), "{}", comments[0].1);
    assert!(comments[0].1.contains("AED 30.00"), "{}", comments[0].1);

    // A ticket that is not a number is refused and nothing is raised.
    let before = SalesReturn::query().count(t.db()).await.unwrap();
    let key = format!("line_{}_refund", items[0].id);
    t.app
        .htmx()
        .post(
            &format!("/sales-orders/{}/refund", order.id),
            &[
                ("reason", "cold"),
                ("category", "cold"),
                ("party", "business"),
                ("ticket", "abc"),
                (key.as_str(), "10.00"),
            ],
        )
        .await
        .assert_status(422);
    assert_eq!(SalesReturn::query().count(t.db()).await.unwrap(), before);
}

/// AC3: Given a refund of AED 30, it resolves without approval.
#[renox::test]
async fn a_refund_within_the_limit_resolves_at_once() {
    let (t, c, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00"), ("cola", "30.00")]).await;

    refund(&t, &order, &items[1], "30.00", &[])
        .await
        .assert_status(303);

    let done = latest(&t).await;
    assert_eq!(done.status, ReturnStatus::Resolved);
    assert!(waiting(&t, &done).await.is_empty());
    assert_eq!(c.gateway.refunds().len(), 1);
    assert_eq!(c.gateway.refunds()[0].0.amount, 3_000);
    assert_eq!(reload(&t, order.id).await.refunded_amount, 3_000);
    assert_eq!(seen.resolved.lock().unwrap().len(), 1);
}

/// AC4: Without a complaint category, `resolve` is refused.
#[renox::test]
async fn without_a_complaint_category_the_refund_is_refused() {
    let (t, c, _seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00")]).await;

    // Through the order's form: the category is a required field.
    let key = format!("line_{}_refund", items[0].id);
    t.app
        .htmx()
        .post(
            &format!("/sales-orders/{}/refund", order.id),
            &[
                ("reason", "cold"),
                ("category", ""),
                ("party", "business"),
                (key.as_str(), "20.00"),
            ],
        )
        .await
        .assert_status(422);
    // A category outside the list of the settings is refused too.
    let key = format!("line_{}_refund", items[0].id);
    t.app
        .htmx()
        .post(
            &format!("/sales-orders/{}/refund", order.id),
            &[
                ("reason", "arrived cold"),
                ("category", "bad_mood"),
                ("party", "driver_or_rider"),
                (key.as_str(), "20.00"),
            ],
        )
        .await
        .assert_status(422);

    // And a no goods return recorded without one is not resolved by a handler.
    let raised = sales_returns::request(
        t.state(),
        NewReturn::no_goods(order.subject())
            .refund_line(items[0].id, 2_000)
            .reason("cold"),
    )
    .await
    .unwrap();
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", raised.id),
            &[("resolution", "refund")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        SalesReturn::find_or_404(t.db(), raised.id)
            .await
            .unwrap()
            .status,
        ReturnStatus::Requested
    );
    assert!(c.gateway.refunds().is_empty());
}

/// AC5: Given Talabat order ORD-1001 (AED 25 line), when the system resolves a
/// *no goods* return of AED 25 *refunded by the channel*, category *cold*,
/// party *business*, no approval, refund or credit note is created and the
/// amount refunded is AED 25.
#[renox::test]
async fn a_channel_refund_moves_no_money_and_needs_no_approval() {
    let (t, c, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "talabat", &[("wrap", "25.00")]).await;

    let done = sales_returns::refund_by_channel(
        t.state(),
        NewReturn::no_goods(order.subject())
            .refund_line(items[0].id, 2_500)
            .complaint("cold", "business")
            .reason("Talabat refunded the customer"),
    )
    .await
    .unwrap();

    assert_eq!(done.status, ReturnStatus::Resolved);
    assert_eq!(done.resolution, Some(ReturnResolution::RefundedByChannel));
    assert!(waiting(&t, &done).await.is_empty());
    assert!(c.gateway.refunds().is_empty());
    assert!(c.ledger.credits().is_empty());
    assert_eq!(reload(&t, order.id).await.refunded_amount, 2_500);
    let events = seen.resolved.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].resolution, "refunded_by_channel");
    assert_eq!(events[0].responsible_party.as_deref(), Some("business"));
}

/// AC6: *Refunded by the channel* on a counter-channel order is refused.
#[renox::test]
async fn a_channel_refund_on_a_counter_order_is_refused() {
    let (t, c, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("wrap", "25.00")]).await;

    let refused = sales_returns::refund_by_channel(
        t.state(),
        NewReturn::no_goods(order.subject())
            .refund_line(items[0].id, 2_500)
            .complaint("cold", "business"),
    )
    .await;

    assert!(refused.is_err());
    assert_eq!(reload(&t, order.id).await.refunded_amount, 0);
    assert!(seen.resolved.lock().unwrap().is_empty());
    // A handler cannot choose it for that order either.
    let raised = latest(&t).await;
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", raised.id),
            &[
                ("resolution", "refunded_by_channel"),
                ("complaint_category", "cold"),
                ("responsible_party", "business"),
            ],
        )
        .await
        .assert_status(422);
    assert_eq!(
        SalesReturn::find_or_404(t.db(), raised.id)
            .await
            .unwrap()
            .status,
        ReturnStatus::Requested
    );
}

/// AC7: `reverse` of that return with reason "statement voided" makes it
/// `reversed`, the amount refunded goes back to 0 and `ReturnReversed` is
/// emitted; `reverse` of a return resolved by refund is refused.
#[renox::test]
async fn a_channel_refund_is_reversed_and_a_paid_refund_is_not() {
    let (t, c, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) =
        completed(&t, &c, "talabat", &[("wrap", "25.00"), ("cola", "30.00")]).await;
    let by_channel = sales_returns::refund_by_channel(
        t.state(),
        NewReturn::no_goods(order.subject())
            .refund_line(items[0].id, 2_500)
            .complaint("cold", "business"),
    )
    .await
    .unwrap();
    // A refund of AED 30 by the business, resolved by refund.
    refund(&t, &order, &items[1], "30.00", &[("party", "business")])
        .await
        .assert_status(303);
    let paid = latest(&t).await;
    assert_eq!(paid.status, ReturnStatus::Resolved);
    assert_eq!(reload(&t, order.id).await.refunded_amount, 5_500);

    assert!(
        sales_returns::reverse_by_channel(t.state(), paid.id, "statement voided")
            .await
            .is_err()
    );
    t.app
        .post(
            &format!("/sales-returns/{}/transitions/reverse", paid.id),
            &[("reason", "oops")],
        )
        .await
        .assert_status(422);
    // A reason is needed.
    assert!(
        sales_returns::reverse_by_channel(t.state(), by_channel.id, " ")
            .await
            .is_err()
    );

    let reversed = sales_returns::reverse_by_channel(t.state(), by_channel.id, "statement voided")
        .await
        .unwrap();

    assert_eq!(reversed.status, ReturnStatus::Reversed);
    assert_eq!(reversed.closing_reason.as_deref(), Some("statement voided"));
    assert_eq!(reload(&t, order.id).await.refunded_amount, 3_000);
    let events = seen.reversed.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].value, events[0].reason.as_str()),
        (2_500, "statement voided")
    );
    // Reversing twice is refused: `reversed` is final.
    assert!(
        sales_returns::reverse_by_channel(t.state(), by_channel.id, "again")
            .await
            .is_err()
    );
    // The handler's page reverses it too, with a reason.
    let (order2, items2) = completed(&t, &c, "talabat", &[("wrap", "25.00")]).await;
    let again = sales_returns::refund_by_channel(
        t.state(),
        NewReturn::no_goods(order2.subject())
            .refund_line(items2[0].id, 2_500)
            .complaint("late", "aggregator"),
    )
    .await
    .unwrap();
    t.app
        .post(
            &format!("/sales-returns/{}/transitions/reverse", again.id),
            &[("reason", "")],
        )
        .await
        .assert_status(422);
    t.app
        .post(
            &format!("/sales-returns/{}/transitions/reverse", again.id),
            &[("reason", "dispute won by the aggregator")],
        )
        .await
        .assert_status(303);
    assert_eq!(reload(&t, order2.id).await.refunded_amount, 0);
}

/// AC8: Given AED 60 already refunded on the pizza line, a second *no goods*
/// return of AED 10 on the same line is refused (above the line value of
/// AED 60); the order's amount refunded does not change.
#[renox::test]
async fn a_second_refund_above_the_line_value_is_refused() {
    let (t, c, _seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00"), ("cola", "30.00")]).await;
    let aya_request = refund(&t, &order, &items[0], "60.00", &[]).await;
    aya_request.assert_status(303);
    let first = latest(&t).await;
    let request = waiting(&t, &first).await.remove(0);
    approvals::approve(t.state(), request.id, &manager(&t).await, "")
        .await
        .unwrap();
    assert_eq!(reload(&t, order.id).await.refunded_amount, 6_000);

    refund(&t, &order, &items[0], "10.00", &[])
        .await
        .assert_status(422);

    assert_eq!(reload(&t, order.id).await.refunded_amount, 6_000);
    assert_eq!(latest(&t).await.id, first.id, "no second return was made");
    // A partial refund leaves room: AED 20 of the AED 30 drink, then AED 15
    // more is above what is left.
    refund(&t, &order, &items[1], "20.00", &[])
        .await
        .assert_status(303);
    refund(&t, &order, &items[1], "15.00", &[])
        .await
        .assert_status(422);
}

/// Edge: a rejected approval leaves the return `requested` and nothing is
/// refunded; the settings' limit decides what needs a manager.
#[renox::test]
async fn a_rejected_approval_leaves_the_return_requested() {
    let (t, c, seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00")]).await;
    refund(&t, &order, &items[0], "60.00", &[])
        .await
        .assert_status(303);
    let raised = latest(&t).await;
    let request = waiting(&t, &raised).await.remove(0);

    approvals::reject(
        t.state(),
        request.id,
        &manager(&t).await,
        "Not the driver's fault",
    )
    .await
    .unwrap();

    let after = SalesReturn::find_or_404(t.db(), raised.id).await.unwrap();
    assert_eq!(after.status, ReturnStatus::Requested);
    assert!(c.gateway.refunds().is_empty());
    assert_eq!(reload(&t, order.id).await.refunded_amount, 0);
    assert!(seen.resolved.lock().unwrap().is_empty());
}

/// Edge: the limit is a setting of the client.
#[renox::test]
async fn the_limit_is_a_setting_of_the_client() {
    let changes = Changes::default();
    let t = support::boot_pair(
        WlSales::new().refund_approval_limit(10_000),
        changes.clone(),
        Seen::default(),
    )
    .await;
    approvals::install(t.db()).await.unwrap();
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &changes, "phone", &[("pizza", "60.00")]).await;

    refund(&t, &order, &items[0], "60.00", &[])
        .await
        .assert_status(303);

    assert_eq!(latest(&t).await.status, ReturnStatus::Resolved);
    assert_eq!(changes.gateway.refunds().len(), 1);
}

/// Edge: a user who may request but not handle returns only records the
/// return; a handler resolves it (and a manager is asked above the limit).
#[renox::test]
async fn someone_who_cannot_handle_returns_only_records_the_refund() {
    let (t, c, _seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let (order, items) = completed(&t, &c, "phone", &[("pizza", "60.00")]).await;
    let clerk = t
        .user("Omar", &[perm::SALES_ORDERS_VIEW, perm::RETURNS_REQUEST])
        .await;
    t.app.acting_as(&clerk);

    refund(&t, &order, &items[0], "20.00", &[])
        .await
        .assert_status(303);

    let raised = latest(&t).await;
    assert_eq!(raised.status, ReturnStatus::Requested);
    assert!(waiting(&t, &raised).await.is_empty());
    assert!(c.gateway.refunds().is_empty());
    // He cannot resolve it; the handler can.
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", raised.id),
            &[("resolution", "refund")],
        )
        .await
        .assert_forbidden();
    t.app.acting_as(&manager(&t).await);
    t.app
        .post(
            &format!("/sales-returns/{}/resolve", raised.id),
            &[
                ("resolution", "refund"),
                ("complaint_category", "cold"),
                ("responsible_party", "driver_or_rider"),
            ],
        )
        .await
        .assert_status(303);
    assert_eq!(latest(&t).await.status, ReturnStatus::Resolved);
    assert_eq!(c.gateway.refunds().len(), 1);
}

/// Edge: an order that was not handed over has no refund after completion.
#[renox::test]
async fn an_order_not_handed_over_cannot_be_refunded_this_way() {
    let (t, c, _seen) = boot().await;
    t.app.acting_as(&cashier(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("pizza", "1", "60.00")]).await;
    let items = lines(&t, &order).await;

    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("offered once the order was handed over")
        .assert_dont_see("refund-order");
    refund(&t, &order, &items[0], "20.00", &[])
        .await
        .assert_status(422);
}
