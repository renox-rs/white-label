//! BL-MFG-08 (#389) through the real stock of wl-inventory, the real tickets
//! and the real approvals: an NCR returned to the supplier makes a draft Return
//! to supplier from the quarantine location at the receipt's cost, its
//! dispatch gives QC the task to close, its cancellation reopens the NCR, a
//! scrap above AED 2,000 waits for a holder of `stock.approve` and then
//! discards the lot, *use as is* releases it, and a lot with an NCR open on it
//! is not released.

mod manufacturing_support;

use host::roles::{PROCUREMENT, PRODUCTION_MANAGER, QC_INSPECTOR};
use manufacturing::ncrs::{close_key, redisposition_key};
use manufacturing::{Disposition, Ncr, NcrStatus};
use manufacturing_support::{Floor, boot, browse, floor, person, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_inventory::goods_receipt::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus};
use wl_inventory::movement::{MovementType, StockMovement};
use wl_inventory::supplier_return::{self, ReturnStatus, SupplierReturn};
use wl_inventory::{Location, LocationType, Lot, LotStatus};
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve};
use wl_kernel::lot_disposal::lot_release_blocked;
use wl_kernel::tickets::task_creator;
use wl_kernel::{PartyId, Subject};

const SUPPLIER: i64 = 7;

