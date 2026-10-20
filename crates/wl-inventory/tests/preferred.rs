//! Approving a change of an item's preferred supplier (wl-inventory §3
//! paragraph *Changing who is preferred*, `STK.ACL.07`, issue #250).

mod support;

use renox::prelude::*;
use renox::testing::TestResponse;
use wl_inventory::item_supplier::{self, RequestStatus};
use wl_inventory::preferred_change::{self, NOTICE};
use wl_inventory::{Item, ItemSupplier, PreferredSupplierRequest, SupplierRole, WlInventory};
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, boot_with};

const AL_NOOR: PartyId = PartyId(21);
const GULF_BUILD: PartyId = PartyId(22);
const GULF_BUILD_LLC: PartyId = PartyId(23);

/// A POST as `by`.
async fn post(t: &Booted, by: &User, path: &str, form: &[(&str, &str)]) -> TestResponse {
    t.app.acting_as(by);
    t.app.htmx().post(path, form).await
}

/// "Cement 50 kg" with Al Noor preferred (5 days) and Gulf Build the first
/// alternative (10 days), listed by a procurement officer.
async fn cement(t: &Booted) -> Item {
    let mut item = Item::new("CEMENT", "Cement 50 kg", "bag");
    item.save(t.db()).await.unwrap();
    let officer = t.procurement("Olga").await;
    for (supplier, role, extra) in [
        (AL_NOOR, "preferred", ("lead_days", "5")),
        (GULF_BUILD, "alternative", ("lead_days", "10")),
    ] {
        let id = supplier.0.to_string();
        post(
            t,
            &officer,
            &format!("/items/{}/suppliers/add", item.id),
            &[("supplier", id.as_str()), ("role", role), extra],
        )
        .await
        .assert_ok();
    }
    item
}

/// A user who lists suppliers and approves the change of a preferred one.
async fn manager(t: &Booted, name: &str) -> User {
    t.user_with(name, "procurement_manager").await
}

async fn request_of(t: &Booted, item: &Item) -> PreferredSupplierRequest {
    PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ITEM_ID.eq(item.id))
        .first(t.db())
        .await
        .unwrap()
        .expect("a request")
}

async fn requests_of(t: &Booted, item: &Item) -> Vec<PreferredSupplierRequest> {
    PreferredSupplierRequest::query()
        .where_(PreferredSupplierRequest::ITEM_ID.eq(item.id))
        .get(t.db())
        .await
        .unwrap()
}

/// `(supplier, role, rank, active)` of the item's rows, preferred first.
async fn rows(t: &Booted, item: &Item) -> Vec<(i64, SupplierRole, i64, bool)> {
    item_supplier::rows_of(t.db(), item.id)
        .await
        .unwrap()
        .iter()
        .map(|row| (row.supplier_party_id, row.role, row.rank, row.active))
        .collect()
}

/// Whether `user` was told that a request ended as `outcome`.
fn told(t: &Booted, user: &User, outcome: &str) -> bool {
    t.notices.all().iter().any(|notice| {
        notice.type_key == NOTICE
            && notice.data["outcome"] == outcome
            && notice
                .recipients
                .iter()
                .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user.id))
    })
}

async fn ask(t: &Booted, by: &User, item: &Item, supplier: PartyId) -> TestResponse {
    let id = supplier.0.to_string();
    post(
        t,
        by,
        &format!("/items/{}/preferred-requests", item.id),
        &[("supplier", id.as_str())],
    )
    .await
}

