//! Revising an issued purchase order (wl-inventory §4 *Purchase order*
//! `revise`, `STK.ACL.01`, `STK.ACL.03`, issue #227): a new revision number,
//! the earlier one kept read-only, the supplier told, committed cost
//! following, and a higher total above a limit waiting for approval while the
//! current revision stays valid.

mod support;

use renox::prelude::*;
use wl_inventory::po_revise::revisions_of;
use wl_inventory::purchase_order::{committed_value, lines_of};
use wl_inventory::{Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, SUPPLIER, boot};

/// An `issued` order of 10 units at AED 900.00 (AED 9,000.00).
async fn nine_thousand(t: &Booted) -> (Item, Location, PurchaseOrder, PurchaseOrderLine) {
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 10_000, 90_000).await;
    (item, store, order, line)
}

async fn set_status(t: &Booted, order: &PurchaseOrder, status: PurchaseOrderStatus) {
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(t.db(), &[("status", &status)])
        .await
        .unwrap();
}

async fn pending(t: &Booted, order: &PurchaseOrder) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &order.subject())
        .await
        .unwrap()
}

/// The notices to the supplier about a revision (the approvers' notices are
/// not among them).
fn sent(t: &Booted) -> Vec<wl_kernel::notify::NotifyRequest> {
    t.notices
        .all()
        .into_iter()
        .filter(|n| n.type_key == "inventory.purchase_order_revised")
        .collect()
}

async fn committed(t: &Booted, order: &PurchaseOrder) -> i64 {
    committed_value(&lines_of(t.db(), order.id).await.unwrap())
}

/// Done when 1: an `issued` PO rev 1 of AED 9,000, user A raises a line price
/// so the total is AED 12,000: an approval request is created, rev 1 stays
/// current, A cannot approve.
#[renox::test]
async fn a_total_above_the_limit_waits_and_the_current_revision_stays() {
    let t = boot().await;
    let (_item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;

    t.revise_as(&a, &order, "price rise", &[(&line, "10", "1200")], &[])
        .await
        .assert_ok();

    let now = t.order_now(&order).await;
    assert_eq!((now.revision, now.status), (1, PurchaseOrderStatus::Issued));
    assert_eq!(t.line_now(&line).await.unit_price, 90_000);
    assert_eq!(committed(&t, &order).await, 900_000);
    let requests = pending(&t, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "inventory.purchase_order_revise");
    assert_eq!(requests[0].transition, "revise");
    assert_eq!(requests[0].requester_id, a.id);
    // One step: AED 12,000 is above 10,000 and below 50,000.
    assert_eq!(requests[0].steps.0.len(), 1);
    // Nothing is sent while it waits, and the creator cannot approve.
    assert!(t.revised.all().is_empty());
    assert!(sent(&t).is_empty());
    let refused = approvals::approve(t.state(), requests[0].id, &a, "").await;
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(t.order_now(&order).await.revision, 1);
    // A second revision is refused while one waits.
    t.revise_as(&a, &order, "again", &[(&line, "10", "1300")], &[])
        .await
        .assert_status(422);
}

/// Done when 2: B approves, rev 2 is current, rev 1 is read-only, the PDF
/// "Revision 2" is sent, committed cost +3,000, `PurchaseOrderRevised` is
/// emitted.
#[renox::test]
async fn the_approval_puts_revision_two_in_force() {
    let t = boot().await;
    let (item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;
    let b = t.procurement("Bilal").await;
    t.revise_as(&a, &order, "price rise", &[(&line, "10", "1200")], &[])
        .await
        .assert_ok();
    let requests = pending(&t, &order).await;

    approvals::approve(t.state(), requests[0].id, &b, "agreed")
        .await
        .unwrap();

    let now = t.order_now(&order).await;
    assert_eq!((now.revision, now.status), (2, PurchaseOrderStatus::Issued));
    assert_eq!(t.line_now(&line).await.unit_price, 120_000);
    // AED 3,000 more is committed.
    assert_eq!(committed(&t, &order).await, 1_200_000);
    // Revision 1 is kept as it was.
    let kept = revisions_of(t.db(), order.id).await.unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].revision, 1);
    assert_eq!(kept[0].total, 900_000);
    assert_eq!(kept[0].lines().len(), 1);
    assert_eq!(kept[0].lines()[0].unit_price, 90_000);
    // The document is marked.
    let document = now.document.clone().expect("a document");
    assert!(document.contains("Revision 2"), "{document}");
    assert!(document.contains("Total: 12,000.00 AED"), "{document}");
    // The event.
    let events = t.revised.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].purchase_order, order.subject());
    assert_eq!(events[0].revision, 2);
    assert_eq!(
        (events[0].old_total, events[0].new_total),
        (900_000, 1_200_000)
    );
    assert_eq!(events[0].reason, "price rise");
    assert_eq!(events[0].lines.len(), 1);
    assert_eq!(
        events[0].lines[0].item.to_string(),
        format!("inventory.item:{}", item.id)
    );
    assert_eq!(
        (
            events[0].lines[0].old_unit_price,
            events[0].lines[0].new_unit_price
        ),
        (90_000, 120_000)
    );
    // The supplier got the revised document.
    let notices = sent(&t);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0]
            .recipients
            .iter()
            .any(|r| matches!(r, NotifyRecipient::Party { id } if *id == SUPPLIER))
    );
    // The document of each revision is served.
    t.app.acting_as(&b);
    t.app
        .get(&format!(
            "/purchase-orders/{}/revisions/2/document",
            order.id
        ))
        .await
        .assert_ok()
        .assert_see("Revision 2");
    t.app
        .get(&format!(
            "/purchase-orders/{}/revisions/3/document",
            order.id
        ))
        .await
        .assert_status(404);
}

