//! BL-STK-19 (#238), `STK.RUL.06`, X-10: at a location set to
//! `allow_consumption` automatic consumption of an item that is not
//! lot-tracked may take the balance below zero; the next receipt covers it
//! and the difference goes to the price variance; a manual issue is still
//! refused and held quantity is never consumed.

mod support;

use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::price_variance::PriceVariance;
use wl_inventory::{DraftLine, Item, Location, LotStatus, NegativeStockPolicy};
use wl_kernel::Subject;

use support::{Booted, boot};

fn sale(n: i64) -> Subject {
    Subject::new("fnb.order", n)
}

/// An outlet location that lets automatic consumption go below zero.
async fn outlet(t: &Booted, code: &str) -> Location {
    let mut location = t.location(code, None).await;
    location.negative_stock_policy = NegativeStockPolicy::AllowConsumption;
    location.save(t.db()).await.unwrap();
    location
}

/// Flour at an average cost of AED 3 a kg.
async fn flour(t: &Booted) -> Item {
    let mut item = t.item("FLOUR").await;
    item.average_cost = 300;
    item.save(t.db()).await.unwrap();
    item
}

async fn on_hand(t: &Booted, item: &Item, location: &Location) -> i64 {
    wl_inventory::balance::on_hand_at(t.db(), item.id, location.id, None)
        .await
        .unwrap()
}

/// AC1: flour at Outlet 1 = 2 kg (avg AED 3), a sale consuming 5 kg posts and
/// leaves -3 kg; nothing becomes a pending issue.
#[renox::test]
async fn a_sale_consuming_more_than_is_on_hand_posts_and_leaves_the_balance_negative() {
    let t = boot().await;
    let item = flour(&t).await;
    let outlet1 = outlet(&t, "OUTLET-1").await;
    t.stock_in(&item, &outlet1, None, 2_000, 300).await;

    let done = t.consume(&item, &outlet1, 5_000, &sale(1)).await;

    assert_eq!((done.posted_milli, done.short_milli), (5_000, 0));
    assert_eq!(done.pending_issue, None);
    assert_eq!(on_hand(&t, &item, &outlet1).await, -3_000);
    assert!(t.open_pending().await.is_empty());
    let consumed: Vec<_> = t
        .movements_of(&item)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::Consume)
        .collect();
    assert_eq!(
        consumed.iter().map(|m| m.quantity_milli).sum::<i64>(),
        5_000
    );
    // The average cost is not changed by the negative consumption.
    assert_eq!(consumed.last().map(|m| m.unit_cost), Some(300));
    assert_eq!(t.negative.all().len(), 1);
    assert_eq!(t.negative.all()[0].quantity_milli, 3_000);
    assert_eq!(t.negative.all()[0].pending_issue, None);
}

/// AC2: a receipt of 10 kg at AED 4 leaves 7 kg; AED 3 (3 kg x AED 1) goes to
/// the price variance.
#[renox::test]
async fn the_next_receipt_covers_the_negative_quantity_and_the_difference_is_the_price_variance() {
    let t = boot().await;
    let item = flour(&t).await;
    let outlet1 = outlet(&t, "OUTLET-1").await;
    t.stock_in(&item, &outlet1, None, 2_000, 300).await;
    t.consume(&item, &outlet1, 5_000, &sale(1)).await;
    let (order, line) = t.order(&outlet1, &item, 10_000, 400).await;
    let receipt = t
        .receipt(
            &order,
            &outlet1,
            vec![DraftLine::on(line.id, 10_000, 10_000)],
        )
        .await;
    let keeper = t.storekeeper("Amal").await;

    t.post_as(&keeper, &receipt).await.assert_ok();

    assert_eq!(on_hand(&t, &item, &outlet1).await, 7_000);
    let variances = PriceVariance::of_receipt(t.db(), receipt.id).await.unwrap();
    assert_eq!(variances.len(), 1);
    assert_eq!(variances[0].quantity_milli, 3_000);
    assert_eq!(
        (variances[0].cost_used, variances[0].receipt_cost),
        (300, 400)
    );
    assert_eq!(variances[0].amount, 300);
    assert!(variances[0].posted);
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, 300);
    assert_eq!(posted[0].cost_code.as_deref(), Some("price_variance"));
}