/// Done when 1: purchaser A requests Gulf Build as preferred; A cannot
/// approve it; manager B approves and Gulf Build is preferred, Al Noor
/// alternative rank 1, the event emitted.
#[renox::test]
async fn a_second_person_approves_the_new_preferred_supplier() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;

    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let request = request_of(&t, &item).await;
    assert_eq!(request.status, RequestStatus::Pending);
    assert_eq!(request.from_supplier_party_id, AL_NOOR.0);
    assert_eq!(request.to_supplier_party_id, Some(GULF_BUILD.0));
    assert_eq!(request.requested_by, Some(a.id));
    // Nothing changed yet, and the managers have a task.
    assert_eq!(rows(&t, &item).await[0].0, AL_NOOR.0);
    assert!(t.tickets.opened().iter().any(|task| {
        task.source_key.as_deref() == Some(preferred_change::task_key(request.id).as_str())
    }));

    let path = format!("/preferred-requests/{}/approve", request.id);
    post(&t, &a, &path, &[]).await.assert_status(422);
    assert_eq!(request_of(&t, &item).await.status, RequestStatus::Pending);
    assert_eq!(rows(&t, &item).await[0].0, AL_NOOR.0);

    post(&t, &b, &path, &[]).await.assert_ok();

    let approved = request_of(&t, &item).await;
    assert_eq!(approved.status, RequestStatus::Approved);
    assert_eq!(approved.decided_by, Some(b.id));
    assert_eq!(
        rows(&t, &item).await,
        [
            (GULF_BUILD.0, SupplierRole::Preferred, 0, true),
            (AL_NOOR.0, SupplierRole::Alternative, 1, true),
        ]
    );
    let changed = t.changed.all();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].old_supplier, Some(AL_NOOR));
    assert_eq!(changed[0].new_supplier, Some(GULF_BUILD));
    assert_eq!(changed[0].requester, Some(a.id));
    assert_eq!(changed[0].approver, Some(b.id));
    assert!(!t.tickets.resolved().is_empty());
}

/// A holder of `item_suppliers.manage` without `item_suppliers.approve`
/// cannot decide.
#[renox::test]
async fn only_a_holder_of_the_approve_permission_decides() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let officer = t.procurement("Pia").await;
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let id = request_of(&t, &item).await.id;

    post(
        &t,
        &officer,
        &format!("/preferred-requests/{id}/approve"),
        &[],
    )
    .await
    .assert_status(403);
    post(
        &t,
        &officer,
        &format!("/preferred-requests/{id}/reject"),
        &[("reason", "no")],
    )
    .await
    .assert_status(403);
    assert_eq!(request_of(&t, &item).await.status, RequestStatus::Pending);
}

/// Done when 2: the request screen shows both suppliers' score, lead time
/// and last price (the price for a holder of `stock.cost.view`).
#[renox::test]
async fn the_request_shows_both_suppliers_with_lead_times_and_last_prices() {
    let t = boot().await;
    let item = cement(&t).await;
    let store = t.location("STORE-1", None).await;
    t.order_from(AL_NOOR, &store, &item, 10_000, 1_850).await;
    t.order_from(GULF_BUILD, &store, &item, 10_000, 1_777).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;
    b.assign_role(t.db(), "cost_viewer").await.unwrap();
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let id = request_of(&t, &item).await.id;

    t.app.acting_as(&b);
    let shown = t.app.get(&format!("/preferred-requests/{id}")).await;
    shown.assert_ok();
    let body: renox::serde_json::Value = renox::serde_json::from_str(&shown.text()).unwrap();
    assert_eq!(body["from"]["supplier_party_id"], AL_NOOR.0);
    assert_eq!(body["from"]["lead_time_days"], 5);
    assert_eq!(body["from"]["last_price"]["unit_price"], 1_850);
    assert_eq!(body["to"]["supplier_party_id"], GULF_BUILD.0);
    assert_eq!(body["to"]["lead_time_days"], 10);
    assert_eq!(body["to"]["last_price"]["unit_price"], 1_777);
    assert!(body["from"].get("score").is_some(), "{body}");
    assert!(body["to"].get("band").is_some(), "{body}");

    // Without `stock.cost.view` the prices are left out.
    t.app.acting_as(&a);
    let plain = t.app.get(&format!("/preferred-requests/{id}")).await;
    let body: renox::serde_json::Value = renox::serde_json::from_str(&plain.text()).unwrap();
    assert!(body["from"]["last_price"].is_null(), "{body}");
    assert_eq!(body["to"]["lead_time_days"], 10);

    // The pending requests are listed for the approvers.
    t.app.acting_as(&b);
    let listed = t.app.get("/preferred-requests").await;
    listed.assert_ok();
    let body: renox::serde_json::Value = renox::serde_json::from_str(&listed.text()).unwrap();
    assert_eq!(body.as_array().map(Vec::len), Some(1));
    let stranger = t.storekeeper("Sam").await;
    t.app.acting_as(&stranger);
    t.app.get("/preferred-requests").await.assert_status(403);
}

