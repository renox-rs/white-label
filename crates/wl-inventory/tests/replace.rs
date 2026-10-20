//! Receiving a free replacement against a dispatched Return to supplier
//! (wl-inventory §4 *Goods receipt* `post`, issue #225): no purchase order,
//! zero cost, nothing to match, and the return is credited when every line is
//! covered.

mod support;

use renox::prelude::*;
use wl_inventory::movement::{self, MovementType};
use wl_inventory::supplier_return::{self, NewReturnLine, RejectedHandling, ReturnStatus};
use wl_inventory::{
    DraftLine, GoodsReceipt, GoodsReceiptStatus, Item, Location, PurchaseOrder, SupplierReturn,
};

use support::{Booted, boot};

/// A store holding 6 units of PIPE at AED 50.00 (average AED 50.00), and a
/// dispatched return of 4 of them to be replaced, with no order line.
async fn dispatched_replace(t: &Booted) -> (Item, Location, SupplierReturn) {
    let mut item = t.item("PIPE").await;
    item.average_cost = 5_000;
    item.save(t.db()).await.unwrap();
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 6_000, 5_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 4_000, "damaged").at_cost(5_000)],
        )
        .await;
    let clerk = t.procurement("Cara").await;
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    let ret = t.return_now(&ret).await;
    assert_eq!(ret.status, ReturnStatus::Dispatched);
    (item, store, ret)
}

/// A draft receipt of `lines` against the return, at the store.
async fn replacement(
    t: &Booted,
    ret: &SupplierReturn,
    store: &Location,
    lines: Vec<DraftLine>,
) -> GoodsReceipt {
    wl_inventory::goods_receipt::create_replacement_draft(
        t.db(),
        None,
        ret.id,
        store.id,
        Some("DN-9".into()),
        lines,
    )
    .await
    .unwrap()
}

/// Done when 1: a dispatched return with a `replace` line of 4 units and no
/// order line; a receipt of 4 at zero cost posts +4, the average cost falls
/// and the return becomes `credited`.
#[renox::test]
async fn a_free_replacement_posts_at_zero_cost_and_credits_the_return() {
    let t = boot().await;
    let (item, store, ret) = dispatched_replace(&t).await;
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    // 2 left at AED 50 after the return went out.
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        2_000
    );
    let receipt = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(lines[0].id, 4_000, 4_000)],
    )
    .await;
    assert_eq!(receipt.purchase_order_id, None);
    assert_eq!(receipt.supplier_return_id, Some(ret.id));
    let amal = t.storekeeper("Amal").await;

    t.post_as(&amal, &receipt).await.assert_ok();

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
    // +4 at zero cost.
    let moves = movement::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].movement_type, MovementType::Receive);
    assert_eq!(moves[0].quantity_milli, 4_000);
    assert_eq!((moves[0].unit_cost, moves[0].value), (0, 0));
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        6_000
    );
    // (2 x 50.00 + 4 x 0) / 6 = 16.67.
    let after = Item::find_or_404(t.db(), item.id).await.unwrap();
    assert_eq!(after.average_cost, 1_667);
    // The return line is replaced and the return credited.
    let done = t.return_now(&ret).await;
    assert_eq!(done.status, ReturnStatus::Credited);
    assert_eq!(done.credit_basis.as_deref(), Some("replacement"));
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    assert_eq!(lines[0].covered_milli, 4_000);
    // No order, nothing to match: the event has no order and no value.
    let received = t.received.all();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].purchase_order, None);
    assert_eq!(received[0].value, 0);
    assert_eq!(received[0].lines[0].po_line, None);
    assert_eq!(received[0].lines[0].accepted_qty_milli, 4_000);
    assert_eq!(PurchaseOrder::query().get(t.db()).await.unwrap().len(), 0);
    let credited = t.credited.all();
    assert_eq!(credited.len(), 1);
    assert_eq!(credited[0].basis, "replacement");
    assert_eq!(credited[0].supplier_return, ret.subject());
}

/// Done when 2: a receipt of 5 against a return of 4 is refused.
#[renox::test]
async fn a_receipt_above_the_returned_quantity_is_refused() {
    let t = boot().await;
    let (item, store, ret) = dispatched_replace(&t).await;
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    let receipt = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(lines[0].id, 5_000, 5_000)],
    )
    .await;
    let amal = t.storekeeper("Amal").await;

    t.post_as(&amal, &receipt).await.assert_status(422);

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Draft);
    assert!(
        movement::of_source(t.db(), &receipt.subject())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        2_000
    );
}

