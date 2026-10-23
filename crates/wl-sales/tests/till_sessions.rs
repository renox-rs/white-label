//! The till session (wl-sales §4.3, `SAL.POS.01`; base for BL-FNB-15 and the
//! till handover of BL-PAY-19, #112): open with a float, count and close,
//! `TillSessionClosed` follows the commit, reopen with a reason, review.

mod support;

use std::sync::{Arc, Mutex};

use renox::prelude::*;
use wl_kernel::sales::{OpenTill, TillSessionClosed, till_desk};
use wl_sales::permissions as perm;
use wl_sales::till_sessions;
use wl_sales::{CounterSale, TillSession, TillSessionStatus};

use support::{Booted, boot_with};

/// Records the `TillSessionClosed` events.
#[derive(Clone, Default)]
struct Closed(Arc<Mutex<Vec<TillSessionClosed>>>);

impl Module for Closed {
    fn name(&self) -> &'static str {
        "till_events"
    }

    fn register(&self, app: &mut Registry) {
        let log = self.0.clone();
        app.listen(move |event: TillSessionClosed, _state: AppState| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                Ok(())
            }
        });
    }
}

async fn boot() -> (Booted, Closed) {
    let seen = Closed::default();
    let t = boot_with(seen.clone()).await;
    (t, seen)
}

fn open_till(cashier: i64, float: i64) -> OpenTill {
    OpenTill {
        till: "T1".to_owned(),
        location: Some("marina".to_owned()),
        legal_entity_id: None,
        business_date: wl_kernel::documents::today(),
        cashier_user: cashier,
        float,
    }
}

/// AC: a cashier opens a till with a float through the page.
#[renox::test]
async fn a_cashier_opens_a_till_session_with_a_float() {
    let (t, _seen) = boot().await;
    let cora = t.user("Cora", &[perm::POS_OPEN_TILL]).await;
    t.app.acting_as(&cora);
    t.app
        .post(
            "/till-sessions",
            &[("till", "T1"), ("location", "marina"), ("float", "300.00")],
        )
        .await
        .assert_status(303);
    let session = TillSession::query().first(t.db()).await.unwrap().unwrap();
    assert_eq!(session.status, TillSessionStatus::Open);
    assert_eq!(session.float_amount, 30_000);
    assert_eq!(session.cashier_user_id, cora.id);
    t.app.get("/till-sessions").await.assert_ok();
    // A second session of the same till and day is refused.
    t.app
        .post("/till-sessions", &[("till", "T1"), ("float", "0")])
        .await
        .assert_status(422);
}

/// AC: counting closes the session, works out the variance and emits
/// `TillSessionClosed` with the expected and counted cash.
#[renox::test]
async fn counting_closes_the_session_and_announces_the_shortage() {
    let (t, seen) = boot().await;
    let cora = t
        .user("Cora", &[perm::POS_OPEN_TILL, perm::POS_CLOSE_TILL])
        .await;
    t.app.acting_as(&cora);
    let session = till_sessions::open(t.state(), None, open_till(cora.id, 30_000))
        .await
        .unwrap();
    t.app
        .post(
            &format!("/till-sessions/{}/count", session.id),
            &[("counted", "265.00")],
        )
        .await
        .assert_status(303);
    let closed = TillSession::find_or_404(t.db(), session.id).await.unwrap();
    assert_eq!(closed.status, TillSessionStatus::Closed);
    assert_eq!(closed.expected_cash, 30_000);
    assert_eq!(closed.counted_cash, Some(26_500));
    assert_eq!(closed.variance, Some(-3_500));
    let events = seen.0.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].session, session.id);
    assert_eq!(events[0].till, "T1");
    assert_eq!(events[0].cashier_user, cora.id);
    assert_eq!((events[0].expected, events[0].counted), (30_000, 26_500));
    assert_eq!(events[0].variance(), -3_500);
}

