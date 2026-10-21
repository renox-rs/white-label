//! BL-HRM-05 (#195): `cancel_onboarding` ends an onboarding that never starts.
//! HR requests it with a reason, another user with `employees.exit.approve`
//! approves (`HRM.ACL.07`); the new visa case must be cancelled or rejected, an
//! issued permit needs a completed cancellation case or *cancellation not
//! needed* from the PRO, and days worked need a paid settlement with the reason
//! *onboarding cancelled*. Visa costs stay on the employee.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use sql_check::count_of;
use wl_hr::{Adjustment, Employee, EmployeeStatus, Receivable, SettlementStatus, VisaCaseStatus};
use wl_kernel::approvals::{self, ApprovalEnd, ApprovalRequest, ApprovalStatus};
use wl_kernel::invoicing::{PostCost, cost_poster};
use wl_kernel::workforce::workforce;

use support::{Booted, nov};

/// Counting rows of a model about an employee.
mod sql_check {
    use renox::prelude::*;

    pub async fn count_of<M: Model>(
        db: &Db,
        column: renox::db::Col<M, i64>,
        employee: i64,
    ) -> usize {
        M::query()
            .where_(column.eq(employee))
            .get(db)
            .await
            .unwrap()
            .len()
    }
}

/// HR asks for `cancel_onboarding` of `e` with `fields` on `day`.
async fn ask(
    t: &Booted,
    hr: &User,
    e: &Employee,
    day: NaiveDate,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let mut all = vec![("transition", "cancel_onboarding")];
    all.extend_from_slice(fields);
    t.on(day, async {
        t.app.acting_as(hr);
        t.app
            .htmx()
            .post(&format!("/hr/employees/{}/exits", e.id), &all)
            .await
    })
    .await
}

/// The MD approves the pending request of `e`.
async fn approve(t: &Booted, md: &User, e: &Employee, day: NaiveDate) {
    let request = ApprovalRequest::pending_about(t.db(), &e.subject())
        .await
        .unwrap()
        .remove(0);
    t.on(day, approvals::approve(t.state(), request.id, md, ""))
        .await
        .unwrap();
}

/// AC1: E in `onboarding` with the *new* case `rejected`, no permit issued and
/// no days worked: the approved request ends in `onboarding_cancelled`, the
/// exit route is set, the portal account is closed and
/// `EmployeeOnboardingCancelled` is emitted.
#[renox::test]
async fn hrm05_ac1_a_rejected_new_case_and_no_days_worked_cancel_the_onboarding() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    let portal = t.user("Ravi").await;
    sqlx_set_user(&t, &e, portal.id).await;
    t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;

    assert!(
        !workforce(t.state())
            .access_disabled(t.db(), portal.id)
            .await
            .unwrap()
    );

    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_ok();
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Onboarding);
    approve(&t, &md, &e, nov(10)).await;
    assert!(
        workforce(t.state())
            .access_disabled(t.db(), portal.id)
            .await
            .unwrap(),
        "the portal account is closed"
    );

    let after = t.employee(&e).await;
    assert_eq!(after.status, EmployeeStatus::OnboardingCancelled);
    assert_eq!(after.exit_route.as_deref(), Some("onboarding_cancelled"));
    assert!(
        after.portal_closed_at.is_some(),
        "the portal account is closed"
    );
    let events = t.onboarding_cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, e.employee_id());
    assert_eq!(events[0].reason, "visa refused");
}

/// An employee with no account has no portal to close.
#[renox::test]
async fn hrm05_without_an_account_no_portal_is_closed() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;

    ask(&t, &hr, &e, nov(10), &[("reason", "candidate withdrew")])
        .await
        .assert_ok();
    approve(&t, &md, &e, nov(10)).await;

    let after = t.employee(&e).await;
    assert_eq!(after.status, EmployeeStatus::OnboardingCancelled);
    assert_eq!(after.portal_closed_at, None);
}

async fn sqlx_set_user(t: &Booted, e: &Employee, user: i64) {
    renox::db::sql("UPDATE wl_hr_employees SET user_id = ? WHERE id = ?")
        .bind(user)
        .bind(e.id)
        .execute(t.db())
        .await
        .unwrap();
}

/// AC2: the *new* case still `in_progress`: the request is refused and no
/// approval request exists; the approval also re-checks.
#[renox::test]
async fn hrm05_ac2_a_new_case_still_in_progress_refuses_the_request() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    let case = t.visa_in(&e, "new", VisaCaseStatus::Open, |_| {}).await;

    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_status(422);
    assert!(
        ApprovalRequest::pending_about(t.db(), &e.subject())
            .await
            .unwrap()
            .is_empty()
    );

    // The case is cancelled: the request is accepted; it is then reopened
    // behind the machine's back, and the approval fails the request.
    renox::db::sql("UPDATE wl_hr_visa_cases SET status = 'cancelled' WHERE id = ?")
        .bind(case.id)
        .execute(t.db())
        .await
        .unwrap();
    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_ok();
    renox::db::sql("UPDATE wl_hr_visa_cases SET status = 'in_progress' WHERE id = ?")
        .bind(case.id)
        .execute(t.db())
        .await
        .unwrap();
    approve(&t, &md, &e, nov(10)).await;
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Onboarding);
}

