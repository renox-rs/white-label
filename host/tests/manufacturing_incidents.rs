//! BL-MFG-15 (#396) with the real supplier scorecard of wl-inventory: an NCR
//! caused by the supplier is a quality incident of the snapshot whose window
//! holds the day it was raised; a cause changed afterwards leaves the stored
//! snapshot as it was and drops the incident from the live view.

mod manufacturing_support;

use host::roles::QC_INSPECTOR;
use manufacturing::Ncr;
use manufacturing_support::{boot, floor, person, post};
use renox::chrono::Duration;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::goods_receipt::{GoodsReceipt, GoodsReceiptStatus};
use wl_inventory::supplier_score::{compute, snapshot_run};
use wl_inventory::{Lot, LotStatus, PurchaseOrder, PurchaseOrderStatus, SupplierScore};
use wl_kernel::documents::today;

const SUPPLIER: i64 = 7;

/// Supplier 7 delivered against an order 45 days ago (so its scorecard has a
/// snapshot), lot L-77 of it is raised on as a supplier-caused NCR 40 days
/// ago. The snapshot holds one incident; QC changes the cause to *internal
/// process* with a reason: the stored snapshot still says one, the live view
/// of the same window says none.
#[renox::test]
async fn a_cause_changed_after_the_snapshot_leaves_the_snapshot_alone() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let mut order = PurchaseOrder::new(wl_kernel::PartyId(SUPPLIER), f.store.id);
    order.status = PurchaseOrderStatus::Issued;
    let mut tx = app.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, PurchaseOrderStatus::Issued, "fixture")
        .await
        .unwrap();
    let mut receipt = GoodsReceipt {
        supplier_party_id: SUPPLIER,
        location_id: f.store.id,
        purchase_order_id: Some(order.id),
        status: GoodsReceiptStatus::Posted,
        posted_at: Some(renox::db::now() - Duration::days(45)),
        ..GoodsReceipt::default()
    };
    receipt
        .insert_in_state(&mut tx, GoodsReceiptStatus::Posted, "fixture")
        .await
        .unwrap();
    let mut lot = Lot {
        item_id: f.resin.id,
        number: "L-77".into(),
        source_type: Some("inventory.goods_receipt".into()),
        source_id: Some(receipt.id),
        status: LotStatus::Quarantine,
        ..Lot::default()
    };
    lot.insert_in_state(&mut tx, LotStatus::Quarantine, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    wl_inventory::StockMovement::receive(
        f.resin.id,
        Some(lot.id),
        f.store.id,
        100_000,
        3_000,
        300_000,
        &receipt.subject(),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    post(
        &app,
        &qc,
        "/manufacturing/ncrs",
        &[
            ("plant_id", &f.plant.id.to_string()),
            ("lot_id", &lot.id.to_string()),
            ("quantity", "10"),
            ("defect", "colour off"),
        ],
    )
    .await
    .assert_status(303);
    let ncr = Ncr::query().first(app.db()).await.unwrap().unwrap();
    // The NCR was raised 40 days ago.
    let raised = today() - Duration::days(40);
    Ncr::query()
        .where_(Ncr::ID.eq(ncr.id))
        .update(app.db(), &[("raised_on", &raised)])
        .await
        .unwrap();

    let made = snapshot_run(app.state(), today()).await.unwrap();
    let with_incident = made
        .iter()
        .find(|snapshot| snapshot.window_from <= raised && raised <= snapshot.window_to)
        .expect("a snapshot whose window holds the day of the NCR");
    assert_eq!(with_incident.incidents, 1);
    let (from, to) = (with_incident.window_from, with_incident.window_to);

    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/cause", ncr.id),
        &[("cause", "internal_process"), ("reason", "dried too long")],
    )
    .await
    .assert_status(303);

    let stored = SupplierScore::find_or_404(app.db(), with_incident.id)
        .await
        .unwrap();
    assert_eq!(stored.incidents, 1, "a stored snapshot is never edited");
    assert_eq!(stored.computed_at, with_incident.computed_at);
    let live = compute(app.state(), SUPPLIER, from, to).await.unwrap();
    assert_eq!(live.incidents, 0, "the live view drops the incident");
}
