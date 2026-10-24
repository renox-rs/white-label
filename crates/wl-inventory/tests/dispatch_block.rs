//! BL-STK-21 (#240): a dispatch does not leave on a failed credit re-check or
//! a picked lot on hold, and `pick` refuses held quantity. Capabilities
//! STK.DSP.01 (the dispatch guards of the Dispatch lifecycle),
//! SAL.ACL.03 through the kernel `CreditChecker` seam.

mod support;

use wl_inventory::{DispatchStatus, NewDispatchLine};
use wl_kernel::sales::CreditFailure;

use support::{ORDER, boot};

/// AC1: Given a picked dispatch for an on-account order whose `credit_recheck`
/// fails, `dispatch` is refused, the dispatch is `picked`, stock still
/// reserved, the order `credit_hold`; once the order is released the dispatch
/// goes (the check ran again).
#[renox::test]
async fn a_failed_credit_recheck_keeps_the_dispatch_picked_and_reserved() {
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
    assert_eq!(t.available(&pump, &store, None).await, 10_000);
    t.credit.fail(ORDER, CreditFailure::Overdue);

    let refusal = t
        .dispatch_action(
            &dispatcher,
            &picked,
            "dispatch",
            &[("handover", "courier"), ("tracking_number", "TRK-7")],
        )
        .await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("credit re-check failed"),
        "{}",
        refusal.text()
    );

    let after = t.dispatch_now(&picked).await;
    assert_eq!(after.status, DispatchStatus::Picked);
    // Still reserved: nothing left stock and the reservation stays.
    assert_eq!(t.available(&pump, &store, None).await, 10_000);
    assert!(t.dispatch_movements(&picked).await.is_empty());
    assert_eq!(t.credit.held(), vec![ORDER]);
    assert!(t.sent.all().is_empty());

    // The order is released later: the guard runs again and the goods go.
    t.credit.release(ORDER);
    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "courier"), ("tracking_number", "TRK-7")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.dispatch_now(&picked).await.status,
        DispatchStatus::Dispatched
    );
    assert_eq!(t.credit.asked(), vec![ORDER, ORDER]);
    assert_eq!(t.sent.all().len(), 1);
}

/// AC2: Given a picked lot put on hold, `dispatch` is refused with "re-pick";
/// after re-pick from another lot it succeeds. The lot hold comes first: the
/// credit is not checked for a dispatch that cannot leave anyway.
#[renox::test]
async fn a_picked_lot_put_on_hold_is_refused_with_repick_and_goes_after_a_repick() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", true, 5_000).await;
    let held = t.lot_expiring(&pump, "L9", 30).await;
    let other = t.lot_expiring(&pump, "L10", 60).await;
    t.stock_in(&pump, &store, Some(&held), 10_000, 5_000).await;
    t.stock_in(&pump, &store, Some(&other), 10_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let picked = t
        .picked_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000).lot(held.id)],
        )
        .await;
    // The quality manager holds the lot after picking; the order would fail
    // its credit too, but the lot comes first.
    t.hold_lot(&held).await;
    t.credit.fail(ORDER, CreditFailure::Limit);

    let refusal = t
        .dispatch_action(
            &dispatcher,
            &picked,
            "dispatch",
            &[("handover", "courier"), ("tracking_number", "TRK-8")],
        )
        .await;
    refusal.assert_status(422);
    let said = refusal.text().to_lowercase();
    assert!(said.contains("re-pick") && said.contains("l9"), "{said}");
    assert_eq!(t.dispatch_now(&picked).await.status, DispatchStatus::Picked);
    assert!(
        t.credit.asked().is_empty(),
        "no credit check for a dispatch that cannot leave"
    );
    assert!(t.credit.held().is_empty());

    // Re-pick the line from the first released lot to expire.
    let line = t.dispatch_lines(&picked).await.remove(0);
    let line_id = line.id.to_string();
    t.dispatch_action(
        &dispatcher,
        &picked,
        "repick",
        &[("line", line_id.as_str())],
    )
    .await
    .assert_ok();
    let lines = t.dispatch_lines(&picked).await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].lot_id, Some(other.id));
    assert_eq!(lines[0].quantity_milli, 10_000);
    // The held lot's reservation is gone; the new lot's is on.
    assert_eq!(t.available(&pump, &store, Some(&other)).await, 0);

    t.credit.release(ORDER);
    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "courier"), ("tracking_number", "TRK-8")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.dispatch_now(&picked).await.status,
        DispatchStatus::Dispatched
    );
    let moved = t.dispatch_movements(&picked).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].lot_id, Some(other.id));
}