/// A receipt at a location that is not negative leaves no variance.
#[renox::test]
async fn a_receipt_at_a_balance_that_is_not_negative_has_no_price_variance() {
    let t = boot().await;
    let item = flour(&t).await;
    let outlet1 = outlet(&t, "OUTLET-1").await;
    t.stock_in(&item, &outlet1, None, 2_000, 300).await;
    let (order, line) = t.order(&outlet1, &item, 10_000, 400).await;
    let receipt = t
        .receipt(
            &order,
            &outlet1,
            vec![DraftLine::on(line.id, 10_000, 10_000)],
        )
        .await;
    let keeper = t.storekeeper("Amal").await;

    t.post_as(&keeper, &receipt).await.assert_ok();

    assert!(
        PriceVariance::of_receipt(t.db(), receipt.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.costs.posted().is_empty());
}

/// AC3: a manual issue at -3 kg is refused.
#[renox::test]
async fn a_manual_issue_at_a_negative_balance_is_refused() {
    let t = boot().await;
    let item = flour(&t).await;
    let outlet1 = outlet(&t, "OUTLET-1").await;
    t.stock_in(&item, &outlet1, None, 2_000, 300).await;
    t.consume(&item, &outlet1, 5_000, &sale(1)).await;
    let keeper = t.storekeeper("Amal").await;

    t.issue_as_user(&keeper, &item, &outlet1, "1")
        .await
        .assert_status(422);

    assert_eq!(on_hand(&t, &item, &outlet1).await, -3_000);
}

/// AC4: flour with 5 kg on hand, all held, a sale consuming 2 kg posts nothing
/// from stock and records a pending issue of 2 kg; on hand stays 5, held 5.
#[renox::test]
async fn held_quantity_is_never_consumed_so_the_sale_becomes_a_pending_issue() {
    let t = boot().await;
    let item = flour(&t).await;
    let outlet1 = outlet(&t, "OUTLET-1").await;
    t.stock_in(&item, &outlet1, None, 5_000, 300).await;
    let holder = t.holder("Hal").await;
    let location_id = outlet1.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "quality"),
            ("reference", "ticketing.ticket:9"),
            ("location", location_id.as_str()),
        ],
    )
    .await
    .assert_ok();

    let done = t.consume(&item, &outlet1, 2_000, &sale(2)).await;

    assert_eq!((done.posted_milli, done.short_milli), (0, 2_000));
    let pending = t.open_pending().await;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].quantity_milli, 2_000);
    let found = wl_inventory::balance::balance(t.db(), item.id, outlet1.id, None)
        .await
        .unwrap();
    assert_eq!((found.on_hand_milli, found.held_milli), (5_000, 5_000));
}

/// Edge: a lot-tracked item at such a location is still a pending issue
/// (BL-STK-18), and a location that blocks stays blocked.
#[renox::test]
async fn lot_tracked_items_and_blocking_locations_still_make_pending_issues() {
    let t = boot().await;
    let outlet1 = outlet(&t, "OUTLET-1").await;
    let tracked = t.tracked_item("SERUM").await;
    let lot = t.lot(&tracked, "L1", LotStatus::Released).await;
    t.stock_in(&tracked, &outlet1, Some(&lot), 1_000, 500).await;
    let plain = flour(&t).await;
    let blocking = t.location("STORE-1", None).await;

    let lot_done = t.consume(&tracked, &outlet1, 3_000, &sale(3)).await;
    let block_done = t.consume(&plain, &blocking, 3_000, &sale(4)).await;

    assert_eq!(
        (lot_done.posted_milli, lot_done.short_milli),
        (1_000, 2_000)
    );
    assert_eq!(
        (block_done.posted_milli, block_done.short_milli),
        (0, 3_000)
    );
    assert_eq!(t.open_pending().await.len(), 2);
}
