//! The stock movements of a work order (07 Manufacturing §7.1 *Work order*,
//! base for BL-MFG-01): the kernel's `ProductionStock` posts a return of
//! unused material, a wastage and the good output inside the caller's
//! transaction, and a consumption tells the cost of each lot it took.

mod support;

use renox::prelude::*;
use wl_inventory::LotStatus;
use wl_inventory::movement::MovementType;
use wl_kernel::Subject;
use wl_kernel::inventory::{ConsumeRequest, stock_service};
use wl_kernel::production::{
    ProduceOutput, ReturnMaterial, VoidOutput, WasteMaterial, production_stock,
};

use support::boot;

fn order() -> Subject {
    Subject::new("manufacturing.work_order", 100)
}

/// A consumption reports the cost of one unit of each lot it took, so a work
/// order can value what goes back or is scrapped.
#[renox::test]
async fn a_consumption_tells_the_cost_of_the_lot_it_took() {
    let t = boot().await;
    let item = t.tracked_item("RESIN").await;
    let store = t.location("STORE-1", None).await;
    let lot = t.lot(&item, "R1", LotStatus::Released).await;
    t.stock_in(&item, &store, Some(&lot), 500_000, 800).await;

    let done = stock_service(t.state())
        .consume(
            t.state(),
            ConsumeRequest::new(item.subject(), store.subject(), 500_000, order()),
        )
        .await
        .unwrap();

    assert_eq!(done.lots.len(), 1);
    assert_eq!(done.lots[0].unit_cost, item.average_cost);
}