/// 100 kg of resin lot `number`, received from supplier 7 at `cost` fils a
/// kilo (receipt cost; the item's average cost is AED 8) and in `status`.
async fn received(app: &TestApp, f: &Floor, number: &str, cost: i64, status: LotStatus) -> Lot {
    let mut receipt = GoodsReceipt {
        supplier_party_id: SUPPLIER,
        location_id: f.store.id,
        status: GoodsReceiptStatus::Posted,
        ..GoodsReceipt::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    receipt
        .insert_in_state(&mut tx, GoodsReceiptStatus::Posted, "fixture")
        .await
        .unwrap();
    let mut lot = Lot {
        item_id: f.resin.id,
        number: number.to_owned(),
        source_type: Some("inventory.goods_receipt".into()),
        source_id: Some(receipt.id),
        status,
        ..Lot::default()
    };
    lot.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = GoodsReceiptLine {
        goods_receipt_id: receipt.id,
        position: 1,
        item_id: f.resin.id,
        received_qty_milli: 100_000,
        accepted_qty_milli: 100_000,
        unit_cost: cost,
        lot_id: Some(lot.id),
        lot_number: Some(number.to_owned()),
        ..GoodsReceiptLine::default()
    };
    line.save(app.db()).await.unwrap();
    StockMovement::receive(
        f.resin.id,
        Some(lot.id),
        f.store.id,
        100_000,
        cost,
        cost * 100,
        &receipt.subject(),
        None,
    )
    .save(app.db())
    .await
    .unwrap();
    lot
}

async fn raise(app: &TestApp, f: &Floor, qc: &User, lot: &Lot, kg: &str) -> Ncr {
    post(
        app,
        qc,
        "/manufacturing/ncrs",
        &[
            ("plant_id", &f.plant.id.to_string()),
            ("lot_id", &lot.id.to_string()),
            ("quantity", kg),
            ("defect", "colour off"),
        ],
    )
    .await
    .assert_status(303);
    Ncr::query()
        .order_by_desc(Ncr::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("an NCR")
}

async fn ncr_now(app: &TestApp, ncr: &Ncr) -> Ncr {
    Ncr::find_or_404(app.db(), ncr.id).await.unwrap()
}

async fn quarantine(app: &TestApp) -> Location {
    Location::query()
        .where_(Location::LOCATION_TYPE.eq(LocationType::Quarantine))
        .first(app.db())
        .await
        .unwrap()
        .expect("the quarantine location")
}

/// Lot L7 (100 kg at AED 30) is returned for credit: a real draft Return to
/// supplier for supplier 7 from the quarantine location lists 100 kg at AED
/// 30 with the NCR as its source; the stock moved to quarantine; the
/// procurement officer (who dispatches every Return to supplier;
/// the storekeeper does not hold `supplier_returns.manage`) dispatches it (AED 3,000) and QC gets the task to close, the
/// emptied lot is rejected and QC closes the NCR.
#[renox::test]
async fn a_returned_ncr_follows_its_return_to_supplier_to_the_close() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", PROCUREMENT, None).await;
    let lot = received(&app, &f, "L7", 3_000, LotStatus::Quarantine).await;
    let ncr = raise(&app, &f, &qc, &lot, "100").await;

    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[("disposition", "return_to_supplier"), ("outcome", "credit")],
    )
    .await
    .assert_status(303);

    let after = ncr_now(&app, &ncr).await;
    assert_eq!(after.status, NcrStatus::Dispositioned);
    let returns = supplier_return::of_source(app.db(), &ncr.subject())
        .await
        .unwrap();
    assert_eq!(returns.len(), 1);
    let ret = &returns[0];
    assert_eq!(ret.status, ReturnStatus::Draft);
    assert_eq!(ret.supplier_party_id, SUPPLIER);
    let place = quarantine(&app).await;
    assert_eq!(ret.location_id, Some(place.id));
    let lines = supplier_return::lines_of(app.db(), ret.id).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].lot_id, Some(lot.id));
    assert_eq!(lines[0].quantity_milli, 100_000);
    assert_eq!(lines[0].unit_cost, 3_000);
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), f.resin.id, place.id, Some(lot.id))
            .await
            .unwrap(),
        100_000
    );
    assert_eq!(after.supplier_return(), Some(ret.subject()));
    let early = post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/close", ncr.id),
        &[],
    )
    .await;
    early.assert_status(422);

    post(
        &app,
        &keeper,
        &format!("/supplier-returns/{}/dispatch", ret.id),
        &[("proof_kind", "driver_signature"), ("proof_ref", "SIG-1")],
    )
    .await
    .assert_ok();

    let sent = SupplierReturn::find_or_404(app.db(), ret.id).await.unwrap();
    assert_eq!(sent.status, ReturnStatus::Dispatched);
    let waiting = ncr_now(&app, &ncr).await;
    assert_eq!(waiting.status, NcrStatus::Dispositioned);
    assert!(waiting.action_done_at.is_some());
    assert!(
        task_creator(app.state())
            .find_open(app.state(), &close_key(ncr.id))
            .await
            .unwrap()
            .is_some(),
        "QC has the task to close"
    );
    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::Rejected,
        "the return took the last quantity"
    );
    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/close", ncr.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(ncr_now(&app, &ncr).await.status, NcrStatus::Closed);
}

