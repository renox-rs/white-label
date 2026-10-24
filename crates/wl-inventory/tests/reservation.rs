//! BL-SAL-11 (#130) point 2, BL-MFG-02 (#383) point 5 and BL-MFG-03 (#384):
//! stock reserved for the lines of a confirmed sales order and for the output
//! lot of a work order that serves a line. The events are emitted as wl-sales
//! emits them; the pick goes through the dispatch route.

mod support;

use renox::prelude::*;
use wl_inventory::dispatch::{
    Dispatch, DispatchStatus, NewDispatch, NewDispatchLine, create_draft,
};
use wl_inventory::reservation::{Reservation, ReservationStatus, SOURCE_ORDER, SOURCE_WORK_ORDER};
use wl_inventory::{Item, Location, Lot, LotStatus};
use wl_kernel::production::{ReserveOutput, production_stock};
use wl_kernel::sales::{
    AmendedLine, SalesOrderAmended, SalesOrderCancelled, SalesOrderConfirmed, SalesOrderId,
    SalesOrderLineCancelled,
};
use wl_kernel::{PartyId, Subject};

use support::{Booted, boot};

fn confirmed(order: i64) -> SalesOrderConfirmed {
    SalesOrderConfirmed {
        order: SalesOrderId(order),
        party: PartyId(1),
        channel: "phone".to_owned(),
    }
}

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

/// BL-SAL-11 point 2: when the held order is released and confirmed
/// (`SalesOrderConfirmed`), the stock of its line is reserved, and a second
/// delivery of the event reserves nothing more (one reservation per line).
#[renox::test]
async fn a_confirmed_order_reserves_the_stock_of_its_lines_once() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "PANEL", 10_000).await;
    t.orders.line(31, 1, "PANEL", 4_000);

    t.state().emit(confirmed(31)).await.unwrap();
    t.state().emit(confirmed(31)).await.unwrap();

    let rows = Reservation::of_line(t.db(), 31, 1).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, ReservationStatus::Active);
    assert_eq!(rows[0].source, SOURCE_ORDER);
    assert_eq!(
        (rows[0].lot_id, rows[0].location_id, rows[0].quantity_milli),
        (Some(lot.id), store.id, 4_000)
    );
    let figures = wl_inventory::balance::balance(t.db(), item.id, store.id, Some(lot.id))
        .await
        .unwrap();
    assert_eq!(figures.reserved_milli, 4_000);
    assert_eq!(figures.available_milli(), 6_000);
}

/// A line with no stock at all reserves nothing, and stock that arrives
/// later is not taken by a replay of the event.
#[renox::test]
async fn a_line_with_no_stock_reserves_nothing_and_a_replay_does_not_catch_up() {
    let t = boot().await;
    let item = t.tracked_item("EMPTY").await;
    let store = t.location("FG-EMPTY", None).await;
    t.orders.line(32, 1, "EMPTY", 4_000);

    t.state().emit(confirmed(32)).await.unwrap();
    let lot = t.lot(&item, "L-LATE", LotStatus::Released).await;
    t.stock_in(&item, &store, Some(&lot), 9_000, 300).await;
    t.state().emit(confirmed(32)).await.unwrap();

    assert!(
        Reservation::of_order(t.db(), 32)
            .await
            .unwrap()
            .iter()
            .all(|row| row.status != ReservationStatus::Active)
    );
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 9_000);
}

/// A reserved lot is not offered to another order, while the dispatch of the
/// order that reserved it can pick it; the pick turns the reservation into
/// the dispatch's own, so the quantity is reserved once, not twice.
#[renox::test]
async fn a_reserved_lot_is_not_picked_by_another_order_but_by_its_own() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "BEAM", 10_000).await;
    t.orders.line(33, 1, "BEAM", 8_000);
    t.state().emit(confirmed(33)).await.unwrap();

    let other = draft_for(&t, 34, 1, &item, &store, &lot, 5_000).await;
    pick(&t, &other).await.assert_status(422);
    assert_eq!(status_of(&t, &other).await, DispatchStatus::Draft);

    let own = draft_for(&t, 33, 1, &item, &store, &lot, 8_000).await;
    pick(&t, &own).await.assert_ok();
    assert_eq!(status_of(&t, &own).await, DispatchStatus::Picked);
    let rows = Reservation::of_line(t.db(), 33, 1).await.unwrap();
    assert_eq!(rows[0].status, ReservationStatus::Consumed);
    let figures = wl_inventory::balance::balance(t.db(), item.id, store.id, Some(lot.id))
        .await
        .unwrap();
    assert_eq!(figures.reserved_milli, 8_000, "reserved once by the pick");
    assert_eq!(figures.available_milli(), 2_000);
}

/// What is not reserved can still be picked by another order.
#[renox::test]
async fn what_the_reservation_leaves_is_picked_by_another_order() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "BOARD", 10_000).await;
    t.orders.line(35, 1, "BOARD", 6_000);
    t.state().emit(confirmed(35)).await.unwrap();

    let other = draft_for(&t, 36, 1, &item, &store, &lot, 4_000).await;
    pick(&t, &other).await.assert_ok();
    assert_eq!(status_of(&t, &other).await, DispatchStatus::Picked);
}

/// BL-SAL-11 point 2, with the F6 cancel flow: the cancelled order gives its
/// reservation back; a cancelled line gives back its own.
#[renox::test]
async fn cancelling_the_order_or_a_line_releases_the_reservation() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "SLAB", 10_000).await;
    t.orders.line(37, 1, "SLAB", 4_000);
    t.orders.line(37, 2, "SLAB", 3_000);
    t.state().emit(confirmed(37)).await.unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 3_000);

    t.orders.cancel(37, 2, 3_000);
    t.state()
        .emit(SalesOrderLineCancelled {
            order: SalesOrderId(37),
            party: PartyId(1),
            line: 2,
            quantity_milli: 3_000,
            reason: "changed".to_owned(),
            outcome: None,
            fee: 0,
            fee_waived: false,
        })
        .await
        .unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 6_000);

    t.state()
        .emit(SalesOrderCancelled {
            order: SalesOrderId(37),
            party: PartyId(1),
            reason: "changed".to_owned(),
            outcomes: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 10_000);
    assert!(
        Reservation::of_order(t.db(), 37)
            .await
            .unwrap()
            .iter()
            .all(|row| row.status == ReservationStatus::Released)
    );
}

/// A line lowered by an amendment keeps only what is still open.
#[renox::test]
async fn an_amendment_that_lowers_a_line_gives_back_the_surplus() {
    let t = boot().await;
    let (item, store, lot) = stocked(&t, "ROD", 10_000).await;
    t.orders.line(38, 1, "ROD", 6_000);
    t.state().emit(confirmed(38)).await.unwrap();
    t.orders.cancel(38, 1, 2_000);

    t.state()
        .emit(SalesOrderAmended {
            order: SalesOrderId(38),
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
    let rows = Reservation::of_line(t.db(), 51, 2).await.unwrap();
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
