//! Refusing a new or extended purchase order to a blocked supplier
//! (wl-inventory §4 *Purchase order* `issue` and `revise`, `PTY.ACL.03`,
//! issue #246): `issue` calls the block guard before anything else and is
//! refused with the neutral *on hold* text unless an exception is approved;
//! a goods receipt against an order already issued is never refused.

mod support;

use renox::prelude::*;
use renox::serde_json::json;
use wl_inventory::PurchaseOrderStatus;
use wl_inventory::{DraftLine, Item, Location, PurchaseOrder, PurchaseOrderLine};
use wl_kernel::BLOCK_EXCEPTION_ACTION;
use wl_kernel::approvals::{self, ApprovalPolicy, ApprovalRequest, ApprovalStatus};

use support::{Booted, SUPPLIER, boot};

/// A draft order PO-310 of AED 4,000 (10 units at AED 400).
async fn po_310(t: &Booted) -> (Item, Location, PurchaseOrder, PurchaseOrderLine) {
    let item = t.item("PARTS").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.draft_order(&store, &item, 10_000, 40_000).await;
    (item, store, order, line)
}

async fn pending(t: &Booted, order: &PurchaseOrder) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &order.subject())
        .await
        .unwrap()
}

/// Done when 1: the supplier is blocked for `fraud_suspected`; `issue` of
/// PO-310 (AED 4,000) by procurement user A is refused with *on hold* and the
/// option to request an exception; PO-310 stays `draft`, with no number and
/// no approval request for the order itself.
#[renox::test]
async fn a_blocked_supplier_refuses_issue_with_on_hold_and_an_exception_offer() {
    let t = boot().await;
    let (_item, _store, order, _line) = po_310(&t).await;
    let a = t.procurement("Amal").await;
    t.blocked.block_for(SUPPLIER, "Fraud suspected");

    let res = t.issue_as(&a, &order, &[]).await;
    res.assert_status(422);
    let body = res.text();
    assert!(body.contains("This party is on hold"), "{body}");
    assert!(!body.contains("Fraud"), "{body}");
    assert!(body.contains("Request exception"), "{body}");
    assert!(body.contains(r#"name="block_exception""#), "{body}");

    let now = t.order_now(&order).await;
    assert_eq!(now.status, PurchaseOrderStatus::Draft);
    assert_eq!(now.number, None);
    assert!(pending(&t, &order).await.is_empty());
}

/// The exception policy is stored by `wl:install` in every boot, whichever
/// test of the binary boots first (the defaults are process-wide, so a boot
/// that registered it after the install depended on a sibling test).
#[renox::test]
async fn every_boot_stores_the_exception_policy() {
    let t = boot().await;
    let policy = ApprovalPolicy::for_action(t.db(), BLOCK_EXCEPTION_ACTION, &json!({}))
        .await
        .unwrap();
    assert!(policy.is_some());
}

/// Done when 2: after the finance manager (screening off) approves the
/// exception, PO-310 is issued once; the requester cannot approve it.
#[renox::test]
async fn an_approved_exception_issues_the_order_once() {
    let t = boot().await;
    let (_item, _store, order, _line) = po_310(&t).await;
    let a = t.procurement("Amal").await;
    let fatima = t.user_with("Fatima", "finance_manager").await;
    t.blocked.block(SUPPLIER);

    t.issue_as(&a, &order, &[("block_exception", "1")])
        .await
        .assert_ok();
    let requests = pending(&t, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(requests[0].transition, "issue");
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Draft);

    // The requester does not decide it.
    let refused = approvals::approve(t.state(), requests[0].id, &a, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Draft);

    approvals::approve(t.state(), requests[0].id, &fatima, "approved")
        .await
        .unwrap();
    let issued = t.order_now(&order).await;
    assert_eq!(issued.status, PurchaseOrderStatus::Issued);
    assert!(
        issued
            .number
            .as_deref()
            .is_some_and(|n| n.starts_with("PO-")),
        "{:?}",
        issued.number
    );
    assert!(issued.document.is_some());
    let request = ApprovalRequest::find_or_404(t.db(), requests[0].id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Approved);
    assert!(pending(&t, &order).await.is_empty());

    // Once: the order is not a draft any more, so it cannot be issued again.
    t.issue_as(&a, &order, &[]).await.assert_status(422);
    assert_eq!(t.order_now(&order).await.number, issued.number);
}

/// An exception is only asked for a blocked supplier, and only by a holder of
/// `purchase_orders.issue`.
#[renox::test]
async fn an_exception_needs_a_block_and_the_permission() {
    let t = boot().await;
    let (_item, _store, order, _line) = po_310(&t).await;
    let a = t.procurement("Amal").await;
    let clerk = t.storekeeper("Sam").await;

    t.issue_as(&a, &order, &[("block_exception", "1")])
        .await
        .assert_status(422);
    t.blocked.block(SUPPLIER);
    t.issue_as(&clerk, &order, &[("block_exception", "1")])
        .await
        .assert_status(403);
    t.issue_as(&clerk, &order, &[]).await.assert_status(403);
    assert!(pending(&t, &order).await.is_empty());
}

/// A supplier that is not blocked is issued at once, the order is numbered
/// and its document kept; an order without a price is not issued.
#[renox::test]
async fn an_order_to_an_active_supplier_is_issued_and_numbered() {
    let t = boot().await;
    let (item, store, order, _line) = po_310(&t).await;
    let a = t.procurement("Amal").await;

    t.issue_as(&a, &order, &[]).await.assert_ok();
    let issued = t.order_now(&order).await;
    assert_eq!(issued.status, PurchaseOrderStatus::Issued);
    assert!(
        issued
            .number
            .as_deref()
            .is_some_and(|n| n.starts_with("PO-"))
    );

    let (free, _line) = t.draft_order(&store, &item, 10_000, 0).await;
    t.issue_as(&a, &free, &[]).await.assert_status(422);
    assert_eq!(t.order_now(&free).await.status, PurchaseOrderStatus::Draft);
    assert_eq!(t.order_now(&free).await.number, None);
}

/// Done when 3: a goods receipt against an issued order to the blocked
/// supplier posts normally.
#[renox::test]
async fn a_receipt_against_an_issued_order_posts_for_a_blocked_supplier() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 10_000, 50_000).await;
    let sam = t.storekeeper("Sam").await;
    t.blocked.block(SUPPLIER);

    let receipt = t
        .receipt(&order, &store, vec![DraftLine::on(line.id, 10_000, 10_000)])
        .await;
    t.post_as(&sam, &receipt).await.assert_ok();

    assert_eq!(
        t.reload(&receipt).await.status,
        wl_inventory::GoodsReceiptStatus::Posted
    );
    assert_eq!(t.line_now(&line).await.accepted_qty_milli, 10_000);
}