/// 30 kg of lot L9 are returned; while the Return to supplier is `draft` the
/// lot is not released; procurement cancels it (the supplier refuses
/// collection) and the NCR is open again with a task for QC and the earlier
/// disposition in its history; QC scraps the 30 kg then (AED 240 at the
/// item's average cost: no approval) and the wastage is posted.
#[renox::test]
async fn a_cancelled_return_reopens_the_ncr_and_the_lot_stays_kept_meanwhile() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", PROCUREMENT, None).await;
    let lot = received(&app, &f, "L9", 3_000, LotStatus::Quarantine).await;
    let subject = Subject::new("inventory.lot", lot.id);
    let ncr = raise(&app, &f, &qc, &lot, "30").await;
    let open = lot_release_blocked(app.state(), &subject).await.unwrap();
    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[
            ("disposition", "return_to_supplier"),
            ("outcome", "replace"),
        ],
    )
    .await
    .assert_status(303);
    let draft = lot_release_blocked(app.state(), &subject).await.unwrap();
    let ret = supplier_return::of_source(app.db(), &ncr.subject())
        .await
        .unwrap()
        .remove(0);

    post(
        &app,
        &keeper,
        &format!("/supplier-returns/{}/cancel", ret.id),
        &[("reason", "the supplier refuses collection")],
    )
    .await
    .assert_ok();

    assert!(
        open.is_some() && draft.is_some(),
        "kept while open and while draft"
    );
    let reopened = ncr_now(&app, &ncr).await;
    assert_eq!(reopened.status, NcrStatus::Open);
    assert!(
        task_creator(app.state())
            .find_open(app.state(), &redisposition_key(ncr.id))
            .await
            .unwrap()
            .is_some()
    );
    let history = manufacturing::ncrs::decisions(app.db(), ncr.id)
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].disposition, Disposition::ReturnToSupplier);
    assert!(history[0].ended_at.is_some());
    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[("disposition", "scrap")],
    )
    .await
    .assert_status(303);
    let scrapped = ncr_now(&app, &ncr).await;
    assert_eq!(scrapped.disposition, Some(Disposition::Scrap));
    assert!(scrapped.action_done_at.is_some());
    let wastage: Vec<_> = StockMovement::query()
        .where_(StockMovement::LOT_ID.eq(Some(lot.id)))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Wastage))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(wastage.len(), 1);
    assert_eq!(wastage[0].quantity_milli, 30_000);
    assert_eq!(
        wl_inventory::balance::lot_left(app.db(), lot.id)
            .await
            .unwrap(),
        70_000
    );
    let lot_after = Lot::find_or_404(app.db(), lot.id).await.unwrap();
    assert_eq!(
        lot_after.status,
        LotStatus::Quarantine,
        "a partial NCR keeps the lot's state"
    );
}

/// Lot L8 (100 kg `on_hold`, AED 8 average cost) scrapped: AED 800, no
/// approval. With the average cost at AED 25 the 100 kg are AED 2,500: an
/// approval under `STK.ACL.02` is requested, the lot is still `on_hold`, and
/// when the production manager approves it is discarded and `rejected`.
#[renox::test]
async fn a_large_scrap_waits_for_the_approval_and_then_discards_the_lot() {
    let app = boot().await;
    let mut f = floor(&app).await;
    f.resin.average_cost = 2_500;
    f.resin.save(app.db()).await.unwrap();
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;
    let lot = received(&app, &f, "L8", 2_500, LotStatus::OnHold).await;
    let ncr = raise(&app, &f, &qc, &lot, "100").await;

    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[("disposition", "scrap")],
    )
    .await
    .assert_status(303);

    assert_eq!(ncr_now(&app, &ncr).await.status, NcrStatus::Open);
    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::OnHold
    );
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(ncr.id))
        .where_(ApprovalRequest::ACTION.eq("manufacturing.ncr_scrap"))
        .first(app.db())
        .await
        .unwrap()
        .expect("a request");
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.payload["scrap_value"], 250_000);
    let page = browse(&app, &manager, &format!("/manufacturing/ncrs/{}", ncr.id)).await;
    page.assert_ok();
    assert!(
        page.text().contains("Scrap goods of an NCR"),
        "{}",
        page.text()
    );

    approve(app.state(), request.id, &manager, "agreed")
        .await
        .unwrap();

    let done = ncr_now(&app, &ncr).await;
    assert_eq!(done.status, NcrStatus::Dispositioned);
    assert_eq!(done.scrap_value, 250_000);
    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::Rejected
    );
    assert_eq!(
        wl_inventory::balance::lot_left(app.db(), lot.id)
            .await
            .unwrap(),
        0
    );
}

