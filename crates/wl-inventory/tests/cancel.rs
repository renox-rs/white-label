//! Cancelling a draft Return to supplier (wl-inventory §4 *Return to
//! supplier* `cancel`, issue #224): the goods stay, the stock held aside for
//! the return is released, and the user says what becomes of rejected goods
//! kept for collection.

mod support;

use renox::prelude::*;
use wl_inventory::movement;
use wl_inventory::supplier_return::{self, NewReturnLine, RejectedHandling, ReturnStatus};
use wl_inventory::{DraftLine, GoodsReceipt, LotStatus};

use support::{Booted, boot};

/// An item and a quarantine store holding 25 units of a lot.
async fn quarantined(
    t: &Booted,
) -> (
    wl_inventory::Item,
    wl_inventory::Location,
    wl_inventory::Lot,
) {
    let item = t.item("PIPE").await;
    let store = t.location("QUARANTINE-1", None).await;
    let lot = t.lot(&item, "Q-1", LotStatus::Quarantine).await;
    t.stock_in(&item, &store, Some(&lot), 25_000, 5_000).await;
    (item, store, lot)
}

/// What draft returns hold aside of `item_id` at `store_id`.
async fn reserved(t: &Booted, item_id: i64, store_id: i64) -> i64 {
    supplier_return::reserved_milli(t.db(), item_id, store_id, None)
        .await
        .unwrap()
}

/// Done when 4: a draft return with 5 units reserved from quarantine;
/// `cancel` releases them and moves no stock.
#[renox::test]
async fn cancelling_a_draft_return_releases_the_reserved_stock() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 5_000, "damaged").lot(lot.id)],
        )
        .await;
    let reserved = || reserved(&t, item.id, store.id);
    assert_eq!(reserved().await, 5_000);
    let clerk = t.procurement("Cara").await;

    t.cancel_as(&clerk, &ret, "supplier agreed to a visit", None)
        .await
        .assert_ok();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Cancelled);
    assert_eq!(
        done.cancel_reason.as_deref(),
        Some("supplier agreed to a visit")
    );
    assert_eq!(done.cancelled_by, Some(clerk.id));
    assert!(done.cancelled_at.is_some());
    assert_eq!(reserved().await, 0);
    // The goods stay: no movement.
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        25_000
    );
    assert!(
        movement::of_source(t.db(), &done.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.dispatched.all().is_empty());
}

/// A cancel needs a reason, a draft and the permission.
#[renox::test]
async fn a_cancel_needs_a_reason_a_draft_and_the_permission() {
    let t = boot().await;
    let (item, store, lot) = quarantined(&t).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 5_000, "damaged").lot(lot.id)],
        )
        .await;
    let clerk = t.procurement("Cara").await;
    let keeper = t.storekeeper("Amal").await;

    t.cancel_as(&clerk, &ret, "  ", None)
        .await
        .assert_status(422);
    t.cancel_as(&keeper, &ret, "no", None)
        .await
        .assert_status(403);
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Draft);

    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    t.cancel_as(&clerk, &ret, "too late", None)
        .await
        .assert_status(422);
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
}

/// The kept goods of a return: 20 of 100 rejected at receipt and kept for
/// collection, on a posted receipt.
async fn kept_return(t: &Booted) -> (wl_inventory::SupplierReturn, wl_inventory::PurchaseOrder) {
    let item = t.item("CHICKEN").await;
    let store = t.location("STORE-1", None).await;
    let (order, po_line) = t.order(&store, &item, 100_000, 1_200).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![
                DraftLine::on(po_line.id, 100_000, 80_000)
                    .rejected_for("quality", RejectedHandling::KeptForCollection),
            ],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let ret = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap()
        .pop()
        .expect("a draft return");
    assert_eq!(ret.status, ReturnStatus::Draft);
    (ret, order)
}

/// Rejected goods kept for collection: the user must choose; *discard* moves
/// nothing, *receive again* opens a draft receipt on the order.
#[renox::test]
async fn kept_goods_are_discarded_or_received_on_a_new_receipt() {
    let t = boot().await;
    let (ret, order) = kept_return(&t).await;
    let clerk = t.procurement("Cara").await;
    let receipts_before = GoodsReceipt::query().get(t.db()).await.unwrap().len();

    // No choice: refused.
    t.cancel_as(&clerk, &ret, "supplier will not come", None)
        .await
        .assert_status(422);
    t.cancel_as(&clerk, &ret, "supplier will not come", Some("burn"))
        .await
        .assert_status(422);
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Draft);

    t.cancel_as(
        &clerk,
        &ret,
        "supplier will not come",
        Some("receive_again"),
    )
    .await
    .assert_ok();
    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Cancelled);
    assert_eq!(done.kept_goods.as_deref(), Some("received_again"));
    let receipts = GoodsReceipt::query().get(t.db()).await.unwrap();
    assert_eq!(receipts.len(), receipts_before + 1);
    let opened = receipts
        .iter()
        .find(|r| r.id != ret.source_id)
        .expect("the new receipt");
    assert_eq!(opened.purchase_order_id, Some(order.id));
    assert_eq!(opened.created_by, Some(clerk.id));
    let lines = t.lines(opened).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].received_qty_milli, 20_000);
}

/// *Discard* opens no receipt and moves no stock.
#[renox::test]
async fn discarding_kept_goods_opens_no_receipt() {
    let t = boot().await;
    let (ret, _order) = kept_return(&t).await;
    let clerk = t.procurement("Cara").await;
    let receipts_before = GoodsReceipt::query().get(t.db()).await.unwrap().len();

    t.cancel_as(&clerk, &ret, "scrapped on site", Some("discard"))
        .await
        .assert_ok();

    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Cancelled);
    assert_eq!(done.kept_goods.as_deref(), Some("discarded"));
    assert_eq!(
        GoodsReceipt::query().get(t.db()).await.unwrap().len(),
        receipts_before
    );
    assert!(
        movement::of_source(t.db(), &done.subject())
            .await
            .unwrap()
            .is_empty()
    );
}
