//! BL-HRM-25 (#215), `HRM.RUL.09`: HR and the accountant are told N days (10)
//! after the last working day that the final settlement is due; the MD is told
//! on the legal deadline (14 days) that it is overdue; for a disputed
//! settlement the reminder is to pay the undisputed amount.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::settlement_alerts;
use wl_hr::{Employee, EmployeeStatus, FinalSettlement, SettlementStatus};

use support::{Booted, nov};

const DUE: &str = "hr.settlement_due";
const UNDISPUTED: &str = "hr.settlement_undisputed_due";
const OVERDUE: &str = "hr.settlement_overdue";

/// H is HR, C the accountant, M the MD. E, Ravi Kumar, left on 1 November.
struct World {
    t: Booted,
    hr: User,
    accountant: User,
    md: User,
    e: Employee,
}

async fn world() -> World {
    let t = support::boot().await;
    let hr = t.user_with("Hira", "hr").await;
    let accountant = t.user_with("Abdul", "accountant").await;
    let md = t.user_with("Mazen", "owner").await;
    let e = t.employee_in(61, EmployeeStatus::Notice).await;
    t.parties.name(61, "Ravi Kumar");
    World {
        t,
        hr,
        accountant,
        md,
        e,
    }
}

/// A settlement in `status` whose last working day is 1 November, net AED
/// 12,000, AED 10,000 undisputed when `disputed`.
async fn settlement(w: &World, status: SettlementStatus) -> FinalSettlement {
    w.t.settlement_in(&w.e, status, |s| {
        s.last_working_day = Some(nov(1));
        s.unpaid_salary = 1_200_000;
        s.net = 1_200_000;
        if status == SettlementStatus::Disputed {
            s.dispute_source = Some("worker".into());
            s.dispute_reference = Some("W-1".into());
            s.undisputed_amount = 1_000_000;
            s.disputed_from = Some("approved".into());
        }
    })
    .await
}

/// The clock to `day`, then the daily alert job.
async fn job_on(w: &World, day: NaiveDate) {
    w.t.on(day, async {}).await;
    w.t.app
        .run_scheduled(settlement_alerts::TASK)
        .await
        .unwrap();
}

fn told(w: &World, kind: &str) -> Vec<Vec<i64>> {
    w.t.notices
        .of_type(kind)
        .iter()
        .map(support::users)
        .collect()
}

fn sorted(mut ids: Vec<i64>) -> Vec<i64> {
    ids.sort_unstable();
    ids
}

fn nothing_sent(w: &World) {
    for kind in [DUE, UNDISPUTED, OVERDUE] {
        assert!(told(w, kind).is_empty(), "{kind} was sent");
    }
}

/// AC1: given last working day 2026-11-01 and settlement `approved`, HR and
/// the accountant are told on 2026-11-11 and the MD on 2026-11-15, once each.
#[renox::test]
async fn hrm25_ac1_hr_and_the_accountant_are_told_at_day_10_and_the_md_at_the_deadline() {
    let w = world().await;
    let s = settlement(&w, SettlementStatus::Approved).await;
    job_on(&w, nov(10)).await;
    nothing_sent(&w);

    job_on(&w, nov(11)).await;
    assert_eq!(told(&w, DUE), vec![sorted(vec![w.hr.id, w.accountant.id])]);
    assert!(told(&w, OVERDUE).is_empty());
    let sent = &w.t.notices.of_type(DUE)[0];
    assert_eq!(sent.data["employee"], "Ravi Kumar");
    assert_eq!(sent.data["deadline"], "15 Nov 2026");

    job_on(&w, nov(14)).await;
    assert!(told(&w, OVERDUE).is_empty());
    job_on(&w, nov(15)).await;
    assert_eq!(told(&w, OVERDUE), vec![vec![w.md.id]]);

    // One alert per point: later runs send nothing more.
    job_on(&w, nov(16)).await;
    assert_eq!(told(&w, DUE).len(), 1);
    assert_eq!(told(&w, OVERDUE).len(), 1);
    let stored = w.t.settlement(&s).await;
    assert_eq!(stored.alerted_on, Some(nov(11)));
    assert_eq!(stored.overdue_alerted_on, Some(nov(15)));
}

