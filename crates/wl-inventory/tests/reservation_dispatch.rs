//! BL-SAL-17 beside BL-STK-26: the goods a sales order reserved are the goods
//! of the dispatch picked for that order, so the pick is not refused for the
//! order's own reservation and the stock is counted once.

mod support;

use renox::prelude::*;
use wl_inventory::dispatch::{
    Dispatch, DispatchStatus, NewDispatch, NewDispatchLine, create_draft,
};
use wl_inventory::reservation;
use wl_inventory::{Item, Location};
use wl_kernel::Subject;
use wl_kernel::backorders::ReserveStock;
use wl_kernel::sales::SalesOrderId;

use support::{Booted, boot};

fn order(id: i64) -> Subject {
    SalesOrderId(id).subject()
}

/// A draft dispatch of order `id` for `quantity_milli` of `item` as sale line
/// `line`, picked by a dispatcher; the dispatch as it is after.
async fn pick(
    t: &Booted,
    location: &Location,
    id: i64,
    item: &Item,
    quantity_milli: i64,
    line: i64,
) -> Dispatch {
    let dispatch = create_draft(
        t.db(),
        None,
        NewDispatch {
            location_id: location.id,
            source: order(id),
            recipient: Some(wl_kernel::PartyId(1)),
            recipient_name: "Customer".to_owned(),
            address: String::new(),
            lines: vec![NewDispatchLine::of(item.id, quantity_milli).for_sale_line(line)],
        },
    )
    .await
    .unwrap();
    let dispatcher = t.dispatcher("Dana").await;
    t.app.acting_as(&dispatcher);
    let _ = t
        .app
        .htmx()
        .post(&format!("/dispatches/{}/pick", dispatch.id), &[])
        .await;
    Dispatch::find_or_404(t.db(), dispatch.id).await.unwrap()
}

/// 5 units in stock, 3 reserved for order 7 line 81: the dispatch of order 7
/// picks them, and the 3 are counted once (2 stay available).
#[renox::test]
async fn a_dispatch_picks_the_goods_its_own_order_reserved_and_they_count_once() {
    let t = boot().await;
    let item = t.item("RICE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 300).await;
    let got = reservation::reserve(
        t.state(),
        ReserveStock::new("RICE", 3_000, order(7), 81).at(store.subject()),
    )
    .await
    .unwrap();
    assert_eq!(got.reserved_milli, 3_000);
    assert_eq!(t.available(&item, &store, None).await, 2_000);

    let picked = pick(&t, &store, 7, &item, 3_000, 81).await;
    assert_eq!(picked.status, DispatchStatus::Picked);
    assert_eq!(t.available(&item, &store, None).await, 2_000);
}

/// The reservation of order 7 does not open the stock to the dispatch of
/// another order.
#[renox::test]
async fn a_dispatch_of_another_order_cannot_pick_what_an_order_reserved() {
    let t = boot().await;
    let item = t.item("RICE").await;
    let store = t.location("OUTLET-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 300).await;
    reservation::reserve(
        t.state(),
        ReserveStock::new("RICE", 3_000, order(7), 81).at(store.subject()),
    )
    .await
    .unwrap();

    let other = pick(&t, &store, 8, &item, 3_000, 91).await;
    assert_eq!(other.status, DispatchStatus::Draft);
    assert_eq!(t.available(&item, &store, None).await, 2_000);
}
