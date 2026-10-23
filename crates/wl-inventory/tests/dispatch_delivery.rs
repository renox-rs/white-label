//! BL-STK-22 (#241): a delivery is confirmed per line from the proof of
//! delivery (`DeliveryDelivered`) or by hand for a courier; the refused goods
//! stay in return until they are brought back and then go to quarantine
//! (`DeliveryRefusedReceived`), the missing ones are a loss with a task for
//! the dispatcher. Capabilities STK.DSP.01, STK.ACL.02.

mod support;

use renox::prelude::*;
use wl_inventory::movement::MovementType;
use wl_inventory::{DispatchStatus, Location, LocationType, NewDispatchLine};
use wl_kernel::tickets::Assignee;

use support::{Booted, ORDER, boot};

/// The quarantine location, once something went there.
async fn quarantine(t: &Booted) -> Location {
    Location::query()
        .where_(Location::LOCATION_TYPE.eq(LocationType::Quarantine))
        .first(t.db())
        .await
        .unwrap()
        .expect("goods were taken to quarantine")
}

/// A courier-free fixture: a pump lot with 20 in the store and a dispatch of
/// 10 of it that left with a courier.
async fn sent_ten(t: &Booted) -> (wl_inventory::Dispatch, wl_inventory::Lot, Location) {
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", true, 5_000).await;
    let lot = t.lot_expiring(&pump, "L9", 30).await;
    t.stock_in(&pump, &store, Some(&lot), 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000).lot(lot.id)],
        )
        .await;
    (sent, lot, store)
}

/// AC1: Given a dispatch line 10 and `DeliveryDelivered` 8 delivered, 2
/// refused *damaged*, the dispatch is `delivered` with 2 *in return* and no
/// movement, and `DispatchDelivered` shows 8/2. A repeated proof changes
/// nothing.
#[renox::test]
async fn a_proof_of_delivery_confirms_the_dispatch_per_line_and_leaves_the_refused_in_return() {
    let t = boot().await;
    let (sent, _lot, _store) = sent_ten(&t).await;
    let line = t.dispatch_lines(&sent).await.remove(0);

    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;

    let delivered = t.dispatch_now(&sent).await;
    assert_eq!(delivered.status, DispatchStatus::Delivered);
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!((line.delivered_milli, line.refused_milli), (8_000, 2_000));
    assert_eq!(line.refusal_reason.as_deref(), Some("damaged"));
    assert_eq!(line.in_return_milli(), 2_000);
    // No movement for the refused quantity: the goods are with the driver.
    let moved = t.dispatch_movements(&sent).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].movement_type, MovementType::Dispatch);

    let events = t.delivered.all();
    assert_eq!(events.len(), 1);
    assert_eq!(
        (
            events[0].lines[0].delivered_milli,
            events[0].lines[0].refused_milli
        ),
        (8_000, 2_000)
    );
    assert_eq!(
        events[0].lines[0].refusal_reason.as_deref(),
        Some("damaged")
    );
    assert_eq!(events[0].source.to_string(), "sales.sales_order:41");

    // One confirmation per proof of delivery.
    t.emit_delivered(&sent, 77, &[(&line, 10_000, 0, None)])
        .await;
    assert_eq!(t.delivered.all().len(), 1);
    assert_eq!(t.dispatch_lines(&sent).await[0].delivered_milli, 8_000);
}

/// AC2: On `DeliveryRefusedReceived` with 2 received back, `return_in` 2 to
/// quarantine with reason *damaged* and the picked lot is posted; the dispatch
/// stays `delivered`. A repeated event posts nothing twice, and the refused
/// goods never reach free stock.
#[renox::test]
async fn refused_goods_received_back_go_to_quarantine_with_the_reason_and_the_lot() {
    let t = boot().await;
    let (sent, lot, store) = sent_ten(&t).await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    let free_before = t
        .available(&t.item_now_of(&line).await, &store, Some(&lot))
        .await;

    t.emit_refused_received(&sent, &[(&line, 2_000, 0)]).await;

    let moved = t.dispatch_movements(&sent).await;
    assert_eq!(moved.len(), 2);
    let back = &moved[1];
    assert_eq!(back.movement_type, MovementType::ReturnIn);
    assert_eq!(back.quantity_milli, 2_000);
    assert_eq!(back.lot_id, Some(lot.id));
    assert_eq!(back.reason.as_deref(), Some("damaged"));
    assert_eq!(back.to_location_id, Some(quarantine(&t).await.id));
    assert_eq!(back.from_location_id, None);
    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Delivered
    );
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!((line.returned_milli, line.in_return_milli()), (2_000, 0));
    // Not back in the store the goods left from.
    let item = t.item_now_of(&line).await;
    assert_eq!(t.available(&item, &store, Some(&lot)).await, free_before);

    t.emit_refused_received(&sent, &[(&line, 2_000, 0)]).await;
    assert_eq!(t.dispatch_movements(&sent).await.len(), 2);
}