/// A `draft` is alerted too.
#[renox::test]
async fn hrm25_a_draft_settlement_is_alerted_as_well() {
    let w = world().await;
    settlement(&w, SettlementStatus::Draft).await;
    job_on(&w, nov(11)).await;
    assert_eq!(told(&w, DUE).len(), 1);
}

/// AC2: given `paid` on 2026-11-05, nothing is sent.
#[renox::test]
async fn hrm25_ac2_a_paid_settlement_is_not_alerted() {
    let w = world().await;
    let pay = w.t.user_with("Pia", "settle_pay").await;
    let s = settlement(&w, SettlementStatus::Approved).await;
    w.t.on(
        nov(5),
        w.t.settle(
            &pay,
            &s,
            "pay",
            &[
                ("amount", "12000"),
                ("method", "transfer"),
                ("reference", "TRF-1"),
            ],
        ),
    )
    .await
    .assert_status(303);
    let paid = w.t.settlement(&s).await;
    assert_eq!(paid.status, SettlementStatus::Paid);
    assert_eq!(paid.paid_on, Some(nov(5)));

    job_on(&w, nov(11)).await;
    job_on(&w, nov(15)).await;
    nothing_sent(&w);
}

/// AC3: given `disputed` with nothing paid, the message reminds to pay the
/// undisputed amount.
#[renox::test]
async fn hrm25_ac3_a_disputed_settlement_reminds_to_pay_the_undisputed_amount() {
    let w = world().await;
    settlement(&w, SettlementStatus::Disputed).await;
    job_on(&w, nov(11)).await;
    assert!(told(&w, DUE).is_empty());
    assert_eq!(
        told(&w, UNDISPUTED),
        vec![sorted(vec![w.hr.id, w.accountant.id])]
    );
    let sent = &w.t.notices.of_type(UNDISPUTED)[0];
    assert_eq!(sent.data["undisputed"], "10000.00");
    job_on(&w, nov(15)).await;
    assert_eq!(told(&w, OVERDUE), vec![vec![w.md.id]]);
}

/// AC4: given `disputed` with the undisputed amount paid in full, or `closed`
/// (or `cancelled`), nothing is sent.
#[renox::test]
async fn hrm25_ac4_nothing_is_sent_once_the_undisputed_amount_is_paid_or_it_is_closed() {
    let w = world().await;
    let disputed = settlement(&w, SettlementStatus::Disputed).await;
    w.t.paid_against(&disputed, 1_000_000).await;
    settlement(&w, SettlementStatus::Closed).await;
    settlement(&w, SettlementStatus::Cancelled).await;
    job_on(&w, nov(11)).await;
    job_on(&w, nov(15)).await;
    nothing_sent(&w);
}

/// A settlement whose last working day is not known has no deadline and is
/// skipped.
#[renox::test]
async fn hrm25_a_settlement_without_a_last_working_day_is_skipped() {
    let w = world().await;
    w.t.settlement_in(&w.e, SettlementStatus::Approved, |_| {})
        .await;
    job_on(&w, nov(20)).await;
    nothing_sent(&w);
}

/// A day the job missed is caught up: both alerts go out on the first run
/// after the deadline.
#[renox::test]
async fn hrm25_a_missed_day_is_caught_up_on_the_next_run() {
    let w = world().await;
    settlement(&w, SettlementStatus::Approved).await;
    job_on(&w, nov(17)).await;
    assert_eq!(told(&w, DUE).len(), 1);
    assert_eq!(told(&w, OVERDUE), vec![vec![w.md.id]]);
}
