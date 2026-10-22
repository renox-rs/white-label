//! BL-SAL-05 (#124): an order is amended as a new revision (wl-sales §3
//! *revision*, §4.2 row `amend`, §5.7 steps 1-2; `SAL.ACL.02`, `SAL.ACL.05`,
//! `SAL.RUL.01`). The earlier revision is kept read-only, the credit check
//! re-runs when the value rises, a manager approves after fulfilment started,
//! and aggregator orders are not amended here. Every check drives the real
//! pages, the machine, the kernel approvals and the order events; the money
//! of an amendment is BL-SAL-06 (`order_amend_money.rs`). Orders are made of
//! zero-rated lines so the totals are the figures of the stories.

mod support;

use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalStatus};
use wl_sales::{OrderRevision, SaleLine, SalesOrder, SalesOrderStatus, WlSales};

use support::Booted;
use support::changes::{
    Changes, amend, boot_changes, boot_changes_with, clerk, hand_over, lines, manager, place,
    reload, waiting,
};

/// The credit notice of the sales rep (`SAL.RUL.01`).
const CREDIT_HOLD: &str = "sales.order_credit_hold";

/// A confirmed on-account order of AED 4,000 (10 × AED 400) for a customer
/// with a limit of `limit` fils whose exposure without this order is AED
/// 36,000 (the story's AED 40,000 with this order counted), entered by Sam.
async fn world(limit: i64) -> (Booted, Changes, SalesOrder, SaleLine, User) {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    c.exposure.set(3_600_000, limit);
    let order = place(&t, &c, "phone", "on_account", &[("meal", "10", "400.00")]).await;
    assert_eq!(order.gross_total, 400_000);
    let line = lines(&t, &order).await.remove(0);
    (t, c, order, line, sam)
}

/// AC1: Given a `confirmed` on-account order (rev 1, AED 4,000, nothing
/// started) for a customer with limit AED 50,000 and exposure AED 40,000, when
/// user A changes line 1 qty 10 → 12 with a reason, then rev 2 exists, rev 1 is
/// read-only, 2 more units are reserved, the credit check runs on exposure
/// 40,800 and passes, the order stays `confirmed`, and `SalesOrderAmended`
/// shows 4,000 → 4,800.
#[renox::test]
async fn a_quantity_increase_makes_revision_two_keeps_revision_one_and_passes_the_credit_check() {
    let (t, c, order, line, _sam) = world(5_000_000).await;
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Two more for the party", &[(&quantity, "12")])
        .await
        .assert_status(303);

    let now = reload(&t, order.id).await;
    assert_eq!(now.revision, 2);
    assert_eq!(now.status, SalesOrderStatus::Confirmed);
    assert_eq!(now.gross_total, 480_000);
    assert_eq!(lines(&t, &now).await[0].quantity_milli, 12_000);
    // Revision 1 is kept as it was.
    let kept = OrderRevision::find_of(t.db(), order.id, 1)
        .await
        .unwrap()
        .expect("revision 1 is kept");
    assert_eq!(kept.gross_total, 400_000);
    assert_eq!(kept.reason, "Two more for the party");
    assert_eq!(kept.lines()[0].quantity_milli, 10_000);
    t.app
        .get(&format!("/sales-orders/{}/revisions/1", order.id))
        .await
        .assert_ok()
        .assert_see("cannot be changed");
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Earlier revisions");
    // The event carries the two more units and the value before and after.
    let events = c.amended.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].order.0, order.id);
    assert_eq!(events[0].revision, 2);
    assert_eq!(
        (events[0].value_before, events[0].value_after),
        (400_000, 480_000)
    );
    assert_eq!(
        (
            events[0].lines[0].quantity_before_milli,
            events[0].lines[0].quantity_after_milli
        ),
        (10_000, 12_000)
    );
    assert_eq!(events[0].reason, "Two more for the party");
    // The credit check passed: no hold, no notice.
    assert!(c.mail.of_type(CREDIT_HOLD).is_empty());
    assert_eq!(now.held_from, None);
}

