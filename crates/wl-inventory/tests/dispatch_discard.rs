//! BL-STK-23 (#242): a dispatch discarded by wl-delivery is recorded as
//! wastage. Capabilities STK.DSP.01, WRK.RUL.16 (the approval of the
//! wastage is wl-delivery's, STK.ACL.02).

mod support;

use wl_inventory::movement::MovementType;
use wl_inventory::{DispatchStatus, NewDispatchLine};

use support::{ORDER, boot};

/// AC1: Given a dispatched dispatch worth AED 180, on `DeliveryDiscarded` it
/// is `discarded`, AED 180 is posted as wastage with the dispatch as its
/// source, the reason and the photo are kept and carried by `DispatchDiscarded`,
/// and no stock movement is posted. A second event changes nothing.
#[renox::test]
async fn a_discarded_delivery_ends_its_dispatch_and_posts_its_value_as_wastage() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let tray = t.priced_item("TRAY", false, 3_000).await;
    t.stock_in(&tray, &store, None, 20_000, 3_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(tray.id, 6_000)],
        )
        .await;
    let before = t.dispatch_movements(&sent).await.len();

    t.emit_discarded(&sent, "Customer closed, food spoiled", Some("photo-77"))
        .await;

    let discarded = t.dispatch_now(&sent).await;
    assert_eq!(discarded.status, DispatchStatus::Discarded);
    assert_eq!(discarded.discard_fils, 18_000);
    assert_eq!(
        discarded.discard_reason.as_deref(),
        Some("Customer closed, food spoiled")
    );
    assert_eq!(discarded.discard_photo.as_deref(), Some("photo-77"));
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, 18_000);
    assert_eq!(posted[0].cost_code.as_deref(), Some("wastage"));
    assert_eq!(posted[0].source, sent.subject());
    let events = t.discarded_dispatches.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value, 18_000);
    assert_eq!(events[0].photo.as_deref(), Some("photo-77"));
    assert_eq!(events[0].reason, "Customer closed, food spoiled");
    assert_eq!(events[0].source.to_string(), "sales.sales_order:41");
    // The goods already left: only the dispatch movement exists.
    let moved = t.dispatch_movements(&sent).await;
    assert_eq!(moved.len(), before);
    assert!(
        moved
            .iter()
            .all(|movement| movement.movement_type == MovementType::Dispatch)
    );

    t.emit_discarded(&sent, "Again", None).await;
    assert_eq!(t.costs.posted().len(), 1);
    assert_eq!(t.discarded_dispatches.all().len(), 1);
}

/// A dispatch that has not left, or that was delivered, is not discarded: the
/// event does nothing and no wastage is posted.
#[renox::test]
async fn only_a_dispatched_dispatch_can_be_discarded() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let tray = t.priced_item("TRAY", false, 3_000).await;
    t.stock_in(&tray, &store, None, 20_000, 3_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let picked = t
        .picked_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(tray.id, 6_000)],
        )
        .await;

    t.emit_discarded(&picked, "Spoiled", None).await;

    assert_eq!(t.dispatch_now(&picked).await.status, DispatchStatus::Picked);
    assert!(t.costs.posted().is_empty());
    assert!(t.discarded_dispatches.all().is_empty());
}