/// *Use as is*: another inspector asks, the production manager approves, and
/// the lot is `released` for real.
#[renox::test]
async fn use_as_is_releases_the_real_lot_after_the_approval() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let omar = person(&app, "Omar", QC_INSPECTOR, Some(f.plant.scope())).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;
    let lot = received(&app, &f, "L5", 3_000, LotStatus::Quarantine).await;
    let ncr = raise(&app, &f, &qc, &lot, "100").await;

    post(
        &app,
        &omar,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[("disposition", "use_as_is")],
    )
    .await
    .assert_status(303);
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(ncr.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("a request");
    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::Quarantine
    );
    approve(app.state(), request.id, &manager, "fine")
        .await
        .unwrap();

    assert_eq!(
        Lot::find_or_404(app.db(), lot.id).await.unwrap().status,
        LotStatus::Released
    );
    assert_eq!(
        ncr_now(&app, &ncr).await.status,
        NcrStatus::Dispositioned,
        "QC closes it"
    );
    assert_eq!(
        lot_release_blocked(app.state(), &Subject::new("inventory.lot", lot.id))
            .await
            .unwrap(),
        None
    );
    let _ = PartyId(SUPPLIER);
}

/// BL-MFG-08 AC7 (#389): 30 kg of lot L9 (100 kg) are returned: the lot cannot
/// be released while the Return to supplier is a draft; when procurement
/// dispatches it the lot holds 70 kg, is still in quarantine, nothing keeps it
/// from release any more and QC releases it for real.
#[renox::test]
async fn after_the_dispatch_of_a_partial_return_the_lot_keeps_70_kg_and_qc_releases_it() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", PROCUREMENT, None).await;
    let lot = received(&app, &f, "L9", 3_000, LotStatus::Quarantine).await;
    let subject = Subject::new("inventory.lot", lot.id);
    let ncr = raise(&app, &f, &qc, &lot, "30").await;
    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
        &[("disposition", "return_to_supplier"), ("outcome", "credit")],
    )
    .await
    .assert_status(303);
    let ret = supplier_return::of_source(app.db(), &ncr.subject())
        .await
        .unwrap()
        .remove(0);
    assert!(
        lot_release_blocked(app.state(), &subject)
            .await
            .unwrap()
            .is_some(),
        "kept from release while the return is a draft"
    );

    post(
        &app,
        &keeper,
        &format!("/supplier-returns/{}/dispatch", ret.id),
        &[("proof_kind", "driver_signature"), ("proof_ref", "SIG-9")],
    )
    .await
    .assert_ok();

    let sent = SupplierReturn::find_or_404(app.db(), ret.id).await.unwrap();
    assert_eq!(sent.status, ReturnStatus::Dispatched);
    assert_eq!(
        wl_inventory::balance::lot_left(app.db(), lot.id)
            .await
            .unwrap(),
        70_000,
        "the lot holds what was not returned"
    );
    assert_eq!(lot_status(&app, lot.id).await, LotStatus::Quarantine);
    assert_eq!(
        lot_release_blocked(app.state(), &subject).await.unwrap(),
        None,
        "nothing keeps the lot from release once the return is out"
    );
    assert!(
        wl_kernel::lot_disposal::lot_disposal(app.state())
            .release(app.state(), &subject)
            .await
            .unwrap()
    );
    assert_eq!(lot_status(&app, lot.id).await, LotStatus::Released);
}

async fn lot_status(app: &TestApp, id: i64) -> LotStatus {
    Lot::find_or_404(app.db(), id).await.unwrap().status
}

