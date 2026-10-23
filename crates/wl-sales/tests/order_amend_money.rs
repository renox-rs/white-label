//! BL-SAL-06 (#125): the money of an amendment (wl-sales §4.2 row `amend`,
//! *Amendment of a prepaid order*, §10 *Amended and cancelled orders*). A
//! prepaid increase sends a payment link for the difference and holds the
//! added quantity back until it is paid; a decrease is refunded with the reason
//! *order amended*; a draft invoice is updated, an issued one is credited or
//! topped up with an extra invoice, never edited; an unpaid link gives the
//! order owner a task to resend it or to amend back. Every check drives the
//! real pages, machine, listeners and job; payments and invoicing are the
//! fakes of `support::changes`. Lines are zero-rated so the figures are the
//! stories'.

mod support;

use renox::chrono::TimeDelta;
use renox::db::now;
use renox::prelude::Model;
use wl_kernel::Subject;
use wl_kernel::approvals;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::payments::RefundSource;
use wl_kernel::sales::{FulfilOrder, FulfilledLine, SalesOrderId, order_fulfilment};
use wl_kernel::tickets::Assignee;
use wl_sales::order_change::{
    OrderSettlement, SETTLE_CREDIT_NOTE, SETTLE_DRAFT_UPDATED, SETTLE_EXTRA_INVOICE, SETTLE_LINK,
    SETTLE_REFUND, STATUS_EXPIRED, STATUS_OPEN, STATUS_PAID, STATUS_REVERTED,
};
use wl_sales::{SalesOrder, SalesOrderStatus, WlSales};

use support::changes::{
    amend, boot_changes, boot_changes_with, clerk, lines, manager, place, reload, waiting,
};

/// The notice that asks the customer to pay the difference.
const LINK_NOTICE: &str = "sales.order_amend_link";

/// Sets the invoice of `order` (the invoicing module's id `invoice`).
async fn bill(t: &support::Booted, order: &SalesOrder, invoice: i64) {
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(t.db(), &[("invoice_id", &Some(invoice))])
        .await
        .unwrap();
}

/// Tries to hand over `quantity_milli` of the first line of `order`.
async fn try_hand_over(
    t: &support::Booted,
    order: &SalesOrder,
    quantity_milli: i64,
    n: i64,
) -> bool {
    let line = lines(t, order).await.remove(0);
    order_fulfilment(t.state())
        .fulfil(
            t.state(),
            FulfilOrder {
                order: SalesOrderId(order.id),
                lines: vec![FulfilledLine {
                    line: line.id,
                    quantity_milli,
                }],
                source: Subject::new("inventory.dispatch", n),
            },
        )
        .await
        .is_ok()
}

/// AC1: Given a prepaid order AED 200 amended to AED 260, the customer gets a
/// payment link for AED 60.
#[renox::test]
async fn a_prepaid_increase_sends_a_payment_link_for_the_difference() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "online", &[("meal", "2", "100.00")]).await;
    assert_eq!(order.gross_total, 20_000);
    let original = c.gateway.links().len();
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Six more tenths", &[(&quantity, "2.6")])
        .await
        .assert_status(303);

    let links = c.gateway.links();
    assert_eq!(links.len(), original + 1);
    let link = links.last().unwrap();
    assert_eq!(link.amount, 6_000);
    assert_eq!(link.prepaid, order.subject());
    assert_eq!(link.party.0, 7);
    // A channel other than the website: the link lasts a day.
    assert!(link.expires_at > now() + TimeDelta::hours(23));
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(
        (
            steps[0].kind.as_str(),
            steps[0].status.as_str(),
            steps[0].amount
        ),
        (SETTLE_LINK, STATUS_OPEN, 6_000)
    );
    assert_eq!(steps[0].link_path.as_deref(), Some("/pay/link-2"));
    assert_eq!(c.mail.of_type(LINK_NOTICE).len(), 1);
    // One link, no refund; the order is at revision 2 and AED 260.
    assert!(c.gateway.refunds().is_empty());
    let now_order = reload(&t, order.id).await;
    assert_eq!((now_order.revision, now_order.gross_total), (2, 26_000));
    // The added quantity is reserved but held back from fulfilment.
    assert_eq!(lines(&t, &now_order).await[0].held_milli, 600);
    assert!(!try_hand_over(&t, &now_order, 2_600, 1).await);
    assert!(try_hand_over(&t, &now_order, 2_000, 2).await);
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Payment link for an increase")
        .assert_see("/pay/link-2");
}

