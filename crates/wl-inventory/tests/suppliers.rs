//! Preferred and alternative suppliers per item, last prices, and the
//! reorder suggestions that use them (wl-inventory §3, §5.9, `STK.RUL.04`,
//! issue #247).

mod support;

use renox::prelude::*;
use renox::testing::TestResponse;
use wl_inventory::item_supplier::{self, RequestStatus};
use wl_inventory::reorder;
use wl_inventory::{
    DraftLine, Item, ItemSupplier, Location, PreferredSupplierRequest, ReorderPolicy,
    ReorderSuggestion, SuggestionStatus, SupplierRole,
};
use wl_kernel::{PartiesMerged, PartyId};

use support::{Booted, boot};

const AL_NOOR: PartyId = PartyId(21);
const GULF_BUILD: PartyId = PartyId(22);
const GULF_BUILD_LLC: PartyId = PartyId(23);

/// `POST /items/{id}/suppliers/add` as `by`.
async fn add(
    t: &Booted,
    by: &User,
    item: &Item,
    supplier: PartyId,
    role: &str,
    extra: &[(&str, &str)],
) -> TestResponse {
    t.app.acting_as(by);
    let id = supplier.0.to_string();
    let mut form = vec![("supplier", id.as_str()), ("role", role)];
    form.extend_from_slice(extra);
    t.app
        .htmx()
        .post(&format!("/items/{}/suppliers/add", item.id), &form)
        .await
}

/// "Cement 50 kg": reordered at or below 20 units up to 100, 10 on hand at
/// the store, preferred "Al Noor Trading" (5 days) and alternative "Gulf
/// Build" (rank 1, 10 days), listed by procurement user A.
async fn cement(t: &Booted, policy: ReorderPolicy) -> (Item, Location, User) {
    let mut item = Item::new("CEMENT", "Cement 50 kg", "bag")
        .reordered(20_000, 100_000)
        .with_policy(policy);
    item.save(t.db()).await.unwrap();
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 10_000, 1_800).await;
    let a = t.procurement("Amal").await;
    add(t, &a, &item, AL_NOOR, "preferred", &[("lead_days", "5")])
        .await
        .assert_ok();
    add(
        t,
        &a,
        &item,
        GULF_BUILD,
        "alternative",
        &[("rank", "1"), ("lead_days", "10")],
    )
    .await
    .assert_ok();
    (item, store, a)
}

/// The 07:00 run.
async fn run(t: &Booted) {
    t.app.run_scheduled(reorder::TASK).await.unwrap();
}

async fn suggestions(t: &Booted, item: &Item) -> Vec<ReorderSuggestion> {
    ReorderSuggestion::query()
        .where_(ReorderSuggestion::ITEM_ID.eq(item.id))
        .get(t.db())
        .await
        .unwrap()
}

/// Done when 1: with the preferred supplier blocked and the item below its
/// reorder point, the run suggests the alternative, marked *alternative*,
/// with the reason *preferred supplier on hold*.
#[renox::test]
async fn a_blocked_preferred_supplier_sends_the_suggestion_to_the_alternative() {
    let t = boot().await;
    let (item, store, _a) = cement(&t, ReorderPolicy::MinMax).await;
    t.blocked.block(AL_NOOR);

    run(&t).await;

    let found = suggestions(&t, &item).await;
    assert_eq!(found.len(), 1);
    let s = &found[0];
    assert_eq!(s.location_id, store.id);
    assert_eq!(s.supplier_party_id, Some(GULF_BUILD.0));
    assert_eq!(s.supplier_role.as_deref(), Some("alternative"));
    assert_eq!(s.reason.as_deref(), Some("preferred supplier on hold"));
    assert_eq!(s.status, SuggestionStatus::Open);
    // 100 up to the maximum less the 10 on hand.
    assert_eq!(s.suggested_qty_milli, 90_000);
    // The blocked supplier is shown as on hold, with no reason.
    let shown = s.alternatives();
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].supplier_party_id, AL_NOOR.0);
    assert!(shown[0].on_hold);
}

