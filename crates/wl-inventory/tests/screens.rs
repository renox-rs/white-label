//! The HTML pages of the supplier screens (the suppliers of an item, a reorder
//! suggestion, a supplier's score, the preferred-supplier requests): a
//! browser, which names `text/html` in `Accept`, gets a page; a request
//! without it gets the JSON it always got.

mod support;

use renox::auth::permissions::define_role;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::{
    Item, PreferredSupplierRequest, ReorderPolicy, ReorderSuggestion, ScoreBand, SupplierScore,
    reorder,
};
use wl_kernel::PartyId;

use support::{Booted, boot};

const AL_NOOR: PartyId = PartyId(21);
const GULF_BUILD: PartyId = PartyId(22);

/// "Cement 50 kg" below its reorder point at STORE-1, Al Noor preferred (5
/// days), Gulf Build an alternative (10 days); the last prices are known.
async fn cement(t: &Booted) -> Item {
    let mut item = Item::new("CEMENT", "Cement 50 kg", "bag")
        .reordered(20_000, 100_000)
        .with_policy(ReorderPolicy::MinMax);
    item.save(t.db()).await.unwrap();
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 10_000, 1_800).await;
    let officer = t.procurement("Olga").await;
    t.app.acting_as(&officer);
    for (supplier, role, lead) in [
        (AL_NOOR, "preferred", "5"),
        (GULF_BUILD, "alternative", "10"),
    ] {
        let id = supplier.0.to_string();
        t.app
            .htmx()
            .post(
                &format!("/items/{}/suppliers/add", item.id),
                &[
                    ("supplier", id.as_str()),
                    ("role", role),
                    ("lead_days", lead),
                ],
            )
            .await
            .assert_ok();
    }
    t.order_from(AL_NOOR, &store, &item, 10_000, 1_850).await;
    t.order_from(GULF_BUILD, &store, &item, 10_000, 1_777).await;
    item
}

/// A user holding `role` and `extra` roles.
async fn user(t: &Booted, name: &str, role: &str, extra: &[&str]) -> User {
    let user = t.user_with(name, role).await;
    for role in extra {
        user.assign_role(t.db(), role).await.unwrap();
    }
    user
}

fn json(text: &str) -> renox::serde_json::Value {
    renox::serde_json::from_str(text).unwrap()
}

#[renox::test]
async fn the_suppliers_of_an_item_are_a_page_with_flags_and_prices() {
    let t = boot().await;
    let item = cement(&t).await;
    t.blocked.block(AL_NOOR);
    let manager = user(&t, "Mona", "procurement_manager", &["cost_viewer"]).await;
    let other = user(&t, "Basil", "procurement_manager", &[]).await;
    t.app.acting_as(&other);
    t.app
        .htmx()
        .post(
            &format!("/items/{}/preferred-requests", item.id),
            &[("supplier", "22")],
        )
        .await
        .assert_ok();
    let request = PreferredSupplierRequest::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a request");

    let path = format!("/items/{}/suppliers", item.id);
    let page = t.browse(&manager, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("Suppliers of CEMENT"), "{html}");
    assert!(html.contains("Cement 50 kg"));
    assert!(html.contains("Preferred") && html.contains("Alternative"));
    assert!(
        html.contains("#21") && html.contains("#22"),
        "the suppliers"
    );
    assert!(html.contains("On hold"), "the blocked supplier is flagged");
    assert!(html.contains("AED 18.50"), "the last price: {html}");
    assert!(
        html.contains(&format!("/preferred-requests/{}", request.id)),
        "the link to the pending request"
    );

    // Without the cost permission the prices are left out, as in the JSON.
    let plain_user = user(&t, "Pat", "procurement", &[]).await;
    let page = t.browse(&plain_user, &path).await;
    page.assert_ok();
    assert!(!page.text().contains("AED 18.50"), "{}", page.text());

    // The JSON is what it was.
    t.app.acting_as(&manager);
    let body = json(&t.app.get(&path).await.text());
    assert_eq!(body[0]["supplier_party_id"], 21);
    assert_eq!(body[0]["on_hold"], true);
    assert_eq!(body[0]["last_price"]["unit_price"], 1_850);

    // A person who may not read the suppliers is refused either way.
    let clerk = t.storekeeper("Sam").await;
    t.browse(&clerk, &path).await.assert_forbidden();
}

