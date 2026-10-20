//! Reopening a received purchase order (wl-inventory §4 *Purchase order*
//! `reopen`, issue #223): goods of a fully received order go back for
//! replacement, so the order expects them again.

mod support;

use renox::prelude::*;
use wl_inventory::movement::{self};
use wl_inventory::purchase_order::{committed_value, lines_of, reopen_in};
use wl_inventory::supplier_return::{NewReturnLine, ReturnStatus};
use wl_inventory::{DraftLine, PurchaseOrder, PurchaseOrderStatus};

use support::{Booted, boot};

/// A `received` order of 50 units at AED 50.00, all accepted and in stock.
async fn received(
    t: &Booted,
) -> (
    wl_inventory::Item,
    wl_inventory::Location,
    PurchaseOrder,
    wl_inventory::PurchaseOrderLine,
) {
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let (order, mut line) = t.order(&store, &item, 50_000, 5_000).await;
    line.accepted_qty_milli = 50_000;
    line.save(t.db()).await.unwrap();
    set_status(t, &order, PurchaseOrderStatus::Received).await;
    t.stock_in(&item, &store, None, 50_000, 5_000).await;
    (item, store, order, line)
}

async fn set_status(t: &Booted, order: &PurchaseOrder, status: PurchaseOrderStatus) {
    PurchaseOrder::query()
        .where_(PurchaseOrder::ID.eq(order.id))
        .update(t.db(), &[("status", &status)])
        .await
        .unwrap();
}

/// Done when 1: a `received` order line of 50 (all accepted); a return of 10
/// with `replace` dispatched: `partially_received`, outstanding 10, committed
/// cost +10 x the order's price.
#[renox::test]
async fn a_replacement_return_reopens_a_received_order() {
    let t = boot().await;
    let (item, store, order, line) = received(&t).await;
    assert_eq!(
        committed_value(&lines_of(t.db(), order.id).await.unwrap()),
        0
    );
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 10_000, "damaged").replacing_on(line.id)],
        )
        .await;
    let clerk = t.procurement("Cara").await;

    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();

    assert_eq!(t.return_now(&ret).await.status, ReturnStatus::Dispatched);
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::PartiallyReceived
    );
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 10_000);
    assert_eq!(
        committed_value(&lines_of(t.db(), order.id).await.unwrap()),
        50_000
    );
}

/// Several returns each add their quantity; the order stays open.
#[renox::test]
async fn each_return_adds_its_quantity() {
    let t = boot().await;
    let (item, store, order, line) = received(&t).await;
    let clerk = t.procurement("Cara").await;
    for (qty, reference) in [(10_000, "C-1"), (5_000, "C-2")] {
        let ret = t
            .stock_return(
                &store,
                vec![NewReturnLine::of(item.id, qty, "damaged").replacing_on(line.id)],
            )
            .await;
        t.dispatch_as(&clerk, &ret, "courier", reference)
            .await
            .assert_ok();
    }
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::PartiallyReceived
    );
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 15_000);
    assert_eq!(
        committed_value(&lines_of(t.db(), order.id).await.unwrap()),
        75_000
    );
    assert_eq!(
        movement::on_hand(t.db(), item.id, store.id).await.unwrap(),
        35_000
    );
}

/// A return dispatched twice restores once: the second try is refused.
#[renox::test]
async fn a_return_line_restores_once() {
    let t = boot().await;
    let (item, store, _, line) = received(&t).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 10_000, "damaged").replacing_on(line.id)],
        )
        .await;
    let clerk = t.procurement("Cara").await;
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_status(422);
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 10_000);
}

/// Done when 2: a `closed` order is not reopened; a replacement on it is
/// refused and a credit leaves it closed.
#[renox::test]
async fn a_closed_order_is_not_reopened() {
    let t = boot().await;
    let (item, store, order, line) = received(&t).await;
    set_status(&t, &order, PurchaseOrderStatus::Closed).await;
    let clerk = t.procurement("Cara").await;

    let replace = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 10_000, "damaged").replacing_on(line.id)],
        )
        .await;
    t.dispatch_as(&clerk, &replace, "courier", "C-1")
        .await
        .assert_status(422);

    let mut credit = NewReturnLine::of(item.id, 10_000, "damaged").credit();
    credit.po_line_id = Some(line.id);
    let credited = t.stock_return(&store, vec![credit]).await;
    t.dispatch_as(&clerk, &credited, "courier", "C-2")
        .await
        .assert_ok();

    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::Closed
    );
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 0);
    let mut tx = t.db().begin().await.unwrap();
    assert!(!reopen_in(&mut tx, order.id).await.unwrap());
    tx.rollback().await.unwrap();
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::Closed
    );
}

/// An order that is not `received` stays as it is.
#[renox::test]
async fn an_order_still_open_is_left_alone() {
    let t = boot().await;
    let (item, store, order, line) = received(&t).await;
    set_status(&t, &order, PurchaseOrderStatus::PartiallyReceived).await;
    let ret = t
        .stock_return(
            &store,
            vec![NewReturnLine::of(item.id, 10_000, "damaged").replacing_on(line.id)],
        )
        .await;
    let clerk = t.procurement("Cara").await;
    t.dispatch_as(&clerk, &ret, "courier", "C-1")
        .await
        .assert_ok();
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::PartiallyReceived
    );
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 10_000);
}

/// Reversing the receipt that completed an order reopens it too.
#[renox::test]
async fn reversing_the_completing_receipt_reopens_the_order() {
    let t = boot().await;
    let item = t.item("CHICKEN").await;
    let store = t.location("STORE-1", None).await;
    let (order, line) = t.order(&store, &item, 100_000, 1_200).await;
    let amal = t.storekeeper("Amal").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(line.id, 100_000, 100_000)],
        )
        .await;
    t.post_as(&amal, &receipt).await.assert_ok();
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::Received
    );
    let manager = t.reverser("Maha").await;
    t.reverse_as(&manager, &receipt).await.assert_ok();
    assert_eq!(
        t.order_now(&order).await.status,
        PurchaseOrderStatus::PartiallyReceived
    );
    assert_eq!(t.line_now(&line).await.outstanding_milli(), 100_000);
}