/// Three posted receipts of `supplier` against `item` in the live window
/// (the last 30 days), `late` of them after the date the order expected
/// them; no rejection, no price history.
async fn receipts_of(t: &Booted, supplier: PartyId, item: &Item, late: usize) {
    use renox::chrono::{Days, NaiveDate, TimeZone, Utc};
    use renox::state_machine::StateMachine;
    use wl_inventory::{GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus, PurchaseOrder};

    let store = t.location(&format!("S-{}", supplier.0), None).await;
    let today: NaiveDate = wl_kernel::documents::today();
    for n in 0..3 {
        let (order, line) = t.order_from(supplier, &store, item, 10_000, 1_000).await;
        let on = today - Days::new(10 + n as u64);
        let expected = if n < late { on - Days::new(5) } else { on };
        PurchaseOrder::query()
            .where_(PurchaseOrder::ID.eq(order.id))
            .update(t.db(), &[("expected_date", &Some(expected))])
            .await
            .unwrap();
        let mut receipt = GoodsReceipt {
            purchase_order_id: Some(order.id),
            supplier_party_id: supplier.0,
            location_id: store.id,
            status: GoodsReceiptStatus::Posted,
            posted_at: Some(Utc.from_utc_datetime(&on.and_hms_opt(8, 0, 0).unwrap())),
            ..GoodsReceipt::default()
        };
        let mut tx = t.db().begin().await.unwrap();
        receipt
            .insert_in_state(&mut tx, GoodsReceiptStatus::Posted, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let mut row = GoodsReceiptLine {
            goods_receipt_id: receipt.id,
            position: 1,
            po_line_id: Some(line.id),
            item_id: item.id,
            received_qty_milli: 10_000,
            accepted_qty_milli: 10_000,
            unit_cost: 1_000,
            ..GoodsReceiptLine::default()
        };
        row.save(t.db()).await.unwrap();
    }
}

/// Done when 2 and 8: a supplier with no receipts in the window shows
/// *not enough data* (no score); with receipts the screen shows the real
/// score and band of each supplier: Al Noor all on time (100, good), Gulf
/// Build one receipt late of three (40 x 2/3 + 30 + 15 of 85: 84, watch).
#[renox::test]
async fn the_request_shows_the_real_score_and_band_of_both_suppliers() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let id = request_of(&t, &item).await.id;
    let path = format!("/preferred-requests/{id}");
    let read = |body: String| -> renox::serde_json::Value {
        renox::serde_json::from_str(&body).unwrap()
    };

    // No receipt yet: nothing to score.
    t.app.acting_as(&b);
    let shown = t.app.get(&path).await;
    shown.assert_ok();
    let body = read(shown.text());
    for side in ["from", "to"] {
        assert!(body[side]["score"].is_null(), "{body}");
        assert_eq!(body[side]["band"], "not_enough_data", "{body}");
    }

    receipts_of(&t, AL_NOOR, &item, 0).await;
    receipts_of(&t, GULF_BUILD, &item, 1).await;

    t.app.acting_as(&b);
    let body = read(t.app.get(&path).await.text());
    assert_eq!(body["from"]["supplier_party_id"], AL_NOOR.0);
    assert_eq!(body["from"]["score"], 100, "{body}");
    assert_eq!(body["from"]["band"], "good", "{body}");
    assert_eq!(body["to"]["supplier_party_id"], GULF_BUILD.0);
    assert_eq!(body["to"]["score"], 84, "{body}");
    assert_eq!(body["to"]["band"], "watch", "{body}");
}

/// Done when 3: adding a new alternative needs no approval.
#[renox::test]
async fn adding_an_alternative_needs_no_approval() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let id = GULF_BUILD_LLC.0.to_string();

    post(
        &t,
        &a,
        &format!("/items/{}/suppliers/add", item.id),
        &[("supplier", id.as_str()), ("role", "alternative")],
    )
    .await
    .assert_ok();

    assert!(requests_of(&t, &item).await.is_empty());
    assert_eq!(rows(&t, &item).await.len(), 3);
    assert!(t.changed.all().is_empty());
}

