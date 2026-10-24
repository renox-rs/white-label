//! BL-SAL-11 (#130) point 2, BL-MFG-02 (#383) point 5 and BL-MFG-03 (#384):
//! stock reserved for the lines of a confirmed sales order and for the output
//! lot of a work order that serves a line. The events are emitted as wl-sales
//! emits them; the pick goes through the dispatch route.

mod support;

use renox::prelude::*;
use wl_inventory::dispatch::{
    Dispatch, DispatchStatus, NewDispatch, NewDispatchLine, create_draft,
};
use wl_inventory::reservation::{SOURCE_ORDER, SOURCE_WORK_ORDER, StockReservation};
use wl_inventory::{Item, Location, Lot, LotStatus};
use wl_kernel::backorders::{ReserveStock, stock_reservations};
use wl_kernel::production::{ReserveOutput, production_stock};
use wl_kernel::sales::{
    AmendedLine, SalesOrderAmended, SalesOrderCancelled, SalesOrderId, SalesOrderLineCancelled,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

/// A store with `quantity_milli` of a released lot of the lot-tracked item
/// `code`.
async fn stocked(t: &Booted, code: &str, quantity_milli: i64) -> (Item, Location, Lot) {
    let item = t.tracked_item(code).await;
    let store = t.location(&format!("FG-{code}"), None).await;
    let lot = t
        .lot(&item, &format!("L-{code}"), LotStatus::Released)
        .await;
    t.stock_in(&item, &store, Some(&lot), quantity_milli, 300)
        .await;
    (item, store, lot)
}

async fn draft_for(
    t: &Booted,
    order: i64,
    sale_line: i64,
    item: &Item,
    store: &Location,
    lot: &Lot,
    quantity_milli: i64,
) -> Dispatch {
    create_draft(
        t.db(),
        None,
        NewDispatch {
            source: SalesOrderId(order).subject(),
            recipient: Some(PartyId(1)),
            recipient_name: "Customer".to_owned(),
            address: String::new(),
            location_id: store.id,
            lines: vec![
                NewDispatchLine::of(item.id, quantity_milli)
                    .lot(lot.id)
                    .for_sale_line(sale_line),
            ],
        },
    )
    .await
    .unwrap()
}

async fn pick(t: &Booted, dispatch: &Dispatch) -> renox::testing::TestResponse {
    let dispatcher = t.dispatcher("Dana").await;
    t.app.acting_as(&dispatcher);
    t.app
        .htmx()
        .post(&format!("/dispatches/{}/pick", dispatch.id), &[])
        .await
}

async fn status_of(t: &Booted, dispatch: &Dispatch) -> DispatchStatus {
    Dispatch::find_or_404(t.db(), dispatch.id)
        .await
        .unwrap()
        .status
}

/// Sets aside `quantity_milli` of `item` at `store` for line `line` of order
/// `order`, as wl-sales asks when it confirms the order (BL-SAL-17), and
/// answers what was reserved.
async fn order_reserves(
    t: &Booted,
    order: i64,
    line: i64,
    item: &Item,
    store: &Location,
    quantity_milli: i64,
) -> i64 {
    stock_reservations(t.state())
        .reserve(
            t.state(),
            ReserveStock::new(
                &item.code,
                quantity_milli,
                SalesOrderId(order).subject(),
                line,
            )
            .at(Subject::new("inventory.location", store.id)),
        )
        .await
        .unwrap()
        .reserved_milli
}

/// BL-SAL-11 point 2 with the one reservation of wl-inventory: what a
/// confirmed order reserves is by item and location (the quantity is out of
/// the item's available stock, whichever lot is picked), source `order`.
#[renox::test]
async fn an_order_reserves_the_stock_of_its_line_by_item_and_location() {
    let t = boot().await;
    let (item, store, _lot) = stocked(&t, "PANEL", 10_000).await;

    assert_eq!(order_reserves(&t, 31, 1, &item, &store, 4_000).await, 4_000);

    let rows = StockReservation::of_line(t.db(), 31, 1).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_active());
    assert_eq!(rows[0].source, SOURCE_ORDER);
    assert_eq!(
        (rows[0].lot_id, rows[0].location_id, rows[0].quantity_milli),
        (None, store.id, 4_000)
    );
    let figures = wl_inventory::balance::balance(t.db(), item.id, store.id, None)
        .await
        .unwrap();
    assert_eq!(figures.reserved_milli, 4_000);
    assert_eq!(figures.available_milli(), 6_000);
}

