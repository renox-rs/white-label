//! The stock screens: the stock of a store with its lots, one lot with its
//! movements, the holds and their decisions, the pending issues, the
//! transfers, and receiving goods (BL-STK-01, BL-STK-11, BL-STK-12, BL-STK-18).

mod support;

use renox::prelude::*;
use wl_inventory::transfer::NewTransferLine;
use wl_inventory::{GoodsReceiptStatus, LotStatus, StockHold, TransferStatus};
use wl_kernel::Subject;
use wl_kernel::documents::today;

use support::{Booted, boot};

/// A user holding the roles `roles`.
async fn user(t: &Booted, name: &str, roles: &[&str]) -> User {
    let user = t.user_with(name, roles[0]).await;
    for role in &roles[1..] {
        user.assign_role(t.db(), role).await.unwrap();
    }
    user
}

/// The stock page lists the items and the lots of a store with what is on
/// hand, held and available; a store that is not the user's is not offered, a
/// user without `stock.view` is refused.
#[renox::test]
async fn the_stock_page_lists_the_items_and_lots_of_a_store() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.item("GLOVES").await;
    let syringes = {
        let mut item = wl_inventory::Item::new("SYR", "Syringes", "box").lot_tracked();
        item.save(t.db()).await.unwrap();
        item
    };
    let lot = t.lot(&syringes, "L-77", LotStatus::Released).await;
    t.stock_in(&item, &store, None, 40_000, 1_200).await;
    t.stock_in(&syringes, &store, Some(&lot), 12_500, 900).await;
    let viewer = t.finance("Fiona").await;
    let nobody = t.user_with("Nora", "stock_clerk").await;

    t.browse(&viewer, "/stock")
        .await
        .assert_ok()
        .assert_see("Jumeirah store")
        .assert_see("GLOVES")
        .assert_see("40.000 kg");
    // The lots are the other view of the same page.
    t.browse(&viewer, "/stock?view=lots")
        .await
        .assert_ok()
        .assert_see("L-77")
        .assert_see("12.500 box");
    t.browse(&nobody, "/stock").await.assert_forbidden();
}

/// A lot's page shows where it is and every movement of it, with the record
/// that caused each; the holder gets a button to hold it.
#[renox::test]
async fn a_lot_shows_where_it_is_and_what_happened_to_it() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.tracked_item("CART").await;
    let lot = t.lot(&item, "L-A2199", LotStatus::Released).await;
    t.stock_in(&item, &store, Some(&lot), 10_000, 4_500).await;
    let viewer = user(&t, "Vera", &["finance", "stock_holder"]).await;
    let looker = t.finance("Fiona").await;

    t.browse(&looker, &format!("/stock/lots/{}", lot.id))
        .await
        .assert_ok()
        .assert_see("L-A2199")
        .assert_see("Jumeirah store")
        .assert_see("+10 kg")
        .assert_see("Received")
        .assert_dont_see("Place a hold");
    t.browse(&viewer, &format!("/stock/lots/{}", lot.id))
        .await
        .assert_see("Place a hold");
}

/// Holding a lot from its page blocks it; the holds page lists the hold with
/// its line and the decision buttons only for those who decide; releasing a
/// line closes the hold.
#[renox::test]
async fn a_lot_is_held_from_its_page_and_released_from_the_holds_page() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.tracked_item("CART").await;
    let lot = t.lot(&item, "L-A2288", LotStatus::Released).await;
    t.stock_in(&item, &store, Some(&lot), 15_000, 4_500).await;
    let holder = t.holder("Hana").await;
    let releaser = t.releaser("Rana").await;

    t.app.acting_as(&holder);
    t.app
        .htmx()
        .post(
            "/stock/holds",
            &[("lot", lot.id.to_string().as_str()), ("reason", "recall")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            "/stock/holds",
            &[("lot", lot.id.to_string().as_str()), ("reason", "quality")],
        )
        .await
        .assert_ok();
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::OnHold);
    let hold = StockHold::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(hold.reference_type.as_deref(), Some("inventory.lot"));

    t.browse(&holder, "/stock/holds")
        .await
        .assert_ok()
        .assert_see("HLD-")
        .assert_see("Quality")
        .assert_see("L-A2288")
        .assert_dont_see("written off as wastage");
    t.browse(&releaser, "/stock/holds")
        .await
        .assert_see("Release")
        .assert_see("Discard")
        .assert_see("Return to supplier");

    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&releaser, &line, "release").await.assert_ok();
    assert_eq!(t.lot_now(&lot).await.status, LotStatus::Released);
    t.browse(&releaser, "/stock/holds")
        .await
        .assert_see("Closed")
        .assert_dont_see("written off as wastage");
}