/// The preferred supplier gets the suggestion while it is usable; the
/// purchasers are told; a second run the same day updates the suggestion.
#[renox::test]
async fn the_preferred_supplier_gets_the_suggestion_and_a_rerun_updates_it() {
    let t = boot().await;
    let (item, store, _a) = cement(&t, ReorderPolicy::MinMax).await;

    run(&t).await;
    let first = suggestions(&t, &item).await;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].supplier_party_id, Some(AL_NOOR.0));
    assert_eq!(first[0].supplier_role.as_deref(), Some("preferred"));
    assert_eq!(first[0].reason, None);
    let told: Vec<_> = t
        .notices
        .all()
        .into_iter()
        .filter(|n| n.type_key == reorder::NOTICE)
        .collect();
    assert_eq!(told.len(), 1);

    // More goods leave: the same suggestion is updated, not repeated.
    let source = wl_kernel::Subject::new("inventory.goods_receipt", 901);
    let mut left = wl_inventory::movement::StockMovement::leaving(
        wl_inventory::movement::MovementType::Adjust,
        item.id,
        None,
        store.id,
        4_000,
        1_800,
        7_200,
        &source,
        None,
        None,
    );
    left.save(t.db()).await.unwrap();
    run(&t).await;
    let again = suggestions(&t, &item).await;
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].id, first[0].id);
    assert_eq!(again[0].suggested_qty_milli, 94_000);
    // Nothing new to tell about.
    assert_eq!(
        t.notices
            .all()
            .into_iter()
            .filter(|n| n.type_key == reorder::NOTICE)
            .count(),
        1
    );
}

/// An item above its reorder point, or with no preferred supplier, gets no
/// suggestion.
#[renox::test]
async fn only_items_at_or_below_the_point_with_a_preferred_supplier_are_suggested() {
    let t = boot().await;
    let (item, store, _a) = cement(&t, ReorderPolicy::MinMax).await;
    t.stock_in(&item, &store, None, 30_000, 1_800).await;
    let mut loose = Item::new("LOOSE", "Loose item", "pcs").reordered(5_000, 10_000);
    loose.save(t.db()).await.unwrap();
    t.stock_in(&loose, &store, None, 1_000, 100).await;

    run(&t).await;

    assert!(suggestions(&t, &item).await.is_empty());
    assert!(suggestions(&t, &loose).await.is_empty());
}

/// Done when 2: with both suppliers blocked the suggestion has no supplier.
#[renox::test]
async fn with_every_supplier_blocked_the_suggestion_has_no_supplier() {
    let t = boot().await;
    let (item, _store, _a) = cement(&t, ReorderPolicy::MinMax).await;
    t.blocked.block(AL_NOOR);
    t.blocked.block(GULF_BUILD);

    run(&t).await;

    let found = suggestions(&t, &item).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].supplier_party_id, None);
    assert_eq!(found[0].supplier_role, None);
    assert_eq!(
        found[0].reason.as_deref(),
        Some("preferred supplier on hold")
    );
}

/// A preferred row that is not in use sends the suggestion to the first
/// active alternative.
#[renox::test]
async fn an_inactive_preferred_row_is_skipped() {
    let t = boot().await;
    let (item, _store, _a) = cement(&t, ReorderPolicy::MinMax).await;
    ItemSupplier::query()
        .where_(ItemSupplier::ITEM_ID.eq(item.id))
        .where_(ItemSupplier::ROLE.eq(SupplierRole::Preferred))
        .update(t.db(), &[("active", &false)])
        .await
        .unwrap();

    run(&t).await;

    let found = suggestions(&t, &item).await;
    assert_eq!(found[0].supplier_party_id, Some(GULF_BUILD.0));
    assert_eq!(
        found[0].reason.as_deref(),
        Some("preferred supplier inactive")
    );
}

