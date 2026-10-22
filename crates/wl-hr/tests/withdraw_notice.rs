//! BL-HRM-06 (#196): `withdraw_notice` returns an employee in `notice` to
//! `active` when both sides agree in writing and the visa is not yet cancelled.
//! HR attaches the agreement and requests it; another user with
//! `employees.exit.approve` approves (`HRM.ACL.07`). The settlement and the
//! cancellation case are cancelled, the last working day is cleared and
//! `EmployeeNoticeWithdrawn` is emitted.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::{Employee, EmployeeStatus, FinalSettlement, SettlementStatus, VisaCaseStatus};
use wl_kernel::approvals::{self, ApprovalRequest};

use support::{Booted, nov};

/// E in `notice` with a last working day of 30 Nov.
async fn serving_notice(t: &Booted) -> Employee {
    let e = t.employee_in(60, EmployeeStatus::Notice).await;
    renox::db::sql("UPDATE wl_hr_employees SET last_working_day = ? WHERE id = ?")
        .bind(nov(30))
        .bind(e.id)
        .execute(t.db())
        .await
        .unwrap();
    e
}

/// HR asks for `withdraw_notice` of `e`, with the written agreement.
async fn ask(t: &Booted, hr: &User, e: &Employee, agreement: &str) -> renox::testing::TestResponse {
    t.app.acting_as(hr);
    t.app
        .htmx()
        .post(
            &format!("/hr/employees/{}/exits", e.id),
            &[
                ("transition", "withdraw_notice"),
                ("reason", "both sides agreed"),
                ("agreement", agreement),
            ],
        )
        .await
}

/// The MD approves the pending request of `e`.
async fn approve(t: &Booted, md: &User, e: &Employee) {
    let request = ApprovalRequest::pending_about(t.db(), &e.subject())
        .await
        .unwrap()
        .remove(0);
    approvals::approve(t.state(), request.id, md, "")
        .await
        .unwrap();
}

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, d).unwrap()
}

/// AC1: E in `notice`, the settlement `approved`, the cancellation case
/// `in_progress`, the agreement attached: when approved, E is `active`, the
/// settlement and the case are `cancelled`, the last working day is empty and
/// `EmployeeNoticeWithdrawn` is emitted.
#[renox::test]
async fn hrm06_ac1_an_approved_withdrawal_cancels_the_settlement_and_the_case() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = serving_notice(&t).await;
    let settlement = t
        .settlement_in(&e, SettlementStatus::Approved, |s| {
            s.last_working_day = Some(day(11, 30));
            s.unpaid_salary = 100_000;
            s.net = 100_000;
        })
        .await;
    let case = t
        .visa_in(&e, "cancellation", VisaCaseStatus::InProgress, |_| {})
        .await;

    ask(&t, &hr, &e, "AGR-1").await.assert_ok();
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Notice);
    approve(&t, &md, &e).await;

    let after = t.employee(&e).await;
    assert_eq!(after.status, EmployeeStatus::Active);
    assert_eq!(after.last_working_day, None);
    assert_eq!(
        t.settlement(&settlement).await.status,
        SettlementStatus::Cancelled
    );
    assert_eq!(t.case(&case).await.status, VisaCaseStatus::Cancelled);
    let events = t.notice_withdrawn.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, e.employee_id());
}

/// A draft settlement is cancelled too, and an employee with no cancellation
/// case (not sponsored) can withdraw.
#[renox::test]
async fn hrm06_a_draft_settlement_is_cancelled_and_no_case_is_needed() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = serving_notice(&t).await;
    let settlement = t.settlement_in(&e, SettlementStatus::Draft, |_| {}).await;

    ask(&t, &hr, &e, "AGR-1").await.assert_ok();
    approve(&t, &md, &e).await;

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Active);
    let after: FinalSettlement = t.settlement(&settlement).await;
    assert_eq!(after.status, SettlementStatus::Cancelled);
}

/// AC2: the cancellation case `completed`: refused, and the case, the
/// settlement and the employee stay as they were.
#[renox::test]
async fn hrm06_ac2_a_completed_cancellation_case_refuses_the_withdrawal() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = serving_notice(&t).await;
    let settlement = t
        .settlement_in(&e, SettlementStatus::Approved, |_| {})
        .await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |_| {})
        .await;

    let refused = ask(&t, &hr, &e, "AGR-1").await;
    refused.assert_status(422);
    assert!(refused.text().contains("completed"), "{}", refused.text());
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Notice);
    assert_eq!(
        t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
    assert!(
        ApprovalRequest::pending_about(t.db(), &e.subject())
            .await
            .unwrap()
            .is_empty()
    );
}

/// AC3: the settlement `paid` is refused with "Settlement already paid"; a
/// `disputed` one with "Settlement is disputed"; an `approved` one with AED 500
/// recorded with "A payment is recorded against the settlement".
#[renox::test]
async fn hrm06_ac3_a_paid_disputed_or_part_paid_settlement_refuses_the_withdrawal() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let cases = [
        (SettlementStatus::Paid, "Settlement already paid"),
        (SettlementStatus::Closed, "Settlement already paid"),
        (SettlementStatus::Disputed, "Settlement is disputed"),
        (
            SettlementStatus::Approved,
            "A payment is recorded against the settlement",
        ),
    ];
    for (n, (status, message)) in cases.into_iter().enumerate() {
        let e = t.employee_in(60 + n as i64, EmployeeStatus::Notice).await;
        let settlement = t.settlement_in(&e, status, |s| s.net = 100_000).await;
        if status == SettlementStatus::Approved {
            t.paid_against(&settlement, 50_000).await;
        }
        let refused = ask(&t, &hr, &e, "AGR-1").await;
        refused.assert_status(422);
        assert!(
            refused.text().contains(message),
            "{status:?}: {}",
            refused.text()
        );
        assert_eq!(t.employee(&e).await.status, EmployeeStatus::Notice);
        assert_eq!(t.settlement(&settlement).await.status, status);
    }
}

/// AC3, at the approval: a payment recorded after the request but before the
/// approval ends the request `failed`; nothing is cancelled.
#[renox::test]
async fn hrm06_a_payment_recorded_before_the_approval_fails_the_request() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = serving_notice(&t).await;
    let settlement = t
        .settlement_in(&e, SettlementStatus::Approved, |s| s.net = 100_000)
        .await;
    ask(&t, &hr, &e, "AGR-1").await.assert_ok();

    t.paid_against(&settlement, 50_000).await;
    approve(&t, &md, &e).await;

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Notice);
    assert_eq!(
        t.settlement(&settlement).await.status,
        SettlementStatus::Approved
    );
    assert!(t.notice_withdrawn.all().is_empty());
}

/// AC4: without the agreement document it is refused.
#[renox::test]
async fn hrm06_ac4_without_the_agreement_it_is_refused() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = serving_notice(&t).await;

    ask(&t, &hr, &e, "  ").await.assert_status(422);

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Notice);
    assert!(
        ApprovalRequest::pending_about(t.db(), &e.subject())
            .await
            .unwrap()
            .is_empty()
    );
}