/// A line with no stock at all reserves nothing (wl-sales back-orders it).
#[renox::test]
async fn a_line_with_no_stock_reserves_nothing() {
    let t = boot().await;
    let item = t.tracked_item("EMPTY").await;
    let store = t.location("FG-EMPTY", None).await;

    assert_eq!(order_reserves(&t, 32, 1, &item, &store, 4_000).await, 0);

    assert!(
        StockReservation::of_order(t.db(), 32)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Stock reserved for an order is not offered to another order's pick of a
/// lot of it, while the dispatch of the order that reserved it can pick it;
/// the pick takes the reservation over, so the quantity is reserved once,
/// not twice.
#[renox::test]
async fn a_reserved_lot_is_not_picked_by_another_order_but_by_its_own() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "BEAM", 10_000).await;
    order_reserves(&t, 33, 1, &item, &store, 8_000).await;

    let other = draft_for(&t, 34, 1, &item, &store, &lot, 5_000).await;
    pick(&t, &other).await.assert_status(422);
    assert_eq!(status_of(&t, &other).await, DispatchStatus::Draft);

    let own = draft_for(&t, 33, 1, &item, &store, &lot, 8_000).await;
    pick(&t, &own).await.assert_ok();
    assert_eq!(status_of(&t, &own).await, DispatchStatus::Picked);
    let whole = wl_inventory::balance::balance(t.db(), item.id, store.id, None)
        .await
        .unwrap();
    assert_eq!(whole.reserved_milli, 8_000, "reserved once, not twice");
    assert_eq!(whole.available_milli(), 2_000);
    let figures = wl_inventory::balance::balance(t.db(), item.id, store.id, Some(lot.id))
        .await
        .unwrap();
    assert_eq!(figures.reserved_milli, 8_000, "the lot is the dispatch's");
    assert_eq!(figures.available_milli(), 2_000);
}

/// What is not reserved can still be picked by another order.
#[renox::test]
async fn what_the_reservation_leaves_is_picked_by_another_order() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "BOARD", 10_000).await;
    order_reserves(&t, 35, 1, &item, &store, 6_000).await;

    let other = draft_for(&t, 36, 1, &item, &store, &lot, 4_000).await;
    pick(&t, &other).await.assert_ok();
    assert_eq!(status_of(&t, &other).await, DispatchStatus::Picked);
}

/// BL-SAL-11 point 2, with the F6 cancel flow: a cancelled line gives back
/// its own reservation, the cancelled order the rest (wl-sales asks through
/// `release`).
#[renox::test]
async fn giving_back_a_line_or_the_order_releases_the_reservation() {
    let t = boot().await;
    let (item, store, _lot) = stocked(&t, "SLAB", 10_000).await;
    order_reserves(&t, 37, 1, &item, &store, 4_000).await;
    order_reserves(&t, 37, 2, &item, &store, 3_000).await;
    let owner = SalesOrderId(37).subject();
    let reservations = stock_reservations(t.state());
    let free = |t: &Booted| {
        let (item, store) = (item.clone(), store.clone());
        let db = t.db().clone();
        async move {
            wl_inventory::balance::balance(&db, item.id, store.id, None)
                .await
                .unwrap()
                .available_milli()
        }
    };
    assert_eq!(free(&t).await, 3_000);

    let back = reservations
        .release(t.state(), &owner, Some(2), None)
        .await
        .unwrap();
    assert_eq!(back, 3_000);
    assert_eq!(free(&t).await, 6_000);

    let back = reservations
        .release(t.state(), &owner, None, None)
        .await
        .unwrap();
    assert_eq!(back, 4_000);
    assert_eq!(free(&t).await, 10_000);
    assert!(
        StockReservation::of_order(t.db(), 37)
            .await
            .unwrap()
            .iter()
            .all(|row| !row.is_active())
    );
}

/// A line lowered by an amendment gives back a part: newest first, and the
/// outputs of work orders after the orders' own.
#[renox::test]
async fn giving_back_a_part_takes_the_orders_stock_before_the_work_order_output() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "ROD", 10_000).await;
    order_reserves(&t, 38, 1, &item, &store, 3_000).await;
    let stock = production_stock(t.state());
    let mut tx = t.db().begin().await.unwrap();
    stock
        .reserve_output(&mut tx, reserve(&lot, &item, &store, 38, 1, 3_000))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let whole = |t: &Booted| {
        let (item, store) = (item.clone(), store.clone());
        let db = t.db().clone();
        async move {
            wl_inventory::balance::balance(&db, item.id, store.id, None)
                .await
                .unwrap()
                .reserved_milli
        }
    };
    assert_eq!(whole(&t).await, 6_000, "both are set aside, once each");

    let back = stock_reservations(t.state())
        .release(t.state(), &SalesOrderId(38).subject(), Some(1), Some(4_000))
        .await
        .unwrap();
    assert_eq!(back, 4_000);

    let rows = StockReservation::of_line(t.db(), 38, 1).await.unwrap();
    let left = |source: &str| {
        rows.iter()
            .filter(|row| row.source == source)
            .map(|row| row.quantity_milli)
            .sum::<i64>()
    };
    assert_eq!(left(SOURCE_ORDER), 0, "the order's stock first");
    assert_eq!(left(SOURCE_WORK_ORDER), 2_000);
}

