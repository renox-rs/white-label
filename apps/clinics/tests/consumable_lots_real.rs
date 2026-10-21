//! The lot of every consumable issued to a visit (BL-CLN-13, #361: `STK.MOV.01`,
//! `STK.LOT.01`), against the **real** wl-inventory: the branch store, the lots
//! with their expiry, the held lot, the movements and the pending issue for the
//! short part. Through the visit page, the booking's check-out and the
//! kernel's stock service; the booking, the visit and the bill are the real
//! ones of the clinics app and wl-scheduling.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_inventory::item::value_of;
use wl_inventory::{Item, Location, Lot, LotStatus, StockMovement};
use wl_kernel::Subject;
use wl_scheduling::service::{self, Actor};
use wl_scheduling::{Booking, Refusal};

use clinics::{ChargeLine, ConsumableIssue, Visit};
use support::desk::{Desk, desk_with_stock};
use support::{at, branch, day};

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The branch store with the filler lots L1 (expires 2027-01-31) and L2
/// (2027-03-31), and the lot L3 on hold.
struct Store {
    store: Location,
    item: Item,
    l1: Lot,
    l2: Lot,
    l3: Lot,
}

/// A lot of `item` with its expiry date in `status`.
async fn lot(d: &Desk, item: &Item, number: &str, expiry: NaiveDate, status: LotStatus) -> Lot {
    let mut lot = Lot {
        item_id: item.id,
        number: number.to_owned(),
        status,
        expiry_date: Some(expiry),
        ..Lot::default()
    };
    let mut tx = d.t.db().begin().await.unwrap();
    lot.insert_in_state(&mut tx, status, "fixture").await.unwrap();
    tx.commit().await.unwrap();
    lot
}

/// `quantity` units of `lot` received at the store.
async fn receive(d: &Desk, s: &Store, lot: &Lot, quantity: i64) {
    let source = Subject::new("inventory.goods_receipt", 900);
    let mut moved = StockMovement::receive(
        s.item.id,
        Some(lot.id),
        s.store.id,
        quantity * 1_000,
        500,
        value_of(quantity * 1_000, 500),
        &source,
        None,
    );
    moved.save(d.t.db()).await.unwrap();
}

/// The store of the branch the doctor works at: `l1` units of L1, `l2` of L2
/// and 5 of the held L3.
async fn store(d: &Desk, l1: i64, l2: i64) -> Store {
    let branch: Subject = branch();
    let mut location = Location::new("STORE-B2", "Branch store").in_scope(&branch);
    location.save(d.t.db()).await.unwrap();
    let mut item = Item::new("filler", "Filler", "pcs").lot_tracked();
    item.save(d.t.db()).await.unwrap();
    let first = lot(d, &item, "L1", date(2027, 1, 31), LotStatus::Released).await;
    let second = lot(d, &item, "L2", date(2027, 3, 31), LotStatus::Released).await;
    let held = lot(d, &item, "L3", date(2027, 2, 28), LotStatus::OnHold).await;
    let s = Store {
        store: location,
        item,
        l1: first,
        l2: second,
        l3: held,
    };
    if l1 > 0 {
        receive(d, &s, &s.l1, l1).await;
    }
    if l2 > 0 {
        receive(d, &s, &s.l2, l2).await;
    }
    receive(d, &s, &s.l3, 5).await;
    d.t.prices.set("filler", day(-400), 20_000);
    s
}

/// A checked-in, started visit of the consultation.
async fn open_visit(d: &Desk) -> (Booking, Visit) {
    let booking = d.booking(&d.consultation, 9).await;
    let visit = d.check_in(&booking).await;
    d.start(&booking).await;
    (booking, visit)
}

/// Records `units` of filler on the visit with `lot` (blank: none).
async fn record(d: &Desk, visit: &Visit, units: i64, lot: &str) -> renox::testing::TestResponse {
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/charges", visit.id),
        &[
            ("kind", "consumable"),
            ("code", "filler"),
            ("units", units.to_string().as_str()),
            ("lot", lot),
        ],
    )
    .await
}

/// The rows the stock issued for the visit, as (lot number, units, pending).
async fn issued(d: &Desk, visit: &Visit) -> Vec<(Option<String>, i64, bool)> {
    ConsumableIssue::of_visit(d.t.db(), visit.id)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.lot_number.clone(),
                row.quantity_milli / 1_000,
                row.pending_issue_id.is_some(),
            )
        })
        .collect()
}

/// Done when 1: given the filler lots L1 (2027-01-31) and L2 (2027-03-31) at
/// Branch 1, the issue screen proposes L1; the held lot is not offered.
#[renox::test]
async fn the_issue_screen_proposes_the_lot_with_the_earliest_expiry() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (_, visit) = open_visit(&d).await;
    let page = d
        .get(&d.rita, &format!("/clinics/visits/{}?code=filler", visit.id))
        .await;
    page.assert_ok().assert_see("L1").assert_see("2027-01-31");
    assert!(!page.text().contains("L2 ("), "{}", page.text());
    assert!(!page.text().contains("L3"), "{}", page.text());
}