/// Done when 3: under the policy *reorder point + lead time* the lead time is
/// that of the supplier the suggestion goes to: 10 days when Al Noor (5 days)
/// is blocked. Under the plain policy there is none.
#[renox::test]
async fn the_lead_time_is_the_chosen_suppliers_under_the_lead_time_policy() {
    let t = boot().await;
    let (item, _store, _a) = cement(&t, ReorderPolicy::ReorderPointLeadTime).await;
    run(&t).await;
    assert_eq!(suggestions(&t, &item).await[0].lead_time_days, Some(5));

    // The next day's run: Al Noor is blocked, Gulf Build's 10 days apply.
    t.blocked.block(AL_NOOR);
    t.clock_at(1);
    run(&t).await;
    let found = suggestions(&t, &item).await;
    assert_eq!(found.len(), 2);
    assert!(
        found
            .iter()
            .any(|s| s.supplier_party_id == Some(GULF_BUILD.0) && s.lead_time_days == Some(10))
    );
}

/// A suggested quantity under the supplier's minimum order is raised to it,
/// and the rounding is shown.
#[renox::test]
async fn the_quantity_is_raised_to_the_minimum_order_quantity() {
    let t = boot().await;
    let mut item = Item::new("ROLL", "Cable roll", "m").reordered(100_000, 130_000);
    item.save(t.db()).await.unwrap();
    let store = t.location("STORE-1", None).await;
    t.stock_in(&item, &store, None, 0, 100).await;
    let a = t.procurement("Amal").await;
    add(&t, &a, &item, AL_NOOR, "preferred", &[("min_qty", "500")])
        .await
        .assert_ok();

    run(&t).await;

    let found = suggestions(&t, &item).await;
    assert_eq!(found[0].suggested_qty_milli, 500_000);
    assert_eq!(found[0].rounded_from_milli, Some(130_000));
}

