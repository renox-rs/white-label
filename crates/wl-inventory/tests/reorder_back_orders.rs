//! `STK.RUL.04` with back-orders of the sales module (BL-STK-17, BL-SAL-17): a
//! same-day rerun drops the suggestion of an item that has a reorder point
//! once its back-order is gone and the stock is above the point.

mod support;

use renox::prelude::*;
use wl_inventory::reorder;
use wl_inventory::{Item, ReorderPolicy, ReorderSuggestion, SuggestionStatus};
use wl_kernel::PartyId;

use support::{Booted, boot};

const AL_NOOR: PartyId = PartyId(21);

async fn run(t: &Booted) {
    t.app.run_scheduled(reorder::TASK).await.unwrap();
}

async fn suggestions(t: &Booted, item: &Item) -> Vec<ReorderSuggestion> {
    ReorderSuggestion::query()
        .where_(ReorderSuggestion::ITEM_ID.eq(item.id))
        .get(t.db())
        .await
        .unwrap()
}

/// "Cement": reorder point 20, 50 on hand (above it), Al Noor preferred, and
/// 5 bags back-ordered by a sales order: the first run suggests the 5; the
/// order is cancelled; the rerun the same day drops the suggestion. An open
/// back-order that is still there is kept by a rerun.
#[renox::test]
async fn a_cancelled_back_order_drops_the_suggestion_of_an_item_with_a_reorder_point() {
    let t = boot().await;
    let mut item = Item::new("CEMENT", "Cement 50 kg", "bag")
        .reordered(20_000, 100_000)
        .with_policy(ReorderPolicy::MinMax);
    item.save(t.db()).await.unwrap();
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 50_000, 1_800).await;
    let a = t.procurement("Amal").await;
    t.app.acting_as(&a);
    let id = AL_NOOR.0.to_string();
    t.app
        .htmx()
        .post(
            &format!("/items/{}/suppliers/add", item.id),
            &[
                ("supplier", id.as_str()),
                ("role", "preferred"),
                ("lead_days", "5"),
            ],
        )
        .await
        .assert_ok();

    run(&t).await;
    assert!(
        suggestions(&t, &item).await.is_empty(),
        "above the point and nothing back-ordered"
    );

    t.back_orders.set("CEMENT", &store, 5_000);
    run(&t).await;
    let found = suggestions(&t, &item).await;
    assert_eq!(found.len(), 1, "the back-order is suggested");
    assert_eq!(found[0].status, SuggestionStatus::Open);

    run(&t).await;
    assert_eq!(suggestions(&t, &item).await.len(), 1, "still back-ordered");

    t.back_orders.set("CEMENT", &store, 0);
    run(&t).await;
    assert!(
        suggestions(&t, &item).await.is_empty(),
        "the back-order is gone and the stock is above the point"
    );
}
