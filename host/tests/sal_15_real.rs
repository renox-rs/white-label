//! BL-SAL-15 (#134) against the REAL wl-inventory and wl-sales of the host:
//! goods a customer brings back from a recalled lot, or an item not tracked by
//! lot sold inside a recall's trace window, are accepted outside the return
//! window, linked to the recall, taken into the quarantine location and onto
//! the recall's stock hold; resolving the return restores no stock. The
//! module's own tests (`crates/wl-sales/tests/sal_15_recalled_returns.rs`) run
//! the same points against a stand-in of the trace.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, person};
use wl_inventory::movement::StockMovement;
use wl_inventory::{
    GoodsReceipt, GoodsReceiptStatus, Item, Location, Lot, LotStatus, MovementType, Recall,
};
use wl_kernel::Subject;
use wl_sales::{CounterSale, ReturnStatus, SaleLine, SalesReturn};

/// The demo auto parts trader with a workshop.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

async fn boot() -> TestApp {
    let config = wl_kernel::ClientConfig::load(CLIENT).expect("the auto client");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// A completed counter sale of 2 of `code` completed at `on`.
async fn sold(app: &TestApp, code: &str, on: DateTime) -> (CounterSale, SaleLine) {
    app.post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(app.db())
        .await
        .unwrap()
        .unwrap();
    app.post(
        &format!("/counter-sales/{}/lines", sale.id),
        &[
            ("item_code", code),
            ("unit_price", "50.00"),
            ("quantity", "2"),
        ],
    )
    .await
    .assert_status(303);
    app.post(
        &format!("/counter-sales/{}/complete", sale.id),
        &[("cash", "105.00")],
    )
    .await
    .assert_status(303);
    CounterSale::query()
        .where_(CounterSale::ID.eq(sale.id))
        .update(app.db(), &[("completed_at", &Some(on))])
        .await
        .unwrap();
    let sale = CounterSale::find_or_404(app.db(), sale.id).await.unwrap();
    let line = SaleLine::of(app.db(), &sale.subject())
        .await
        .unwrap()
        .remove(0);
    (sale, line)
}

/// Requests a return of `lines` of `sale`.
async fn asked(app: &TestApp, sale: &CounterSale, lines: &str) -> SalesReturn {
    app.post(
        "/sales-returns",
        &[
            ("sale_type", "counter_sale"),
            ("sale_id", &sale.id.to_string()),
            ("lines", lines),
            ("reason", "recalled by the supplier"),
        ],
    )
    .await
    .assert_status(303);
    SalesReturn::query()
        .order_by_desc(SalesReturn::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the return was requested")
}

async fn latest_recall(app: &TestApp) -> Recall {
    Recall::query()
        .order_by_desc(Recall::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("a recall")
}

/// Done when 1 and 2: recall RCL-1 open on lot L7 and a return of 2 units of
/// L7 sold 60 days ago (window 14): accepted without approval, linked to the
/// recall, the 2 units in quarantine and on the recall's hold; resolving with a
/// credit note restores no stock.
#[renox::test]
async fn sal_15_real_a_late_return_of_a_recalled_lot_joins_the_hold_and_restores_no_stock() {
    let app = boot().await;
    let owner = person(&app, "Omar", host::roles::ADMIN).await;
    app.acting_as(&owner);
    let mut pads = Item::new("BRK-PAD", "Brake pad set", "pcs").lot_tracked();
    pads.save(app.db()).await.unwrap();
    let mut store = Location::new("BRANCH-1", "Branch 1");
    store.save(app.db()).await.unwrap();
    let mut l7 = Lot {
        item_id: pads.id,
        number: "L7".to_owned(),
        status: LotStatus::Released,
        ..Lot::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    l7.insert_in_state(&mut tx, LotStatus::Released, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut received = StockMovement::receive(
        pads.id,
        Some(l7.id),
        store.id,
        10_000,
        8_000,
        80_000,
        &Subject::new("inventory.goods_receipt", 900),
        None,
    );
    received.save(app.db()).await.unwrap();
    let (sale, line) = sold(&app, "BRK-PAD", renox::db::now() - TimeDelta::days(60)).await;
    app.htmx()
        .post(
            "/recalls",
            &[
                ("source", "supplier"),
                ("reason", "Pads below the friction standard"),
                ("items", pads.id.to_string().as_str()),
                ("lot_numbers", "L7"),
            ],
        )
        .await
        .assert_ok();
    let recall = latest_recall(&app).await;
    let hold_id = recall.stock_hold_id.expect("the recall's hold");
    let before = wl_inventory::stock_hold::lines_of(app.db(), hold_id)
        .await
        .unwrap()
        .len();
    let r = asked(&app, &sale, &format!("{}:2:{}", line.id, l7.id)).await;

    app.post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);

    let r = SalesReturn::find_or_404(app.db(), r.id).await.unwrap();
    assert_eq!(r.status, ReturnStatus::Received);
    assert_eq!(r.recall(), Some(recall.subject()));
    let quarantine = Location::query()
        .where_(Location::LOCATION_TYPE.eq(wl_inventory::LocationType::Quarantine))
        .first(app.db())
        .await
        .unwrap()
        .expect("goods came into quarantine");
    assert_eq!(
        wl_inventory::balance::on_hand_at(app.db(), pads.id, quarantine.id, Some(l7.id))
            .await
            .unwrap(),
        2_000
    );
    let lines = wl_inventory::stock_hold::lines_of(app.db(), hold_id)
        .await
        .unwrap();
    assert_eq!(lines.len(), before + 1);
    let joined = lines.last().unwrap();
    assert_eq!(
        (joined.location_id, joined.lot_id, joined.quantity_milli),
        (quarantine.id, Some(l7.id), 2_000)
    );
    // Nothing of it can be taken: the recall holds it.
    assert_eq!(
        wl_inventory::balance::balance(app.db(), pads.id, quarantine.id, Some(l7.id))
            .await
            .unwrap()
            .available_milli(),
        0
    );

    app.post(
        &format!("/sales-returns/{}/resolve", r.id),
        &[
            ("resolution", "credit_note"),
            ("credit_note", "5"),
            ("inspected", "on"),
        ],
    )
    .await
    .assert_status(303);
    let r = SalesReturn::find_or_404(app.db(), r.id).await.unwrap();
    assert_eq!(r.status, ReturnStatus::Resolved);
    let back = StockMovement::query()
        .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::ReturnIn))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(back.len(), 1, "resolving restored no stock");
    assert_eq!(back[0].to_location_id, Some(quarantine.id));
}

/// Done when 3: recall on part P (not lot-tracked, received 2026-08-01 to
/// 2026-08-31, first receipt 2026-08-05, no trace end) and a return of 1 P
/// sold 2026-09-10: linked to the recall and joins its hold; a P sold
/// 2026-08-01 is a normal return.
#[renox::test]
async fn sal_15_real_an_item_not_tracked_by_lot_is_recalled_by_its_receipts_and_sale_date() {
    let app = boot().await;
    let owner = person(&app, "Omar", host::roles::ADMIN).await;
    app.acting_as(&owner);
    let supplier = company(&app, "Brake Parts FZE").await;
    let mut part = Item::new("BRK-DISC", "Brake disc", "pcs");
    part.save(app.db()).await.unwrap();
    let mut store = Location::new("BRANCH-1", "Branch 1");
    store.save(app.db()).await.unwrap();
    let arrived = NaiveDate::from_ymd_opt(2026, 8, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
        .and_utc();
    let mut receipt = GoodsReceipt {
        supplier_party_id: supplier.0,
        location_id: store.id,
        arrived_at: Some(arrived),
        posted_at: Some(arrived),
        ..GoodsReceipt::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    receipt
        .insert_in_state(&mut tx, GoodsReceiptStatus::Posted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut moved = StockMovement::receive(
        part.id,
        None,
        store.id,
        10_000,
        8_000,
        80_000,
        &Subject::new("inventory.goods_receipt", receipt.id),
        None,
    );
    moved.moved_at = arrived;
    moved.save(app.db()).await.unwrap();
    let day = |m, d| {
        NaiveDate::from_ymd_opt(2026, m, d)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc()
    };
    let (recalled_sale, recalled_line) = sold(&app, "BRK-DISC", day(9, 10)).await;
    let (old_sale, old_line) = sold(&app, "BRK-DISC", day(8, 1)).await;
    app.htmx()
        .post(
            "/recalls",
            &[
                ("source", "supplier"),
                ("reason", "Discs out of tolerance"),
                ("supplier", supplier.0.to_string().as_str()),
                ("items", part.id.to_string().as_str()),
                ("received_from", "2026-08-01"),
                ("received_to", "2026-08-31"),
            ],
        )
        .await
        .assert_ok();
    let recall = latest_recall(&app).await;

    let joined = asked(&app, &recalled_sale, &format!("{}:1", recalled_line.id)).await;
    app.post(
        &format!("/sales-returns/{}/receive", joined.id),
        &[("x", "")],
    )
    .await
    .assert_status(303);
    let joined = SalesReturn::find_or_404(app.db(), joined.id).await.unwrap();
    assert_eq!(joined.recall(), Some(recall.subject()));
    let lines = wl_inventory::stock_hold::lines_of(app.db(), recall.stock_hold_id.unwrap())
        .await
        .unwrap();
    let last = lines.last().unwrap();
    assert_eq!((last.lot_id, last.quantity_milli), (None, 1_000));
    assert_ne!(last.location_id, store.id, "it is in quarantine");

    let normal = asked(&app, &old_sale, &format!("{}:1", old_line.id)).await;
    app.post(
        &format!("/sales-returns/{}/receive", normal.id),
        &[("x", "")],
    )
    .await
    .assert_status(422);
    let normal = SalesReturn::find_or_404(app.db(), normal.id).await.unwrap();
    assert_eq!(normal.status, ReturnStatus::Requested);
    assert_eq!(normal.recall_id, None);
}
