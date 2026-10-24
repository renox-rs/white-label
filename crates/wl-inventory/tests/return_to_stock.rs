//! Goods put back into stock by the record that took them (wl-kernel
//! `StockService::return_to_stock`, 08 Auto `AUT.ACL.21`): a job card's parts
//! issued and not fitted go back to the store as a `return_in` movement.

mod support;

use wl_inventory::LotStatus;
use wl_inventory::movement::MovementType;
use wl_kernel::Subject;
use wl_kernel::inventory::{ReturnRequest, stock_service};

use support::boot;

fn job(n: i64) -> Subject {
    Subject::new("auto.job_card", n)
}

/// A job card took 3 of 5 units and gives 2 back: the balance is 4 and the
/// movement is a `return_in` at the average cost against the job card.
#[renox::test]
async fn parts_not_fitted_go_back_to_the_store() {
    let t = boot().await;
    let item = t.item("FILTER").await;
    let store = t.location("WORKSHOP-1", None).await;
    t.stock_in(&item, &store, None, 5_000, 800).await;
    let taken = t.consume(&item, &store, 3_000, &job(10)).await;
    assert_eq!(taken.posted_milli, 3_000);
    assert_eq!(t.available(&item, &store, None).await, 2_000);

    let back = stock_service(t.state())
        .return_to_stock(
            t.state(),
            ReturnRequest::new(item.subject(), store.subject(), 2_000, job(10))
                .cost_object(job(10))
                .reason("not fitted"),
        )
        .await
        .unwrap();

    assert_eq!(back.posted_milli, 2_000);
    assert_eq!(t.available(&item, &store, None).await, 4_000);
    let moved = t.movements_of(&item).await;
    let returned: Vec<_> = moved
        .iter()
        .filter(|m| m.movement_type == MovementType::ReturnIn)
        .collect();
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].quantity_milli, 2_000);
    assert_eq!(returned[0].to_location_id, Some(store.id));
    assert_eq!(returned[0].source(), job(10));
    assert_eq!(returned[0].cost_object_id, Some(10));
    assert_eq!(returned[0].reason.as_deref(), Some("not fitted"));
}

/// A quantity that is not above zero is refused, and an item tracked by lot
/// goes back to a lot, not to nowhere.
#[renox::test]
async fn a_return_names_a_quantity_and_the_lot_of_a_tracked_item() {
    let t = boot().await;
    let plain = t.item("BULB").await;
    let store = t.location("WORKSHOP-1", None).await;
    let service = stock_service(t.state());
    let none = service
        .return_to_stock(
            t.state(),
            ReturnRequest::new(plain.subject(), store.subject(), 0, job(1)),
        )
        .await;
    assert!(none.is_err());

    let tracked = t.tracked_item("OIL").await;
    let without = service
        .return_to_stock(
            t.state(),
            ReturnRequest::new(tracked.subject(), store.subject(), 1_000, job(1)),
        )
        .await
        .unwrap_err();
    assert!(without.to_string().contains("tracked by lot"), "{without}");

    let lot = t.lot(&tracked, "L1", LotStatus::Released).await;
    let with = service
        .return_to_stock(
            t.state(),
            ReturnRequest::new(tracked.subject(), store.subject(), 1_000, job(1))
                .lot(Subject::new("inventory.lot", lot.id)),
        )
        .await
        .unwrap();
    assert_eq!(with.posted_milli, 1_000);
    assert_eq!(t.available(&tracked, &store, Some(&lot)).await, 1_000);
}