/// A recall hold waits for a second person: the page shows the line as
/// waiting with its approval card, and no decision button.
#[renox::test]
async fn a_release_that_waits_for_approval_shows_its_card() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.tracked_item("CART").await;
    let lot = t.lot(&item, "L-A2199", LotStatus::Released).await;
    t.stock_in(&item, &store, Some(&lot), 10_000, 4_500).await;
    let holder = t.holder("Hana").await;
    let releaser = t.releaser("Rana").await;
    let second = t.user_with("Sam", "lot_releaser").await;
    t.hold_as(
        &holder,
        &[
            ("reason", "recall"),
            ("reference", "ticketing.ticket:1"),
            ("lots", lot.id.to_string().as_str()),
        ],
    )
    .await
    .assert_ok();
    let hold = StockHold::query().first(t.db()).await.unwrap().unwrap();
    let line = t.hold_lines(&hold).await.remove(0);
    t.decide_as(&releaser, &line, "release").await.assert_ok();

    t.browse(&second, "/stock/holds")
        .await
        .assert_ok()
        .assert_see("Waiting for approval")
        .assert_dont_see("written off as wastage");
}

/// The pending issues are a page for a browser and JSON for the rest; the
/// storekeeper clears one with a reason and the lot.
#[renox::test]
async fn pending_issues_are_a_page_and_are_cleared_with_a_reason() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.tracked_item("CART").await;
    let lot = t.lot(&item, "L-A2301", LotStatus::Released).await;
    let source = Subject::new("clinics.visit", 5);
    t.consume(&item, &store, 2_000, &source).await;
    let clerk = t.user_with("Cora", "stock_clerk").await;
    let pending = t.open_pending().await.remove(0);

    t.browse(&clerk, "/pending-issues")
        .await
        .assert_ok()
        .assert_see("Pending issues")
        .assert_see("CART")
        .assert_see("2.000 kg")
        .assert_see("L-A2301");
    t.app.acting_as(&clerk);
    t.app
        .get("/pending-issues")
        .await
        .assert_ok()
        .assert_json_path("0.item", "CART");
    let (id, lot_id) = (pending.id.to_string(), lot.id.to_string());
    t.app
        .htmx()
        .post(
            &format!("/pending-issues/{id}/clear"),
            &[("reason", "counted"), ("lot", lot_id.as_str())],
        )
        .await
        .assert_ok();
    assert!(t.open_pending().await.is_empty());
    t.browse(&clerk, "/pending-issues")
        .await
        .assert_see("No pending issues");
}

/// The transfers page prepares a transfer, sends it and receives it; a
/// receiving store on hold shows its warning in the sheet.
#[renox::test]
async fn transfers_are_prepared_sent_and_received_from_the_page() {
    let t = boot().await;
    let from = t.named_location("STORE-1", "Jumeirah store").await;
    let to = t.named_location("STORE-2", "Bay store").await;
    let item = t.item("GLOVES").await;
    t.stock_in(&item, &from, None, 40_000, 1_200).await;
    let mover = t.mover("Mo").await;
    let (f, d, i) = (from.id.to_string(), to.id.to_string(), item.id.to_string());

    t.app.acting_as(&mover);
    t.app
        .htmx()
        .post(
            "/stock/transfers",
            &[
                ("from", f.as_str()),
                ("to", f.as_str()),
                ("item", i.as_str()),
                ("quantity", "5"),
            ],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            "/stock/transfers",
            &[
                ("from", f.as_str()),
                ("to", d.as_str()),
                ("item", i.as_str()),
                ("quantity", "0"),
            ],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            "/stock/transfers",
            &[
                ("from", f.as_str()),
                ("to", d.as_str()),
                ("item", i.as_str()),
                ("quantity", "5"),
            ],
        )
        .await
        .assert_ok();
    let transfer = wl_inventory::Transfer::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(transfer.status, TransferStatus::Draft);

    t.browse(&mover, "/stock/transfers")
        .await
        .assert_ok()
        .assert_see("TRF-")
        .assert_see("Jumeirah store")
        .assert_see("GLOVES · 5 kg")
        .assert_see("Send");
    t.send_as(&mover, &transfer).await.assert_ok();
    t.browse(&mover, "/stock/transfers")
        .await
        .assert_see("On the road")
        .assert_see("Receive");
    t.receive_as(&mover, &transfer, false).await.assert_ok();
    assert_eq!(
        t.transfer_now(&transfer).await.status,
        TransferStatus::Received
    );

    // A transfer into a store on hold: the sheet carries the warning.
    let second = t
        .transfer(&from, &to, vec![NewTransferLine::of(item.id, 1_000)])
        .await;
    let holder = t.holder("Hana").await;
    t.hold_as(
        &holder,
        &[
            ("reason", "temperature"),
            ("reference", "ticketing.ticket:9"),
            ("location", d.as_str()),
        ],
    )
    .await
    .assert_ok();
    t.send_as(&mover, &second).await.assert_ok();
    t.browse(&mover, "/stock/transfers")
        .await
        .assert_see("is on temperature hold");
}