/// Paying the link releases the held quantity; the payment is not refunded as
/// a late payment.
#[renox::test]
async fn paying_the_link_releases_the_added_quantity() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "online", &[("meal", "2", "100.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);
    amend(&t, &order, "More", &[(&quantity, "2.6")])
        .await
        .assert_status(303);
    let now_order = reload(&t, order.id).await;

    let paid = c
        .gateway
        .received(&now_order, support::changes::payment_id(), 6_000);
    t.state().emit(paid).await.unwrap();

    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps[0].status, STATUS_PAID);
    assert_eq!(lines(&t, &now_order).await[0].held_milli, 0);
    let released = c.released.lock().unwrap().clone();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].lines[0].quantity_milli, 600);
    assert!(c.gateway.refunds().is_empty());
    assert!(try_hand_over(&t, &now_order, 2_600, 3).await);
}

/// AC2 (other channels): the link of a phone order lasts 24 hours: it is
/// still open after 23, and expired after 24 with the task for the outlet
/// manager.
#[renox::test]
async fn the_link_of_a_phone_order_expires_after_twenty_four_hours() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "online", &[("meal", "2", "100.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);
    amend(&t, &order, "More", &[(&quantity, "2.6")])
        .await
        .assert_status(303);
    let link = c.gateway.links().last().cloned().unwrap();
    assert!(link.expires_at > now() + TimeDelta::hours(23));
    assert!(link.expires_at <= now() + TimeDelta::hours(24) + TimeDelta::minutes(1));

    let early = wl_sales::settle::lapse_links(t.state(), now() + TimeDelta::hours(23))
        .await
        .unwrap();
    assert_eq!(early, 0, "still open after 23 hours");
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps[0].status, STATUS_OPEN);

    let late = wl_sales::settle::lapse_links(t.state(), now() + TimeDelta::hours(24) + TimeDelta::minutes(2))
        .await
        .unwrap();
    assert_eq!(late, 1);
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps[0].status, STATUS_EXPIRED);
    assert!(
        c.desk
            .opened()
            .iter()
            .any(|task| task.title.contains("Payment link expired"))
    );
}

