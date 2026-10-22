//! BL-HRM-24 (#214): a paid final settlement closes 30 days after `pay` with no
//! dispute, or as soon as the acknowledgement is signed; one that is no longer
//! needed is cancelled while nothing was paid.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::settlement_close;
use wl_hr::{Employee, EmployeeStatus, FinalSettlement, SettlementStatus};

use support::{Booted, nov};

/// A prepares and disputes, P pays. E, Ravi Kumar, is the employee.
struct World {
    t: Booted,
    a: User,
    p: User,
    e: Employee,
}

async fn world() -> World {
    let t = support::boot().await;
    let a = t.user_with("Amal", "settle_prep").await;
    let p = t.user_with("Pia", "settle_pay").await;
    let e = t.employee_in(61, EmployeeStatus::Active).await;
    World { t, a, p, e }
}

fn dec(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 12, day).unwrap()
}

/// An `approved` settlement of AED 1,000 net, nothing paid.
async fn approved(w: &World) -> FinalSettlement {
    w.t.settlement_in(&w.e, SettlementStatus::Approved, |s| {
        s.unpaid_salary = 100_000;
        s.net = 100_000;
    })
    .await
}

/// P pays `settlement` on `day`.
async fn pay_on(w: &World, settlement: &FinalSettlement, day: NaiveDate) {
    w.t.on(
        day,
        w.t.settle(
            &w.p,
            settlement,
            "pay",
            &[
                ("amount", "1000"),
                ("method", "transfer"),
                ("reference", "TRF-1"),
            ],
        ),
    )
    .await
    .assert_status(303);
}

/// The clock to `day`, then the daily closing job.
async fn job_on(w: &World, day: NaiveDate) {
    w.t.on(day, async {}).await;
    w.t.app.run_scheduled(settlement_close::TASK).await.unwrap();
}

/// AC1: given `pay` on 2026-11-01 and no signature, on 2026-12-01 it is
/// `closed`; the day before it is still `paid`.
#[renox::test]
async fn hrm24_ac1_a_paid_settlement_closes_thirty_days_after_payment() {
    let w = world().await;
    let settlement = approved(&w).await;
    pay_on(&w, &settlement, nov(1)).await;
    let paid = w.t.settlement(&settlement).await;
    assert_eq!(paid.status, SettlementStatus::Paid);
    assert_eq!(paid.paid_on, Some(nov(1)));
    assert_eq!(paid.closes_on, Some(dec(1)));
    assert_eq!(paid.acknowledgement_requested_on, Some(nov(1)));

    job_on(&w, nov(30)).await;
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
    job_on(&w, dec(1)).await;
    let closed = w.t.settlement(&settlement).await;
    assert_eq!(closed.status, SettlementStatus::Closed);
    assert_eq!(closed.closed_on, Some(dec(1)));
    // A second run changes nothing.
    job_on(&w, dec(2)).await;
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Closed
    );
}

/// AC2: given the acknowledgement signed on 2026-11-03, it closes on
/// 2026-11-03.
#[renox::test]
async fn hrm24_ac2_a_signed_acknowledgement_closes_the_settlement_at_once() {
    let w = world().await;
    let settlement = approved(&w).await;
    pay_on(&w, &settlement, nov(1)).await;
    w.t.on(
        nov(3),
        w.t.settle(
            &w.p,
            &settlement,
            "acknowledgement",
            &[("signed_on", "2026-11-03")],
        ),
    )
    .await
    .assert_status(303);
    let closed = w.t.settlement(&settlement).await;
    assert_eq!(closed.status, SettlementStatus::Closed);
    assert_eq!(closed.acknowledgement_signed_on, Some(nov(3)));
    assert_eq!(closed.closed_on, Some(nov(3)));
}

/// The estate is not asked for an acknowledgement: it closes at 30 days.
#[renox::test]
async fn hrm24_the_estate_has_no_acknowledgement_and_closes_at_thirty_days() {
    let w = world().await;
    let settlement =
        w.t.settlement_in(&w.e, SettlementStatus::Approved, |s| {
            s.payee = "estate".into();
            s.unpaid_salary = 100_000;
            s.net = 100_000;
            s.representative = Some("Sunil Kumar".into());
            s.right_document = Some("Court order 12".into());
        })
        .await;
    pay_on(&w, &settlement, nov(1)).await;
    let paid = w.t.settlement(&settlement).await;
    assert_eq!(paid.status, SettlementStatus::Paid);
    assert_eq!(paid.acknowledgement_requested_on, None);
    w.t.settle(&w.p, &settlement, "acknowledgement", &[])
        .await
        .assert_status(422);
    job_on(&w, dec(1)).await;
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Closed
    );
}

/// A settlement disputed before the period ends does not close by the clock.
#[renox::test]
async fn hrm24_a_dispute_stops_the_settlement_from_closing() {
    let w = world().await;
    let settlement = approved(&w).await;
    pay_on(&w, &settlement, nov(1)).await;
    w.t.settle(
        &w.a,
        &settlement,
        "dispute",
        &[
            ("source", "worker"),
            ("reference", "WRK-1"),
            ("undisputed_amount", "1000"),
        ],
    )
    .await
    .assert_status(303);
    job_on(&w, dec(5)).await;
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Disputed
    );
}

/// AC3: given `approved` with an AED 500 payment recorded, `cancel` is
/// refused.
#[renox::test]
async fn hrm24_ac3_a_settlement_with_a_payment_cannot_be_cancelled() {
    let w = world().await;
    let settlement = approved(&w).await;
    w.t.paid_against(&settlement, 50_000).await;
    w.t.settle_transition(&w.a, &settlement, "cancel")
        .await
        .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
}

/// A draft or approved settlement with nothing paid is cancelled by HR, or by
/// the system for a withdrawn notice or a reinstatement.
#[renox::test]
async fn hrm24_a_settlement_with_nothing_paid_is_cancelled_by_hr_or_by_the_system() {
    let w = world().await;
    let draft =
        w.t.settlement_in(&w.e, SettlementStatus::Draft, |_| {})
            .await;
    w.t.settle_transition(&w.a, &draft, "cancel")
        .await
        .assert_status(303);
    assert_eq!(
        w.t.settlement(&draft).await.status,
        SettlementStatus::Cancelled
    );

    let approved = approved(&w).await;
    let cancelled = settlement_close::cancel_open(w.t.state(), w.e.id)
        .await
        .unwrap()
        .expect("the employee has an open settlement");
    assert_eq!(cancelled.id, approved.id);
    assert_eq!(cancelled.status, SettlementStatus::Cancelled);
    assert!(
        settlement_close::cancel_open(w.t.state(), w.e.id)
            .await
            .unwrap()
            .is_none()
    );
}

/// A paid settlement can no longer be cancelled, by HR or by the system.
#[renox::test]
async fn hrm24_a_paid_settlement_cannot_be_cancelled() {
    let w = world().await;
    let settlement =
        w.t.settlement_in(&w.e, SettlementStatus::Paid, |s| s.net = 100_000)
            .await;
    w.t.settle_transition(&w.a, &settlement, "cancel")
        .await
        .assert_status(422);
    assert_eq!(
        w.t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );
}
