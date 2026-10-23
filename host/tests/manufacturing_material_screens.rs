//! BL-MFG-05 (#386) and BL-MFG-09 (#390) through the real stock of
//! wl-inventory and the real tickets: a short issue gives the line's
//! supervisor a task to hold the work order, the arrival of the material a
//! task to resume it, and a lot-tracked material is issued with its lot.

mod manufacturing_support;

use host::roles::{STOREKEEPER, SUPERVISOR};
use manufacturing::material_hold::{hold_key, resume_key};
use manufacturing::{MaterialIssue, WorkOrder, WorkOrderStatus};
use manufacturing_support::{boot, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::movement::StockMovement;
use wl_inventory::pending_issue::post_on_arrival;
use wl_inventory::transfer::{NewTransfer, NewTransferLine, create_draft};
use wl_inventory::{Location, LotStatus};
use wl_kernel::tickets::task_creator;

/// The resin is short at the store: the issue of 20 kg waits as a pending
/// issue, the supervisor gets one task to hold the work order, holds it for
/// material, and when 30 kg of released lot R1 are received and the pending
/// issue posts she gets the task to resume it; the row takes the lot and the
/// cost. A transfer of 5 kg out of an empty store is refused.
#[renox::test]
async fn a_short_issue_holds_and_the_arrival_resumes_through_the_real_stock() {
    let app = boot().await;
    let f = floor(&app).await;
    let sami = person(&app, "Sami", SUPERVISOR, Some(f.line.scope())).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let path = format!("/manufacturing/work-orders/{}", f.order.id);
    post(&app, &sami, &format!("{path}/start"), &[])
        .await
        .assert_status(303);
    let creator = task_creator(app.state());

    // Nothing is on hand: the issue waits.
    post(
        &app,
        &keeper,
        &format!("{path}/issue"),
        &[("item_id", &f.resin.id.to_string()), ("quantity", "20")],
    )
    .await
    .assert_status(303);
    let rows = MaterialIssue::of_order(app.db(), f.order.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].pending_issue_id.is_some() && rows[0].lot_id.is_none());
    assert!(
        creator
            .find_open(app.state(), &hold_key(f.order.id))
            .await
            .unwrap()
            .is_some(),
        "the supervisor has the task to hold"
    );

    post(
        &app,
        &sami,
        &format!("{path}/hold"),
        &[("reason", "material")],
    )
    .await
    .assert_status(303);
    let held = WorkOrder::find_or_404(app.db(), f.order.id).await.unwrap();
    assert_eq!(held.status, WorkOrderStatus::OnHold);
    assert!(
        creator
            .find_open(app.state(), &hold_key(f.order.id))
            .await
            .unwrap()
            .is_none(),
        "holding closed the task"
    );

    // Nothing of pigment P is at the store: a transfer of 5 kg is refused.
    let mut pigment = wl_inventory::Item::new("PIG", "Pigment", "kg");
    pigment.save(app.db()).await.unwrap();
    let mut other = Location::new("OTHER", "Other store");
    other.save(app.db()).await.unwrap();
    let draft = create_draft(
        app.db(),
        Some(keeper.id),
        NewTransfer {
            from_location_id: f.store.id,
            to_location_id: other.id,
            lines: vec![NewTransferLine::of(pigment.id, 5_000)],
        },
    )
    .await
    .unwrap();
    let refused = post(&app, &keeper, &format!("/transfers/{}/send", draft.id), &[]).await;
    refused.assert_status(422);
    assert!(refused.text().contains("available"), "{}", refused.text());

    // 30 kg of lot R1 are received; the pending issue posts from it.
    let lot = wl_inventory::Lot {
        item_id: f.resin.id,
        number: "R1".into(),
        status: LotStatus::Released,
        ..wl_inventory::Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    let mut lot = lot;
    lot.insert_in_state(&mut tx, LotStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut received = StockMovement::receive(
        f.resin.id,
        Some(lot.id),
        f.store.id,
        30_000,
        800,
        24_000,
        &wl_kernel::Subject::new("inventory.goods_receipt", 1),
        None,
    );
    received.save(app.db()).await.unwrap();
    post_on_arrival(app.state(), f.store.id, &[f.resin.id], "receipt")
        .await
        .unwrap();

    assert!(
        creator
            .find_open(app.state(), &resume_key(f.order.id))
            .await
            .unwrap()
            .is_some(),
        "the supervisor has the task to resume"
    );
    let rows = MaterialIssue::of_order(app.db(), f.order.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].lot_id, Some(lot.id));
    assert_eq!(rows[0].unit_cost, 800);
    assert!(rows[0].pending_issue_id.is_none());

    post(&app, &sami, &format!("{path}/resume"), &[])
        .await
        .assert_status(303);
    assert!(
        creator
            .find_open(app.state(), &resume_key(f.order.id))
            .await
            .unwrap()
            .is_none()
    );
}

/// BL-MFG-09: 10 kg of the lot-tracked resin (lot R3, released) are at the
/// store. An issue of 10 kg without a lot is refused naming the lot; an issue
/// of 25 kg with lot R3 posts 10 kg from R3 and keeps a pending issue of 15 kg
/// with no lot; a lot number that is not at the store is refused.
#[renox::test]
async fn a_tracked_material_needs_its_lot_and_only_the_short_part_has_none() {
    let app = boot().await;
    let f = floor(&app).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let path = format!("/manufacturing/work-orders/{}/issue", f.order.id);
    let mut lot = wl_inventory::Lot {
        item_id: f.resin.id,
        number: "R3".into(),
        status: LotStatus::Released,
        ..wl_inventory::Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, LotStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    StockMovement::receive(
        f.resin.id,
        Some(lot.id),
        f.store.id,
        10_000,
        800,
        8_000,
        &wl_kernel::Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    let resin = f.resin.id.to_string();

    let refused = post(
        &app,
        &keeper,
        &path,
        &[("item_id", &resin), ("quantity", "10")],
    )
    .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("Name the lot"),
        "{}",
        refused.text()
    );
    assert!(refused.text().contains("R3"), "{}", refused.text());
    let other = post(
        &app,
        &keeper,
        &path,
        &[("item_id", &resin), ("quantity", "10"), ("lot", "R9")],
    )
    .await;
    other.assert_status(422);
    assert!(
        MaterialIssue::of_order(app.db(), f.order.id)
            .await
            .unwrap()
            .is_empty()
    );

    post(
        &app,
        &keeper,
        &path,
        &[("item_id", &resin), ("quantity", "25"), ("lot", "R3")],
    )
    .await
    .assert_status(303);

    let rows = MaterialIssue::of_order(app.db(), f.order.id).await.unwrap();
    let seen: Vec<(Option<i64>, i64, bool)> = rows
        .iter()
        .map(|r| (r.lot_id, r.quantity_milli, r.pending_issue_id.is_some()))
        .collect();
    assert_eq!(
        seen,
        vec![(Some(lot.id), 10_000, false), (None, 15_000, true)]
    );
    assert_eq!(rows[0].unit_cost, 800);
}