/// AC3: `pick` of a held quantity is refused: a lot on hold, and a quantity
/// held at the location.
#[renox::test]
async fn pick_refuses_a_held_lot_and_a_held_quantity() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", true, 5_000).await;
    let lot = t.lot_expiring(&pump, "L9", 30).await;
    let fridge_item = t.priced_item("MILK", false, 400).await;
    t.stock_in(&pump, &store, Some(&lot), 10_000, 5_000).await;
    t.stock_in(&fridge_item, &store, None, 10_000, 400).await;
    let dispatcher = t.dispatcher("Dina").await;

    // A lot on hold.
    t.hold_lot(&lot).await;
    let draft = t
        .draft_dispatch(
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 5_000).lot(lot.id)],
        )
        .await;
    let refusal = t.dispatch_action(&dispatcher, &draft, "pick", &[]).await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("L9 is on hold"),
        "{}",
        refusal.text()
    );
    assert_eq!(t.dispatch_now(&draft).await.status, DispatchStatus::Draft);

    // A quantity held at the location (a temperature hold on the store).
    let holder = t.holder("Hal").await;
    let store_id = store.id.to_string();
    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "clinics.checklist_reading:3"),
            ("location", store_id.as_str()),
            ("items", fridge_item.id.to_string().as_str()),
        ],
    )
    .await
    .assert_ok();
    let milk = t
        .draft_dispatch(
            &store,
            ORDER,
            vec![NewDispatchLine::of(fridge_item.id, 4_000)],
        )
        .await;
    let refusal = t.dispatch_action(&dispatcher, &milk, "pick", &[]).await;
    refusal.assert_status(422);
    assert!(refusal.text().contains("on hold"), "{}", refusal.text());
    assert_eq!(t.dispatch_now(&milk).await.status, DispatchStatus::Draft);
}

/// Picking reserves the stock, cancelling gives it back, and a person without
/// `dispatch.manage` is refused.
#[renox::test]
async fn picking_reserves_stock_cancelling_releases_it_and_only_a_dispatcher_picks() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 10_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let clerk = t.clerk("Carl").await;
    let draft = t
        .draft_dispatch(&store, ORDER, vec![NewDispatchLine::of(pump.id, 6_000)])
        .await;

    t.dispatch_action(&clerk, &draft, "pick", &[])
        .await
        .assert_forbidden();
    t.dispatch_action(&dispatcher, &draft, "pick", &[])
        .await
        .assert_ok();
    assert_eq!(t.available(&pump, &store, None).await, 4_000);

    // A second dispatch cannot take what the first reserved.
    let greedy = t
        .draft_dispatch(&store, ORDER + 1, vec![NewDispatchLine::of(pump.id, 6_000)])
        .await;
    let refusal = t.dispatch_action(&dispatcher, &greedy, "pick", &[]).await;
    refusal.assert_status(422);
    assert!(refusal.text().contains("only 4"), "{}", refusal.text());

    t.dispatch_action(
        &dispatcher,
        &draft,
        "cancel",
        &[("reason", "Customer asked to wait")],
    )
    .await
    .assert_ok();
    let cancelled = t.dispatch_now(&draft).await;
    assert_eq!(cancelled.status, DispatchStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("Customer asked to wait")
    );
    assert_eq!(t.available(&pump, &store, None).await, 10_000);
}

/// A dispatch leaves stock as `dispatch` movements at the item's cost, with a
/// delivery note, and an own delivery asks wl-delivery for a delivery that
/// names the dispatch lines; a courier needs a tracking number and a
/// collection the collector's signature.
#[renox::test]
async fn a_dispatch_posts_its_movements_a_delivery_note_and_the_handover() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&pump, &store, None, 30_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let picked = t
        .picked_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(pump.id, 10_000)],
        )
        .await;

    // The hand-over is chosen and complete.
    for (form, said) in [
        (vec![], "Choose how the goods are handed over"),
        (vec![("handover", "courier")], "tracking number"),
        (vec![("handover", "collection")], "signature"),
    ] {
        let refusal = t
            .dispatch_action(&dispatcher, &picked, "dispatch", &form)
            .await;
        refusal.assert_status(422);
        assert!(refusal.text().contains(said), "{}", refusal.text());
    }
    assert!(t.deliveries.requested().is_empty());

    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "own_delivery")],
    )
    .await
    .assert_ok();
    let sent = t.dispatch_now(&picked).await;
    assert_eq!(sent.status, DispatchStatus::Dispatched);
    assert_eq!(sent.handover.as_deref(), Some("own_delivery"));
    assert_eq!(sent.delivery_id, Some(1));
    assert!(
        sent.delivery_note
            .as_deref()
            .is_some_and(|note| note.starts_with("%PDF"))
    );
    let moved = t.dispatch_movements(&picked).await;
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].movement_type, wl_inventory::MovementType::Dispatch);
    assert_eq!((moved[0].quantity_milli, moved[0].value), (10_000, 50_000));
    assert_eq!(t.available(&pump, &store, None).await, 20_000);

    let asked = t.deliveries.requested();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].source, picked.subject());
    assert_eq!(asked[0].recipient_name, "Al Noor Trading");
    let lines = t.dispatch_lines(&picked).await;
    assert_eq!(asked[0].items[0].source_line, Some(lines[0].subject()));
    let event = t.sent.all().remove(0);
    assert_eq!(event.handover, "own_delivery");
    assert_eq!(event.delivery, Some(1));
    assert_eq!(event.lines[0].value, 50_000);
}