/// Done when 2: a line for filler without a lot blocks the check-out with
/// "lot required"; setting the lot (the proposed one) lets it through, and
/// the stock leaves the store with that lot.
#[renox::test]
async fn a_lot_tracked_line_without_a_lot_blocks_the_check_out() {
    let d = desk_with_stock().await;
    let s = store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "").await.assert_ok();
    let line = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap().remove(0);
    assert!(line.lot_tracked);
    assert_eq!(line.lot_id, None);
    d.get(&d.rita, &format!("/clinics/visits/{}", visit.id))
        .await
        .assert_see("Lot required");

    let refused = service::complete(
        d.t.state(),
        booking.id,
        Actor::staff(d.rita.id),
        at(day(1), 18),
    )
    .await;
    let Err(Refusal::Invalid(text)) = refused else {
        panic!("the check-out is refused: {refused:?}");
    };
    assert!(text.contains("Lot required"), "{text}");
    assert!(text.contains("filler"), "{text}");
    assert!(issued(&d, &visit).await.is_empty());

    // The desk takes the proposed lot (blank) and the visit can be checked out.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/lots", visit.id),
        &[("line", line.id.to_string().as_str()), ("lot", "")],
    )
    .await
    .assert_ok();
    let line = ChargeLine::find_or_404(d.t.db(), line.id).await.unwrap();
    assert_eq!(line.lot_number.as_deref(), Some("L1"));
    d.complete(&booking).await;
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false)]
    );
    let moved = StockMovement::query()
        .where_(StockMovement::ITEM_ID.eq(s.item.id))
        .where_(StockMovement::LOT_ID.eq(Some(s.l1.id)))
        .where_(StockMovement::FROM_LOCATION_ID.eq(Some(s.store.id)))
        .get(d.t.db())
        .await
        .unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].quantity_milli, 1_000);
    let line = ChargeLine::find_or_404(d.t.db(), line.id).await.unwrap();
    assert!(line.issued_at.is_some());
}

/// A wrong lot is edited as the line before the check-out: the line takes
/// L2, and L2 is what leaves the store.
#[renox::test]
async fn a_wrong_lot_is_edited_before_the_check_out() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "L1").await.assert_ok();
    let line = ChargeLine::active_of(d.t.db(), visit.id).await.unwrap().remove(0);
    assert_eq!(line.lot_number.as_deref(), Some("L1"));
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/lots", visit.id),
        &[("line", line.id.to_string().as_str()), ("lot", "l2")],
    )
    .await
    .assert_ok();
    d.complete(&booking).await;
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L2".to_owned()), 1, false)]
    );
    // After the check-out the lot is not edited here.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/lots", visit.id),
        &[("line", line.id.to_string().as_str()), ("lot", "L1")],
    )
    .await
    .assert_status(422);
}

/// Done when 3: 1 unit at the store and 2 used: one row posts with L1 and 1
/// unit becomes a pending issue shown as "lot uncertain"; the check-out
/// succeeds.
#[renox::test]
async fn the_short_quantity_becomes_a_pending_issue_and_the_check_out_succeeds() {
    let d = desk_with_stock().await;
    store(&d, 1, 0).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 2, "L1").await.assert_ok();
    d.complete(&booking).await;

    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false), (None, 1, true)]
    );
    let pending = wl_inventory::pending_issue::open_of(d.t.db()).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].quantity_milli, 1_000);
    assert_eq!(pending[0].source_id, visit.id);
    d.get(&d.rita, &format!("/clinics/visits/{}", visit.id))
        .await
        .assert_ok()
        .assert_see("Lot uncertain");
}

/// Done when 4: L1 with 1 unit, L2 with 1 unit and 3 used: the rows take 1
/// unit from L1 and 1 unit from L2, and 1 unit becomes a pending issue.
#[renox::test]
async fn a_quantity_larger_than_one_lot_is_split_per_lot_first_expiry_first() {
    let d = desk_with_stock().await;
    store(&d, 1, 1).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 3, "L1").await.assert_ok();
    d.complete(&booking).await;

    assert_eq!(
        issued(&d, &visit).await,
        vec![
            (Some("L1".to_owned()), 1, false),
            (Some("L2".to_owned()), 1, false),
            (None, 1, true),
        ]
    );
    assert_eq!(
        wl_inventory::pending_issue::open_of(d.t.db()).await.unwrap().len(),
        1
    );
}

/// Done when 5: scanning the held lot L3 is refused, and so is a lot nobody
/// has; nothing is recorded.
#[renox::test]
async fn a_held_lot_is_refused() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (_, visit) = open_visit(&d).await;
    let refused = record(&d, &visit, 1, "L3").await;
    refused.assert_status(422);
    assert!(refused.text().contains("cannot be issued"), "{}", refused.text());
    record(&d, &visit, 1, "L9").await.assert_status(422);
    assert!(ChargeLine::active_of(d.t.db(), visit.id).await.unwrap().is_empty());
}

/// A visit cancelled after the check-in issues what was used before it
/// stopped; a line without a lot takes first expiry first.
#[renox::test]
async fn a_visit_that_stops_issues_what_was_used() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "").await.assert_ok();
    d.cancel(&booking, "walked_out").await.assert_status(303);
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false)]
    );
}

/// A consumable the stock does not know is a charge only: nothing is issued,
/// nothing blocks the check-out.
#[renox::test]
async fn a_consumable_the_stock_does_not_know_is_only_a_charge() {
    let d = desk_with_stock().await;
    store(&d, 1, 1).await;
    d.t.prices.set("gauze", day(-400), 500);
    let (booking, visit) = open_visit(&d).await;
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/charges", visit.id),
        &[("kind", "consumable"), ("code", "gauze"), ("units", "2")],
    )
    .await
    .assert_ok();
    d.complete(&booking).await;
    assert!(issued(&d, &visit).await.is_empty());
}