/// AC2: Given the same amendment with limit AED 40,500, the check fails
/// (40,800 > 40,500): the order is `credit_hold` with state before the hold
/// `confirmed`, and `SAL.RUL.01` notifies.
#[renox::test]
async fn an_increase_over_the_credit_limit_puts_the_order_on_hold_and_tells_the_rep() {
    let (t, c, order, line, _sam) = world(4_050_000).await;
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(303);

    let now = reload(&t, order.id).await;
    assert_eq!(now.revision, 2, "the amendment itself stands");
    assert_eq!(now.status, SalesOrderStatus::CreditHold);
    assert_eq!(now.held_from.as_deref(), Some("confirmed"));
    assert_eq!(c.mail.of_type(CREDIT_HOLD).len(), 1);
    assert!(c.desk.opened().iter().any(|task| {
        task.source_key
            .as_deref()
            .is_some_and(|k| k.starts_with("sales.credit_hold"))
    }));
}

/// AC3: Given a reduction to AED 3,600, no credit check runs.
#[renox::test]
async fn a_reduction_runs_no_credit_check() {
    // A limit the order could never pass: a check would put it on hold.
    let (t, c, order, line, _sam) = world(100).await;
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "One less", &[(&quantity, "9")])
        .await
        .assert_status(303);

    let now = reload(&t, order.id).await;
    assert_eq!((now.revision, now.gross_total), (2, 360_000));
    assert_eq!(now.status, SalesOrderStatus::Confirmed);
    assert!(c.mail.of_type(CREDIT_HOLD).is_empty());
}

/// AC4: Given line 1 picked, the same change creates an approval request;
/// user A cannot approve; user B approves → rev 2.
#[renox::test]
async fn after_fulfilment_started_a_manager_other_than_the_requester_approves_the_amendment() {
    let (t, c, order, line, sam) = world(5_000_000).await;
    c.started.mark(line.id);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(303);

    // The revision in force stays until the last yes.
    let still = reload(&t, order.id).await;
    assert_eq!(still.revision, 1);
    assert_eq!(lines(&t, &still).await[0].quantity_milli, 10_000);
    let requests = waiting(&t, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "sales.order_amend");
    assert_eq!(requests[0].requester_id, sam.id);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("A manager must approve");

    // The requester cannot approve; another holder of the permission can.
    assert!(
        approvals::approve(t.state(), requests[0].id, &sam, "")
            .await
            .is_err()
    );
    assert_eq!(reload(&t, order.id).await.revision, 1);
    let mia = manager(&t).await;
    approvals::approve(t.state(), requests[0].id, &mia, "")
        .await
        .unwrap();

    let done = reload(&t, order.id).await;
    assert_eq!(
        (done.revision, done.status),
        (2, SalesOrderStatus::Confirmed)
    );
    assert_eq!(lines(&t, &done).await[0].quantity_milli, 12_000);
    assert_eq!(c.amended.lock().unwrap().len(), 1);
    assert!(waiting(&t, &done).await.is_empty());
}

/// Edge: a rejected request drops the proposal: nothing changes.
#[renox::test]
async fn a_rejected_amendment_changes_nothing() {
    let (t, c, order, line, _sam) = world(5_000_000).await;
    c.started.mark(line.id);
    let quantity = format!("line_{}_quantity", line.id);
    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(303);
    let request = waiting(&t, &order).await.remove(0);

    let mia = manager(&t).await;
    approvals::reject(t.state(), request.id, &mia, "Not now")
        .await
        .unwrap();

    let now = reload(&t, order.id).await;
    assert_eq!((now.revision, now.gross_total), (1, 400_000));
    assert!(c.amended.lock().unwrap().is_empty());
    let stored = wl_kernel::approvals::ApprovalRequest::find(t.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Rejected);
}

/// Edge: a started line is not reduced by `amend`: it goes through
/// `cancel_line`, which asks for an outcome.
#[renox::test]
async fn a_started_line_is_not_reduced_by_an_amendment() {
    let (t, c, order, line, _sam) = world(5_000_000).await;
    c.started.mark(line.id);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "One less", &[(&quantity, "9")])
        .await
        .assert_status(422);

    assert_eq!(reload(&t, order.id).await.revision, 1);
    assert!(waiting(&t, &order).await.is_empty());
}

/// AC5: Given a line with 5 fulfilled, setting it to 4 is refused.
#[renox::test]
async fn a_line_cannot_go_below_what_it_has_fulfilled() {
    let (t, _c, order, line, _sam) = world(5_000_000).await;
    hand_over(&t, &order, &line, 5_000, 1).await;
    let quantity = format!("line_{}_quantity", line.id);
    let order = reload(&t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::PartiallyFulfilled);

    amend(&t, &order, "Four is enough", &[(&quantity, "4")])
        .await
        .assert_status(422);
    // Five is the floor.
    amend(&t, &order, "Five is enough", &[(&quantity, "5")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, order.id).await.revision, 2);
}