/// Done when 4: a receipt of 100 bags at AED 18.50 from Gulf Build makes
/// AED 18.50 its last price for the item; before any receipt the last price
/// is that of the latest issued order, and a later order does not replace it.
#[renox::test]
async fn the_last_price_comes_from_the_latest_receipt() {
    let t = boot().await;
    let (item, store, _a) = cement(&t, ReorderPolicy::MinMax).await;
    assert_eq!(
        item_supplier::last_price(t.db(), item.id, GULF_BUILD.0)
            .await
            .unwrap(),
        None
    );

    let (order, line) = t
        .order_from(GULF_BUILD, &store, &item, 100_000, 1_850)
        .await;
    let issued = item_supplier::last_price(t.db(), item.id, GULF_BUILD.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(issued.unit_price, 1_850);

    let sam = t.storekeeper("Sam").await;
    let receipt = t
        .receipt(
            &order,
            &store,
            vec![DraftLine::on(line.id, 100_000, 100_000)],
        )
        .await;
    t.post_as(&sam, &receipt).await.assert_ok();

    let last = item_supplier::last_price(t.db(), item.id, GULF_BUILD.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(last.unit_price, 1_850);
    assert_eq!(last.currency, "AED");

    // A later order does not replace the receipt's price.
    t.order_from(GULF_BUILD, &store, &item, 10_000, 2_000).await;
    let still = item_supplier::last_price(t.db(), item.id, GULF_BUILD.0)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.unit_price, 1_850);
}

/// Done when 5 and 8: purchaser A switches an open suggestion from Al Noor to
/// Gulf Build with a reason; the item's preferred supplier stays Al Noor, the
/// switch is audited and counted for the month. Without a reason it is
/// refused.
#[renox::test]
async fn switching_a_suggestion_needs_a_reason_and_leaves_the_item_alone() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;
    run(&t).await;
    let open = suggestions(&t, &item).await.remove(0);
    let path = format!("/reorder-suggestions/{}/switch", open.id);
    let gulf = GULF_BUILD.0.to_string();

    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&path, &[("supplier", gulf.as_str()), ("reason", " ")])
        .await
        .assert_status(422);
    assert_eq!(
        suggestions(&t, &item).await[0].supplier_party_id,
        Some(AL_NOOR.0)
    );

    t.app
        .htmx()
        .post(
            &path,
            &[
                ("supplier", gulf.as_str()),
                ("reason", "Al Noor is late again"),
            ],
        )
        .await
        .assert_ok();

    let switched = suggestions(&t, &item).await.remove(0);
    assert_eq!(switched.supplier_party_id, Some(GULF_BUILD.0));
    assert_eq!(switched.supplier_role.as_deref(), Some("alternative"));
    assert_eq!(switched.switched_by, Some(a.id));
    assert_eq!(switched.switched_from_party_id, Some(AL_NOOR.0));
    assert!(switched.switched_from_preferred);
    let rows = item_supplier::rows_of(t.db(), item.id).await.unwrap();
    assert_eq!(rows[0].role, SupplierRole::Preferred);
    assert_eq!(rows[0].supplier_party_id, AL_NOOR.0);

    let log = renox::audit::for_subject(t.db(), ReorderSuggestion::TABLE, open.id, 20)
        .await
        .unwrap();
    let entry = log
        .iter()
        .find(|entry| entry.action == "inventory.reorder_switched")
        .expect("an audit entry");
    assert_eq!(entry.user_id, Some(a.id));
    assert_eq!(entry.data["reason"], "Al Noor is late again");

    let day = wl_kernel::documents::today();
    let counted = reorder::switches_away_from_preferred(t.db(), day)
        .await
        .unwrap();
    assert_eq!(counted, vec![(a.id, 1)]);

    // A second run the same day keeps the purchaser's choice.
    run(&t).await;
    assert_eq!(
        suggestions(&t, &item).await[0].supplier_party_id,
        Some(GULF_BUILD.0)
    );
}

/// A suggestion is never switched to a supplier that is on hold, one that is
/// not listed, or without the permission.
#[renox::test]
async fn a_suggestion_is_not_switched_to_a_blocked_or_unlisted_supplier() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;
    run(&t).await;
    let open = suggestions(&t, &item).await.remove(0);
    let path = format!("/reorder-suggestions/{}/switch", open.id);
    let clerk = t.storekeeper("Sam").await;
    let gulf = GULF_BUILD.0.to_string();
    let stranger = "99".to_owned();

    t.blocked.block(GULF_BUILD);
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&path, &[("supplier", gulf.as_str()), ("reason", "cheaper")])
        .await
        .assert_status(422);
    t.blocked.unblock(GULF_BUILD);
    t.app
        .htmx()
        .post(
            &path,
            &[("supplier", stranger.as_str()), ("reason", "cheaper")],
        )
        .await
        .assert_status(422);
    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post(&path, &[("supplier", gulf.as_str()), ("reason", "cheaper")])
        .await
        .assert_status(403);
    assert_eq!(
        suggestions(&t, &item).await[0].supplier_party_id,
        Some(AL_NOOR.0)
    );
}

/// Done when 6: a second row for the same item and supplier is refused; and
/// a later preferred supplier is a request, not a direct change.
#[renox::test]
async fn one_row_per_item_and_supplier_and_one_preferred() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;

    add(&t, &a, &item, GULF_BUILD, "alternative", &[])
        .await
        .assert_status(422);
    add(&t, &a, &item, GULF_BUILD, "preferred", &[])
        .await
        .assert_status(422);
    let third = PartyId(24);
    add(&t, &a, &item, third, "preferred", &[])
        .await
        .assert_status(422);
    assert_eq!(
        item_supplier::rows_of(t.db(), item.id).await.unwrap().len(),
        2
    );

    let clerk = t.storekeeper("Sam").await;
    add(&t, &clerk, &item, third, "alternative", &[])
        .await
        .assert_status(403);
}

