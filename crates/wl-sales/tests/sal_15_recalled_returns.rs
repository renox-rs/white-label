//! BL-SAL-15 (#134): accept returns of recalled goods into the recall's hold
//! (wl-sales §3 *Sales return*, §4.5 `receive`, §5.6 step 2). `LotTrace`
//! (wl-inventory) says whether a lot, or an item not tracked by lot sold on a
//! day, is in an open recall; the return is then accepted outside the return
//! window without approval, linked to the recall, taken into quarantine and
//! onto the recall's hold, and resolving it never restores stock. A stand-in of
//! `LotTrace` answers here; `host/tests/sal_15_real.rs` runs the real
//! wl-inventory.

mod support;

use std::sync::{Arc, Mutex};

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::inventory::{LotTrace, ReturnedGoods, TraceFuture, provide_lot_trace};
use wl_sales::permissions as perm;
use wl_sales::{CounterSale, ReturnStatus, SaleLine, SalesReturn};

use support::{Booted, boot_configured};

/// A `LotTrace`: lot 7 is under recall 1; the tees sold from 2026-08-05 are
/// under recall 2 (a part not tracked by lot, first received 2026-08-05).
#[derive(Clone, Default)]
struct Recalls {
    received: Arc<Mutex<Vec<(ReturnedGoods, Subject)>>>,
}

impl LotTrace for Recalls {
    fn recall_of<'a>(
        &'a self,
        _state: &'a AppState,
        goods: &'a ReturnedGoods,
    ) -> TraceFuture<'a, Option<Subject>> {
        Box::pin(async move {
            let first = NaiveDate::from_ymd_opt(2026, 8, 5).unwrap();
            Ok(match (&goods.lot, goods.sold_on) {
                (Some(lot), _) if lot.subject_id == 7 => {
                    Some(Subject::new("inventory.recall", 1))
                }
                (None, Some(day)) if day >= first => Some(Subject::new("inventory.recall", 2)),
                _ => None,
            })
        })
    }

    fn receive_returned<'a>(
        &'a self,
        _state: &'a AppState,
        recall: &'a Subject,
        goods: &'a ReturnedGoods,
    ) -> TraceFuture<'a, bool> {
        Box::pin(async move {
            self.received
                .lock()
                .unwrap()
                .push((goods.clone(), recall.clone()));
            Ok(true)
        })
    }
}

impl Module for Recalls {
    fn name(&self) -> &'static str {
        "recalls_stand_in"
    }

    fn register(&self, app: &mut Registry) {
        provide_lot_trace(app, self.clone());
    }
}

async fn boot() -> (Booted, Recalls) {
    let recalls = Recalls::default();
    let t = boot_configured(wl_sales::WlSales::new(), recalls.clone()).await;
    (t, recalls)
}

async fn clerk(t: &Booted) -> User {
    t.user(
        "Clara",
        &[perm::POS_SELL, perm::RETURNS_REQUEST, perm::RETURNS_HANDLE],
    )
    .await
}

/// A completed counter sale of 2 tees, completed on `on`.
async fn sold_on(t: &Booted, on: DateTime) -> (CounterSale, SaleLine) {
    t.app
        .post("/counter-sales", &[("x", "")])
        .await
        .assert_status(303);
    let sale = CounterSale::query()
        .order_by_desc(CounterSale::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[
                ("item_code", "tee"),
                ("unit_price", "50.00"),
                ("quantity", "2"),
            ],
        )
        .await
        .assert_status(303);
    t.app
        .post(
            &format!("/counter-sales/{}/complete", sale.id),
            &[("cash", "105.00")],
        )
        .await
        .assert_status(303);
    CounterSale::query()
        .where_(CounterSale::ID.eq(sale.id))
        .update(t.db(), &[("completed_at", &Some(on))])
        .await
        .unwrap();
    let sale = CounterSale::find_or_404(t.db(), sale.id).await.unwrap();
    let line = SaleLine::of(t.db(), &sale.subject()).await.unwrap().remove(0);
    (sale, line)
}

