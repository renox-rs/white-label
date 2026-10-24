//! BL-STK-24 (#243): a lost delivery holds its dispatch's value as a loss
//! pending the claim and reverses it when the goods are found. Capabilities
//! STK.DSP.01, WRK.ACL.10 (the delivery side is wl-delivery's).

mod support;

use wl_inventory::movement::MovementType;
use wl_inventory::{DispatchStatus, NewDispatchLine};

use support::{Booted, ORDER, boot};

/// A dispatch of 10 pumps at AED 50.00 (AED 500.00) that left with a courier.
async fn sent_ten(t: &Booted) -> wl_inventory::Dispatch {
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    t.sent_dispatch(
        &dispatcher,
        &store,
        ORDER,
        vec![NewDispatchLine::of(pump.id, 10_000)],
    )
    .await
}

/// AC1: Given a dispatched dispatch AED 500, on `DeliveryLost` it is `lost`
/// and AED 500 is a pending loss; `DispatchLost` is emitted and no stock
/// movement is posted. A second `DeliveryLost` changes nothing.
#[renox::test]
async fn a_lost_delivery_holds_the_value_of_its_dispatch_as_a_loss() {
    let t = boot().await;
    let sent = sent_ten(&t).await;

    t.emit_lost(&sent, false).await;

    let lost = t.dispatch_now(&sent).await;
    assert_eq!(lost.status, DispatchStatus::Lost);
    assert!(lost.loss_open);
    assert_eq!(lost.loss_fils, 50_000);
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, 50_000);
    assert_eq!(posted[0].cost_code.as_deref(), Some("loss_pending_claim"));
    assert_eq!(posted[0].cost_object, sent.subject());
    let events = t.lost.all();
    assert_eq!(events.len(), 1);
    assert!(!events[0].was_delivered);
    assert_eq!(events[0].value, 50_000);
    assert_eq!(events[0].source.to_string(), "sales.sales_order:41");
    // The goods already left: only the dispatch movement exists.
    let moved = t.dispatch_movements(&sent).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].movement_type, MovementType::Dispatch);

    t.emit_lost(&sent, false).await;
    assert_eq!(t.costs.posted().len(), 1);
    assert_eq!(t.lost.all().len(), 1);
}

/// AC2: On `DeliveryFound`, it is `dispatched` and the AED 500 loss is
/// reversed; the delivery continues and ends in a confirmation. A
/// `DeliveryFound` for a dispatch that is not lost does nothing.
#[renox::test]
async fn found_goods_reverse_the_loss_and_the_delivery_continues() {
    let t = boot().await;
    let sent = sent_ten(&t).await;
    t.emit_found(&sent).await;
    assert!(t.costs.reversed().is_empty());

    t.emit_lost(&sent, false).await;
    t.emit_found(&sent).await;

    let found = t.dispatch_now(&sent).await;
    assert_eq!(found.status, DispatchStatus::Dispatched);
    assert!(!found.loss_open);
    let reversed = t.costs.reversed();
    assert_eq!(reversed.len(), 1);
    assert_eq!(reversed[0].lines.len(), 1);
    assert_eq!(reversed[0].lines[0].amount_aed, 50_000);
    assert_eq!(reversed[0].lines[0].cost_object, sent.subject());
    assert_eq!(t.dispatch_movements(&sent).await.len(), 1);

    // The delivery continues and ends in a confirmation.
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 3, &[(&line, 10_000, 0, None)])
        .await;
    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Delivered
    );

    // The same found event again reverses nothing twice.
    t.emit_found(&sent).await;
    assert_eq!(t.costs.reversed().len(), 1);
}

/// AC3: Given a `delivered` dispatch (8 delivered, 2 refused) whose delivery
/// was under review, on `DeliveryLost` it is `lost`, delivered = 0, the 8 are
/// a pending loss, the 2 refused are unchanged (also once received back), and
/// `DispatchLost` is emitted.
#[renox::test]
async fn a_delivered_dispatch_lost_under_review_loses_only_what_was_delivered() {
    let t = boot().await;
    let sent = sent_ten(&t).await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_refused_received(&sent, &[(&line, 2_000, 0)]).await;
    let movements_before = t.dispatch_movements(&sent).await.len();

    t.emit_lost(&sent, true).await;

    let lost = t.dispatch_now(&sent).await;
    assert_eq!(lost.status, DispatchStatus::Lost);
    assert_eq!(lost.lost_from.as_deref(), Some("delivered"));
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!(line.delivered_milli, 0);
    assert_eq!((line.refused_milli, line.returned_milli), (2_000, 2_000));
    assert_eq!(lost.loss_fils, 40_000);
    let posted = t.costs.posted();
    assert_eq!(posted.last().unwrap().amount_aed, 40_000);
    let events = t.lost.all();
    assert_eq!(events.len(), 1);
    assert!(events[0].was_delivered);
    assert_eq!(events[0].value, 40_000);
    assert_eq!(events[0].lines[0].delivered_milli, 0);
    assert_eq!(events[0].lines[0].refused_milli, 2_000);
    // No stock movement; the refused goods in quarantine stay.
    assert_eq!(t.dispatch_movements(&sent).await.len(), movements_before);

    // Found again: the delivery continues with the refused quantity kept.
    t.emit_found(&sent).await;
    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Dispatched
    );
    assert_eq!(t.costs.reversed()[0].lines[0].amount_aed, 40_000);
    t.emit_delivered(&sent, 78, &[(&line, 10_000, 0, None)])
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!((line.delivered_milli, line.refused_milli), (8_000, 2_000));
}