/// AC2: a *new* case in `in_progress` (a step is done) or `on_hold` is live
/// as well: the request is refused with 422 and no approval request exists;
/// when the case is closed, the request is accepted.
#[renox::test]
async fn hrm05_ac2_a_new_case_in_progress_or_on_hold_refuses_the_request() {
    for live in [VisaCaseStatus::InProgress, VisaCaseStatus::OnHold] {
        let t = support::boot().await;
        let hr = t.user_with("Hana", "hr").await;
        let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
        let case = t.visa_in(&e, "new", live, |_| {}).await;

        let refused = ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")]).await;
        refused.assert_status(422);
        assert!(
            refused.text().contains("still open"),
            "{live:?}: {}",
            refused.text()
        );
        assert!(
            ApprovalRequest::pending_about(t.db(), &e.subject())
                .await
                .unwrap()
                .is_empty(),
            "{live:?}"
        );
        assert_eq!(t.employee(&e).await.status, EmployeeStatus::Onboarding);

        renox::db::sql("UPDATE wl_hr_visa_cases SET status = 'rejected' WHERE id = ?")
            .bind(case.id)
            .execute(t.db())
            .await
            .unwrap();
        ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
            .await
            .assert_ok();
    }
}

/// AC3: a work permit issued and no completed cancellation case and no
/// *cancellation not needed* record: refused; after the PRO records
/// *cancellation not needed* with a reason, it succeeds. A user without
/// `visas.manage` cannot record it.
#[renox::test]
async fn hrm05_ac3_an_issued_permit_needs_a_cancellation_or_the_pro_record() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let pro = t.user_with("Pia", "pro").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    let case = t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;
    t.step_done(&case, "work_permit").await;

    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_status(422);

    // HR cannot record it; a record without a reason is refused.
    let path = format!("/hr/visa-cases/{}/cancellation-not-needed", case.id);
    t.app.acting_as(&hr);
    t.app
        .post(&path, &[("reason", "permit expired unused")])
        .await
        .assert_forbidden();
    t.app.acting_as(&pro);
    t.app
        .post(&path, &[("reason", " ")])
        .await
        .assert_status(422);
    t.app
        .post(&path, &[("reason", "permit expired unused")])
        .await
        .assert_status(303);
    let recorded = t.case(&case).await;
    assert_eq!(recorded.cancellation_not_needed_by, Some(pro.id));
    assert_eq!(
        recorded.cancellation_not_needed_reason.as_deref(),
        Some("permit expired unused")
    );

    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_ok();
    approve(&t, &md, &e, nov(10)).await;
    assert_eq!(
        t.employee(&e).await.status,
        EmployeeStatus::OnboardingCancelled
    );
}

/// *Cancellation not needed* is for a new case whose permit was issued; a
/// completed cancellation case also lets the cancellation through.
#[renox::test]
async fn hrm05_the_record_needs_a_permit_and_a_completed_cancellation_also_serves() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let pro = t.user_with("Pia", "pro").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    let case = t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;
    let path = format!("/hr/visa-cases/{}/cancellation-not-needed", case.id);
    t.app.acting_as(&pro);
    t.app
        .post(&path, &[("reason", "nothing issued")])
        .await
        .assert_status(422);

    t.step_done(&case, "entry_permit").await;
    t.visa_in(&e, "cancellation", VisaCaseStatus::Completed, |_| {})
        .await;
    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_ok();
    approve(&t, &md, &e, nov(10)).await;
    assert_eq!(
        t.employee(&e).await.status,
        EmployeeStatus::OnboardingCancelled
    );
}