/// Requests a return of `lines` (like `12:2:7`) of `sale` and returns it.
async fn asked(t: &Booted, sale: &CounterSale, lines: &str) -> SalesReturn {
    t.app
        .post(
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
        .first(t.db())
        .await
        .unwrap()
        .expect("the return was requested")
}

fn days_ago(days: i64) -> DateTime {
    renox::db::now() - TimeDelta::days(days)
}

/// Done when 1 and 2: a return of 2 units of lot 7 (under recall 1) sold 60
/// days ago, window 14: `receive` accepts it without approval, links it to the
/// recall and takes the 2 units into the recall's hold; resolving it with a
/// credit note restores no stock.
#[renox::test]
async fn sal_15_a_late_return_of_a_recalled_lot_joins_the_recall_and_restores_no_stock() {
    let (t, recalls) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let (sale, line) = sold_on(&t, days_ago(60)).await;
    let r = asked(&t, &sale, &format!("{}:2:7", line.id)).await;

    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);

    let r = SalesReturn::find_or_404(t.db(), r.id).await.unwrap();
    assert_eq!(r.status, ReturnStatus::Received);
    assert_eq!(r.recall(), Some(Subject::new("inventory.recall", 1)));
    assert!(!r.stock_follows_the_return());
    let lines = r.lines(t.db()).await.unwrap();
    assert_eq!(lines[0].lot_id, Some(7));
    let received = recalls.received.lock().unwrap().clone();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].0.item_code, "tee");
    assert_eq!(received[0].0.quantity_milli, 2_000);
    assert_eq!(received[0].0.lot, Some(Subject::new("inventory.lot", 7)));
    assert_eq!(received[0].1, Subject::new("inventory.recall", 1));

    t.app
        .post(
            &format!("/sales-returns/{}/resolve", r.id),
            &[
                ("resolution", "credit_note"),
                ("credit_note", "5"),
                ("inspected", "on"),
            ],
        )
        .await
        .assert_status(303);
    let r = SalesReturn::find_or_404(t.db(), r.id).await.unwrap();
    assert_eq!(r.status, ReturnStatus::Resolved);
    assert_eq!(r.credit_note_id, Some(5));
    // Nothing else moved: resolving restores no stock.
    assert_eq!(recalls.received.lock().unwrap().len(), 1);
}

/// Done when 3: a part not tracked by lot, first received 2026-08-05 in the
/// recall: a return of 1 sold 2026-09-10 is linked and joins the hold; one sold
/// 2026-08-01 is a normal return, refused outside the window.
#[renox::test]
async fn sal_15_an_item_not_tracked_by_lot_is_recalled_by_its_sale_date() {
    let (t, recalls) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let in_window = NaiveDate::from_ymd_opt(2026, 9, 10)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc();
    let (sale, line) = sold_on(&t, in_window).await;
    let r = asked(&t, &sale, &format!("{}:1", line.id)).await;
    t.app
        .post(&format!("/sales-returns/{}/receive", r.id), &[("x", "")])
        .await
        .assert_status(303);
    let r = SalesReturn::find_or_404(t.db(), r.id).await.unwrap();
    assert_eq!(r.recall(), Some(Subject::new("inventory.recall", 2)));
    let received = recalls.received.lock().unwrap().clone();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].0.quantity_milli, 1_000);
    assert_eq!(received[0].0.lot, None);

    let before = NaiveDate::from_ymd_opt(2026, 8, 1)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc();
    let (older, older_line) = sold_on(&t, before).await;
    let normal = asked(&t, &older, &format!("{}:1", older_line.id)).await;
    t.app
        .post(&format!("/sales-returns/{}/receive", normal.id), &[("x", "")])
        .await
        .assert_status(422);
    let normal = SalesReturn::find_or_404(t.db(), normal.id).await.unwrap();
    assert_eq!(normal.status, ReturnStatus::Requested);
    assert_eq!(normal.recall_id, None);
    assert_eq!(recalls.received.lock().unwrap().len(), 1);
}

/// A return that is not under a recall keeps the window: refused outside it
/// without approval, accepted with it, and nothing joins a hold.
#[renox::test]
async fn sal_15_a_late_ordinary_return_still_needs_the_approval() {
    let (t, recalls) = boot().await;
    t.app.acting_as(&clerk(&t).await);
    let (sale, line) = sold_on(&t, days_ago(60)).await;
    // The tee is not in a recall here: no lot, sold in the (stand-in) past.
    let ordinary = {
        let before = NaiveDate::from_ymd_opt(2026, 7, 1)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap()
            .and_utc();
        CounterSale::query()
            .where_(CounterSale::ID.eq(sale.id))
            .update(t.db(), &[("completed_at", &Some(before))])
            .await
            .unwrap();
        asked(&t, &sale, &format!("{}:1", line.id)).await
    };
    t.app
        .post(&format!("/sales-returns/{}/receive", ordinary.id), &[("x", "")])
        .await
        .assert_status(422);
    t.app
        .post(
            &format!("/sales-returns/{}/receive", ordinary.id),
            &[("approved", "on")],
        )
        .await
        .assert_status(303);
    let ordinary = SalesReturn::find_or_404(t.db(), ordinary.id).await.unwrap();
    assert_eq!(ordinary.status, ReturnStatus::Received);
    assert_eq!(ordinary.recall_id, None);
    assert!(ordinary.stock_follows_the_return());
    assert!(recalls.received.lock().unwrap().is_empty());
}
