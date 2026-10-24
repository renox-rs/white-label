//! The vertical's item-flag guard at `pick` and `dispatch` (BL-MFG-11, #392;
//! wl-inventory §4 *Dispatch* rows `pick` and `dispatch`, STK.DSP.01; 07
//! `MFG.ACL.11`) through the kernel's `DispatchGuard` seam: a refusal is a 422
//! with the vertical's words, the dispatch stays as it is and can be
//! cancelled, the guard is asked again on the dispatch, and a deployment
//! without a guard blocks nothing.

mod support;

use wl_inventory::{DispatchStatus, NewDispatchLine};

use support::{ORDER, boot};

/// A dispatch of a blocked item is refused at `pick` with the words of the
/// guard and stays `draft` (and can be cancelled); an item that is not
/// blocked on the same kind of dispatch is picked.
#[renox::test]
async fn pick_is_refused_with_the_words_of_the_guard() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let lamp = t.priced_item("LAMP", false, 5_000).await;
    let pump = t.priced_item("PUMP", false, 5_000).await;
    t.stock_in(&lamp, &store, None, 20_000, 5_000).await;
    t.stock_in(&pump, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    t.item_guard
        .block(lamp.id, "LAMP has no valid ECAS certificate");
    let blocked = t
        .draft_dispatch(&store, ORDER, vec![NewDispatchLine::of(lamp.id, 5_000)])
        .await;
    let free = t
        .draft_dispatch(&store, ORDER, vec![NewDispatchLine::of(pump.id, 5_000)])
        .await;

    let refusal = t.dispatch_action(&dispatcher, &blocked, "pick", &[]).await;
    refusal.assert_status(422);
    assert!(
        refusal.text().contains("no valid ECAS certificate"),
        "{}",
        refusal.text()
    );
    assert_eq!(
        t.dispatch_now(&blocked).await.status,
        DispatchStatus::Draft,
        "nothing is reserved"
    );
    assert_eq!(t.available(&lamp, &store, None).await, 20_000);
    t.dispatch_action(&dispatcher, &free, "pick", &[])
        .await
        .assert_ok();
    assert_eq!(t.dispatch_now(&free).await.status, DispatchStatus::Picked);

    // The refused dispatch is cancelled.
    t.dispatch_action(
        &dispatcher,
        &blocked,
        "cancel",
        &[("reason", "no certificate")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.dispatch_now(&blocked).await.status,
        DispatchStatus::Cancelled
    );
}

/// The guard is asked again at `dispatch`, on that day: a picked dispatch
/// whose item is blocked after the pick is refused, stays `picked` and
/// reserved, and can be cancelled; once the block is gone it leaves.
#[renox::test]
async fn dispatch_asks_the_guard_again_on_the_day_and_keeps_the_dispatch_picked() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jebel Ali warehouse").await;
    let lamp = t.priced_item("LAMP", false, 5_000).await;
    t.stock_in(&lamp, &store, None, 20_000, 5_000).await;
    let dispatcher = t.dispatcher("Dina").await;
    let picked = t
        .picked_dispatch(
            &dispatcher,
            &store,
            ORDER,
            vec![NewDispatchLine::of(lamp.id, 5_000)],
        )
        .await;
    t.item_guard
        .block(lamp.id, "LAMP certificate expired on 2026-11-30");

    let refusal = t
        .dispatch_action(
            &dispatcher,
            &picked,
            "dispatch",
            &[("handover", "courier"), ("tracking_number", "TRK-1")],
        )
        .await;
    refusal.assert_status(422);
    assert!(refusal.text().contains("expired on 2026-11-30"));
    assert_eq!(t.dispatch_now(&picked).await.status, DispatchStatus::Picked);
    assert_eq!(t.available(&lamp, &store, None).await, 15_000);
    assert!(t.sent.all().is_empty());
    assert_eq!(
        t.item_guard
            .asked()
            .iter()
            .map(|(step, _)| step.as_str())
            .collect::<Vec<_>>(),
        vec!["dispatch.pick", "dispatch.dispatch"]
    );

    t.item_guard.clear();
    t.dispatch_action(
        &dispatcher,
        &picked,
        "dispatch",
        &[("handover", "courier"), ("tracking_number", "TRK-1")],
    )
    .await
    .assert_ok();
    assert_eq!(
        t.dispatch_now(&picked).await.status,
        DispatchStatus::Dispatched
    );
}