/// AC3: Given 1 received and 1 missing, `return_in` 1 is posted, the value of
/// 1 goes to a loss (*missing on return*) and the dispatcher gets a task.
#[renox::test]
async fn a_missing_refused_quantity_is_a_loss_and_a_task_for_the_dispatcher() {
    let t = boot().await;
    let (sent, _lot, _store) = sent_ten(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("wrong_item"))])
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);

    t.emit_refused_received(&sent, &[(&line, 1_000, 1_000)])
        .await;

    let moved = t.dispatch_movements(&sent).await;
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[1].quantity_milli, 1_000);
    assert_eq!(moved[1].reason.as_deref(), Some("wrong_item"));
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].amount_aed, 5_000);
    assert_eq!(posted[0].cost_code.as_deref(), Some("missing_on_return"));
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].title.contains("missing on return") || tasks[0].title.contains("Goods missing")
    );
    assert_eq!(tasks[0].assignee, Some(Assignee::user(dispatcher.id)));
    let line = t.dispatch_lines(&sent).await.remove(0);
    assert_eq!((line.returned_milli, line.missing_milli), (1_000, 1_000));

    // The same event again posts no second loss and opens no second task.
    t.emit_refused_received(&sent, &[(&line, 1_000, 1_000)])
        .await;
    assert_eq!(t.costs.posted().len(), 1);
    assert_eq!(t.tickets.opened().len(), 1);
}

/// Whatever the refusal reason, the goods go to quarantine: a wrong item and
/// a temperature refusal alike.
#[renox::test]
async fn every_refusal_reason_goes_to_quarantine() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let milk = t.priced_item("MILK", false, 400).await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&milk, &store, None, 20_000, 400).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let sent = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![
                NewDispatchLine::of(milk.id, 10_000),
                NewDispatchLine::of(pump.id, 4_000),
            ],
        )
        .await;
    let lines = t.dispatch_lines(&sent).await;
    t.emit_delivered(
        &sent,
        5,
        &[
            (&lines[0], 7_000, 3_000, Some("temperature")),
            (&lines[1], 3_000, 1_000, Some("short_dated")),
        ],
    )
    .await;
    let lines = t.dispatch_lines(&sent).await;

    t.emit_refused_received(&sent, &[(&lines[0], 3_000, 0), (&lines[1], 1_000, 0)])
        .await;

    let quarantine = quarantine(&t).await.id;
    let back: Vec<_> = t
        .dispatch_movements(&sent)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::ReturnIn)
        .collect();
    assert_eq!(back.len(), 2);
    assert!(back.iter().all(|m| m.to_location_id == Some(quarantine)));
    assert_eq!(back[0].reason.as_deref(), Some("temperature"));
    assert_eq!(back[1].reason.as_deref(), Some("short_dated"));
}