/// Done when 4: revising PO-300 from AED 5,000 to AED 6,000 is refused; to
/// AED 4,500 it applies.
#[renox::test]
async fn a_revision_to_a_higher_total_is_refused_and_a_lower_one_applies() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    // 10 units at AED 500.
    let (order, line) = t.order(&store, &item, 10_000, 50_000).await;
    let a = t.procurement("Amal").await;
    t.blocked.block(SUPPLIER);

    t.revise_as(&a, &order, "dearer", &[(&line, "10", "600")], &[])
        .await
        .assert_status(422);
    assert_eq!(t.order_now(&order).await.revision, 1);

    t.revise_as(&a, &order, "discount", &[(&line, "10", "450")], &[])
        .await
        .assert_ok();
    let now = t.order_now(&order).await;
    assert_eq!(now.revision, 2);
    assert_eq!(t.line_now(&line).await.unit_price, 45_000);
}

/// Done when 5: with a `sanctions` block, procurement sees *on hold*, the
/// MLRO sees "Blocked: Sanctions".
#[renox::test]
async fn only_a_holder_of_the_reason_permission_sees_the_reason() {
    let t = boot().await;
    let (_item, _store, order, _line) = po_310(&t).await;
    let a = t.procurement("Amal").await;
    let mlro = t.procurement("Mona").await;
    t.blocked.block_for(SUPPLIER, "Sanctions");
    t.blocked.show_reasons_to(&mlro);

    let seen_by_procurement = t.issue_as(&a, &order, &[]).await;
    seen_by_procurement.assert_status(422);
    let body = seen_by_procurement.text();
    assert!(body.contains("This party is on hold"), "{body}");
    assert!(!body.contains("Sanctions"), "{body}");

    let seen_by_mlro = t.issue_as(&mlro, &order, &[]).await;
    seen_by_mlro.assert_status(422);
    let body = seen_by_mlro.text();
    assert!(body.contains("Blocked: Sanctions"), "{body}");
    assert_eq!(t.order_now(&order).await.status, PurchaseOrderStatus::Draft);
}

/// A supplier blocked while the exception waits has the exception failed at
/// the final approval only for other requests; the exception itself still
/// issues the order once (`PTY.ACL.03`).
#[renox::test]
async fn unblocking_does_not_stop_a_pending_exception_from_issuing() {
    let t = boot().await;
    let (_item, _store, order, _line) = po_310(&t).await;
    let a = t.procurement("Amal").await;
    let fatima = t.user_with("Fatima", "finance_manager").await;
    t.blocked.block(SUPPLIER);
    t.issue_as(&a, &order, &[("block_exception", "1")])
        .await
        .assert_ok();
    let requests = pending(&t, &order).await;
    t.blocked.unblock(SUPPLIER);

    approvals::approve(t.state(), requests[0].id, &fatima, "")
        .await
        .unwrap();
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::Issued
    );
}
