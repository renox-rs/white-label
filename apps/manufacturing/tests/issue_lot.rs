//! BL-MFG-09 (#390): the lot on every issue of a lot-tracked material to a
//! work order (`MFG.PRD.02`, `STK.MOV.01`, `STK.LOT.01`, 07 §4.2 step 5, §9):
//! an issue without a lot is refused while stock is available, one movement
//! per lot, an item that is not lot-tracked needs none, and only the short
//! quantity that becomes a pending issue has no lot.

mod support;

use manufacturing::{MaterialIssue, WorkOrder};
use renox::prelude::*;
use support::{Booted, RESIN, boot};

/// The carton, not lot-tracked.
const CARTON: i64 = 40;

/// WO on L1, `released`; resin is lot-tracked, cartons are not.
struct Scene {
    t: Booted,
    keeper: User,
    order: WorkOrder,
}

async fn scene() -> Scene {
    let t = boot().await;
    let plant = t.plant("Plant 1").await;
    let line = t.line(&plant, "L1").await;
    let planner = t.planner("Amal", &plant).await;
    let keeper = t.keeper("Karim", &plant).await;
    t.stock.track(RESIN);
    let order = t.released(&planner, &plant, &line).await;
    Scene { t, keeper, order }
}

async fn issue(s: &Scene, item: i64, quantity: &str, lot: &str) -> renox::testing::TestResponse {
    let item = item.to_string();
    let mut form = vec![("item_id", item.as_str()), ("quantity", quantity)];
    if !lot.is_empty() {
        form.push(("lot", lot));
    }
    s.t.act(&s.keeper, &s.order, "issue", &form).await
}

async fn rows(s: &Scene) -> Vec<MaterialIssue> {
    MaterialIssue::of_order(s.t.db(), s.order.id).await.unwrap()
}

/// AC1: 50 kg of resin R (lot-tracked) without a lot is refused, with the lots
/// that can be named; nothing is issued.
#[renox::test]
async fn an_issue_of_a_tracked_material_without_a_lot_is_refused() {
    let s = scene().await;
    s.t.stock.put(RESIN, Some(1), 30_000, 800);
    s.t.stock.put(RESIN, Some(2), 40_000, 800);

    let refused = issue(&s, RESIN, "50", "").await;

    refused.assert_status(422);
    let text = refused.text();
    assert!(text.contains("Name the lot"), "{text}");
    assert!(text.contains("R1") && text.contains("R2"), "{text}");
    assert!(rows(&s).await.is_empty());
    assert_eq!(
        s.t.stock.consumed(),
        0,
        "nothing was taken from the shelves"
    );
}

/// AC2: 30 kg of lot R1 and 20 kg of lot R2 are two issues, two rows with
/// their lots (named by the number printed on the pack).
#[renox::test]
async fn one_row_per_lot_when_the_storekeeper_names_each() {
    let s = scene().await;
    s.t.stock.put(RESIN, Some(1), 30_000, 800);
    s.t.stock.put(RESIN, Some(2), 40_000, 800);

    issue(&s, RESIN, "30", "R1").await.assert_status(303);
    issue(&s, RESIN, "20", "R2").await.assert_status(303);

    let rows = rows(&s).await;
    let issued: Vec<(Option<i64>, i64)> =
        rows.iter().map(|r| (r.lot_id, r.quantity_milli)).collect();
    assert_eq!(issued, vec![(Some(1), 30_000), (Some(2), 20_000)]);
}

/// AC3: cartons are not lot-tracked: an issue without a lot is accepted.
#[renox::test]
async fn a_material_that_is_not_tracked_needs_no_lot() {
    let s = scene().await;
    s.t.stock.put(CARTON, None, 500_000, 150);

    issue(&s, CARTON, "100", "").await.assert_status(303);

    let rows = rows(&s).await;
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].lot_id, rows[0].quantity_milli), (None, 100_000));
}

/// AC4: 10 kg of resin (lot R3) at the store: an issue of 10 kg without a lot
/// is refused; an issue of 25 kg with lot R3 posts 10 kg from R3 and keeps a
/// pending issue of 15 kg with no lot.
#[renox::test]
async fn only_the_short_quantity_has_no_lot() {
    let s = scene().await;
    s.t.stock.put(RESIN, Some(3), 10_000, 800);

    issue(&s, RESIN, "10", "").await.assert_status(422);
    assert!(rows(&s).await.is_empty());

    issue(&s, RESIN, "25", "R3").await.assert_status(303);

    let rows = rows(&s).await;
    let issued: Vec<(Option<i64>, i64, bool)> = rows
        .iter()
        .map(|r| (r.lot_id, r.quantity_milli, r.pending_issue_id.is_some()))
        .collect();
    assert_eq!(issued, vec![(Some(3), 10_000, false), (None, 15_000, true)]);
}

/// With nothing on the shelves the whole quantity waits as a pending issue,
/// which has no lot.
#[renox::test]
async fn with_no_stock_at_all_the_issue_waits_without_a_lot() {
    let s = scene().await;

    issue(&s, RESIN, "20", "").await.assert_status(303);

    let rows = rows(&s).await;
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].lot_id, rows[0].quantity_milli), (None, 20_000));
    assert!(rows[0].pending_issue_id.is_some());
}

/// A lot that cannot be taken from (an unknown number, or one with nothing
/// at the store while other lots have stock) is refused rather than
/// replaced by another lot behind the storekeeper's back.
#[renox::test]
async fn a_lot_that_cannot_be_taken_is_refused() {
    let s = scene().await;
    s.t.stock.put(RESIN, Some(1), 30_000, 800);

    let unknown = issue(&s, RESIN, "10", "R9").await;
    unknown.assert_status(422);
    assert!(unknown.text().contains("R9"), "{}", unknown.text());
    assert!(rows(&s).await.is_empty());
}