/// AC6: Given an aggregator order, `amend` is not offered.
#[renox::test]
async fn an_aggregator_order_is_not_amended_here() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "talabat", "on_account", &[("meal", "10", "400.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(422);

    assert_eq!(reload(&t, order.id).await.revision, 1);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("changed on the aggregator")
        .assert_dont_see("amend-order");
    // A phone order offers it.
    let phone = place(&t, &c, "phone", "on_account", &[("meal", "1", "10.00")]).await;
    t.app
        .get(&format!("/sales-orders/{}", phone.id))
        .await
        .assert_ok()
        .assert_see("amend-order");
}

/// AC7: Given a discount of 12% at a client with limit 10%, `amend` needs a
/// manager under `SAL.ACL.02`.
#[renox::test]
async fn a_discount_beyond_the_limit_needs_a_manager() {
    let (t, c, order, line, _sam) = world(5_000_000).await;
    let discount = format!("line_{}_discount", line.id);

    // 12% of AED 4,000.
    amend(&t, &order, "Loyal customer", &[(&discount, "480.00")])
        .await
        .assert_status(303);
    assert_eq!(reload(&t, order.id).await.revision, 1);
    let request = waiting(&t, &order).await.remove(0);
    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();
    let done = reload(&t, order.id).await;
    assert_eq!((done.revision, done.gross_total), (2, 352_000));
    assert_eq!(c.amended.lock().unwrap().len(), 1);

    // 10% is within the limit: applied at once.
    let again = reload(&t, order.id).await;
    amend(&t, &again, "Ten percent", &[(&discount, "400.00")])
        .await
        .assert_status(303);
    let now = reload(&t, order.id).await;
    assert_eq!(now.revision, 3);
    assert!(waiting(&t, &now).await.is_empty());
}

/// Edge: a client with another limit (5%) asks for a manager earlier.
#[renox::test]
async fn the_discount_limit_is_a_setting() {
    let (t, c) = boot_changes_with(WlSales::new().manual_discount_limit_percent(5)).await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("meal", "10", "400.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let discount = format!("line_{}_discount", line.id);

    amend(&t, &order, "Ten percent", &[(&discount, "400.00")])
        .await
        .assert_status(303);

    assert_eq!(waiting(&t, &order).await.len(), 1);
    assert_eq!(reload(&t, order.id).await.revision, 1);
}

/// Edge: two users amending at once: the second save is refused (the
/// revision changed) and they reload.
#[renox::test]
async fn the_second_of_two_amendments_on_the_same_revision_is_refused() {
    let (t, _c, order, line, _sam) = world(5_000_000).await;
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "First", &[(&quantity, "12")])
        .await
        .assert_status(303);
    // The second form was opened on revision 1.
    amend(&t, &order, "Second", &[(&quantity, "11")])
        .await
        .assert_status(422);

    let now = reload(&t, order.id).await;
    assert_eq!(
        (now.revision, lines(&t, &now).await[0].quantity_milli),
        (2, 12_000)
    );
}

/// Edge: an amendment may add a line; amending while on credit hold keeps the
/// hold; an amendment needs a reason and a change.
#[renox::test]
async fn a_line_is_added_an_order_on_hold_stays_on_hold_and_the_guards_hold() {
    let (t, c, order, line, _sam) = world(4_050_000).await;
    let quantity = format!("line_{}_quantity", line.id);
    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(303);
    let held = reload(&t, order.id).await;
    assert_eq!(held.status, SalesOrderStatus::CreditHold);

    // Amended while on hold: allowed, still on hold.
    amend(
        &t,
        &held,
        "A dessert",
        &[
            ("add_item", "dessert"),
            ("add_quantity", "2"),
            ("add_price", "10.00"),
        ],
    )
    .await
    .assert_status(303);
    let now = reload(&t, order.id).await;
    assert_eq!(
        (now.revision, now.status),
        (3, SalesOrderStatus::CreditHold)
    );
    assert_eq!(lines(&t, &now).await.len(), 2);
    assert_eq!(now.gross_total, 482_000);
    assert_eq!(c.amended.lock().unwrap().len(), 2);

    // No reason, or nothing changed: refused.
    let now = reload(&t, order.id).await;
    amend(&t, &now, "  ", &[(&quantity, "13")])
        .await
        .assert_status(422);
    amend(&t, &now, "Same", &[(&quantity, "12")])
        .await
        .assert_status(422);
    assert_eq!(reload(&t, order.id).await.revision, 3);
}
