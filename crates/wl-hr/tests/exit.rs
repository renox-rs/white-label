//! BL-HRM-10 (#200): HR runs `exit` on an absconded or deceased employee once
//! the cancellation (or the authority's reference) and the settlement allow
//! it. No signed acknowledgement is needed and, for an absconded worker, no
//! payment. The portal turns read-only at once and closes 60 days later;
//! `EmployeeExited` carries the exit route.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::{Employee, EmployeeStatus, FinalSettlement, SettlementStatus, VisaCaseStatus};
use wl_kernel::workforce::workforce;

use support::{Booted, nov};

/// `user` runs `exit` on `e` on `day`.
async fn exit(
    t: &Booted,
    day: NaiveDate,
    user: &User,
    e: &Employee,
) -> renox::testing::TestResponse {
    t.on(day, async {
        t.app.acting_as(user);
        t.app
            .post(
                &format!("/hr/employees/{}/transitions/exit", e.id),
                &[("reason", "")],
            )
            .await
    })
    .await
}

/// `user` records the authority's reference "CANC-12" for `e`.
async fn reference(t: &Booted, user: &User, e: &Employee) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .post(
            &format!("/hr/employees/{}/cancellation-reference", e.id),
            &[("reference", "CANC-12")],
        )
        .await
}

/// Whether the portal of user `user` is read-only and whether it is closed on
/// `day`.
async fn portal(t: &Booted, user: i64, day: NaiveDate) -> (bool, bool) {
    t.on(day, async {
        let w = workforce(t.state());
        (
            w.read_only(t.db(), user).await.unwrap(),
            w.access_disabled(t.db(), user).await.unwrap(),
        )
    })
    .await
}

fn estate(s: &mut FinalSettlement) {
    s.payee = "estate".into();
    s.reason = "death".into();
    s.unpaid_salary = 100_000;
    s.net = 100_000;
}

/// AC1: E `absconded`, the cancellation case `completed`, the settlement
/// `approved` (not paid, not signed): `exit` makes E `exited` and
/// `EmployeeExited` has the route `absconding`.
#[renox::test]
async fn hrm10_ac1_an_absconded_employee_exits_with_an_approved_settlement() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Absconded).await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |c| {
        c.variant = "absconding".into();
    })
    .await;
    let settlement = t
        .settlement_in(&e, SettlementStatus::Approved, |s| {
            s.reason = "absconding".into();
        })
        .await;

    exit(&t, nov(10), &hr, &e).await.assert_status(303);

    let exited = t.employee(&e).await;
    assert_eq!(exited.status, EmployeeStatus::Exited);
    assert_eq!(exited.exit_route.as_deref(), Some("absconding"));
    assert_eq!(exited.exited_on, Some(nov(10)));
    let events = t.exited.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, e.employee_id());
    assert_eq!(events[0].exit_date, nov(10));
    assert_eq!(events[0].route, "absconding");
    // The approved settlement stays as a liability.
    assert_eq!(
        t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
}

/// AC2: E `deceased` and the settlement `approved` (not paid): `exit` is
/// refused; after `paid` it succeeds with the route `death`.
#[renox::test]
async fn hrm10_ac2_a_deceased_employee_exits_only_after_the_estate_is_paid() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Deceased).await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |c| {
        c.variant = "death".into();
    })
    .await;
    let settlement = t
        .settlement_in(&e, SettlementStatus::Approved, estate)
        .await;

    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Deceased);
    assert!(t.exited.all().is_empty());

    renox::db::sql("UPDATE wl_hr_final_settlements SET status = 'paid' WHERE id = ?")
        .bind(settlement.id)
        .execute(t.db())
        .await
        .unwrap();
    exit(&t, nov(10), &hr, &e).await.assert_status(303);

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Exited);
    assert_eq!(t.exited.all()[0].route, "death");
}