#[renox::test]
async fn a_reorder_suggestion_is_a_page_with_the_quantity_and_the_alternatives() {
    let t = boot().await;
    let item = cement(&t).await;
    t.app.run_scheduled(reorder::TASK).await.unwrap();
    let suggestion = ReorderSuggestion::query()
        .where_(ReorderSuggestion::ITEM_ID.eq(item.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("a suggestion");
    let buyer = user(&t, "Mona", "procurement_manager", &["cost_viewer"]).await;

    let path = format!("/reorder-suggestions/{}", suggestion.id);
    let page = t.browse(&buyer, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(
        html.contains(&format!("Reorder suggestion {}", suggestion.id)),
        "{html}"
    );
    assert!(html.contains("Cement 50 kg"), "the item");
    assert!(html.contains("STORE-1"), "the location");
    assert!(html.contains("bag"), "the quantity with its unit");
    assert!(html.contains("#21"), "the supplier it goes to");
    assert!(html.contains("Other suppliers") && html.contains("#22"));
    assert!(html.contains("AED 17.77"), "the alternative's last price");
    assert!(html.contains(&format!("/items/{}/suppliers", item.id)));

    let body = json(&t.app.get(&path).await.text());
    assert_eq!(body["item_id"], item.id);
    assert_eq!(body["status"], "open");
    assert_eq!(body["alternatives"][0]["supplier_party_id"], 22);
}

#[renox::test]
async fn a_supplier_score_is_a_page_with_the_live_window_and_the_snapshots() {
    let t = boot().await;
    let day = |year, month, day| NaiveDate::from_ymd_opt(year, month, day).unwrap();
    let mut snapshot = SupplierScore {
        supplier_party_id: GULF_BUILD.0,
        snapshot_date: day(2026, 9, 1),
        window_from: day(2026, 6, 1),
        window_to: day(2026, 8, 31),
        receipts: 8,
        on_time: 6,
        on_time_bp: Some(7_500),
        rejected_bp: Some(250),
        received_value: 4_000_000,
        incidents: 1,
        debit_notes: 1,
        score: Some(78),
        band: ScoreBand::Watch,
        ..SupplierScore::default()
    };
    snapshot.save(t.db()).await.unwrap();
    define_role(
        t.db(),
        "scorer",
        &[wl_inventory::permissions::SUPPLIER_SCORES_VIEW],
    )
    .await
    .unwrap();
    let viewer = user(&t, "Vera", "scorer", &["cost_viewer"]).await;
    let plain = user(&t, "Pat", "scorer", &[]).await;
    let path = format!("/suppliers/{}/score", GULF_BUILD.0);

    let page = t.browse(&viewer, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(html.contains("Score of #22"), "{html}");
    assert!(html.contains("Live window"), "the live window");
    assert!(html.contains("Monthly snapshots"));
    assert!(html.contains("2026-09-01"), "the snapshot");
    assert!(html.contains("78"), "its score");
    assert!(html.contains("Watch"), "its band");
    assert!(html.contains("75.00%"), "the on-time share");
    assert!(
        html.contains("AED 40,000.00"),
        "the value, for a cost viewer"
    );
    let page = t.browse(&plain, &path).await;
    page.assert_ok();
    assert!(!page.text().contains("AED 40,000.00"), "{}", page.text());

    t.app.acting_as(&viewer);
    let body = json(&t.app.get(&path).await.text());
    assert_eq!(body["snapshots"][0]["score"], 78);
    assert_eq!(body["snapshots"][0]["band"], "watch");
    let clerk = t.storekeeper("Sam").await;
    t.browse(&clerk, &path).await.assert_forbidden();
}

#[renox::test]
async fn the_preferred_requests_are_a_list_and_a_page_for_an_approver() {
    let t = boot().await;
    let item = cement(&t).await;
    let asker = user(&t, "Amal", "procurement_manager", &["cost_viewer"]).await;
    let approver = user(&t, "Basil", "procurement_manager", &["cost_viewer"]).await;
    t.app.acting_as(&asker);
    t.app
        .htmx()
        .post(
            &format!("/items/{}/preferred-requests", item.id),
            &[("supplier", "22")],
        )
        .await
        .assert_ok();
    let request = PreferredSupplierRequest::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a request");

    let listed = t.browse(&approver, "/preferred-requests").await;
    listed.assert_ok();
    let html = listed.text();
    assert!(html.contains("Preferred supplier requests"), "{html}");
    assert!(html.contains("Cement 50 kg"), "the item");
    assert!(html.contains("Waiting for a decision"), "the status");
    assert!(html.contains(&format!("/preferred-requests/{}", request.id)));
    assert!(html.contains(&format!("/items/{}/suppliers", item.id)));

    let path = format!("/preferred-requests/{}", request.id);
    let page = t.browse(&approver, &path).await;
    page.assert_ok();
    let html = page.text();
    assert!(
        html.contains(&format!("Preferred supplier request {}", request.id)),
        "{html}"
    );
    assert!(
        html.contains("#21") && html.contains("#22"),
        "both suppliers"
    );
    assert!(html.contains("AED 18.50") && html.contains("AED 17.77"));
    assert!(html.contains("Amal"), "who asked");
    assert!(html.contains("Approve"), "an approver may decide");
    assert!(html.contains(&format!("/items/{}/suppliers", item.id)));
    // The requester cannot approve their own request: no buttons.
    let own = t.browse(&asker, &path).await;
    own.assert_ok();
    assert!(!own.text().contains("Approve"), "{}", own.text());

    t.app.acting_as(&approver);
    let body = json(&t.app.get("/preferred-requests").await.text());
    assert_eq!(body.as_array().map(Vec::len), Some(1));
    let body = json(&t.app.get(&path).await.text());
    assert_eq!(body["to"]["supplier_party_id"], 22);

    // An approved request leaves the list; an empty list says so.
    t.app
        .htmx()
        .post(&format!("{path}/approve"), &[])
        .await
        .assert_ok();
    let empty = t.browse(&approver, "/preferred-requests").await;
    empty.assert_ok();
    assert!(empty.text().contains("Nothing waits for a decision"));
    let clerk = t.storekeeper("Sam").await;
    t.browse(&clerk, "/preferred-requests")
        .await
        .assert_forbidden();
    t.browse(&clerk, &path).await.assert_forbidden();
}