/// A user who may not send from the store cannot prepare a transfer from it.
#[renox::test]
async fn a_transfer_needs_the_permission_at_the_source() {
    let t = boot().await;
    let from = t.named_location("STORE-1", "Jumeirah store").await;
    let to = t.named_location("STORE-2", "Bay store").await;
    let item = t.item("GLOVES").await;
    let clerk = t.user_with("Cora", "stock_clerk").await;
    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post(
            "/stock/transfers",
            &[
                ("from", from.id.to_string().as_str()),
                ("to", to.id.to_string().as_str()),
                ("item", item.id.to_string().as_str()),
                ("quantity", "5"),
            ],
        )
        .await
        .assert_forbidden();
}

/// Receiving: the orders waiting for goods, a receipt started from one, the
/// line entered with its lot and expiry, posted, and the stock is in.
#[renox::test]
async fn goods_are_received_from_an_order_through_the_pages() {
    let t = boot().await;
    let store = t.named_location("STORE-1", "Jumeirah store").await;
    let item = t.tracked_item("CART").await;
    let (order, line) = t.order(&store, &item, 20_000, 4_500).await;
    let keeper = t.storekeeper("Kim").await;
    let viewer = t.finance("Fiona").await;

    t.browse(&keeper, "/purchase-orders")
        .await
        .assert_ok()
        .assert_see("CART · 20 kg")
        .assert_see("Start a receipt");
    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(&format!("/purchase-orders/{}/receive", order.id), &[])
        .await
        .assert_forbidden();

    t.app.acting_as(&keeper);
    t.app
        .htmx()
        .post(
            &format!("/purchase-orders/{}/receive", order.id),
            &[("delivery_note", "DN-9")],
        )
        .await
        .assert_ok();
    let receipt = wl_inventory::GoodsReceipt::query()
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, GoodsReceiptStatus::Draft);
    t.browse(&keeper, "/purchase-orders")
        .await
        .assert_see("Open the draft receipt")
        .assert_dont_see("Start a receipt");
    assert_eq!(receipt.delivery_note.as_deref(), Some("DN-9"));
    let lines = wl_inventory::goods_receipt::lines_of(t.db(), receipt.id)
        .await
        .unwrap();
    assert_eq!(lines[0].accepted_qty_milli, 20_000);
    assert_eq!(lines[0].po_line_id, Some(line.id));

    t.browse(&keeper, &format!("/goods-receipts/{}", receipt.id))
        .await
        .assert_ok()
        .assert_see("Draft receipt")
        .assert_see("DN-9")
        .assert_see("20 kg");
    let expiry = (today() + renox::chrono::Duration::days(200)).to_string();
    t.app
        .htmx()
        .post(
            &format!("/goods-receipts/{}/lines/{}", receipt.id, lines[0].id),
            &[
                ("received", "20"),
                ("accepted", "20"),
                ("lot", "L-NEW"),
                ("expiry", expiry.as_str()),
            ],
        )
        .await
        .assert_ok();
    t.post_as(&keeper, &receipt).await.assert_ok();
    t.browse(&keeper, "/goods-receipts")
        .await
        .assert_ok()
        .assert_see("GRN-")
        .assert_see("Posted");
    t.browse(&keeper, "/purchase-orders")
        .await
        .assert_see("No order is waiting for goods");
    assert_eq!(
        t.available(
            &item,
            &store,
            Some(
                &wl_inventory::Lot::query()
                    .first(t.db())
                    .await
                    .unwrap()
                    .unwrap()
            )
        )
        .await,
        20_000
    );
}