/// Alternatives are ranked 1..n, renumbered on every save; a new alternative
/// ranked 1 pushes the others down; a deactivated row keeps its rank.
#[renox::test]
async fn alternatives_are_renumbered_and_a_deactivated_row_keeps_its_rank() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;
    add(
        &t,
        &a,
        &item,
        GULF_BUILD_LLC,
        "alternative",
        &[("rank", "1")],
    )
    .await
    .assert_ok();
    add(&t, &a, &item, PartyId(24), "alternative", &[])
        .await
        .assert_ok();

    let rows = item_supplier::rows_of(t.db(), item.id).await.unwrap();
    let ranks: Vec<(i64, i64)> = rows
        .iter()
        .map(|row| (row.supplier_party_id, row.rank))
        .collect();
    assert_eq!(ranks, [(21, 0), (23, 1), (22, 2), (24, 3)]);

    let llc = rows.iter().find(|r| r.supplier_party_id == 23).unwrap();
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&format!("/item-suppliers/{}/deactivate", llc.id), &[])
        .await
        .assert_ok();
    let after = item_supplier::rows_of(t.db(), item.id).await.unwrap();
    let llc = after.iter().find(|r| r.supplier_party_id == 23).unwrap();
    assert!(!llc.active);
    assert_eq!(llc.rank, 1);
}

/// Done when 7: deactivating the preferred row opens a request for a second
/// person; until it is approved the row is in use and suggestions still go to
/// Al Noor.
#[renox::test]
async fn deactivating_the_preferred_row_opens_a_request_and_changes_nothing() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;
    let preferred = item_supplier::rows_of(t.db(), item.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(preferred.role, SupplierRole::Preferred);

    t.app.acting_as(&a);
    let path = format!("/item-suppliers/{}/deactivate", preferred.id);
    t.app.htmx().post(&path, &[]).await.assert_ok();
    // Asking again does not open a second request.
    t.app.htmx().post(&path, &[]).await.assert_ok();

    let requests = PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ITEM_ID.eq(item.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].status, RequestStatus::Pending);
    assert_eq!(requests[0].from_supplier_party_id, AL_NOOR.0);
    assert_eq!(requests[0].requested_by, Some(a.id));
    let rows = item_supplier::rows_of(t.db(), item.id).await.unwrap();
    assert!(rows[0].active);

    run(&t).await;
    assert_eq!(
        suggestions(&t, &item).await[0].supplier_party_id,
        Some(AL_NOOR.0)
    );
}

/// Done when 9: an expired certificate of the chosen supplier is a warning on
/// its row and on the suggestion, and nothing is blocked.
#[renox::test]
async fn an_expired_certificate_is_a_warning_and_blocks_nothing() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;
    let rows = item_supplier::rows_of(t.db(), item.id).await.unwrap();
    let gulf = rows.iter().find(|r| r.supplier_party_id == 22).unwrap();
    ItemSupplier::query()
        .where_(ItemSupplier::ID.eq(gulf.id))
        .update(
            t.db(),
            &[("certificate_kinds", &"halal_certificate".to_owned())],
        )
        .await
        .unwrap();
    t.certificates.expire(GULF_BUILD, "halal_certificate");
    t.blocked.block(AL_NOOR);

    run(&t).await;

    let s = suggestions(&t, &item).await.remove(0);
    assert_eq!(s.supplier_party_id, Some(GULF_BUILD.0));
    assert_eq!(s.warnings(), ["halal_certificate"]);
    assert_eq!(s.status, SuggestionStatus::Open);

    t.app.acting_as(&a);
    let listed = t.app.get(&format!("/items/{}/suppliers", item.id)).await;
    listed.assert_ok();
    let body: renox::serde_json::Value = renox::serde_json::from_str(&listed.text()).unwrap();
    let row = body
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["supplier_party_id"] == 22)
        .unwrap();
    assert_eq!(row["warnings"][0], "halal_certificate");
    let preferred = body
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["supplier_party_id"] == 21)
        .unwrap();
    assert_eq!(preferred["on_hold"], true);
}

