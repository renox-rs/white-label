//! What a vertical reads before it consumes (`STK.MOV.01`, `STK.LOT.01`): the
//! store of a branch, the lots to take from first expiry first, the lot a
//! user picked going first, and the lots a consumption posted against.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::{LotStatus, StockMovement};
use wl_kernel::Subject;
use wl_kernel::inventory::{ConsumeRequest, LOT_SUBJECT_TYPE, stock_service};

use support::boot;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The store of a scope, the lots first expiry first with the balance there
/// (a held or expired lot left out), and a consumption that takes the lot
/// the user picked first and names the lots it posted.
#[renox::test]
async fn the_choices_are_first_expiry_first_and_a_picked_lot_goes_first() {
    let t = boot().await;
    let branch = Subject::new("test.branch", 2);
    let store = t.location("STORE-B2", Some(&branch)).await;
    let item = t.tracked_item("FILLER").await;
    let mut early = t.lot(&item, "L1", LotStatus::Released).await;
    early.expiry_date = Some(day(2027, 1, 31));
    early.save(t.db()).await.unwrap();
    let mut late = t.lot(&item, "L2", LotStatus::Released).await;
    late.expiry_date = Some(day(2027, 3, 31));
    late.save(t.db()).await.unwrap();
    let held = t.lot(&item, "L3", LotStatus::OnHold).await;
    t.stock_in(&item, &store, Some(&early), 1_000, 500).await;
    t.stock_in(&item, &store, Some(&late), 2_000, 500).await;
    t.stock_in(&item, &store, Some(&held), 1_000, 500).await;

    let stock = stock_service(t.state());
    let found = stock
        .store_of(t.state(), "test.branch", 2)
        .await
        .unwrap()
        .expect("the branch has a store");
    assert_eq!(found, store.subject());
    assert_eq!(stock.store_of(t.state(), "test.branch", 9).await.unwrap(), None);

    let choice = stock
        .choices(t.state(), "FILLER", &found)
        .await
        .unwrap()
        .expect("the item exists");
    assert!(choice.lot_tracked);
    let numbers: Vec<&str> = choice.lots.iter().map(|l| l.number.as_str()).collect();
    assert_eq!(numbers, ["L1", "L2"]);
    assert_eq!(choice.lots[0].available_milli, 1_000);
    assert_eq!(choice.lots[0].expiry_date, Some(day(2027, 1, 31)));
    assert!(stock.choices(t.state(), "NOPE", &found).await.unwrap().is_none());

    // The user picked L2: it is taken first, then L1 first expiry first.
    let source = Subject::new("clinics.visit", 1);
    let done = stock
        .consume(
            t.state(),
            ConsumeRequest::new(item.subject(), found.clone(), 2_500, source.clone())
                .cost_object(source)
                .lot(Subject::new(LOT_SUBJECT_TYPE, late.id)),
        )
        .await
        .unwrap();
    assert_eq!(done.posted_milli, 2_500);
    assert_eq!(done.short_milli, 0);
    let lots: Vec<(Option<i64>, i64)> = done
        .lots
        .iter()
        .map(|l| (l.lot.as_ref().map(|s| s.subject_id), l.quantity_milli))
        .collect();
    assert_eq!(lots, [(Some(late.id), 2_000), (Some(early.id), 500)]);
    let moved: Vec<StockMovement> = t.movements_of(&item).await;
    assert_eq!(
        moved
            .iter()
            .filter(|m| m.lot_id == Some(early.id) && m.from_location_id.is_some())
            .count(),
        1
    );
}
