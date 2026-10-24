//! BL-MFG-02 (#383): the unlink of a work order from its sales order on the
//! record page (07 §7.1 *Work order* `unlink`).

mod manufacturing_support;

use host::roles::{PRODUCTION_PLANNER, QC_INSPECTOR, STOREKEEPER, SUPERVISOR};
use manufacturing::WorkOrder;
use manufacturing_support::{boot, browse, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::movement::StockMovement;
use wl_inventory::{Lot, LotStatus};

/// BL-MFG-02 (#383): the planner unlinks a work order from its sales order on
/// the record page; the page then says it is for stock, the activity tab has
/// the reason, and the action is no longer offered.
#[renox::test]
async fn unlink_is_offered_while_linked_and_its_reason_shows_in_the_activity() {
    let app = boot().await;
    let f = floor(&app).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let inspector = person(&app, "Qasim", QC_INSPECTOR, Some(f.plant.scope())).await;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(f.order.id))
        .update(
            app.db(),
            &[
                ("source", &manufacturing::WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(50_i64)),
                ("sales_order_line_id", &Some(1_i64)),
            ],
        )
        .await
        .unwrap();
    let path = format!("/manufacturing/work-orders/{}", f.order.id);

    let before = browse(&app, &planner, &path).await;
    before.assert_see("unlink-order");
    before.assert_see("Sales order 50");
    browse(&app, &inspector, &path)
        .await
        .assert_dont_see("unlink-order");
    // A reason is needed; the refusal is the sheet's 422.
    post(&app, &planner, &format!("{path}/unlink"), &[("reason", "")])
        .await
        .assert_status(422);
    post(
        &app,
        &planner,
        &format!("{path}/unlink"),
        &[("reason", "customer cancelled, standard item")],
    )
    .await
    .assert_status(303);

    let after = browse(&app, &planner, &path).await;
    after.assert_ok();
    after.assert_see("Stock");
    after.assert_see("customer cancelled, standard item");
    after.assert_dont_see("unlink-order");
    after.assert_see("Released");
}

/// BL-MFG-02 AC2 (#383): after the unlink the work order is completed for
/// 1,000 units through the real stock: its output lot is in the finished
/// goods store with the whole quantity and nothing of it is set aside for
/// the sales order that no longer needs it.
#[renox::test]
async fn the_output_of_an_unlinked_work_order_is_not_reserved_for_the_sales_order() {
    let app = boot().await;
    let f = floor(&app).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", STOREKEEPER, Some(f.plant.scope())).await;
    let sami = person(&app, "Sami", SUPERVISOR, Some(f.line.scope())).await;
    let id = f.order.id;
    WorkOrder::query()
        .where_(WorkOrder::ID.eq(id))
        .update(
            app.db(),
            &[
                ("source", &manufacturing::WorkOrderSource::SalesOrder),
                ("sales_order_id", &Some(50_i64)),
                ("sales_order_line_id", &Some(1_i64)),
                ("quantity_milli", &1_000_000_i64),
            ],
        )
        .await
        .unwrap();
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
    let path = format!("/manufacturing/work-orders/{id}");

    post(
        &app,
        &keeper,
        &format!("{path}/issue"),
        &[
            ("item_id", &f.resin.id.to_string()),
            ("quantity", "500"),
            ("lot", "R1"),
        ],
    )
    .await
    .assert_status(303);
    post(&app, &sami, &format!("{path}/start"), &[])
        .await
        .assert_status(303);
    post(
        &app,
        &planner,
        &format!("{path}/unlink"),
        &[("reason", "customer cancelled, standard item")],
    )
    .await
    .assert_status(303);
    post(&app, &sami, &format!("{path}/report"), &[("good", "1000")])
        .await
        .assert_status(303);
    post(&app, &sami, &format!("{path}/complete"), &[])
        .await
        .assert_status(303);

    let done = WorkOrder::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(done.status, manufacturing::WorkOrderStatus::Completed);
    assert_eq!(done.order(), None);
    let output = done.output_lot_id.expect("an output lot");
    let on_hand =
        wl_inventory::balance::on_hand_at(app.db(), f.panel.id, f.finished.id, Some(output))
            .await
            .unwrap();
    assert_eq!(on_hand, 1_000_000, "the whole 1,000 units are in the lot");
    let reserved =
        wl_inventory::balance::reserved_at(app.db(), f.panel.id, f.finished.id, Some(output))
            .await
            .unwrap();
    assert_eq!(reserved, 0, "no sales order holds any of the output");
}
