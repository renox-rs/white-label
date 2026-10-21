//! The lot of every consumable issued to a visit (BL-CLN-13, #361: `STK.MOV.01`,
//! `STK.LOT.01`), against the **real** wl-inventory: the branch store, the lots
//! with their expiry, the held lot, the movements and the pending issue for the
//! short part. Through the visit page, the booking's check-out and the
//! kernel's stock service; the booking, the visit and the bill are the real
//! ones of the clinics app and wl-scheduling.

mod support;

use renox::prelude::*;
use wl_inventory::{LotStatus, StockMovement};
use wl_scheduling::Refusal;
use wl_scheduling::service::{self, Actor};

use clinics::ChargeLine;
use support::desk::desk_with_stock;
use support::stock::{date, issued, lot, open_visit, receive, record, store};
use support::{at, day};

/// Done when 1: given the filler lots L1 (2027-01-31) and L2 (2027-03-31) at
/// Branch 1, the issue screen proposes L1; the held lot is not offered.
#[renox::test]
async fn the_issue_screen_proposes_the_lot_with_the_earliest_expiry() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (_, visit) = open_visit(&d).await;
    let page = d
        .get(
            &d.rita,
            &format!("/clinics/visits/{}?code=filler", visit.id),
        )
        .await;
    page.assert_ok().assert_see("L1").assert_see("2027-01-31");
    assert!(!page.text().contains("L2 ("), "{}", page.text());
    assert!(!page.text().contains("L3 ("), "{}", page.text());
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
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
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
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
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
    let pending = wl_inventory::pending_issue::open_of(d.t.db())
        .await
        .unwrap();
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
        wl_inventory::pending_issue::open_of(d.t.db())
            .await
            .unwrap()
            .len(),
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
    assert!(
        refused.text().contains("cannot be issued"),
        "{}",
        refused.text()
    );
    record(&d, &visit, 1, "L9").await.assert_status(422);
    assert!(
        ChargeLine::active_of(d.t.db(), visit.id)
            .await
            .unwrap()
            .is_empty()
    );
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

/// An expired lot is never offered: it is not proposed (although its expiry
/// is the earliest), the pick refuses it, `POST /lots` refuses it and so
/// does a new charge line naming it; the stock then leaves with L1.
#[renox::test]
async fn an_expired_lot_is_not_proposed_and_is_refused() {
    let d = desk_with_stock().await;
    let s = store(&d, 2, 2).await;
    let expired = lot(&d, &s.item, "L0", date(2020, 1, 31), LotStatus::Released).await;
    receive(&d, &s, &expired, 5).await;
    let (booking, visit) = open_visit(&d).await;

    // Proposed: L1, not the earlier-expiring L0.
    let page = d
        .get(
            &d.rita,
            &format!("/clinics/visits/{}?code=filler", visit.id),
        )
        .await;
    page.assert_ok().assert_see("L1");
    assert!(!page.text().contains("L0"), "{}", page.text());

    // A new line naming it is refused, nothing is recorded.
    let refused = record(&d, &visit, 1, "L0").await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("cannot be issued"),
        "{}",
        refused.text()
    );
    assert!(
        ChargeLine::active_of(d.t.db(), visit.id)
            .await
            .unwrap()
            .is_empty()
    );

    // So is `POST /lots` for a line that has none yet.
    record(&d, &visit, 1, "").await.assert_ok();
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    let refused = d
        .post(
            &d.rita,
            &format!("/clinics/visits/{}/lots", visit.id),
            &[("line", line.id.to_string().as_str()), ("lot", "l0")],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("cannot be issued"),
        "{}",
        refused.text()
    );
    let line = ChargeLine::find_or_404(d.t.db(), line.id).await.unwrap();
    assert_eq!(line.lot_id, None);

    // The check-out takes the lot with the earliest expiry that can be issued.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/lots", visit.id),
        &[("line", line.id.to_string().as_str()), ("lot", "")],
    )
    .await
    .assert_ok();
    d.complete(&booking).await;
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false)]
    );
}

/// A lot assigned to the line and put on hold (by a recall) before the
/// check-out: the check-out is refused ("lot no longer available") instead of
/// the stock silently leaving with another lot than the line shows; after the
/// desk picks again, the stock leaves with the new lot.
#[renox::test]
async fn a_lot_put_on_hold_after_it_was_assigned_blocks_the_check_out() {
    let d = desk_with_stock().await;
    let s = store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "L1").await.assert_ok();
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(line.lot_number.as_deref(), Some("L1"));

    // A recall of L1 holds it.
    let mia = d
        .t
        .user("Mia", &[wl_inventory::permissions::RECALLS_MANAGE])
        .await;
    let item = s.item.id.to_string();
    d.post(
        &mia,
        "/recalls",
        &[
            ("source", "supplier"),
            ("reason", "Contamination"),
            ("items", item.as_str()),
            ("lot_numbers", "L1"),
        ],
    )
    .await
    .assert_ok();
    let held = wl_inventory::Lot::find_or_404(d.t.db(), s.l1.id)
        .await
        .unwrap();
    assert_eq!(held.status, LotStatus::OnHold);

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
    assert!(text.contains("Lot no longer available"), "{text}");
    assert!(text.contains("filler"), "{text}");
    assert!(issued(&d, &visit).await.is_empty());

    // The held lot is not offered again; the blank pick takes L2.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/lots", visit.id),
        &[("line", line.id.to_string().as_str()), ("lot", "L1")],
    )
    .await
    .assert_status(422);
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/lots", visit.id),
        &[("line", line.id.to_string().as_str()), ("lot", "")],
    )
    .await
    .assert_ok();
    d.complete(&booking).await;
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L2".to_owned()), 1, false)]
    );
}

/// The clinic's complete guard answers for visits only: a booking that was
/// never checked in (no visit) and one that does not exist are not blocked,
/// while the guard still refuses the visit with a line missing its lot.
#[renox::test]
async fn a_booking_that_is_not_a_clinic_visit_is_never_blocked_by_the_lot_guard() {
    use clinics::lots::LotGuard;
    use wl_kernel::scheduling::{BookingId, CheckInAnswer, CompleteGuard};

    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let plain = d.booking(&d.consultation, 11).await;
    assert!(
        clinics::Visit::of_booking(d.t.db(), plain.id)
            .await
            .unwrap()
            .is_none()
    );
    let when = at(day(1), 18);
    assert_eq!(
        LotGuard
            .check(d.t.state(), BookingId(plain.id), when)
            .await
            .unwrap(),
        CheckInAnswer::Allowed
    );
    assert_eq!(
        LotGuard
            .check(d.t.state(), BookingId(9_999), when)
            .await
            .unwrap(),
        CheckInAnswer::Allowed
    );

    // A visit with a line that lacks its lot is refused by the same guard.
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "").await.assert_ok();
    let answer = LotGuard
        .check(d.t.state(), BookingId(booking.id), when)
        .await
        .unwrap();
    assert!(
        matches!(&answer, CheckInAnswer::Refused(text) if text.contains("Lot required")),
        "{answer:?}"
    );
}