/// BL-MFG-14 *Stock* (#395) with the 07 value: a Return to supplier of AED
/// 5,100 waits for a second holder of `supplier_returns.manage` before it
/// goes out (the requester cannot approve), and one of AED 5,000 goes at once.
#[renox::test]
async fn a_return_to_supplier_of_5100_dirhams_needs_approval_and_5000_does_not() {
    let app = boot().await;
    let f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    let karim = person(&app, "Karim", PROCUREMENT, None).await;
    let pia = person(&app, "Pia", PROCUREMENT, None).await;
    // 100 kg at AED 51.00 a kilo are AED 5,100.00; 100 kg at AED 50.00 are
    // AED 5,000.00.
    let dear = received(&app, &f, "L51", 5_100, LotStatus::Quarantine).await;
    let cheap = received(&app, &f, "L50", 5_000, LotStatus::Quarantine).await;
    let mut returns = Vec::new();
    for lot in [&dear, &cheap] {
        let ncr = raise(&app, &f, &qc, lot, "100").await;
        post(
            &app,
            &qc,
            &format!("/manufacturing/ncrs/{}/disposition", ncr.id),
            &[("disposition", "return_to_supplier"), ("outcome", "credit")],
        )
        .await
        .assert_status(303);
        returns.push(
            supplier_return::of_source(app.db(), &ncr.subject())
                .await
                .unwrap()
                .remove(0),
        );
    }
    for ret in &returns {
        post(
            &app,
            &karim,
            &format!("/supplier-returns/{}/dispatch", ret.id),
            &[("proof_kind", "driver_signature"), ("proof_ref", "SIG-1")],
        )
        .await
        .assert_ok();
    }

    let status = |id: i64| {
        let app = &app;
        async move {
            SupplierReturn::find_or_404(app.db(), id)
                .await
                .unwrap()
                .status
        }
    };
    assert_eq!(status(returns[1].id).await, ReturnStatus::Dispatched);
    assert_eq!(
        status(returns[0].id).await,
        ReturnStatus::Draft,
        "AED 5,100 waits for the approval"
    );
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(returns[0].id))
        .where_(ApprovalRequest::STATUS.eq(ApprovalStatus::Pending))
        .first(app.db())
        .await
        .unwrap()
        .expect("a request for the dear return");
    assert!(
        approve(app.state(), request.id, &karim, "mine")
            .await
            .is_err(),
        "the requester cannot approve"
    );
    approve(app.state(), request.id, &pia, "agreed")
        .await
        .unwrap();
    assert_eq!(status(returns[0].id).await, ReturnStatus::Dispatched);
}

/// BL-MFG-14 *Stock* (#395) with the 07 value: scrapping goods worth AED
/// 2,100 needs a holder of `stock.approve`, goods worth AED 2,000 are
/// scrapped at once.
#[renox::test]
async fn a_scrap_of_2100_dirhams_needs_approval_and_2000_does_not() {
    let app = boot().await;
    let mut f = floor(&app).await;
    let qc = person(&app, "Ines", QC_INSPECTOR, Some(f.plant.scope())).await;
    // 100 kg at AED 21.00 a kilo: AED 2,100.00.
    f.resin.average_cost = 2_100;
    f.resin.save(app.db()).await.unwrap();
    let dear = received(&app, &f, "S21", 2_100, LotStatus::OnHold).await;
    let first = raise(&app, &f, &qc, &dear, "100").await;
    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", first.id),
        &[("disposition", "scrap")],
    )
    .await
    .assert_status(303);
    assert_eq!(ncr_now(&app, &first).await.status, NcrStatus::Open);
    let waiting = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(first.id))
        .where_(ApprovalRequest::ACTION.eq("manufacturing.ncr_scrap"))
        .first(app.db())
        .await
        .unwrap();
    assert!(waiting.is_some(), "AED 2,100 needs the approval");

    // 100 kg at AED 20.00 a kilo: AED 2,000.00, scrapped without one.
    f.resin.average_cost = 2_000;
    f.resin.save(app.db()).await.unwrap();
    let cheap = received(&app, &f, "S20", 2_000, LotStatus::OnHold).await;
    let second = raise(&app, &f, &qc, &cheap, "100").await;
    post(
        &app,
        &qc,
        &format!("/manufacturing/ncrs/{}/disposition", second.id),
        &[("disposition", "scrap")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        ncr_now(&app, &second).await.status,
        NcrStatus::Dispositioned
    );
    assert!(
        ApprovalRequest::query()
            .where_(ApprovalRequest::SUBJECT_ID.eq(second.id))
            .where_(ApprovalRequest::ACTION.eq("manufacturing.ncr_scrap"))
            .first(app.db())
            .await
            .unwrap()
            .is_none(),
        "AED 2,000 needs none"
    );
}