/// Several receipts: the sum per line never goes above the returned quantity;
/// the return is credited with the last one.
#[renox::test]
async fn partial_replacements_add_up_to_the_returned_quantity() {
    let t = boot().await;
    let (_item, store, ret) = dispatched_replace(&t).await;
    let line_id = supplier_return::lines_of(t.db(), ret.id).await.unwrap()[0].id;
    let amal = t.storekeeper("Amal").await;

    let first = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(line_id, 3_000, 3_000)],
    )
    .await;
    t.post_as(&amal, &first).await.assert_ok();
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    assert_eq!(
        supplier_return::lines_of(t.db(), ret.id).await.unwrap()[0].covered_milli,
        3_000
    );
    assert!(t.credited.all().is_empty());

    // 2 more would be 5 of 4.
    let too_many = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(line_id, 2_000, 2_000)],
    )
    .await;
    t.post_as(&amal, &too_many).await.assert_status(422);

    let last = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(line_id, 1_000, 1_000)],
    )
    .await;
    t.post_as(&amal, &last).await.assert_ok();
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Credited);
    assert_eq!(t.credited.all().len(), 1);

    // A credited return takes no more.
    t.post_as(&amal, &too_many).await.assert_status(422);
}

/// Goods rejected again at receipt are not counted as replaced and go on a
/// new return.
#[renox::test]
async fn goods_rejected_again_are_not_counted_and_go_on_a_new_return() {
    let t = boot().await;
    let (_item, store, ret) = dispatched_replace(&t).await;
    let line_id = supplier_return::lines_of(t.db(), ret.id).await.unwrap()[0].id;
    let amal = t.storekeeper("Amal").await;
    let receipt = replacement(
        &t,
        &ret,
        &store,
        vec![
            DraftLine::replacing(line_id, 4_000, 3_000)
                .rejected_for("quality", RejectedHandling::KeptForCollection),
        ],
    )
    .await;

    t.post_as(&amal, &receipt).await.assert_ok();

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    assert_eq!(
        supplier_return::lines_of(t.db(), ret.id).await.unwrap()[0].covered_milli,
        3_000
    );
    let again = supplier_return::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].status, ReturnStatus::Draft);
    let rows = supplier_return::lines_of(t.db(), again[0].id)
        .await
        .unwrap();
    assert_eq!(rows[0].quantity_milli, 1_000);
    // Only the accepted quantity came into stock.
    let moves = movement::of_source(t.db(), &receipt.subject())
        .await
        .unwrap();
    assert_eq!(moves[0].quantity_milli, 3_000);
}

/// A replacement with an open order line is received on the order, a credit
/// line is not replaced, and a draft return takes no receipt; a closed order
/// lets the replacement come against the return.
#[renox::test]
async fn only_replace_lines_without_an_open_order_line_are_received_on_the_return() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 10_000, 5_000).await;
    let (order, po_line) = t.order(&store, &item, 10_000, 5_000).await;
    let ret = t
        .stock_return(
            &store,
            vec![
                NewReturnLine::of(item.id, 2_000, "damaged").replacing_on(po_line.id),
                NewReturnLine::of(item.id, 1_000, "damaged").credit(),
            ],
        )
        .await;
    // A draft return is not a source.
    let lines = supplier_return::lines_of(t.db(), ret.id).await.unwrap();
    let amal = t.storekeeper("Amal").await;
    let clerk = t.procurement("Cara").await;
    assert!(
        wl_inventory::goods_receipt::create_replacement_draft(
            t.db(),
            None,
            ret.id,
            store.id,
            None,
            vec![DraftLine::replacing(lines[0].id, 2_000, 2_000)],
        )
        .await
        .is_err()
    );
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();

    // The credit line is not replaced.
    let credit_line = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(lines[1].id, 1_000, 1_000)],
    )
    .await;
    t.post_as(&amal, &credit_line).await.assert_status(422);
    // The order is open: the replacement is received on it.
    let on_order = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(lines[0].id, 2_000, 2_000)],
    )
    .await;
    t.post_as(&amal, &on_order).await.assert_status(422);
    assert_eq!(t.reload(&on_order).await.status, GoodsReceiptStatus::Draft);

    // The order is closed since: the replacement comes against the return.
    let mut tx = t.db().begin().await.unwrap();
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(&mut tx, &[("status", &"closed".to_owned())])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    t.post_as(&amal, &on_order).await.assert_ok();
    let line = &supplier_return::lines_of(t.db(), ret.id).await.unwrap()[0];
    assert_eq!(line.covered_milli, 2_000);
    // The credit line is still to be credited.
    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    // The closed order is left as it is: nothing was received on it.
    assert_eq!(t.line_now(&po_line).await.accepted_qty_milli, 0);
}

/// A replacement counted on a return cannot be reversed.
#[renox::test]
async fn a_replacement_receipt_cannot_be_reversed() {
    let t = boot().await;
    let (_item, store, ret) = dispatched_replace(&t).await;
    let line_id = supplier_return::lines_of(t.db(), ret.id).await.unwrap()[0].id;
    let amal = t.storekeeper("Amal").await;
    let receipt = replacement(
        &t,
        &ret,
        &store,
        vec![DraftLine::replacing(line_id, 4_000, 4_000)],
    )
    .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    let manager = t.reverser("Mona").await;

    t.reverse_as(&manager, &receipt).await.assert_status(422);

    assert_eq!(t.reload(&receipt).await.status, GoodsReceiptStatus::Posted);
}