/// AC4: Given a courier dispatch, a user enters 10/0 and the dispatch is
/// `delivered`. The entry is refused for a missing reason, too much, or an
/// own delivery (its proof comes from the driver); the user records the
/// refused goods that came back by hand.
#[renox::test]
async fn a_dispatcher_confirms_a_courier_delivery_and_records_the_refused_goods_back() {
    let t = boot().await;
    let (sent, lot, store) = sent_ten(&t).await;
    let dispatcher = t.dispatcher("Dina").await;
    let clerk = t.clerk("Carl").await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    let (delivered, refused, reason) = (
        format!("delivered_{}", line.id),
        format!("refused_{}", line.id),
        format!("reason_{}", line.id),
    );

    t.dispatch_action(&clerk, &sent, "confirm", &[(delivered.as_str(), "10")])
        .await
        .assert_forbidden();
    // Refused goods without a reason; more than was sent.
    let refusal = t
        .dispatch_action(
            &dispatcher,
            &sent,
            "confirm",
            &[(delivered.as_str(), "8"), (refused.as_str(), "2")],
        )
        .await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("give the reason"),
        "{}",
        refusal.text()
    );
    let refusal = t
        .dispatch_action(
            &dispatcher,
            &sent,
            "confirm",
            &[
                (delivered.as_str(), "9"),
                (refused.as_str(), "2"),
                (reason.as_str(), "damaged"),
            ],
        )
        .await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("more than the 10"),
        "{}",
        refusal.text()
    );
    assert_eq!(
        t.dispatch_now(&sent).await.status,
        DispatchStatus::Dispatched
    );

    t.dispatch_action(
        &dispatcher,
        &sent,
        "confirm",
        &[(delivered.as_str(), "10"), (refused.as_str(), "0")],
    )
    .await
    .assert_ok();
    let done = t.dispatch_now(&sent).await;
    assert_eq!(done.status, DispatchStatus::Delivered);
    assert_eq!(t.dispatch_lines(&sent).await[0].delivered_milli, 10_000);
    assert_eq!(t.delivered.all().len(), 1);

    // A second courier delivery with a refusal, recorded back by hand.
    let item = t.item_now_of(&line).await;
    t.stock_in(&item, &store, Some(&lot), 10_000, 5_000).await;
    let again = t
        .sent_dispatch(
            &dispatcher,
            &store,
            ORDER + 1,
            vec![NewDispatchLine::of(item.id, 10_000).lot(lot.id)],
        )
        .await;
    let line = t.dispatch_lines(&again).await.remove(0);
    let (delivered, refused, reason) = (
        format!("delivered_{}", line.id),
        format!("refused_{}", line.id),
        format!("reason_{}", line.id),
    );
    t.dispatch_action(
        &dispatcher,
        &again,
        "confirm",
        &[
            (delivered.as_str(), "8"),
            (refused.as_str(), "2"),
            (reason.as_str(), "damaged"),
        ],
    )
    .await
    .assert_ok();
    let (received, missing) = (
        format!("received_{}", line.id),
        format!("missing_{}", line.id),
    );
    let refusal = t
        .dispatch_action(
            &dispatcher,
            &again,
            "refused-back",
            &[(received.as_str(), "3")],
        )
        .await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("only 2 is still in return"),
        "{}",
        refusal.text()
    );
    t.dispatch_action(&clerk, &again, "refused-back", &[(received.as_str(), "2")])
        .await
        .assert_forbidden();
    t.dispatch_action(
        &dispatcher,
        &again,
        "refused-back",
        &[(received.as_str(), "2"), (missing.as_str(), "0")],
    )
    .await
    .assert_ok();
    let back: Vec<_> = t
        .dispatch_movements(&again)
        .await
        .into_iter()
        .filter(|m| m.movement_type == MovementType::ReturnIn)
        .collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].to_location_id, Some(quarantine(&t).await.id));
    assert_eq!(back[0].reason.as_deref(), Some("damaged"));
}

/// An own delivery cannot be confirmed by hand: the driver's proof of
/// delivery does it.
#[renox::test]
async fn an_own_delivery_is_not_confirmed_by_hand() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let picked = t
        .picked_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000)],
        )
        .await;
    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "own_delivery")],
    )
    .await
    .assert_ok();
    let line = t.dispatch_lines(&picked).await.remove(0);
    let delivered = format!("delivered_{}", line.id);

    let refusal = t
        .dispatch_action(
            &dispatcher,
            &picked,
            "confirm",
            &[(delivered.as_str(), "10")],
        )
        .await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("proof of delivery, not by hand"),
        "{}",
        refusal.text()
    );

    t.emit_delivered(&picked, 9, &[(&line, 10_000, 0, None)])
        .await;
    assert_eq!(
        t.dispatch_now(&picked).await.status,
        DispatchStatus::Delivered
    );
}

/// A missing quantity worth more than the wastage limit (`STK.ACL.02`) is
/// written off and the dispatcher's task says a second person must approve it.
#[renox::test]
async fn a_missing_loss_above_the_limit_says_a_second_person_must_approve() {
    let t = support::boot_with(wl_inventory::WlInventory::new().wastage_approval_fils(1_000)).await;
    let (sent, _lot, _store) = sent_ten(&t).await;
    let line = t.dispatch_lines(&sent).await.remove(0);
    t.emit_delivered(&sent, 77, &[(&line, 8_000, 2_000, Some("damaged"))])
        .await;
    let line = t.dispatch_lines(&sent).await.remove(0);

    t.emit_refused_received(&sent, &[(&line, 0, 2_000)]).await;

    assert_eq!(t.costs.posted()[0].amount_aed, 10_000);
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].description.contains("second person must approve"),
        "{}",
        tasks[0].description
    );
    // Nothing came back: no movement into quarantine.
    assert_eq!(t.dispatch_movements(&sent).await.len(), 1);
}
