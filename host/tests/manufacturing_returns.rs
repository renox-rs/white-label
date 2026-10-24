//! BL-MFG-07 (#388): returned or refused goods are inspected and sales decides
//! (07 §4.1, §7.1 *Dispatch*, §7.2 `MFG.RUL.10`), with the real wl-inventory
//! dispatch, the real wl-ticketing tasks, the roles of the host and the
//! pages.

mod manufacturing_support;

use host::roles::{QC_INSPECTOR, SALES, STOREKEEPER};
use manufacturing::returns::{ReturnDecision, ReturnInspection};
use manufacturing_support::{boot_with_orders, browse, floor, person, post, stocked, words};
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::Subject;
use wl_kernel::delivery::{
    DeliveredLine, DeliveryDelivered, DeliveryId, DeliveryRefusedReceived, RefusedLineReceived,
    RunId,
};

/// A dispatch of 100 lids for sale line 1 of order 80 that left with a
/// courier; 80 are accepted and 20 refused (damaged); the 20 come back at run
/// close. QC inspects, sales decides, the real pages show both.
#[renox::test]
async fn refused_goods_of_a_real_dispatch_reach_qc_and_sales() {
    let (app, orders) = boot_with_orders().await;
    let f = floor(&app).await;
    let keeper = person(&app, "Karim", STOREKEEPER, None).await;
    let qc = person(&app, "Qasim", QC_INSPECTOR, Some(f.plant.scope())).await;
    let sales = person(&app, "Sara", SALES, None).await;
    orders.open(80, 1, 100_000, 4001);
    let lid = stocked(&app, "LID", &f.finished, 300).await;
    let dispatch = wl_inventory::create_dispatch(
        app.db(),
        None,
        wl_inventory::NewDispatch {
            source: Subject::new("sales.sales_order", 80),
            recipient: Some(PartyId(4001)),
            recipient_name: "Al Noor Trading".into(),
            address: "Dubai".into(),
            location_id: f.finished.id,
            lines: vec![wl_inventory::NewDispatchLine::of(lid.id, 100_000).for_sale_line(1)],
        },
    )
    .await
    .unwrap();
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/pick", dispatch.id),
        &[],
    )
    .await
    .assert_ok();
    post(
        &app,
        &keeper,
        &format!("/dispatches/{}/dispatch", dispatch.id),
        &[("handover", "courier"), ("tracking_number", "TRK-9")],
    )
    .await
    .assert_ok();
    let line = wl_inventory::dispatch::lines_of(app.db(), dispatch.id)
        .await
        .unwrap()
        .remove(0);

    app.state()
        .emit(DeliveryDelivered {
            delivery: DeliveryId(1),
            source: dispatch.subject(),
            attempt: 1,
            proof: 1,
            lines: vec![DeliveredLine {
                line: Some(line.subject()),
                description: "Lid".to_owned(),
                quantity_milli: 100_000,
                delivered_milli: 80_000,
                refused_milli: 20_000,
                refusal_reason: Some("damaged".to_owned()),
            }],
            cash_expected: 0,
            cash_collected: 0,
            cod_difference: 0,
            cod_difference_reason: None,
            absorbed_by: None,
            cheque: None,
        })
        .await
        .unwrap();
    assert!(
        ReturnInspection::query()
            .get(app.db())
            .await
            .unwrap()
            .is_empty(),
        "nothing is inspected before the goods are booked into quarantine"
    );
    app.state()
        .emit(DeliveryRefusedReceived {
            delivery: DeliveryId(1),
            source: dispatch.subject(),
            run: RunId(1),
            lines: vec![RefusedLineReceived {
                line: Some(line.subject()),
                description: "Lid".to_owned(),
                refused_milli: 20_000,
                received_milli: 20_000,
                missing_milli: 0,
                refusal_reason: Some("damaged".to_owned()),
            }],
        })
        .await
        .unwrap();

    let inspections = ReturnInspection::query().get(app.db()).await.unwrap();
    assert_eq!(inspections.len(), 1);
    assert_eq!(inspections[0].quantity_milli, 20_000);
    assert_eq!(inspections[0].item_id, lid.id);
    let decisions = ReturnDecision::query().get(app.db()).await.unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0].open_milli, 20_000);
    assert_eq!(decisions[0].owner_id, Some(sales.id));

    let returns = browse(&app, &sales, "/manufacturing/returns").await;
    returns.assert_ok();
    let text = words(&returns.text());
    for word in ["Returned goods", "SO-80", "damaged", "20", "New dispatch"] {
        assert!(text.contains(word), "{word} missing in {text}");
    }
    let inspect = browse(&app, &qc, "/manufacturing/inspections").await;
    inspect.assert_ok();
    let text = words(&inspect.text());
    for word in ["Returned lots", "damaged", "Pass", "Fail", "Waiting"] {
        assert!(text.contains(word), "{word} missing in {text}");
    }

    // Sales makes a draft dispatch for 10; the real inventory copies the
    // original (same store, customer and order line).
    post(
        &app,
        &sales,
        &format!("/manufacturing/returns/{}/dispatch", decisions[0].id),
        &[("quantity", "10")],
    )
    .await
    .assert_status(303);
    let made = wl_inventory::Dispatch::query()
        .order_by_desc(wl_inventory::Dispatch::ID)
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(made.id, dispatch.id);
    assert_eq!(made.status, wl_inventory::DispatchStatus::Draft);
    assert_eq!(made.recipient_name, "Al Noor Trading");
    let made_lines = wl_inventory::dispatch::lines_of(app.db(), made.id)
        .await
        .unwrap();
    assert_eq!(made_lines[0].quantity_milli, 10_000);
    assert_eq!(made_lines[0].sale_line_id(), Some(1));

    // QC passes the inspection.
    post(
        &app,
        &qc,
        &format!("/manufacturing/inspections/{}/pass", inspections[0].id),
        &[("note", "packaging damaged, goods fine")],
    )
    .await
    .assert_status(303);
    let passed = ReturnInspection::find_or_404(app.db(), inspections[0].id)
        .await
        .unwrap();
    assert_eq!(passed.status, "passed");
    // The sales person has no right to record the result.
    post(
        &app,
        &sales,
        &format!("/manufacturing/inspections/{}/pass", inspections[0].id),
        &[],
    )
    .await
    .assert_forbidden();
}