/// Done when 3: a line with 30 accepted cannot be reduced to 25; to 30 it
/// can, and the line is then fully received.
#[renox::test]
async fn a_line_cannot_go_below_what_it_accepted() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, mut line) = t.order(&store, &item, 50_000, 10_000).await;
    line.accepted_qty_milli = 30_000;
    line.save(t.db()).await.unwrap();
    set_status(&t, &order, PurchaseOrderStatus::PartiallyReceived).await;
    let a = t.procurement("Amal").await;

    t.revise_as(&a, &order, "less", &[(&line, "25", "100")], &[])
        .await
        .assert_status(422);
    assert_eq!(t.line_now(&line).await.quantity_milli, 50_000);
    assert_eq!(t.order_now(&order).await.revision, 1);

    t.revise_as(&a, &order, "less", &[(&line, "30", "100")], &[])
        .await
        .assert_ok();
    let now = t.order_now(&order).await;
    assert_eq!(
        (now.revision, now.status),
        (2, PurchaseOrderStatus::PartiallyReceived)
    );
    assert_eq!(t.line_now(&line).await.quantity_milli, 30_000);
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 0);
}

/// Done when 4: a lower total applies without approval, even above a limit.
#[renox::test]
async fn a_lower_total_applies_at_once() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    // AED 60,000 down to AED 55,000: both above the second limit.
    let (order, line) = t.order(&store, &item, 10_000, 600_000).await;
    let a = t.procurement("Amal").await;

    t.revise_as(&a, &order, "discount", &[(&line, "10", "5500")], &[])
        .await
        .assert_ok();

    assert!(pending(&t, &order).await.is_empty());
    assert_eq!(t.order_now(&order).await.revision, 2);
    assert_eq!(committed(&t, &order).await, 5_500_000);
    assert_eq!(t.revised.all().len(), 1);
}

/// Done when 5: AED 9,000 to AED 9,500 stays below the first limit (10,000)
/// and applies at once as rev 2.
#[renox::test]
async fn a_higher_total_below_the_first_limit_applies_at_once() {
    let t = boot().await;
    let (_item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;

    t.revise_as(&a, &order, "freight", &[(&line, "10", "950")], &[])
        .await
        .assert_ok();

    assert!(pending(&t, &order).await.is_empty());
    let now = t.order_now(&order).await;
    assert_eq!(now.revision, 2);
    assert_eq!(committed(&t, &order).await, 950_000);
    assert_eq!(revisions_of(t.db(), order.id).await.unwrap().len(), 1);
}

/// The new total decides the steps: AED 40,000 to 60,000 needs both, and
/// AED 12,000 to 13,000 needs the first step again even though the earlier
/// revision was approved.
#[renox::test]
async fn the_new_total_decides_the_steps() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 10_000, 400_000).await;
    let a = t.procurement("Amal").await;
    let b = t.procurement("Bilal").await;
    let c = t.procurement("Chen").await;

    t.revise_as(&a, &order, "steel", &[(&line, "10", "6000")], &[])
        .await
        .assert_ok();
    let requests = pending(&t, &order).await;
    assert_eq!(requests[0].steps.0.len(), 2);
    approvals::approve(t.state(), requests[0].id, &b, "")
        .await
        .unwrap();
    assert_eq!(t.order_now(&order).await.revision, 1);
    approvals::approve(t.state(), requests[0].id, &c, "")
        .await
        .unwrap();
    assert_eq!(t.order_now(&order).await.revision, 2);
    assert_eq!(committed(&t, &order).await, 6_000_000);

    // Down to 12,000 at once, then up to 13,000: the first step again.
    t.revise_as(&a, &order, "cheaper", &[(&line, "10", "1200")], &[])
        .await
        .assert_ok();
    assert_eq!(t.order_now(&order).await.revision, 3);
    t.revise_as(&a, &order, "back up", &[(&line, "10", "1300")], &[])
        .await
        .assert_ok();
    let requests = pending(&t, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].steps.0.len(), 1);
    assert_eq!(t.order_now(&order).await.revision, 3);
}

