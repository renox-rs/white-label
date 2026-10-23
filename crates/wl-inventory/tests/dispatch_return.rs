//! The returned dispatch (wl-inventory §4 *Dispatch* row `return`, §5.4;
//! needed by BL-SAL-13 #132 and BL-MFG-07 #388). Capabilities STK.DSP.01,
//! WRK.RUL.16.

mod support;

use wl_inventory::movement::MovementType;
use wl_inventory::{DispatchStatus, NewDispatchLine};

use support::{ORDER, boot};

/// A returned delivery puts the whole quantity into quarantine with the
/// picked lot and the reason, opens one complaint ticket, ends the dispatch
/// and emits `DispatchReturned` and `DispatchGoodsQuarantined`; a second event
/// changes nothing.
#[renox::test]
async fn a_returned_delivery_brings_the_goods_to_quarantine_and_opens_a_complaint() {
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
            vec![NewDispatchLine::of(pump.id, 10_000).for_sale_line(7)],
        )
        .await;

    t.emit_returned(&sent, "customer_unavailable").await;

    let returned = t.dispatch_now(&sent).await;
    assert_eq!(returned.status, DispatchStatus::Returned);
    assert_eq!(returned.return_reason.as_deref(), Some("customer_unavailable"));
    let moved = t.dispatch_movements(&sent).await;
    let back: Vec<_> = moved
        .iter()
        .filter(|movement| movement.movement_type == MovementType::ReturnIn)
        .collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].quantity_milli, 10_000);
    assert_eq!(back[0].reason.as_deref(), Some("customer_unavailable"));
    let opened = t.tickets.opened();
    assert_eq!(opened.len(), 1);
    assert_eq!(returned.complaint_ticket_id, Some(1));
    let events = t.returned.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "customer_unavailable");
    assert_eq!(
        events[0].complaint.as_ref().map(ToString::to_string),
        Some("ticketing.ticket:1".to_owned())
    );
    let booked = t.quarantined.all();
    assert_eq!(booked.len(), 1);
    assert!(booked[0].returned);
    assert_eq!(booked[0].lines.len(), 1);
    assert_eq!(booked[0].lines[0].quantity_milli, 10_000);
    assert_eq!(
        booked[0].lines[0].source_line.as_ref().map(ToString::to_string),
        Some("sales.sale_line:7".to_owned())
    );

    t.emit_returned(&sent, "again").await;
    assert_eq!(t.tickets.opened().len(), 1);
    assert_eq!(t.returned.all().len(), 1);
}

/// A dispatcher returns a dispatch by hand with a reason; without one the
/// page answers 422.
#[renox::test]
async fn a_dispatcher_returns_a_dispatch_with_a_reason() {
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
            vec![NewDispatchLine::of(pump.id, 4_000)],
        )
        .await;

    t.dispatch_action(&dispatcher, &sent, "return", &[("reason", "")])
        .await
        .assert_status(422);
    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Dispatched
    );

    t.dispatch_action(&dispatcher, &sent, "return", &[("reason", "Wrong address")])
        .await
        .assert_ok();
    let returned = t.dispatch_now(&sent).await;
    assert_eq!(returned.status, DispatchStatus::Returned);
    assert_eq!(returned.return_reason.as_deref(), Some("Wrong address"));
}

/// Refused goods received back at run close announce the lots that are in
/// quarantine now (`DispatchGoodsQuarantined`, not a return).
#[renox::test]
async fn refused_goods_received_back_announce_the_quarantined_lot() {
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
    t.emit_delivered(&sent, 5, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;
    assert!(t.quarantined.all().is_empty());
    let line = t.dispatch_lines(&sent).await.remove(0);

    t.emit_refused_received(&sent, &[(&line, 2_000, 0)]).await;

    let booked = t.quarantined.all();
    assert_eq!(booked.len(), 1);
    assert!(!booked[0].returned);
    assert!(booked[0].complaint.is_none());
    assert_eq!(booked[0].lines[0].quantity_milli, 2_000);
    assert_eq!(booked[0].lines[0].reason.as_deref(), Some("damaged"));
    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Delivered
    );
}

/// The record page of a dispatched dispatch offers *Return*; once returned it
/// says why and the goods are in quarantine, and a discarded one says what was
/// written off.
#[renox::test]
async fn the_dispatch_page_offers_return_and_says_how_it_ended() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 40_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let first = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 4_000)],
        )
        .await;
    let second = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 2_000)],
        )
        .await;

    let page = t
        .browse(&dispatcher, &format!("/dispatches/{}", first.id))
        .await;
    page.assert_ok();
    assert!(page.text().contains("return-dispatch"));

    t.emit_returned(&first, "Wrong address").await;
    let page = t
        .browse(&dispatcher, &format!("/dispatches/{}", first.id))
        .await;
    assert!(page.text().contains("Came back: Wrong address"));
    assert!(!page.text().contains("return-dispatch"));

    t.emit_discarded(&second, "Spoiled", None).await;
    let page = t
        .browse(&dispatcher, &format!("/dispatches/{}", second.id))
        .await;
    assert!(page.text().contains("written off as wastage"));
}