/// Done when 10: "Gulf Build LLC" merged into "Gulf Build": both rows on the
/// item become one row for "Gulf Build" with the stronger role; the merged
/// preferred row beats an alternative.
#[renox::test]
async fn merged_suppliers_become_one_row_with_the_stronger_role() {
    let t = boot().await;
    let (item, _store, a) = cement(&t, ReorderPolicy::MinMax).await;
    // Gulf Build is alternative 1; Gulf Build LLC alternative 2: the survivor
    // keeps the better rank, which is its own.
    add(&t, &a, &item, GULF_BUILD_LLC, "alternative", &[])
        .await
        .assert_ok();

    t.state()
        .emit(PartiesMerged {
            survivor: GULF_BUILD,
            merged: GULF_BUILD_LLC,
        })
        .await
        .unwrap();

    let rows = item_supplier::rows_of(t.db(), item.id).await.unwrap();
    let ranks: Vec<(i64, i64)> = rows
        .iter()
        .map(|row| (row.supplier_party_id, row.rank))
        .collect();
    assert_eq!(ranks, [(21, 0), (22, 1)]);

    // The merged one is the preferred supplier of another item.
    let mut other = Item::new("SAND", "Sand", "t");
    other.save(t.db()).await.unwrap();
    add(&t, &a, &other, GULF_BUILD_LLC, "preferred", &[])
        .await
        .assert_ok();
    add(&t, &a, &other, GULF_BUILD, "alternative", &[])
        .await
        .assert_ok();
    t.state()
        .emit(PartiesMerged {
            survivor: GULF_BUILD,
            merged: GULF_BUILD_LLC,
        })
        .await
        .unwrap();
    let rows = item_supplier::rows_of(t.db(), other.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].supplier_party_id, GULF_BUILD.0);
    assert_eq!(rows[0].role, SupplierRole::Preferred);
}

/// The last price is shown only to a holder of `stock.cost.view`.
#[renox::test]
async fn the_last_price_is_for_the_holders_of_cost_view() {
    let t = boot().await;
    let (item, store, a) = cement(&t, ReorderPolicy::MinMax).await;
    t.order_from(AL_NOOR, &store, &item, 10_000, 1_850).await;
    t.order_from(GULF_BUILD, &store, &item, 10_000, 1_777).await;
    let reader = t.procurement("Rana").await;
    reader.assign_role(t.db(), "cost_viewer").await.unwrap();
    let path = format!("/items/{}/suppliers", item.id);

    t.app.acting_as(&a);
    let plain = t.app.get(&path).await;
    plain.assert_ok();
    let body: renox::serde_json::Value = renox::serde_json::from_str(&plain.text()).unwrap();
    assert!(body[0]["last_price"].is_null(), "{body}");

    t.app.acting_as(&reader);
    let costly = t.app.get(&path).await;
    let body: renox::serde_json::Value = renox::serde_json::from_str(&costly.text()).unwrap();
    assert_eq!(body[0]["last_price"]["unit_price"], 1_850);

    run(&t).await;
    let open = suggestions(&t, &item).await.remove(0);
    t.app.acting_as(&a);
    let shown = t
        .app
        .get(&format!("/reorder-suggestions/{}", open.id))
        .await;
    shown.assert_ok();
    assert!(!shown.text().contains("1777"), "{}", shown.text());
    t.app.acting_as(&reader);
    let costly = t
        .app
        .get(&format!("/reorder-suggestions/{}", open.id))
        .await;
    assert!(costly.text().contains("1777"), "{}", costly.text());
}