/// AC3: E `absconded`, the settlement `approved`, no completed case but the
/// authority's cancellation reference recorded: `exit` succeeds. Without the
/// reference it is refused.
#[renox::test]
async fn hrm10_ac3_the_authoritys_cancellation_reference_replaces_the_completed_case() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Absconded).await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::InProgress, |_| {})
        .await;
    t.settlement_in(&e, SettlementStatus::Approved, |_| {})
        .await;

    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    // A blank reference records nothing.
    t.app.acting_as(&hr);
    t.app
        .post(
            &format!("/hr/employees/{}/cancellation-reference", e.id),
            &[("reference", "  ")],
        )
        .await
        .assert_status(422);
    reference(&t, &hr, &e).await.assert_ok();
    assert_eq!(
        t.employee(&e).await.cancellation_reference.as_deref(),
        Some("CANC-12")
    );

    exit(&t, nov(10), &hr, &e).await.assert_status(303);
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Exited);
}

/// AC4: E exited on 2026-11-10: the portal is read-only at once and closes 60
/// days later, on 2027-01-09.
#[renox::test]
async fn hrm10_ac4_the_portal_is_read_only_at_once_and_closes_sixty_days_later() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let account = t.user("Ravi").await;
    let e = t.employee_in(60, EmployeeStatus::Absconded).await;
    renox::db::sql("UPDATE wl_hr_employees SET user_id = ? WHERE id = ?")
        .bind(account.id)
        .bind(e.id)
        .execute(t.db())
        .await
        .unwrap();
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |_| {})
        .await;
    t.settlement_in(&e, SettlementStatus::Approved, |_| {})
        .await;

    exit(&t, nov(10), &hr, &e).await.assert_status(303);
    let jan = |d| NaiveDate::from_ymd_opt(2027, 1, d).unwrap();
    assert_eq!(portal(&t, account.id, nov(10)).await, (true, false));
    assert_eq!(portal(&t, account.id, jan(8)).await, (true, false));
    assert_eq!(portal(&t, account.id, jan(9)).await, (false, true));
}

/// AC5: E `absconded` with the settlement `disputed`: `exit` is refused.
#[renox::test]
async fn hrm10_ac5_a_disputed_settlement_blocks_the_exit() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Absconded).await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |_| {})
        .await;
    t.settlement_in(&e, SettlementStatus::Disputed, |_| {})
        .await;

    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Absconded);
    assert!(t.exited.all().is_empty());
}

/// Edge: an absconded worker with no settlement, or only a `draft` one, does
/// not exit; a cancelled one does not count; a `paid` one is enough.
#[renox::test]
async fn hrm10_the_settlement_must_be_approved_or_paid_not_a_draft_or_cancelled() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Absconded).await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |_| {})
        .await;
    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    t.settlement_in(&e, SettlementStatus::Draft, |_| {}).await;
    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    t.settlement_in(&e, SettlementStatus::Cancelled, |_| {})
        .await;
    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    t.settlement_in(&e, SettlementStatus::Paid, |_| {}).await;
    exit(&t, nov(10), &hr, &e).await.assert_status(303);
    assert_eq!(t.exited.all()[0].route, "absconding");
}

/// Edge: from `notice` the standard exit needs a paid settlement and no live
/// cancellation case, and emits the route `standard`; an `active` employee
/// cannot exit, and only HR (`employees.manage`) may.
#[renox::test]
async fn hrm10_the_standard_exit_and_who_may_run_it() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let active = t.employee_in(61, EmployeeStatus::Active).await;
    exit(&t, nov(10), &hr, &active).await.assert_status(422);

    let e = t.employee_in(60, EmployeeStatus::Notice).await;
    let settlement = t
        .settlement_in(&e, SettlementStatus::Approved, |_| {})
        .await;
    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    renox::db::sql("UPDATE wl_hr_final_settlements SET status = 'paid' WHERE id = ?")
        .bind(settlement.id)
        .execute(t.db())
        .await
        .unwrap();
    let case = t
        .visa_in(&e, "cancellation", VisaCaseStatus::InProgress, |_| {})
        .await;
    exit(&t, nov(10), &hr, &e).await.assert_status(422);
    renox::db::sql("UPDATE wl_hr_visa_cases SET status = 'completed' WHERE id = ?")
        .bind(case.id)
        .execute(t.db())
        .await
        .unwrap();
    exit(&t, nov(10), &md, &e).await.assert_status(403);
    exit(&t, nov(10), &hr, &e).await.assert_status(303);
    assert_eq!(t.exited.all()[0].route, "standard");
}