/// AC4: 3 days of attendance and no `paid` settlement: refused. After HR drafts
/// a settlement with the reason *onboarding cancelled* (AED 300 unpaid salary),
/// B approves and it is paid, the request succeeds.
#[renox::test]
async fn hrm05_ac4_days_worked_need_a_paid_onboarding_cancelled_settlement() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let approver = t.user_with("Basma", "settle_appr").await;
    let payer = t.user_with("Pia", "settle_pay").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;
    for d in 3..=5 {
        t.attendance.attended(&e, nov(d));
    }

    ask(&t, &hr, &e, nov(10), &[("reason", "medically unfit")])
        .await
        .assert_status(422);

    // A draft, and one for another reason, are not enough.
    let settlement = t
        .settlement_in(&e, SettlementStatus::Draft, |s| {
            s.reason = "onboarding_cancelled".into();
            s.unpaid_salary = 30_000;
            s.net = 30_000;
        })
        .await;
    ask(&t, &hr, &e, nov(10), &[("reason", "medically unfit")])
        .await
        .assert_status(422);

    t.settle_transition(&approver, &settlement, "approve")
        .await
        .assert_status(303);
    t.settle(
        &payer,
        &settlement,
        "pay",
        &[
            ("amount", "300"),
            ("method", "transfer"),
            ("reference", "TRF-3"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.settlement(&settlement).await.status,
        SettlementStatus::Paid
    );

    ask(&t, &hr, &e, nov(10), &[("reason", "medically unfit")])
        .await
        .assert_ok();
    approve(&t, &md, &e, nov(10)).await;
    assert_eq!(
        t.employee(&e).await.status,
        EmployeeStatus::OnboardingCancelled
    );
}

/// AC5: visa fees of AED 3,000 stay on E's cost object; no adjustment against E
/// and no receivable from E exists after the cancellation.
#[renox::test]
async fn hrm05_ac5_visa_fees_stay_on_the_employee_and_nothing_is_recovered() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;
    cost_poster(t.state())
        .post(
            t.state(),
            PostCost::new(e.subject(), "hr.visa_fee:1", e.subject(), 300_000),
        )
        .await
        .unwrap();

    ask(&t, &hr, &e, nov(10), &[("reason", "visa refused")])
        .await
        .assert_ok();
    approve(&t, &md, &e, nov(10)).await;

    assert_eq!(
        t.employee(&e).await.status,
        EmployeeStatus::OnboardingCancelled
    );
    let posted = t.costs.posted();
    assert_eq!(posted.len(), 1, "the fees are neither reversed nor moved");
    assert_eq!(posted[0].cost_object, e.subject());
    assert_eq!(posted[0].amount_aed, 300_000);
    assert_eq!(
        count_of::<Adjustment>(t.db(), Adjustment::EMPLOYEE_ID, e.id).await,
        0
    );
    assert_eq!(
        count_of::<Receivable>(t.db(), Receivable::EMPLOYEE_ID, e.id).await,
        0
    );
}

/// AC4 with the real wl-attendance: E has attendance records of 3, 4 and 5
/// November saved by the attendance service, and no paid settlement: the
/// request is refused; after the settlement (reason *onboarding cancelled*,
/// AED 300) is drafted, approved and paid, it succeeds.
#[renox::test]
async fn hrm05_ac4_three_days_of_real_attendance_need_a_paid_onboarding_cancelled_settlement() {
    let t = support::boot_with_attendance().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let approver = t.user_with("Basma", "settle_appr").await;
    let payer = t.user_with("Pia", "settle_pay").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;
    for d in 3..=5 {
        t.check_in(&e, nov(d)).await;
    }
    let history = wl_kernel::attendance::attendance_history(t.state())
        .attended_days(t.state(), e.employee_id(), nov(2), nov(5))
        .await
        .unwrap();
    assert_eq!(history, [nov(3), nov(4), nov(5)], "the days are recorded");

    ask(&t, &hr, &e, nov(10), &[("reason", "medically unfit")])
        .await
        .assert_status(422);
    assert!(
        ApprovalRequest::pending_about(t.db(), &e.subject())
            .await
            .unwrap()
            .is_empty()
    );

    let settlement = t
        .settlement_in(&e, SettlementStatus::Draft, |s| {
            s.reason = "onboarding_cancelled".into();
            s.unpaid_salary = 30_000;
            s.net = 30_000;
        })
        .await;
    t.settle_transition(&approver, &settlement, "approve")
        .await
        .assert_status(303);
    t.settle(
        &payer,
        &settlement,
        "pay",
        &[
            ("amount", "300"),
            ("method", "transfer"),
            ("reference", "TRF-3"),
        ],
    )
    .await
    .assert_status(303);

    ask(&t, &hr, &e, nov(10), &[("reason", "medically unfit")])
        .await
        .assert_ok();
    approve(&t, &md, &e, nov(10)).await;
    assert_eq!(
        t.employee(&e).await.status,
        EmployeeStatus::OnboardingCancelled
    );
}

/// AC4, checked again at the approval: E had no attendance when HR asked, so
/// the request was accepted; E then checks in (a record saved by the real
/// wl-attendance) before the MD approves, and the approval fails the request:
/// E stays in `onboarding` and the days worked need the paid settlement.
#[renox::test]
async fn hrm05_ac4_a_check_in_between_the_request_and_the_approval_fails_the_approval() {
    let t = support::boot_with_attendance().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Onboarding).await;
    t.visa_in(&e, "new", VisaCaseStatus::Rejected, |_| {}).await;

    ask(&t, &hr, &e, nov(10), &[("reason", "medically unfit")])
        .await
        .assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &e.subject())
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        request.payload.0["has_attendance"],
        renox::serde_json::json!(false),
        "no attendance at the request"
    );

    t.check_in(&e, nov(11)).await;

    let decided = t
        .on(nov(11), approvals::approve(t.state(), request.id, &md, ""))
        .await
        .unwrap()
        .expect("the request ends");
    assert_eq!(decided.decision, ApprovalEnd::Failed);
    let after = ApprovalRequest::find_or_404(t.db(), request.id)
        .await
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Failed);
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Onboarding);
    assert!(t.onboarding_cancelled.all().is_empty());
}