/// Done when 4 and 5: deactivating the preferred row opens a request and the
/// row stays preferred until B approves; after the approval the item has no
/// preferred supplier, and setting one later needs a request again.
#[renox::test]
async fn deactivating_the_preferred_row_is_a_request_and_a_later_preferred_needs_one_too() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;
    let preferred = item_supplier::rows_of(t.db(), item.id)
        .await
        .unwrap()
        .remove(0);

    post(
        &t,
        &a,
        &format!("/item-suppliers/{}/deactivate", preferred.id),
        &[],
    )
    .await
    .assert_ok();
    let request = request_of(&t, &item).await;
    assert_eq!(request.status, RequestStatus::Pending);
    assert_eq!(request.kind, "deactivate");
    let row = &rows(&t, &item).await[0];
    assert_eq!(
        (row.0, row.1, row.3),
        (AL_NOOR.0, SupplierRole::Preferred, true)
    );

    post(
        &t,
        &b,
        &format!("/preferred-requests/{}/approve", request.id),
        &[],
    )
    .await
    .assert_ok();
    assert_eq!(
        rows(&t, &item).await,
        [
            (GULF_BUILD.0, SupplierRole::Alternative, 1, true),
            (AL_NOOR.0, SupplierRole::Alternative, 2, false),
        ]
    );
    let changed = t.changed.all();
    assert_eq!(changed[0].old_supplier, Some(AL_NOOR));
    assert_eq!(changed[0].new_supplier, None);

    // The item had a preferred supplier before: adding one is refused, and a
    // request is needed.
    let id = GULF_BUILD.0.to_string();
    post(
        &t,
        &a,
        &format!("/items/{}/suppliers/add", item.id),
        &[("supplier", id.as_str()), ("role", "preferred")],
    )
    .await
    .assert_status(422);
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let later = requests_of(&t, &item).await;
    assert_eq!(later.len(), 2);
    assert!(
        later
            .iter()
            .any(|r| r.status == RequestStatus::Pending && r.kind == "replace")
    );
    assert!(
        !rows(&t, &item)
            .await
            .iter()
            .any(|row| row.1 == SupplierRole::Preferred)
    );
}

/// Done when 6: B rejects A's request; the item is unchanged and A is told.
#[renox::test]
async fn a_rejected_request_changes_nothing_and_the_requester_is_told() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let id = request_of(&t, &item).await.id;
    let before = rows(&t, &item).await;
    let path = format!("/preferred-requests/{id}/reject");

    // A reason is needed, and the requester cannot reject their own request.
    post(&t, &b, &path, &[("reason", " ")])
        .await
        .assert_status(422);
    post(&t, &a, &path, &[("reason", "changed my mind")])
        .await
        .assert_status(422);
    post(&t, &b, &path, &[("reason", "Al Noor delivers on time")])
        .await
        .assert_ok();

    let rejected = request_of(&t, &item).await;
    assert_eq!(rejected.status, RequestStatus::Rejected);
    assert_eq!(rejected.decided_by, Some(b.id));
    assert_eq!(
        rejected.decision_reason.as_deref(),
        Some("Al Noor delivers on time")
    );
    assert_eq!(rows(&t, &item).await, before);
    assert!(told(&t, &a, "rejected"));
    assert!(t.changed.all().is_empty());
    // A decided request cannot be approved any more.
    post(&t, &b, &format!("/preferred-requests/{id}/approve"), &[])
        .await
        .assert_status(422);
}

/// Edge cases: one request per item may be pending; the requester can take
/// it back, nobody else.
#[renox::test]
async fn one_request_may_be_pending_and_only_the_requester_withdraws_it() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    ask(&t, &b, &item, GULF_BUILD).await.assert_status(422);
    assert_eq!(requests_of(&t, &item).await.len(), 1);
    let id = request_of(&t, &item).await.id;
    let path = format!("/preferred-requests/{id}/withdraw");

    post(&t, &b, &path, &[]).await.assert_status(422);
    post(&t, &a, &path, &[]).await.assert_ok();

    assert_eq!(request_of(&t, &item).await.status, RequestStatus::Withdrawn);
    assert_eq!(rows(&t, &item).await[0].0, AL_NOOR.0);
    // Another request can be made now.
    ask(&t, &b, &item, GULF_BUILD).await.assert_ok();
}

