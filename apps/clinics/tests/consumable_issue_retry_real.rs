//! The stock issue of a visit that ended but did not go through
//! (BL-CLN-13, #361: `STK.MOV.01`), against the **real** wl-inventory: the
//! stock refuses (a database rule on the movements), the visit says so and
//! is billed anyway, the desk and the daily job issue it again without
//! issuing anything twice; a lot-tracked line the stock cannot place is
//! flagged to the desk, not silently dropped.

mod support;

use renox::prelude::*;
use wl_inventory::{Location, StockMovement};

use clinics::{ChargeLine, Visit};
use support::desk::{Desk, desk_with_stock};
use support::stock::{issued, open_visit, record, store};

/// Makes the real stock refuse every movement.
async fn stock_offline(d: &Desk) {
    renox::db::sql(
        "CREATE TRIGGER stock_offline BEFORE INSERT ON wl_inventory_stock_movements \
         BEGIN SELECT RAISE(ABORT, 'stock offline'); END",
    )
    .execute(d.t.db())
    .await
    .unwrap();
}

/// The stock accepts movements again.
async fn stock_online(d: &Desk) {
    renox::db::sql("DROP TRIGGER stock_offline")
        .execute(d.t.db())
        .await
        .unwrap();
}

/// The movements that took filler out of the store.
async fn movements(d: &Desk) -> usize {
    StockMovement::query()
        .where_(StockMovement::FROM_LOCATION_ID.is_not_null())
        .get(d.t.db())
        .await
        .unwrap()
        .len()
}

/// The stock refuses at the check-out: the booking and the visit are
/// completed, the visit is billed, the issue is flagged with its reason and
/// shown to the desk; a retry that fails keeps the flag; a retry that works
/// issues the line once and clears the flag; another retry issues nothing.
#[renox::test]
async fn a_failed_issue_is_flagged_and_the_desk_retries_it() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "L1").await.assert_ok();
    stock_offline(&d).await;

    d.complete(&booking).await;
    let after = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert_eq!(after.status, clinics::VisitStatus::Completed);
    assert!(after.issue_pending_at.is_some());
    assert!(
        after
            .issue_pending_reason
            .as_deref()
            .is_some_and(|reason| reason.starts_with("stock:")),
        "{:?}",
        after.issue_pending_reason
    );
    assert!(after.billed_at.is_some(), "the bill does not wait for the stock");
    assert!(issued(&d, &visit).await.is_empty());
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    assert!(line.issued_at.is_none());
    let path = format!("/clinics/visits/{}", visit.id);
    d.get(&d.rita, &path)
        .await
        .assert_ok()
        .assert_see("Pending: the stock was not issued")
        .assert_see("Issue the stock again");

    // Still offline: the retry is accepted and the flag stays.
    d.post(&d.rita, &format!("{path}/issue"), &[("x", "y")])
        .await
        .assert_ok();
    let still = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert!(still.issue_pending_at.is_some());
    assert!(issued(&d, &visit).await.is_empty());

    // Back online: one retry issues the line with its lot, once.
    stock_online(&d).await;
    d.post(&d.rita, &format!("{path}/issue"), &[("x", "y")])
        .await
        .assert_ok();
    let done = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert!(done.issue_pending_at.is_none());
    assert!(done.issue_pending_reason.is_none());
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false)]
    );
    assert_eq!(movements(&d).await, 1);
    d.post(&d.rita, &format!("{path}/issue"), &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(issued(&d, &visit).await.len(), 1, "nothing is issued twice");
    assert_eq!(movements(&d).await, 1);
    let page = d.get(&d.rita, &path).await;
    assert!(!page.text().contains("Issue the stock again"), "{}", page.text());
}

/// The daily job issues what a failed check-out left, for a cancelled visit
/// too, and an open visit is never retried by hand.
#[renox::test]
async fn the_daily_job_issues_what_a_failed_run_left() {
    let d = desk_with_stock().await;
    store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "").await.assert_ok();
    // An open visit has nothing to retry.
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/issue", visit.id),
        &[("x", "y")],
    )
    .await
    .assert_status(422);

    stock_offline(&d).await;
    d.cancel(&booking, "walked_out").await.assert_status(303);
    let after = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert_eq!(after.status, clinics::VisitStatus::Cancelled);
    assert!(after.issue_pending_at.is_some());
    assert!(issued(&d, &visit).await.is_empty());

    stock_online(&d).await;
    d.t.app
        .kernel()
        .call(clinics::lots::TASK, Vec::<String>::new())
        .await
        .expect("the daily job runs");
    let done = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert!(done.issue_pending_at.is_none());
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false)]
    );
    // A second run finds nothing.
    d.t.app
        .kernel()
        .call(clinics::lots::TASK, Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(movements(&d).await, 1);
}

/// A lot-tracked line whose branch store is unknown at the check-out is not
/// silently a charge only: the visit is flagged "store not known" for the
/// desk, and the retry issues it once the store is back.
#[renox::test]
async fn a_lot_tracked_line_without_a_store_is_flagged_to_the_desk() {
    let d = desk_with_stock().await;
    let s = store(&d, 2, 2).await;
    let (booking, visit) = open_visit(&d).await;
    record(&d, &visit, 1, "L1").await.assert_ok();
    let line = ChargeLine::active_of(d.t.db(), visit.id)
        .await
        .unwrap()
        .remove(0);
    assert!(line.lot_tracked);
    // The store moves to a place that is not the branch.
    Location::query()
        .where_(Location::ID.eq(s.store.id))
        .update(d.t.db(), &[("scope_id", &Some(9_999_i64))])
        .await
        .unwrap();

    d.complete(&booking).await;
    let after = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert_eq!(
        after.issue_pending_reason.as_deref(),
        Some(clinics::lots::UNKNOWN_STORE)
    );
    assert!(issued(&d, &visit).await.is_empty());
    d.get(&d.rita, &format!("/clinics/visits/{}", visit.id))
        .await
        .assert_ok()
        .assert_see("Pending: the branch store or the item is not known");

    Location::query()
        .where_(Location::ID.eq(s.store.id))
        .update(
            d.t.db(),
            &[("scope_id", &Some(support::branch().subject_id))],
        )
        .await
        .unwrap();
    d.post(
        &d.rita,
        &format!("/clinics/visits/{}/issue", visit.id),
        &[("x", "y")],
    )
    .await
    .assert_ok();
    let done = Visit::find_or_404(d.t.db(), visit.id).await.unwrap();
    assert!(done.issue_pending_at.is_none());
    assert_eq!(
        issued(&d, &visit).await,
        vec![(Some("L1".to_owned()), 1, false)]
    );
}