/// AC2: Given a website order whose AED 60 link is not paid within 15 min, the
/// outlet manager gets a "payment link expired" task, the added quantity is
/// still not released to the kitchen, and the order stays at rev 2; choosing
/// *amend back* creates rev 3 at AED 200 with no refund or link.
#[renox::test]
async fn an_unpaid_link_gives_the_outlet_manager_a_task_and_amend_back_removes_the_increase() {
    let (t, c) = boot_changes_with(WlSales::new().order_manager_role("role_olive")).await;
    let olive = t.user("Olive", &[]).await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "website", "online", &[("meal", "2", "100.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);
    amend(&t, &order, "More", &[(&quantity, "2.6")])
        .await
        .assert_status(303);
    let link = c.gateway.links().last().cloned().unwrap();
    assert!(
        link.expires_at <= now() + TimeDelta::minutes(16),
        "the website window"
    );
    let before = c.gateway.links().len();

    // Fifteen minutes pass without a payment.
    let expired = wl_sales::settle::lapse_links(t.state(), now() + TimeDelta::minutes(16))
        .await
        .unwrap();

    assert_eq!(expired, 1);
    let tasks = c.desk.opened();
    let task = tasks
        .iter()
        .find(|task| task.title.contains("Payment link expired"))
        .expect("the owner's task");
    assert!(matches!(task.assignee, Some(Assignee::User { id }) if id == olive.id));
    assert!(task.title.contains(&order.number));
    let now_order = reload(&t, order.id).await;
    assert_eq!(now_order.revision, 2);
    assert_eq!(lines(&t, &now_order).await[0].held_milli, 600);
    assert!(!try_hand_over(&t, &now_order, 2_600, 1).await);
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps[0].status, STATUS_EXPIRED);
    assert_eq!(c.gateway.closed_links().len(), 1);
    // The page offers both ways out.
    t.app
        .get(&format!("/sales-orders/{}", order.id))
        .await
        .assert_ok()
        .assert_see("Resend the link")
        .assert_see("Amend back");

    // Amend back: revision 3 at AED 200, nothing charged or refunded.
    t.app
        .post(
            &format!("/sales-orders/{}/amend-back", order.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    let back = reload(&t, order.id).await;
    assert_eq!((back.revision, back.gross_total), (3, 20_000));
    assert_eq!(lines(&t, &back).await[0].quantity_milli, 2_000);
    assert_eq!(lines(&t, &back).await[0].held_milli, 0);
    assert_eq!(c.gateway.links().len(), before, "no new link");
    assert!(c.gateway.refunds().is_empty(), "no refund");
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, STATUS_REVERTED);
}

/// Edge: *resend link* makes a new link for the same amount; the first is
/// marked resent and paying the new one releases the quantity.
#[renox::test]
async fn resending_the_link_makes_a_new_link_for_the_same_amount() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "website", "online", &[("meal", "2", "100.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);
    amend(&t, &order, "More", &[(&quantity, "2.6")])
        .await
        .assert_status(303);
    wl_sales::settle::lapse_links(t.state(), now() + TimeDelta::minutes(16))
        .await
        .unwrap();

    t.app
        .post(
            &format!("/sales-orders/{}/resend-link", order.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);

    let links = c.gateway.links();
    assert_eq!(links.last().unwrap().amount, 6_000);
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].status, "resent");
    assert_eq!(steps[1].status, STATUS_OPEN);
    assert_eq!(c.mail.of_type(LINK_NOTICE).len(), 2);
    // Nothing to resend once the link is open: the page offers nothing.
    t.app
        .post(
            &format!("/sales-orders/{}/resend-link", order.id),
            &[("x", "")],
        )
        .await
        .assert_status(422);
}

/// AC3: Given it amended to AED 150, a refund request of AED 50 reason *order
/// amended* exists.
#[renox::test]
async fn a_prepaid_decrease_is_refunded_with_the_reason_order_amended() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "online", &[("meal", "2", "100.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);
    let links = c.gateway.links().len();

    amend(&t, &order, "Half a meal less", &[(&quantity, "1.5")])
        .await
        .assert_status(303);

    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 5_000);
    assert_eq!(refunds[0].0.reason, "order amended");
    assert_eq!(refunds[0].0.source, RefundSource::Order(order.subject()));
    assert_eq!(refunds[0].0.party.0, 7);
    assert_eq!(c.gateway.links().len(), links, "a decrease sends no link");
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(
        (steps[0].kind.as_str(), steps[0].amount),
        (SETTLE_REFUND, 5_000)
    );
}

/// Edge: a mixed change nets the difference: one link or one refund, never
/// both.
#[renox::test]
async fn a_mixed_change_is_netted_into_one_link() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(
        &t,
        &c,
        "phone",
        "online",
        &[("meal", "2", "100.00"), ("drink", "2", "10.00")],
    )
    .await;
    assert_eq!(order.gross_total, 22_000);
    let items = lines(&t, &order).await;
    let (meal, drink) = (
        format!("line_{}_quantity", items[0].id),
        format!("line_{}_quantity", items[1].id),
    );
    let links = c.gateway.links().len();

    // Meals +1 (AED 100), drinks -1 (AED 10): net AED 90 more.
    amend(&t, &order, "Swap", &[(&meal, "3"), (&drink, "1")])
        .await
        .assert_status(303);

    assert_eq!(c.gateway.links().len(), links + 1);
    assert_eq!(c.gateway.links().last().unwrap().amount, 9_000);
    assert!(c.gateway.refunds().is_empty());
}

/// Edge: a decrease that needs a manager is not refunded before the
/// amendment is approved; a rejected one has no money effect.
#[renox::test]
async fn nothing_is_refunded_before_the_approval() {
    let (t, c) = boot_changes().await;
    let sam = clerk(&t).await;
    t.app.acting_as(&sam);
    let order = place(&t, &c, "phone", "online", &[("meal", "2", "100.00")]).await;
    let line = lines(&t, &order).await.remove(0);
    c.started.mark(line.id);
    let price = format!("line_{}_price", line.id);

    // A cheaper price on a started line asks a manager.
    amend(&t, &order, "Price match", &[(&price, "75.00")])
        .await
        .assert_status(303);
    assert!(c.gateway.refunds().is_empty());
    let request = waiting(&t, &order).await.remove(0);

    let mia = manager(&t).await;
    approvals::approve(t.state(), request.id, &mia, "")
        .await
        .unwrap();

    let refunds = c.gateway.refunds();
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].0.amount, 5_000);

    // Rejected: no effect.
    let again = reload(&t, order.id).await;
    amend(&t, &again, "Again", &[(&price, "50.00")])
        .await
        .assert_status(303);
    let second = waiting(&t, &again).await.remove(0);
    approvals::reject(t.state(), second.id, &mia, "No")
        .await
        .unwrap();
    assert_eq!(c.gateway.refunds().len(), 1);
}