/// Edge case: the new supplier is blocked when the request is approved: the
/// approval fails, the request is returned to its requester and nothing
/// changed; once the block is lifted it can be approved.
#[renox::test]
async fn a_supplier_blocked_at_approval_time_returns_the_request() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    let b = manager(&t, "Basil").await;
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
    let id = request_of(&t, &item).await.id;
    let path = format!("/preferred-requests/{id}/approve");

    t.blocked.block(GULF_BUILD);
    post(&t, &b, &path, &[]).await.assert_status(422);

    let returned = request_of(&t, &item).await;
    assert_eq!(returned.status, RequestStatus::Pending);
    assert!(returned.returned_note.is_some());
    assert_eq!(rows(&t, &item).await[0].0, AL_NOOR.0);
    assert!(told(&t, &a, "returned"));
    assert!(t.changed.all().is_empty());

    t.blocked.unblock(GULF_BUILD);
    post(&t, &b, &path, &[]).await.assert_ok();
    assert_eq!(rows(&t, &item).await[0].0, GULF_BUILD.0);
}

/// Edge case: the system cancels a pending request when the supplier it
/// would make preferred is deactivated.
#[renox::test]
async fn deactivating_the_new_supplier_cancels_the_pending_request() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    // A supplier the item does not list yet is added as an alternative first.
    ask(&t, &a, &item, GULF_BUILD_LLC).await.assert_ok();
    assert_eq!(request_of(&t, &item).await.status, RequestStatus::Pending);
    let row: ItemSupplier = item_supplier::rows_of(t.db(), item.id)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.supplier_party_id == GULF_BUILD_LLC.0)
        .expect("the new row");
    assert_eq!(row.role, SupplierRole::Alternative);

    post(
        &t,
        &a,
        &format!("/item-suppliers/{}/deactivate", row.id),
        &[],
    )
    .await
    .assert_ok();

    let cancelled = request_of(&t, &item).await;
    assert_eq!(cancelled.status, RequestStatus::Cancelled);
    assert!(cancelled.decision_reason.is_some());
    assert!(told(&t, &a, "cancelled"));
    assert_eq!(rows(&t, &item).await[0].0, AL_NOOR.0);

    // The item can be asked again now.
    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();
}

/// Edge cases: a request for the supplier that already is preferred, and one
/// for an item that never had a preferred supplier, are refused.
#[renox::test]
async fn a_request_that_changes_nothing_is_refused() {
    let t = boot().await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;
    ask(&t, &a, &item, AL_NOOR).await.assert_status(422);

    let mut bare = Item::new("SAND", "Sand", "t");
    bare.save(t.db()).await.unwrap();
    ask(&t, &a, &bare, GULF_BUILD).await.assert_status(422);
    assert!(requests_of(&t, &bare).await.is_empty());
}

/// Edge case, setting off: the change applies at once, audited with the user,
/// and the event names the requester as the approver.
#[renox::test]
async fn with_the_setting_off_a_change_applies_at_once() {
    let t = boot_with(WlInventory::new().preferred_change_needs_approval(false)).await;
    let item = cement(&t).await;
    let a = manager(&t, "Amal").await;

    ask(&t, &a, &item, GULF_BUILD).await.assert_ok();

    let applied = request_of(&t, &item).await;
    assert_eq!(applied.status, RequestStatus::Approved);
    assert_eq!(applied.decided_by, Some(a.id));
    assert_eq!(
        rows(&t, &item).await,
        [
            (GULF_BUILD.0, SupplierRole::Preferred, 0, true),
            (AL_NOOR.0, SupplierRole::Alternative, 1, true),
        ]
    );
    let changed = t.changed.all();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].requester, Some(a.id));
    assert_eq!(changed[0].approver, Some(a.id));
    let audited =
        renox::audit::for_subject(t.db(), PreferredSupplierRequest::TABLE, applied.id, 10)
            .await
            .unwrap()
            .iter()
            .any(|entry| entry.action == "inventory.preferred_supplier_changed");
    assert!(audited);
}
