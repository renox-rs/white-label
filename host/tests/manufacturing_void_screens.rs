//! BL-MFG-04 (#385): the void of a production report through the real stock of
//! wl-inventory and the record page (`PLT.ACL.09`, `MFG.ACL.10`): the output
//! leaves the lot with a reversing movement, the report stays on the page
//! struck through, and a lot that no longer holds the output refuses the void
//! in words.

mod manufacturing_support;

use host::roles::SUPERVISOR;
use manufacturing::{ProductionReport, ReportStatus, WorkOrder};
use manufacturing_support::{boot, browse, floor, person, post};
use renox::prelude::*;
use wl_inventory::movement::{MovementType, StockMovement};
use wl_kernel::approvals::{ApprovalRequest, approve};

/// Two supervisors report 100 good and 3 scrap, then 40 good on the wrong work
/// order; the second supervisor approves the void of the second report: the
/// lot holds 100 again, an `adjust` movement and the reversed `wastage` exist,
/// and the page shows the struck report.
#[renox::test]
async fn a_void_reverses_the_real_movements_and_the_page_strikes_the_report() {
    let app = boot().await;
    let f = floor(&app).await;
    let sami = person(&app, "Sami", SUPERVISOR, Some(f.line.scope())).await;
    let sara = person(&app, "Sara", SUPERVISOR, Some(f.line.scope())).await;
    let path = format!("/manufacturing/work-orders/{}", f.order.id);
    post(&app, &sami, &format!("{path}/start"), &[])
        .await
        .assert_status(303);
    post(
        &app,
        &sami,
        &format!("{path}/report"),
        &[("good", "100"), ("scrap", "3")],
    )
    .await
    .assert_status(303);
    post(&app, &sami, &format!("{path}/report"), &[("good", "40")])
        .await
        .assert_status(303);
    let wrong = ProductionReport::query()
        .order_by_desc(ProductionReport::ID)
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    let lot_id = wrong.lot_id.expect("the output lot");
    let on_hand = || {
        let db = app.db().clone();
        let (item, place) = (f.panel.id, f.finished.id);
        async move {
            wl_inventory::balance::on_hand_at(&db, item, place, Some(lot_id))
                .await
                .unwrap()
        }
    };
    assert_eq!(on_hand().await, 140_000);

    // The scrap had its wastage movement when it was reported.
    let movements = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(f.panel.id))
        .get(app.db())
        .await
        .unwrap();
    let wastage: Vec<_> = movements
        .iter()
        .filter(|m| m.movement_type == MovementType::Wastage)
        .collect();
    assert_eq!(wastage.len(), 1);
    assert_eq!(wastage[0].quantity_milli, 3_000);
    assert_eq!(wastage[0].value, 7_200, "3 panels at the standard AED 24");

    post(
        &app,
        &sami,
        &format!("/manufacturing/reports/{}/void", wrong.id),
        &[("reason", "wrong WO")],
    )
    .await
    .assert_status(303);
    let waiting = browse(&app, &sara, &path).await;
    waiting.assert_see("Void waits for approval");
    waiting.assert_see("approve-");
    let request = ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_ID.eq(wrong.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the request");
    approve(app.state(), request.id, &sara, "ok").await.unwrap();

    assert_eq!(on_hand().await, 100_000);
    let voided = ProductionReport::find_or_404(app.db(), wrong.id)
        .await
        .unwrap();
    assert_eq!(voided.status, ReportStatus::Voided);
    assert!(voided.stock_reversed_at.is_some());
    let order = WorkOrder::find_or_404(app.db(), f.order.id).await.unwrap();
    assert_eq!(order.good_milli, 100_000);
    let reversing: Vec<_> = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(f.panel.id))
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Adjust))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(reversing.len(), 1, "the voided report had no scrap");
    assert_eq!(reversing[0].quantity_milli, 40_000);
    assert!(
        reversing[0]
            .reason
            .as_deref()
            .unwrap_or_default()
            .contains("wrong WO")
    );

    let page = browse(&app, &sami, &path).await;
    page.assert_see("<s>");
    page.assert_see("Voided");
    page.assert_see("wrong WO");
}

/// 120 of the 140 panels in the lot were moved out: the void of the 40 is
/// refused in words and nothing is asked.
#[renox::test]
async fn a_void_is_refused_in_words_when_the_lot_was_used() {
    let app = boot().await;
    let f = floor(&app).await;
    let sami = person(&app, "Sami", SUPERVISOR, Some(f.line.scope())).await;
    let path = format!("/manufacturing/work-orders/{}", f.order.id);
    post(&app, &sami, &format!("{path}/start"), &[])
        .await
        .assert_status(303);
    post(&app, &sami, &format!("{path}/report"), &[("good", "100")])
        .await
        .assert_status(303);
    post(&app, &sami, &format!("{path}/report"), &[("good", "40")])
        .await
        .assert_status(303);
    let wrong = ProductionReport::query()
        .order_by_desc(ProductionReport::ID)
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    let mut moved = StockMovement::leaving(
        MovementType::TransferOut,
        f.panel.id,
        wrong.lot_id,
        f.finished.id,
        120_000,
        2_400,
        288_000,
        &wl_kernel::Subject::new("inventory.transfer", 1),
        None,
        None,
    );
    moved.save(app.db()).await.unwrap();

    let refused = post(
        &app,
        &sami,
        &format!("/manufacturing/reports/{}/void", wrong.id),
        &[("reason", "wrong WO")],
    )
    .await;

    refused.assert_status(422);
    assert!(
        refused.text().contains("Output already used"),
        "{}",
        refused.text()
    );
    let still = ProductionReport::find_or_404(app.db(), wrong.id)
        .await
        .unwrap();
    assert_eq!(still.status, ReportStatus::Posted);
}
