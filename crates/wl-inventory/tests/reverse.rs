//! Reversing a posted goods receipt (wl-inventory §4 *Goods receipt*
//! `reverse`, issue #221): refused once a return from it has left or when
//! part of the stock is gone, otherwise the stock, the order and the draft
//! returns are taken back.

mod support;

use renox::prelude::*;
use wl_inventory::movement::{self, MovementType, StockMovement};
use wl_inventory::supplier_return::{self, RejectedHandling, ReturnStatus, SupplierReturn};
use wl_inventory::{
    DraftLine, GoodsReceipt, GoodsReceiptStatus, Lot, LotStatus, PurchaseOrder, PurchaseOrderLine,
    PurchaseOrderStatus,
};

use support::{Booted, boot};

/// 100 kg of chicken at AED 12.00 a kg, ordered for the store.
async fn chicken(
    t: &Booted,
) -> (
    wl_inventory::Item,
    wl_inventory::Location,
    PurchaseOrder,
    PurchaseOrderLine,
) {
    let item = t.item("CHICKEN").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 100_000, 1_200).await;
    (item, store, order, line)
}

/// Posts a receipt of 100 kg of which 80 are accepted and 20 rejected for
/// `handling`.
async fn posted_80(
    t: &Booted,
    store: &wl_inventory::Location,
    order: &PurchaseOrder,
    line: &PurchaseOrderLine,
    handling: RejectedHandling,
) -> GoodsReceipt {
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            order,
            store,
            vec![DraftLine::on(line.id, 100_000, 80_000).rejected_for("damaged", handling)],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let posted = t.reload(&receipt).await;
    assert_eq!(posted.status, GoodsReceiptStatus::Posted);
    posted
}

/// Moves the status of return `id` without a transition, as a fixture.
async fn force_return_status(t: &Booted, id: i64, status: ReturnStatus) {
    SupplierReturn::query()
        .where_(SupplierReturn::ID.eq(id))
        .update(t.db(), &[("status", &status)])
        .await
        .unwrap();
}

/// Done when 1: a posted receipt (80 accepted) with a door-refused return
/// `dispatched`: `reverse` is refused, nothing is posted.
#[renox::test]
async fn a_receipt_whose_return_has_left_cannot_be_reversed() {
    let t = boot().await;
    let (item, store, order, line) = chicken(&t).await;
    let receipt = posted_80(&t, &store, &order, &line, RejectedHandling::RefusedAtDoor).await;
    let returns = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(returns.len(), 1);
    force_return_status(&t, returns[0].id, ReturnStatus::Dispatched).await;
    let manager = t.reverser("Maha").await;

    t.reverse_as(&manager, &receipt)
        .await
        .assert_status(422)
        .assert_see("has left with the supplier");

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        80_000
    );
    assert_eq!(
        movement::of_source(t.db(), &receipt.subject())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 20_000);

    // A credited return has left as well.
    force_return_status(&t, returns[0].id, ReturnStatus::Credited).await;
    t.reverse_as(&manager, &receipt).await.assert_status(422);
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
}

/// Done when 2: a posted receipt with only a `draft` return: `reverse` posts
/// -80, the order's outstanding quantity is +80 and the draft return is
/// `cancelled`.
#[renox::test]
async fn reversing_takes_the_stock_back_and_cancels_the_draft_return() {
    let t = boot().await;
    let (item, store, order, line) = chicken(&t).await;
    let receipt = posted_80(
        &t,
        &store,
        &order,
        &line,
        RejectedHandling::KeptForCollection,
    )
    .await;
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 20_000);
    let manager = t.reverser("Maha").await;

    t.reverse_as(&manager, &receipt).await.assert_ok();

    assert_eq!(
        t.reload(&receipt).await.status,
        GoodsReceiptStatus::Reversed
    );
    // -80 kg out of the store, from the receipt, as a reversing movement.
    let moves = movement::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(moves.len(), 2);
    let out: &StockMovement = &moves[1];
    assert_eq!(out.movement_type, MovementType::Adjust);
    assert_eq!((out.quantity_milli, out.value), (80_000, 96_000));
    assert_eq!(out.from_location_id, Some(store.id));
    assert_eq!(out.user_id, Some(manager.id));
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        0
    );
    // The order expects all 100 again.
    assert_eq!(t.line_now(&line).await.accepted_qty_milli, 0);
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 100_000);
    // The draft return is withdrawn.
    let returns = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(returns.len(), 1);
    assert_eq!(returns[0].status, ReturnStatus::Cancelled);

    // Reversed is terminal.
    t.reverse_as(&manager, &receipt).await.assert_status(422);
}

