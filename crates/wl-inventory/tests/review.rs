//! Asking procurement to review a supplier under the threshold
//! (wl-inventory §5.9 steps 4 and 5, `STK.RUL.09`, issue #249).

mod support;

use renox::chrono::{Datelike, Days, NaiveDate};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_inventory::PurchaseOrder;
use wl_inventory::supplier_review::{self, ReviewStatus, SupplierReview, block_task_key, task_key};
use wl_inventory::supplier_score::{self, ScoreBand};
use wl_inventory::{
    GoodsReceipt, GoodsReceiptLine, GoodsReceiptStatus, Item, ItemSupplier, PreferredSupplierRequest,
    SupplierRole, SupplierScore,
};
use wl_kernel::PartyId;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::Assignee;

use support::{Booted, boot};

const GULF: PartyId = PartyId(22);
const AL_NOOR: PartyId = PartyId(21);

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A snapshot of `GULF` named by `date` with `score`, as the run would keep
/// it, and the rule applied to it.
async fn snapshot(t: &Booted, date: NaiveDate, score: Option<i64>) -> SupplierScore {
    let band = supplier_score::band_of(score, &wl_inventory::ScoreSettings::default());
    let mut row = SupplierScore {
        supplier_party_id: GULF.0,
        snapshot_date: date,
        window_from: date.checked_sub_months(renox::chrono::Months::new(3)).unwrap(),
        window_to: date.pred_opt().unwrap(),
        receipts: if score.is_some() { 5 } else { 2 },
        on_time: 2,
        on_time_bp: Some(4_000),
        rejected_bp: Some(300),
        score,
        band,
        computed_at: Some(renox::db::now()),
        ..SupplierScore::default()
    };
    row.save(t.db()).await.unwrap();
    supplier_review::on_snapshots(t.state(), std::slice::from_ref(&row))
        .await
        .unwrap();
    row
}

async fn reviews(t: &Booted) -> Vec<SupplierReview> {
    SupplierReview::query()
        .where_(SupplierReview::SUPPLIER_PARTY_ID.eq(GULF.0))
        .get(t.db())
        .await
        .unwrap()
}

/// `GULF` as the preferred supplier of `item`, with `AL_NOOR` its alternative.
async fn preferred_for(t: &Booted, code: &str) -> Item {
    let item = t.item(code).await;
    for (supplier, role, rank) in [
        (GULF, SupplierRole::Preferred, 0),
        (AL_NOOR, SupplierRole::Alternative, 1),
    ] {
        let mut row = ItemSupplier {
            item_id: item.id,
            supplier_party_id: supplier.0,
            role,
            rank,
            active: true,
            ..ItemSupplier::default()
        };
        row.save(t.db()).await.unwrap();
    }
    item
}

/// `POST /supplier-reviews/{id}/close` as `by`.
async fn close_as(
    t: &Booted,
    by: &User,
    review: &SupplierReview,
    form: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app
        .htmx()
        .post(&format!("/supplier-reviews/{}/close", review.id), form)
        .await
}

/// Done when 1 and 2: a snapshot of 64 gives one review task for procurement
/// and a message to the procurement manager; the task still open, a snapshot
/// of 60 updates the same review.
#[renox::test]
async fn a_snapshot_under_the_threshold_opens_one_review_and_updates_it() {
    let t = boot().await;
    let manager = t.user_with("Mira", "procurement_manager").await;

    let first = snapshot(&t, day(2026, 12, 1), Some(64)).await;

    let found = reviews(&t).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, ReviewStatus::Open);
    assert_eq!(found[0].snapshots(), [first.id]);
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(task_key(GULF.0).as_str())
    );
    assert!(tasks[0].description.contains("Score 64"), "{}", tasks[0].description);
    let told: Vec<_> = t
        .notices
        .all()
        .into_iter()
        .filter(|n| n.type_key == supplier_review::NOTICE)
        .collect();
    assert_eq!(told.len(), 1);
    assert!(told[0]
        .recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == manager.id)));
    assert_eq!(t.below.all().len(), 1);
    assert_eq!(t.below.all()[0].score, 64);

    let second = snapshot(&t, day(2027, 1, 1), Some(60)).await;

    let found = reviews(&t).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].snapshots(), [first.id, second.id]);
    assert_eq!(t.tickets.opened().len(), 1);
    assert_eq!(
        t.notices
            .all()
            .into_iter()
            .filter(|n| n.type_key == supplier_review::NOTICE)
            .count(),
        1
    );
    assert_eq!(t.below.all().len(), 2);
}

/// A score back above the threshold while the review is open is added to it
/// and the review stays open.
#[renox::test]
async fn a_better_score_is_added_to_the_open_review() {
    let t = boot().await;
    let first = snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let better = snapshot(&t, day(2027, 1, 1), Some(90)).await;

    let found = reviews(&t).await;
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, ReviewStatus::Open);
    assert_eq!(found[0].snapshots(), [first.id, better.id]);
    assert_eq!(t.below.all().len(), 1);
}

