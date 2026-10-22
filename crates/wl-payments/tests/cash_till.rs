//! BL-PAY-19 (#112), `FIN.ACL.05`, `FIN.ACL.06`, X-07, D-12, 06-N9-a: a closed
//! till session (`TillSessionClosed` of wl-sales) creates the till's cash
//! handover with the session's expected and counted cash, the till as holder
//! and the cashier as responsible person, and counts it; a shortage is
//! recovered from the cashier or written off as in BL-PAY-17.

mod support;

use renox::prelude::*;
use wl_kernel::custody::{HandoverKind, HandoverStatus, PeriodKind};
use wl_kernel::sales::{TILL_SESSION_SUBJECT_TYPE, TillSessionClosed};
use wl_payments::cash_recovery::RecoveryKind;
use wl_payments::cash_till;

use support::cash::date;
use support::{Booted, boot};

fn closed(session: i64, till: &str, cashier: &User, expected: i64, counted: i64) -> TillSessionClosed {
    TillSessionClosed {
        session,
        till: till.to_owned(),
        cashier_user: cashier.id,
        date: date("2027-04-01"),
        expected,
        counted,
        location: Some("Marina outlet".to_owned()),
        legal_entity_id: None,
    }
}

async fn handover_of(t: &Booted, session: i64) -> wl_payments::CashHandover {
    cash_till::of_session(t.db(), session)
        .await
        .unwrap()
        .expect("the till session has a handover")
}

/// AC1: till session TS-7 of till T1 closed with expected AED 4,000.00,
/// counted AED 3,950.00, cashier K: a handover with holder T1 and responsible
/// person K is `short`; `CashShortage` names T1, K, TS-7 and AED 50.00.
#[renox::test]
async fn a_short_till_session_creates_a_short_handover_of_the_till_for_its_cashier() {
    let t = boot().await;
    let k = t.holder("Kamal").await;

    t.state()
        .emit(closed(7, "T1", &k, 400_000, 395_000))
        .await
        .unwrap();

    let handover = handover_of(&t, 7).await;
    assert_eq!(handover.status, HandoverStatus::Short);
    assert_eq!(handover.kind, HandoverKind::Period);
    assert_eq!(handover.period_kind, PeriodKind::TillSession);
    assert_eq!(handover.holder_till.as_deref(), Some("T1"));
    assert_eq!(handover.responsible_user_id, k.id);
    assert_eq!(
        (handover.expected, handover.counted, handover.difference),
        (400_000, 395_000, 5_000)
    );
    let shortages = t.cash_shortage.all();
    assert_eq!(shortages.len(), 1);
    assert_eq!(shortages[0].holder_till.as_deref(), Some("T1"));
    assert_eq!(shortages[0].responsible_user, k.id);
    assert_eq!(shortages[0].till_session.as_ref().map(|s| s.subject_id), Some(7));
    assert_eq!(
        shortages[0].till_session.as_ref().map(|s| s.subject_type.as_str()),
        Some(TILL_SESSION_SUBJECT_TYPE)
    );
    assert_eq!(shortages[0].difference, 5_000);
}

/// AC2: `recover` with AED 50.00 cash records it as received from K, the
/// responsible person, while the holder stays the till.
#[renox::test]
async fn the_shortage_of_a_till_is_recovered_from_the_cashier() {
    let t = boot().await;
    let k = t.holder("Kamal").await;
    let d = t.counter("Dev").await;
    t.state()
        .emit(closed(7, "T1", &k, 400_000, 395_000))
        .await
        .unwrap();
    let handover = handover_of(&t, 7).await;

    t.cash_post(
        &d,
        &format!("/cash/{}/recover", handover.id),
        &[("amount", "50.00")],
    )
    .await
    .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Recovered);
    assert_eq!(after.holder_till.as_deref(), Some("T1"));
    let recoveries = t.recoveries(&after).await;
    assert_eq!(recoveries.len(), 1);
    assert_eq!(recoveries[0].kind, RecoveryKind::Cash);
    assert_eq!(recoveries[0].responsible_user_id, k.id);
    assert_eq!(recoveries[0].amount, 5_000);
}

/// AC3: counted = expected: the handover is `counted`, no shortage, and no
/// user counted it.
#[renox::test]
async fn a_till_session_without_a_variance_is_counted_by_the_system() {
    let t = boot().await;
    let k = t.holder("Kamal").await;

    t.state()
        .emit(closed(8, "T1", &k, 400_000, 400_000))
        .await
        .unwrap();

    let handover = handover_of(&t, 8).await;
    assert_eq!(handover.status, HandoverStatus::Counted);
    assert_eq!(handover.counted_by, None);
    assert_eq!(handover.difference, 0);
    assert!(t.cash_shortage.all().is_empty());
}

/// AC4: the same event again creates nothing.
#[renox::test]
async fn the_same_till_session_closed_twice_makes_one_handover() {
    let t = boot().await;
    let k = t.holder("Kamal").await;
    let event = closed(9, "T2", &k, 100_000, 99_000);

    t.state().emit(event.clone()).await.unwrap();
    t.state().emit(event).await.unwrap();

    let all = wl_payments::CashHandover::query()
        .where_(wl_payments::CashHandover::PERIOD_ID.eq(9_i64))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(t.cash_shortage.all().len(), 1);
}

/// Guard: nobody counts the till again here; a user with `cash.count` cannot
/// run `count` on a handover the session already counted.
#[renox::test]
async fn a_user_cannot_count_a_till_that_the_session_already_counted() {
    let t = boot().await;
    let k = t.holder("Kamal").await;
    let c = t.counter("Chandra").await;
    t.state()
        .emit(closed(10, "T1", &k, 100_000, 100_000))
        .await
        .unwrap();
    let handover = handover_of(&t, 10).await;

    let _response = t.count_cash(&c, &handover, "1000.00").await;

    assert_eq!(t.reloaded(&handover).await.counted_by, None);
}
