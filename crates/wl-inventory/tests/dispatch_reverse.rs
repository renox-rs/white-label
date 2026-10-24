//! BL-STK-25 (#244): an approved delivery reversal reopens a delivered
//! dispatch. Capabilities STK.DSP.01, WRK.ACL.10 (approved in wl-delivery).

mod support;

use wl_inventory::movement::MovementType;
use wl_inventory::{DispatchStatus, NewDispatchLine};

use support::{ORDER, boot};

/// AC1: Given a `delivered` dispatch 8 delivered / 2 refused, on
/// `DeliveryReversed` it is `dispatched`, delivered = 0, the 2 refused stay as
/// they were (in return here), `DispatchReversed` is emitted, and the proof is
/// kept and marked reversed.
#[renox::test]
async fn a_delivery_reversal_reopens_the_dispatch_and_keeps_the_refused_quantity() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000)],
        )
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;

    t.emit_reversed(&sent).await;

    let reopened = t.dispatch_now(&sent).await;
    assert_eq!(reopened.status, DispatchStatus::Dispatched);
    assert_eq!(reopened.proof_id, Some(77));
    assert!(reopened.proof_reversed);
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!(line.delivered_milli, 0);
    assert_eq!(line.refused_milli, 2_000);
    assert_eq!(line.in_return_milli(), 2_000);
    assert_eq!(line.refusal_reason.as_deref(), Some("damaged"));
    let events = t.reopened.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source.to_string(), "sales.sales_order:41");
    assert_eq!(events[0].lines[0].delivered_milli, 0);
    assert_eq!(events[0].lines[0].refused_milli, 2_000);
    assert_eq!(t.dispatch_movements(&sent).await.len(), 1);

    // A repeated reversal, or one for a dispatch not delivered, does nothing.
    t.emit_reversed(&sent).await;
    assert_eq!(t.reopened.all().len(), 1);
}

/// The refused goods already received into quarantine stay there, and the next
/// confirmation uses only the remaining quantity: it ends the dispatch again.
#[renox::test]
async fn refused_goods_in_quarantine_stay_and_the_next_confirmation_takes_the_rest() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000)],
        )
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_refused_received(&sent, &[(&line, 2_000, 0)]).await;
    let movements = t.dispatch_movements(&sent).await.len();
    assert_eq!(movements, 2);

    t.emit_reversed(&sent).await;

    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!(line.delivered_milli, 0);
    assert_eq!((line.refused_milli, line.returned_milli), (2_000, 2_000));
    // Its return_in movement is not undone.
    let moved = t.dispatch_movements(&sent).await;
    assert_eq!(moved.len(), movements);
    assert_eq!(moved[1].movement_type, MovementType::ReturnIn);

    // The delivery ends again: a new proof for the whole line takes only the
    // remaining 8; the refused 2 are not delivered a second time.
    t.emit_delivered(&sent, 78, &[(&line, 10_000, 0, None)])
        .await;
    let done = t.dispatch_now(&sent).await;
    assert_eq!(done.status, DispatchStatus::Delivered);
    assert_eq!(done.proof_id, Some(78));
    assert!(!done.proof_reversed);
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!((line.delivered_milli, line.refused_milli), (8_000, 2_000));
    assert_eq!(t.delivered.all().len(), 2);
}

/// A reversal for a dispatch that is not `delivered` does nothing.
#[renox::test]
async fn a_reversal_of_a_dispatch_that_is_not_delivered_does_nothing() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000)],
        )
        .await;

    t.emit_reversed(&sent).await;

    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Dispatched
    );
    assert!(t.reopened.all().is_empty());
}