/// Done when 3: `not_enough_data` never fires.
#[renox::test]
async fn not_enough_data_creates_nothing() {
    let t = boot().await;

    let snap = snapshot(&t, day(2026, 12, 1), None).await;

    assert_eq!(snap.band, ScoreBand::NotEnoughData);
    assert!(reviews(&t).await.is_empty());
    assert!(t.tickets.opened().is_empty());
    assert!(t.below.all().is_empty());
}

/// Done when 4: closing with *change preferred* for two items makes two
/// requests for the second person; no item supplier changes by itself.
#[renox::test]
async fn change_preferred_makes_one_request_per_item() {
    let t = boot().await;
    let first = preferred_for(&t, "CEMENT").await;
    let second = preferred_for(&t, "SAND").await;
    snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let review = reviews(&t).await.remove(0);
    let a = t.procurement("Amal").await;

    close_as(
        &t,
        &a,
        &review,
        &[
            ("decision", "change_preferred"),
            ("items", &format!("{},{}", first.id, second.id)),
        ],
    )
    .await
    .assert_ok();

    let requests = PreferredSupplierRequest::query()
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.from_supplier_party_id, GULF.0);
        assert_eq!(request.to_supplier_party_id, Some(AL_NOOR.0));
        assert_eq!(request.kind, "replace");
        assert_eq!(request.requested_by, Some(a.id));
    }
    let closed = SupplierReview::find_or_404(t.db(), review.id).await.unwrap();
    assert_eq!(closed.status, ReviewStatus::Closed);
    assert_eq!(closed.decision.as_deref(), Some("change_preferred"));
    assert_eq!(closed.items(), [first.id, second.id]);
    assert_eq!(closed.decided_by, Some(a.id));
    // Nothing changed on the items, and the review task is done.
    for item in [&first, &second] {
        let rows = wl_inventory::item_supplier::rows_of(t.db(), item.id)
            .await
            .unwrap();
        assert!(rows[0].active && rows[0].role == SupplierRole::Preferred);
        assert_eq!(rows[0].supplier_party_id, GULF.0);
    }
    assert_eq!(t.tickets.resolved().len(), 1);
}

/// Done when 5: closing with *keep* closes the task and changes nothing; a
/// snapshot of 62 on 2027-02-01 opens a new review.
#[renox::test]
async fn keep_closes_the_review_and_a_later_low_score_opens_a_new_one() {
    let t = boot().await;
    let item = preferred_for(&t, "CEMENT").await;
    snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let review = reviews(&t).await.remove(0);
    let a = t.procurement("Amal").await;

    close_as(&t, &a, &review, &[("decision", "keep")])
        .await
        .assert_ok();

    let closed = SupplierReview::find_or_404(t.db(), review.id).await.unwrap();
    assert_eq!(closed.status, ReviewStatus::Closed);
    assert_eq!(closed.decision.as_deref(), Some("keep"));
    assert!(PreferredSupplierRequest::query().get(t.db()).await.unwrap().is_empty());
    let rows = wl_inventory::item_supplier::rows_of(t.db(), item.id)
        .await
        .unwrap();
    assert!(rows.iter().all(|row| row.active));
    assert!(!supplier_review::is_watched(t.state(), GULF.0).await.unwrap());

    snapshot(&t, day(2027, 2, 1), Some(62)).await;
    let found = reviews(&t).await;
    assert_eq!(found.len(), 2);
    assert_eq!(
        found.iter().filter(|r| r.status == ReviewStatus::Open).count(),
        1
    );
}

/// Done when 6: closing with *watch* shows the supplier watched, on the
/// scorecard too; a score still under the threshold keeps the mark.
#[renox::test]
async fn watch_marks_the_supplier_until_a_score_at_the_threshold() {
    let t = boot().await;
    preferred_for(&t, "CEMENT").await;
    snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let review = reviews(&t).await.remove(0);
    let a = t.procurement("Amal").await;
    close_as(&t, &a, &review, &[("decision", "watch")])
        .await
        .assert_ok();
    assert!(supplier_review::is_watched(t.state(), GULF.0).await.unwrap());

    // The supplier shows watched on the scorecard and on the item's rows.
    let reader = t.user_with("Vera", "procurement").await;
    renox::auth::permissions::define_role(
        t.db(),
        "scorer",
        &[wl_inventory::permissions::SUPPLIER_SCORES_VIEW],
    )
    .await
    .unwrap();
    reader.assign_role(t.db(), "scorer").await.unwrap();
    t.app.acting_as(&reader);
    let body: renox::serde_json::Value = renox::serde_json::from_str(
        &t.app
            .get(&format!("/suppliers/{}/score", GULF.0))
            .await
            .text(),
    )
    .unwrap();
    assert_eq!(body["watched"], true);

    // A score under the threshold keeps the mark (and opens a new review).
    snapshot(&t, day(2027, 1, 1), Some(69)).await;
    assert_eq!(reviews(&t).await.len(), 2);
    assert!(supplier_review::is_watched(t.state(), GULF.0).await.unwrap());
}