/// A return goes back to the store with its lot, a wastage writes the value
/// off, and neither is posted when the caller's transaction rolls back.
#[renox::test]
async fn a_return_and_a_wastage_post_inside_the_callers_transaction() {
    let t = boot().await;
    let item = t.tracked_item("RESIN").await;
    let store = t.location("STORE-1", None).await;
    let lot = t.lot(&item, "R1", LotStatus::Released).await;
    let stock = production_stock(t.state());

    let mut tx = t.db().begin().await.unwrap();
    stock
        .return_material(
            &mut tx,
            ReturnMaterial {
                source: order(),
                item: item.subject(),
                lot: Some(wl_kernel::Subject::new("inventory.lot", lot.id)),
                location: store.subject(),
                quantity_milli: 200_000,
                unit_cost: 800,
                user: None,
            },
        )
        .await
        .unwrap();
    stock
        .waste_material(
            &mut tx,
            WasteMaterial {
                source: order(),
                item: item.subject(),
                lot: None,
                quantity_milli: 187_500,
                value: 150_000,
                reason: "wrong BOM".into(),
                user: None,
            },
        )
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert!(t.movements_of(&item).await.is_empty());

    let mut tx = t.db().begin().await.unwrap();
    stock
        .return_material(
            &mut tx,
            ReturnMaterial {
                source: order(),
                item: item.subject(),
                lot: Some(wl_kernel::Subject::new("inventory.lot", lot.id)),
                location: store.subject(),
                quantity_milli: 200_000,
                unit_cost: 800,
                user: None,
            },
        )
        .await
        .unwrap();
    stock
        .waste_material(
            &mut tx,
            WasteMaterial {
                source: order(),
                item: item.subject(),
                lot: None,
                quantity_milli: 187_500,
                value: 150_000,
                reason: "wrong BOM".into(),
                user: None,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let moved = t.movements_of(&item).await;
    assert_eq!(moved.len(), 2);
    assert_eq!(moved[0].movement_type, MovementType::ReturnIn);
    assert_eq!(moved[0].value, 160_000);
    assert_eq!(moved[0].to_location_id, Some(store.id));
    assert_eq!(moved[0].lot_id, Some(lot.id));
    assert_eq!(moved[1].movement_type, MovementType::Wastage);
    assert_eq!(moved[1].value, 150_000);
    assert_eq!(moved[1].reason.as_deref(), Some("wrong BOM"));
    assert_eq!(moved[1].source(), order());
    assert_eq!(t.available(&item, &store, Some(&lot)).await, 200_000);
}

/// The first output of a work order opens its lot in quarantine; the next
/// one goes into the same lot.
#[renox::test]
async fn the_first_output_opens_the_lot_in_quarantine() {
    let t = boot().await;
    let item = t.tracked_item("PANEL").await;
    let finished = t.location("FG-1", None).await;
    let stock = production_stock(t.state());
    let request = |lot: Option<Subject>, quantity_milli: i64| ProduceOutput {
        source: order(),
        item: item.subject(),
        location: finished.subject(),
        lot,
        lot_number: "F-WO-100".into(),
        quantity_milli,
        unit_cost: 2_400,
        user: None,
    };

    let mut tx = t.db().begin().await.unwrap();
    let lot = stock
        .produce(&mut tx, request(None, 30_000))
        .await
        .unwrap()
        .expect("a lot");
    let again = stock
        .produce(&mut tx, request(Some(lot.clone()), 20_000))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    assert_eq!(again, Some(lot.clone()));
    let stored = wl_inventory::Lot::find_or_404(t.db(), lot.subject_id)
        .await
        .unwrap();
    assert_eq!(stored.status, LotStatus::Quarantine);
    assert_eq!(stored.number, "F-WO-100");
    assert_eq!(stored.source(), Some(order()));
    let moved = t.movements_of(&item).await;
    assert_eq!(moved.len(), 2);
    assert!(
        moved
            .iter()
            .all(|m| m.movement_type == MovementType::Produce)
    );
    assert_eq!(t.available(&item, &finished, Some(&stored)).await, 50_000);
}

/// BL-MFG-04: the good output of a voided report leaves the lot with a
/// reversing movement; when part of the lot is gone it is refused with
/// "output already used" and nothing is posted; the wastage of the scrap is
/// reversed without a location.
#[renox::test]
async fn a_voided_reports_output_leaves_the_lot_unless_it_was_used() {
    let t = boot().await;
    let item = t.tracked_item("PANEL").await;
    let finished = t.location("FG-1", None).await;
    let other = t.location("FG-2", None).await;
    let stock = production_stock(t.state());
    let mut tx = t.db().begin().await.unwrap();
    let lot = stock
        .produce(
            &mut tx,
            ProduceOutput {
                source: order(),
                item: item.subject(),
                location: finished.subject(),
                lot: None,
                lot_number: "F-WO-100".into(),
                quantity_milli: 300_000,
                unit_cost: 2_400,
                user: None,
            },
        )
        .await
        .unwrap()
        .expect("a lot");
    tx.commit().await.unwrap();
    let void = |quantity_milli: i64| VoidOutput {
        source: order(),
        item: item.subject(),
        lot: Some(lot.clone()),
        location: finished.subject(),
        quantity_milli,
        unit_cost: 2_400,
        reason: "wrong WO".into(),
        user: None,
    };

    assert_eq!(
        stock
            .output_available(t.state(), &item.subject(), Some(&lot), &finished.subject())
            .await
            .unwrap(),
        Some(300_000)
    );
    let mut tx = t.db().begin().await.unwrap();
    stock.void_output(&mut tx, void(120_000)).await.unwrap();
    tx.commit().await.unwrap();
    let stored = wl_inventory::Lot::find_or_404(t.db(), lot.subject_id)
        .await
        .unwrap();
    assert_eq!(t.available(&item, &finished, Some(&stored)).await, 180_000);

    // 100 are left after 80 moved out: a void of 120 is refused, nothing posts.
    t.stock_out(&item, &finished, Some(&stored), 80_000).await;
    let before = t.movements_of(&item).await.len();
    let mut tx = t.db().begin().await.unwrap();
    let refused = stock.void_output(&mut tx, void(120_000)).await;
    tx.rollback().await.unwrap();
    assert!(matches!(
        refused,
        Err(Error::Status(StatusCode::UNPROCESSABLE_ENTITY, ref message))
            if message.contains("Output already used")
    ));
    assert_eq!(t.movements_of(&item).await.len(), before);
    let _ = other;

    let mut tx = t.db().begin().await.unwrap();
    stock
        .void_waste(
            &mut tx,
            WasteMaterial {
                source: order(),
                item: item.subject(),
                lot: None,
                quantity_milli: 5_000,
                value: 12_000,
                reason: "wrong WO".into(),
                user: None,
            },
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let last = t.movements_of(&item).await.pop().unwrap();
    assert_eq!(last.movement_type, MovementType::Adjust);
    assert_eq!((last.from_location_id, last.to_location_id), (None, None));
    assert_eq!(last.value, 12_000);
}