/// Done when 3: 80 received, 10 issued to a job: `reverse` is refused naming
/// the line and the 10 missing; nothing is posted.
#[renox::test]
async fn a_receipt_whose_stock_was_used_is_refused_all_or_nothing() {
    let t = boot().await;
    let (item, store, order, line) = chicken(&t).await;
    let receipt = posted_80(
        &t,
        &store,
        &order,
        &line,
        RejectedHandling::KeptForCollection,
    )
    .await;
    let job = wl_kernel::Subject::new("construction.project", 4);
    let mut issued = StockMovement::leaving(
        MovementType::Issue,
        item.id,
        None,
        store.id,
        10_000,
        1_200,
        12_000,
        &job,
        None,
        None,
    );
    issued.save(t.db()).await.unwrap();
    let manager = t.reverser("Maha").await;

    t.reverse_as(&manager, &receipt)
        .await
        .assert_status(422)
        .assert_see("Line 1")
        .assert_see("CHICKEN")
        .assert_see("10 missing");

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        70_000
    );
    assert_eq!(
        movement::of_source(t.db(), &receipt.subject())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(t.line_now(&line).await.accepted_qty_milli, 80_000);
    let returns = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(returns[0].status, ReturnStatus::Draft);
}

/// Stock of another location does not count: it has to be at the receipt's
/// location.
#[renox::test]
async fn stock_moved_to_another_location_is_not_there() {
    let t = boot().await;
    let (item, store, order, line) = chicken(&t).await;
    let receipt = posted_80(
        &t,
        &store,
        &order,
        &line,
        RejectedHandling::KeptForCollection,
    )
    .await;
    let other = t.location("STORE-2", None).await;
    let transfer = wl_kernel::Subject::new("inventory.transfer", 1);
    let mut out = StockMovement::leaving(
        MovementType::TransferOut,
        item.id,
        None,
        store.id,
        80_000,
        1_200,
        96_000,
        &transfer,
        None,
        None,
    );
    out.to_location_id = Some(other.id);
    out.save(t.db()).await.unwrap();
    let manager = t.reverser("Maha").await;
    t.reverse_as(&manager, &receipt)
        .await
        .assert_status(422)
        .assert_see("80 missing");
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
}

/// A lot on hold is not free stock; a lot of a lot-tracked line that is
/// still free reverses.
#[renox::test]
async fn a_lot_on_hold_blocks_the_reversal() {
    let t = boot().await;
    let item = t.tracked_item("MILK").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 50_000, 800).await;
    let amal = t.storekeeper("Amal").await;
    let expiry = renox::chrono::NaiveDate::from_ymd_opt(2030, 1, 31).unwrap();
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(line.id, 50_000, 50_000).lot("L-1", expiry)],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let manager = t.reverser("Maha").await;

    Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .update(t.db(), &[("status", &LotStatus::OnHold)])
        .await
        .unwrap();
    t.reverse_as(&manager, &receipt)
        .await
        .assert_status(422)
        .assert_see("L-1");
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);

    Lot::query()
        .where_(Lot::ITEM_ID.eq(item.id))
        .update(t.db(), &[("status", &LotStatus::Released)])
        .await
        .unwrap();
    t.reverse_as(&manager, &receipt).await.assert_ok();
    assert_eq!(
        t.reload(&receipt).await.status,
        GoodsReceiptStatus::Reversed
    );
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        0
    );
    let moves = movement::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert!(moves.iter().all(|m| m.lot_id.is_some()));
}

/// A receipt of a billed (closed) order cannot be reversed.
#[renox::test]
async fn a_receipt_of_a_closed_order_is_billed_and_stays() {
    let t = boot().await;
    let (_, store, order, line) = chicken(&t).await;
    let receipt = posted_80(
        &t,
        &store,
        &order,
        &line,
        RejectedHandling::KeptForCollection,
    )
    .await;
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(t.db(), &[("status", &PurchaseOrderStatus::Closed)])
        .await
        .unwrap();
    let manager = t.reverser("Maha").await;
    t.reverse_as(&manager, &receipt)
        .await
        .assert_status(422)
        .assert_see("billed");
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
}

/// Only a holder of `goods_receipts.reverse` reverses; a draft cannot be.
#[renox::test]
async fn only_a_holder_of_the_permission_reverses_a_posted_receipt() {
    let t = boot().await;
    let (_, store, order, line) = chicken(&t).await;
    let receipt = posted_80(
        &t,
        &store,
        &order,
        &line,
        RejectedHandling::KeptForCollection,
    )
    .await;
    let bea = t.storekeeper("Bea").await;
    t.reverse_as(&bea, &receipt).await.assert_status(403);
    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);

    let draft = t
        .receipt(&order, &store, vec![DraftLine::on(line.id, 10_000, 10_000)])
        .await;
    let manager = t.reverser("Maha").await;
    t.reverse_as(&manager, &draft).await.assert_status(422);
    assert_eq!(t.reload(&draft).await.status, GoodsReceiptStatus::Draft);
}