/// Done when 6 (second part): after *watch*, a snapshot of 75 clears the mark
/// without a new task.
#[renox::test]
async fn a_good_score_clears_the_watch_without_a_task() {
    let t = boot().await;
    snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let review = reviews(&t).await.remove(0);
    let a = t.procurement("Amal").await;
    close_as(&t, &a, &review, &[("decision", "watch")])
        .await
        .assert_ok();
    assert!(supplier_review::is_watched(t.state(), GULF.0).await.unwrap());

    snapshot(&t, day(2027, 2, 1), Some(75)).await;

    assert!(!supplier_review::is_watched(t.state(), GULF.0).await.unwrap());
    assert_eq!(reviews(&t).await.len(), 1);
    assert_eq!(t.tickets.opened().len(), 1);
}

/// Done when 7: closing with *request block* makes a task for a holder of
/// `parties.block`; nobody is blocked by it.
#[renox::test]
async fn request_block_only_asks_the_holders_of_parties_block() {
    let t = boot().await;
    let blocker = t.user_with("Bea", "party_blocker").await;
    snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let review = reviews(&t).await.remove(0);
    let a = t.procurement("Amal").await;

    close_as(&t, &a, &review, &[("decision", "request_block")])
        .await
        .assert_ok();

    let tasks = t.tickets.opened();
    let ask = tasks
        .iter()
        .find(|ticket| ticket.source_key.as_deref() == Some(block_task_key(GULF.0).as_str()))
        .expect("a task to decide on the block");
    assert!(matches!(&ask.assignee, Some(Assignee::User { id }) if *id == blocker.id));
    assert!(t.blocked.blocked().is_empty());
    let closed = SupplierReview::find_or_404(t.db(), review.id).await.unwrap();
    assert_eq!(closed.decision.as_deref(), Some("request_block"));
}

/// A decision is required, and only a holder of `purchase_orders.issue`
/// decides; a closed review is not closed again; *change preferred* names
/// items where the supplier is preferred.
#[renox::test]
async fn closing_needs_a_decision_the_permission_and_a_preferred_item() {
    let t = boot().await;
    let other = t.item("OTHER").await;
    snapshot(&t, day(2026, 12, 1), Some(64)).await;
    let review = reviews(&t).await.remove(0);
    let a = t.procurement("Amal").await;
    let clerk = t.storekeeper("Sam").await;

    close_as(&t, &a, &review, &[]).await.assert_status(422);
    close_as(&t, &a, &review, &[("decision", "maybe")])
        .await
        .assert_status(422);
    close_as(&t, &a, &review, &[("decision", "change_preferred")])
        .await
        .assert_status(422);
    close_as(
        &t,
        &a,
        &review,
        &[
            ("decision", "change_preferred"),
            ("items", &other.id.to_string()),
        ],
    )
    .await
    .assert_status(422);
    close_as(&t, &clerk, &review, &[("decision", "keep")])
        .await
        .assert_status(403);
    assert_eq!(reviews(&t).await[0].status, ReviewStatus::Open);

    close_as(&t, &a, &review, &[("decision", "keep")])
        .await
        .assert_ok();
    close_as(&t, &a, &review, &[("decision", "keep")])
        .await
        .assert_status(422);
}

/// The daily run builds the snapshot of day 1 and, after it, the review of a
/// supplier under the threshold (`STK.RUL.09` runs after the snapshot run).
#[renox::test]
async fn the_daily_run_opens_the_review_after_the_snapshot() {
    let t = boot().await;
    let item = t.item("PIPE").await;
    let store = t.location("STORE-1", None).await;
    let today = wl_kernel::documents::today();
    let month = today.with_day(1).unwrap();
    // Three receipts a few days before this month began, a month late.
    for n in 0..3_u64 {
        let (order, line) = t.order_from(GULF, &store, &item, 10_000, 1_000).await;
        let expected = month.checked_sub_days(Days::new(40)).unwrap();
        let arrived = month.checked_sub_days(Days::new(5 + n)).unwrap();
        PurchaseOrder::query()
            .where_(PurchaseOrder::ID.eq(order.id))
            .update(t.db(), &[("expected_date", &Some(expected))])
            .await
            .unwrap();
        let mut receipt = GoodsReceipt {
            purchase_order_id: Some(order.id),
            supplier_party_id: GULF.0,
            location_id: store.id,
            status: GoodsReceiptStatus::Posted,
            posted_at: Some(support_at(arrived)),
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

    t.app.run_scheduled(supplier_score::TASK).await.unwrap();

    let found = reviews(&t).await;
    assert_eq!(found.len(), 1);
    let snapshots = SupplierScore::query()
        .where_(SupplierScore::SUPPLIER_PARTY_ID.eq(GULF.0))
        .where_(SupplierScore::SNAPSHOT_DATE.eq(month))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].band, ScoreBand::BelowThreshold);
    assert_eq!(t.tickets.opened().len(), 1);
}

/// Midday in Asia/Dubai on `date`.
fn support_at(date: NaiveDate) -> renox::db::DateTime {
    use renox::chrono::{TimeZone, Utc};
    Utc.from_utc_datetime(&date.and_hms_opt(8, 0, 0).unwrap())
}
