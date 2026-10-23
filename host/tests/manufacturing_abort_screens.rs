//! BL-MFG-01 (#382): the abort of a work order through the real stock of
//! wl-inventory and the record page (`MFG.ACL.12`, 07 §7.1 *Work order*
//! `abort`): the movements, the output lot in quarantine, the outcome and the
//! loss on the page, shown to the people who may see costs only.

mod manufacturing_support;

use host::roles::{PRODUCTION_MANAGER, PRODUCTION_PLANNER, STOREKEEPER, SUPERVISOR};
use manufacturing::{WorkOrder, WorkOrderStatus};
use manufacturing_support::{boot, browse, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::movement::{MovementType, StockMovement};
use wl_inventory::{Lot, LotStatus};
use wl_kernel::approvals::{ApprovalRequest, approve};

/// The abort goes through the real stock: the resin is issued from the store of
/// the line at its cost, 50 panels are reported into a lot in quarantine, the
/// planner asks, the production manager approves, and the `return_in` and
/// `wastage` movements exist; the page then shows the outcome and the loss to
/// the manager only.
#[renox::test]
async fn an_abort_posts_the_real_movements_and_the_page_shows_the_outcome() {
    let app = boot().await;
    let f = floor(&app).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", STOREKEEPER, Some(f.plant.scope())).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;
    let sami = person(&app, "Sami", SUPERVISOR, Some(f.line.scope())).await;
    // 500 kg of resin lot R1 in the store.
    let mut lot = Lot {
        item_id: f.resin.id,
        number: "R1".into(),
        status: LotStatus::Released,
        ..Lot::default()
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
        500_000,
        800,
        400_000,
        &wl_kernel::Subject::new("inventory.goods_receipt", 1),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    let id = f.order.id;
    let item = f.resin.id.to_string();

    post(
        &app,
        &keeper,
        &format!("/manufacturing/work-orders/{id}/issue"),
        &[("item_id", &item), ("quantity", "500")],
    )
    .await
    .assert_status(303);
    post(
        &app,
        &sami,
        &format!("/manufacturing/work-orders/{id}/start"),
        &[],
    )
    .await
    .assert_status(303);
    post(
        &app,
        &sami,
        &format!("/manufacturing/work-orders/{id}/report"),
        &[("good", "50")],
    )
    .await
    .assert_status(303);

    // The output lot is opened in quarantine at the finished goods location.
    let running = WorkOrder::find_or_404(app.db(), id).await.unwrap();
    let output = Lot::find_or_404(app.db(), running.output_lot_id.expect("an output lot"))
        .await
        .unwrap();
    assert_eq!(output.status, LotStatus::Quarantine);
    assert_eq!(output.item_id, f.panel.id);
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), f.panel.id, f.finished.id, Some(output.id))
            .await
            .unwrap(),
        50_000
    );
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), f.resin.id, f.store.id, Some(lot.id))
            .await
            .unwrap(),
        0
    );

    let return_key = format!("return_{}_{}", f.resin.id, lot.id);
    post(
        &app,
        &planner,
        &format!("/manufacturing/work-orders/{id}/abort"),
        &[("reason", "wrong BOM"), (&return_key, "200")],
    )
    .await
    .assert_status(303);
    // While it waits the page says so and the buttons are disabled.
    let waiting = browse(&app, &planner, &format!("/manufacturing/work-orders/{id}")).await;
    waiting.assert_see("Abort pending");
    let request = ApprovalRequest::query()
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    approve(app.state(), request.id, &manager, "agreed")
        .await
        .unwrap();

    let done = WorkOrder::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(done.status, WorkOrderStatus::Aborted);
    let moves = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(f.resin.id))
        .order_by(StockMovement::ID)
        .get(app.db())
        .await
        .unwrap();
    let kinds: Vec<MovementType> = moves.iter().map(|m| m.movement_type).collect();
    assert_eq!(
        kinds,
        vec![
            MovementType::Receive,
            MovementType::Consume,
            MovementType::ReturnIn,
            MovementType::Wastage
        ]
    );
    assert_eq!(moves[2].quantity_milli, 200_000);
    assert_eq!(moves[2].to_location_id, Some(f.store.id));
    assert_eq!(moves[3].value, done.abort_scrap_value.unwrap());
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), f.resin.id, f.store.id, Some(lot.id))
            .await
            .unwrap(),
        200_000
    );
    // The good output is still in quarantine.
    assert_eq!(
        Lot::find_or_404(app.db(), output.id).await.unwrap().status,
        LotStatus::Quarantine
    );

    let page = browse(&app, &manager, &format!("/manufacturing/work-orders/{id}")).await;
    page.assert_ok();
    page.assert_see("Aborted");
    page.assert_see("wrong BOM");
    page.assert_see("Returned to the store");
    page.assert_see("Loss");
    let for_planner = browse(&app, &planner, &format!("/manufacturing/work-orders/{id}")).await;
    for_planner.assert_see("wrong BOM");
    for_planner.assert_dont_see("Loss");
}
