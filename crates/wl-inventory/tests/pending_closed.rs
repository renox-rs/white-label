//! `PendingIssueClosed` (BL-MFG-05, #386, wl-inventory §3 *Pending issue*):
//! when a pending issue posts, is cleared or its source is cancelled, the
//! record that consumed is told what became of the short part, after the
//! commit, with the lot and cost that posted and how many pending issues of
//! the same source are still open.

mod support;

use renox::prelude::*;
use wl_inventory::{Item, LotStatus};
use wl_kernel::Subject;
use wl_kernel::inventory::{LOT_SUBJECT_TYPE, stock_service};

use support::boot;

fn order(n: i64) -> Subject {
    Subject::new("manufacturing.work_order", n)
}

/// A work order's short issue of 10 kg posts when 6 kg arrive (a covered
/// part of 6 as a row of its own, 4 still open: one open for the source) and
/// again when 4 more arrive (the original closes, none open); the lot and the
/// cost are those of the consume movement.
#[renox::test]
async fn a_partly_covered_pending_issue_is_announced_in_two_parts() {
    let t = boot().await;
    let item = t.priced_item("RESIN", true, 800).await;
    let store = t.location("STORE-1", None).await;
    let lot = t.lot(&item, "R1", LotStatus::Released).await;
    t.consume(&item, &store, 10_000, &order(7)).await;
    let original = t.open_pending().await.remove(0);
    assert!(t.pending_closed.all().is_empty(), "still open");

    t.stock_in(&item, &store, Some(&lot), 6_000, 800).await;
    wl_inventory::pending_issue::post_on_arrival(t.state(), store.id, &[item.id], "receipt")
        .await
        .unwrap();
    t.stock_in(&item, &store, Some(&lot), 4_000, 800).await;
    wl_inventory::pending_issue::post_on_arrival(t.state(), store.id, &[item.id], "receipt")
        .await
        .unwrap();

    let closed = t.pending_closed.all();
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[0].original, original.subject());
    assert_ne!(
        closed[0].pending_issue, closed[0].original,
        "a part of its own"
    );
    assert_eq!(closed[0].quantity_milli, 6_000);
    assert_eq!(closed[0].open_for_source, 1);
    assert_eq!(closed[0].how, "posted");
    assert_eq!(closed[0].lot, Some(Subject::new(LOT_SUBJECT_TYPE, lot.id)));
    assert_eq!(closed[0].source, order(7));
    assert_eq!(closed[1].pending_issue, closed[1].original);
    assert_eq!(closed[1].quantity_milli, 4_000);
    assert_eq!(closed[1].open_for_source, 0);
    assert_eq!((closed[0].unit_cost, closed[1].unit_cost), (800, 800));
}

/// A pending issue the storekeeper clears is announced as `cleared` with the
/// lot, one whose source was cancelled as `source_cancelled` with no cost;
/// the other pending issue of the source keeps `open_for_source` up.
#[renox::test]
async fn a_cleared_and_a_cancelled_pending_issue_are_announced() {
    let t = boot().await;
    let mut item = Item::new("SUTURE", "SUTURE", "pcs");
    item.average_cost = 4_000;
    item.save(t.db()).await.unwrap();
    let store = t.location("STORE-1", None).await;
    let other = t.location("STORE-2", None).await;
    t.consume(&item, &store, 3_000, &order(8)).await;
    t.consume(&item, &other, 2_000, &order(8)).await;
    let pending = t.open_pending().await;
    let clerk = t.clerk("Cass").await;

    t.clear_as(&clerk, &pending[0], "counted, used", None)
        .await
        .assert_ok();
    let cancelled = stock_service(t.state())
        .source_cancelled(t.state(), &order(8))
        .await
        .unwrap();

    assert_eq!(cancelled, 1);
    let closed = t.pending_closed.all();
    assert_eq!(closed.len(), 2);
    assert_eq!(closed[0].how, "cleared");
    assert_eq!(closed[0].open_for_source, 1);
    assert_eq!(
        (closed[0].quantity_milli, closed[0].unit_cost),
        (3_000, 4_000)
    );
    assert_eq!(closed[1].how, "source_cancelled");
    assert_eq!(closed[1].open_for_source, 0);
    assert_eq!(closed[1].unit_cost, 0);
}