/// AC4: Given an issued invoice of AED 4,000 and the order amended to AED
/// 3,600, a credit note of AED 400 (incl. VAT) is requested; the invoice is
/// unchanged.
#[renox::test]
async fn an_issued_invoice_is_corrected_by_a_credit_note_and_never_edited() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("meal", "10", "400.00")]).await;
    bill(&t, &order, 101).await;
    c.ledger.set_status(101, InvoiceStatus::Issued);
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "One less", &[(&quantity, "9")])
        .await
        .assert_status(303);

    let credits = c.ledger.credits();
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].0, 101);
    assert_eq!(credits[0].1.len(), 1);
    assert_eq!(
        (credits[0].1[0].invoice_line, credits[0].1[0].net),
        (1, 40_000)
    );
    assert_eq!(credits[0].2, "order amended");
    assert!(
        c.ledger.updates().is_empty(),
        "an issued invoice is never edited"
    );
    assert!(c.ledger.drafts().is_empty());
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(
        (steps[0].kind.as_str(), steps[0].amount),
        (SETTLE_CREDIT_NOTE, 40_000)
    );
    // An on-account order has nothing to refund or charge.
    assert!(c.gateway.refunds().is_empty());
}

/// Edge: value up on an issued invoice: an extra invoice for the added lines.
#[renox::test]
async fn an_increase_on_an_issued_invoice_is_billed_on_an_extra_invoice() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("meal", "10", "400.00")]).await;
    bill(&t, &order, 101).await;
    c.ledger.set_status(101, InvoiceStatus::Issued);
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(303);

    let drafts = c.ledger.drafts();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].1.lines.len(), 1);
    assert_eq!(drafts[0].1.lines[0].net(), 80_000);
    assert_eq!(drafts[0].1.source, order.subject());
    assert!(c.ledger.credits().is_empty());
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(
        (steps[0].kind.as_str(), steps[0].amount),
        (SETTLE_EXTRA_INVOICE, 80_000)
    );
}

/// AC5: Given a draft invoice, it shows the rev 2 lines.
#[renox::test]
async fn a_draft_invoice_is_updated_to_the_new_revision() {
    let (t, c) = boot_changes().await;
    t.app.acting_as(&clerk(&t).await);
    let order = place(&t, &c, "phone", "on_account", &[("meal", "10", "400.00")]).await;
    bill(&t, &order, 102).await;
    let line = lines(&t, &order).await.remove(0);
    let quantity = format!("line_{}_quantity", line.id);

    amend(&t, &order, "Two more", &[(&quantity, "12")])
        .await
        .assert_status(303);

    let updates = c.ledger.updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].0, 102);
    assert_eq!(updates[0].1.lines.len(), 1);
    assert_eq!(updates[0].1.lines[0].net(), 480_000);
    assert!(c.ledger.credits().is_empty() && c.ledger.drafts().is_empty());
    let steps = OrderSettlement::of_order(t.db(), order.id).await.unwrap();
    assert_eq!(steps[0].kind, SETTLE_DRAFT_UPDATED);
    assert_eq!(
        reload(&t, order.id).await.status,
        SalesOrderStatus::Confirmed
    );
}