/// The currency cannot change, a reason is needed and something must change.
#[renox::test]
async fn the_currency_the_reason_and_a_change_are_needed() {
    let t = boot().await;
    let (_item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;

    t.revise_as(
        &a,
        &order,
        "to dollars",
        &[(&line, "10", "950")],
        &[("currency", "USD")],
    )
    .await
    .assert_status(422);
    t.revise_as(&a, &order, "  ", &[(&line, "10", "950")], &[])
        .await
        .assert_status(422);
    t.revise_as(&a, &order, "nothing", &[(&line, "10", "900")], &[])
        .await
        .assert_status(422);
    assert_eq!(t.order_now(&order).await.revision, 1);
    assert!(revisions_of(t.db(), order.id).await.unwrap().is_empty());
}

/// A later expected date says who asked for it and is kept on the order.
#[renox::test]
async fn a_later_date_records_who_asked() {
    let t = boot().await;
    let (_item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;
    let date = ("expected_date", "2027-01-31");

    t.revise_as(&a, &order, "late", &[(&line, "10", "880")], &[date])
        .await
        .assert_status(422);
    t.revise_as(
        &a,
        &order,
        "late",
        &[(&line, "10", "880")],
        &[date, ("requested_by", "supplier")],
    )
    .await
    .assert_ok();

    let now = t.order_now(&order).await;
    assert_eq!(now.revision, 2);
    assert_eq!(
        now.expected_date.map(|d| d.to_string()).as_deref(),
        Some("2027-01-31")
    );
    assert_eq!(now.date_requested_by.as_deref(), Some("supplier"));
}

/// A blocked supplier refuses a higher total and a later date, not a lower
/// total (`PTY.ACL.03`).
#[renox::test]
async fn a_blocked_supplier_refuses_an_extension_only() {
    let t = boot().await;
    let (_item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;
    t.blocked.block(SUPPLIER);

    t.revise_as(&a, &order, "freight", &[(&line, "10", "950")], &[])
        .await
        .assert_status(422);
    t.revise_as(
        &a,
        &order,
        "late",
        &[(&line, "10", "880")],
        &[("expected_date", "2027-01-31"), ("requested_by", "us")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.order_now(&order).await.revision, 1);

    t.revise_as(&a, &order, "discount", &[(&line, "10", "850")], &[])
        .await
        .assert_ok();
    assert_eq!(t.order_now(&order).await.revision, 2);
}

/// Goods accepted while the approval waits: at approval the line may no
/// longer go below what it accepted, and the request fails.
#[renox::test]
async fn goods_accepted_while_it_waits_fail_the_request() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    // 50 units at AED 300 (AED 15,000); the revision asks for 40 at AED 400.
    let (order, mut line) = t.order(&store, &item, 50_000, 30_000).await;
    let a = t.procurement("Amal").await;
    let b = t.procurement("Bilal").await;
    t.revise_as(&a, &order, "fewer, dearer", &[(&line, "40", "400")], &[])
        .await
        .assert_ok();
    let requests = pending(&t, &order).await;
    assert_eq!(requests.len(), 1);
    line.accepted_qty_milli = 45_000;
    line.save(t.db()).await.unwrap();

    approvals::approve(t.state(), requests[0].id, &b, "")
        .await
        .unwrap();

    let request = ApprovalRequest::find_or_404(t.db(), requests[0].id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Failed);
    assert_eq!(t.order_now(&order).await.revision, 1);
    assert_eq!(t.line_now(&line).await.quantity_milli, 50_000);
    assert!(t.revised.all().is_empty());
}

/// Only `issued` and `partially_received` orders are revised, and only by a
/// holder of `purchase_orders.issue`.
#[renox::test]
async fn only_an_open_order_is_revised_by_a_purchaser() {
    let t = boot().await;
    let (_item, _store, order, line) = nine_thousand(&t).await;
    let a = t.procurement("Amal").await;
    let clerk = t.storekeeper("Sam").await;

    t.revise_as(&clerk, &order, "freight", &[(&line, "10", "950")], &[])
        .await
        .assert_status(403);
    set_status(&t, &order, PurchaseOrderStatus::Received).await;
    t.revise_as(&a, &order, "freight", &[(&line, "10", "950")], &[])
        .await
        .assert_status(422);
    assert_eq!(t.order_now(&order).await.revision, 1);
}