/// The lines of an order that is cancelled or lowered give back the output
/// lot of a work order that was set aside for them.
#[renox::test]
async fn a_cancelled_order_a_cancelled_line_or_a_lowered_line_trim_the_output_lot() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "BAR", 10_000).await;
    t.orders.line(39, 1, "BAR", 6_000);
    let stock = production_stock(t.state());
    let mut tx = t.db().begin().await.unwrap();
    stock
        .reserve_output(&mut tx, reserve(&lot, &item, &store, 39, 1, 6_000))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 4_000);

    t.orders.cancel(39, 1, 2_000);
    t.state()
        .emit(SalesOrderAmended {
            order: SalesOrderId(39),
            party: PartyId(1),
            revision: 2,
            lines: vec![AmendedLine {
                line: 1,
                quantity_before_milli: 6_000,
                quantity_after_milli: 4_000,
                unit_price_before: 1_000,
                unit_price_after: 1_000,
                held_milli: 0,
            }],
            value_before: 0,
            value_after: 0,
            reason: "less".to_owned(),
        })
        .await
        .unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 6_000);

    t.orders.cancel(39, 1, 1_000);
    t.state()
        .emit(SalesOrderLineCancelled {
            order: SalesOrderId(39),
            party: PartyId(1),
            line: 1,
            quantity_milli: 1_000,
            reason: "changed".to_owned(),
            outcome: None,
            fee: 0,
            fee_waived: false,
        })
        .await
        .unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 7_000);

    t.state()
        .emit(SalesOrderCancelled {
            order: SalesOrderId(39),
            party: PartyId(1),
            reason: "changed".to_owned(),
            outcomes: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 10_000);
    assert!(
        StockReservation::of_order(t.db(), 39)
            .await
            .unwrap()
            .iter()
            .all(|row| !row.is_active())
    );
}

fn reserve(
    lot: &Lot,
    item: &Item,
    store: &Location,
    order: i64,
    line: i64,
    quantity_milli: i64,
) -> ReserveOutput {
    ReserveOutput {
        source: Subject::new("manufacturing.work_order", 5),
        item: Subject::new("inventory.item", item.id),
        lot: Subject::new("inventory.lot", lot.id),
        location: Subject::new("inventory.location", store.id),
        order: SalesOrderId(order),
        line,
        quantity_milli,
    }
}

/// BL-MFG-02 point 5: the output lot of a work order is reserved for the line;
/// `release_reservation` answers `true` once, the lot is free stock again.
#[renox::test]
async fn the_output_lot_reserved_for_a_line_is_free_stock_after_it_is_released() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "F9", 10_000).await;
    let stock = production_stock(t.state());
    let mut tx = t.db().begin().await.unwrap();
    assert!(
        stock
            .reserve_output(&mut tx, reserve(&lot, &item, &store, 51, 2, 10_000))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 0);
    let rows = StockReservation::of_line(t.db(), 51, 2).await.unwrap();
    assert_eq!(rows[0].source, SOURCE_WORK_ORDER);

    let mut tx = t.db().begin().await.unwrap();
    let lot_subject = Subject::new("inventory.lot", lot.id);
    assert!(
        stock
            .release_reservation(&mut tx, &lot_subject, SalesOrderId(51), 2)
            .await
            .unwrap()
    );
    assert!(
        !stock
            .release_reservation(&mut tx, &lot_subject, SalesOrderId(51), 2)
            .await
            .unwrap(),
        "nothing left to release"
    );
    tx.commit().await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 10_000);
}

/// BL-MFG-03: the part of the output that is for stock is not reserved:
/// reserving less than the lot holds leaves the rest free, and the
/// reservation can be changed (a decrease of the line) or given back at 0.
#[renox::test]
async fn only_the_part_for_the_line_is_reserved_and_the_rest_is_free_stock() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "F8", 10_000).await;
    let stock = production_stock(t.state());

    let mut tx = t.db().begin().await.unwrap();
    stock
        .reserve_output(&mut tx, reserve(&lot, &item, &store, 52, 1, 6_000))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 4_000);

    let mut tx = t.db().begin().await.unwrap();
    stock
        .reserve_output(&mut tx, reserve(&lot, &item, &store, 52, 1, 2_000))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 8_000);

    let mut tx = t.db().begin().await.unwrap();
    assert!(
        !stock
            .reserve_output(&mut tx, reserve(&lot, &item, &store, 52, 1, 0))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 10_000);
}

/// The output lot reserved for a line is not offered to another order's pick.
#[renox::test]
async fn an_output_lot_reserved_for_a_line_is_not_picked_by_another_order() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "F7", 10_000).await;
    let stock = production_stock(t.state());
    let mut tx = t.db().begin().await.unwrap();
    stock
        .reserve_output(&mut tx, reserve(&lot, &item, &store, 53, 1, 10_000))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let other = draft_for(&t, 54, 1, &item, &store, &lot, 1_000).await;
    pick(&t, &other).await.assert_status(422);
    let own = draft_for(&t, 53, 1, &item, &store, &lot, 10_000).await;
    pick(&t, &own).await.assert_ok();
}