/// AC: the cash of the counter sales rung up on the session is part of what
/// the till should hold.
#[renox::test]
async fn the_cash_sales_of_the_session_are_part_of_what_the_till_should_hold() {
    let (t, seen) = boot().await;
    let cora = t
        .user(
            "Cora",
            &[perm::POS_OPEN_TILL, perm::POS_CLOSE_TILL, perm::POS_SELL],
        )
        .await;
    t.app.acting_as(&cora);
    let session = till_sessions::open(t.state(), None, open_till(cora.id, 10_000))
        .await
        .unwrap();
    // The cart is rung up on the cashier's open session.
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
    assert_eq!(sale.till_session_id, Some(session.id));
    t.app
        .post(
            &format!("/counter-sales/{}/lines", sale.id),
            &[("item_code", "tee"), ("unit_price", "100.00")],
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
    let view = till_desk(t.state())
        .find(t.state(), session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(view.cash_sales, 10_500);
    assert_eq!(view.expected, 20_500);
    till_sessions::submit_count(t.state(), None, session.id, 20_500)
        .await
        .unwrap();
    let events = seen.0.lock().unwrap().clone();
    assert_eq!(events[0].expected, 20_500);
    assert_eq!(events[0].variance(), 0);
}

/// AC: a closed session is reopened with a reason by a holder of
/// `pos.reopen_till`, counted again, and marked reviewed by `pos.review`.
#[renox::test]
async fn a_closed_session_is_reopened_with_a_reason_and_reviewed() {
    let (t, _seen) = boot().await;
    let cora = t
        .user("Cora", &[perm::POS_OPEN_TILL, perm::POS_CLOSE_TILL])
        .await;
    let mia = t
        .user("Mia", &[perm::POS_REOPEN_TILL, perm::POS_REVIEW])
        .await;
    let session = till_sessions::open(t.state(), None, open_till(cora.id, 0))
        .await
        .unwrap();
    till_sessions::submit_count(t.state(), None, session.id, 0)
        .await
        .unwrap();

    // The cashier cannot reopen or review.
    t.app.acting_as(&cora);
    t.app
        .post(
            &format!("/till-sessions/{}/reopen", session.id),
            &[("reason", "typo")],
        )
        .await
        .assert_status(403);
    t.app
        .post(
            &format!("/till-sessions/{}/review", session.id),
            &[("x", "")],
        )
        .await
        .assert_status(403);

    t.app.acting_as(&mia);
    // A reopening needs its reason.
    t.app
        .post(
            &format!("/till-sessions/{}/reopen", session.id),
            &[("reason", "")],
        )
        .await
        .assert_status(422);
    t.app
        .post(
            &format!("/till-sessions/{}/reopen", session.id),
            &[("reason", "Counted the wrong drawer")],
        )
        .await
        .assert_status(303);
    let reopened = TillSession::find_or_404(t.db(), session.id).await.unwrap();
    assert_eq!(reopened.status, TillSessionStatus::Open);
    assert_eq!(reopened.reopen_count, 1);
    assert_eq!(
        reopened.reopen_reason.as_deref(),
        Some("Counted the wrong drawer")
    );
    assert_eq!(reopened.counted_cash, None);

    till_sessions::submit_count(t.state(), None, session.id, 0)
        .await
        .unwrap();
    t.app
        .post(
            &format!("/till-sessions/{}/review", session.id),
            &[("x", "")],
        )
        .await
        .assert_status(303);
    let reviewed = TillSession::find_or_404(t.db(), session.id).await.unwrap();
    assert_eq!(reviewed.status, TillSessionStatus::Reviewed);
    assert_eq!(reviewed.reviewed_by, Some(mia.id));
}

/// AC: the desk of the kernel opens, counts and reads a session for a vertical,
/// as the system.
#[renox::test]
async fn the_kernel_desk_runs_a_session_for_a_vertical() {
    let (t, seen) = boot().await;
    let desk = till_desk(t.state());
    let view = desk
        .open(t.state(), None, open_till(7, 5_000))
        .await
        .unwrap()
        .expect("the sales module provides the desk");
    assert_eq!(view.status, "open");
    assert_eq!(view.expected, 5_000);
    let closed = desk
        .submit_count(t.state(), None, view.id, 4_000)
        .await
        .unwrap();
    assert_eq!(closed.status, "closed");
    assert_eq!(closed.variance, Some(-1_000));
    assert_eq!(seen.0.lock().unwrap().len(), 1);
    // The same till and day cannot be opened twice.
    assert!(desk.open(t.state(), None, open_till(7, 0)).await.is_err());
}
