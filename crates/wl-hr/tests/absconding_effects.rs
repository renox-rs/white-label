//! BL-HRM-30 (#480): the effects of `report_absconding` that need payroll, the
//! final settlement and the visa case (held pay, the draft settlement and the
//! *cancellation* case) run only when the BL-HRM-07 checks pass. The positive
//! Done-when points (the line held with *absence report*, the draft settlement
//! with reason *absconding*, the case from the *absconding* template, the
//! settlement kept from `notice`, the sent line held when the bank fails it)
//! are covered by `absconding.rs`; this file covers the refusals.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_hr::{
    AbsenceReport, Employee, EmployeeStatus, FinalSettlement, LineStatus, PayrollLine,
    PayrollRunStatus, VisaCase,
};
use wl_kernel::ApprovalEnd;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::workforce::AbsenceCaseClosed;

use support::{Booted, nov};

fn report() -> Vec<(&'static str, &'static str)> {
    vec![
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_number", "MOHRE-777"),
        ("report_date", "2026-11-09"),
    ]
}

async fn post(
    t: &Booted,
    day: NaiveDate,
    user: &User,
    employee: &Employee,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.on(day, async {
        t.app.acting_as(user);
        t.app
            .htmx()
            .post(&format!("/hr/employees/{}/exits", employee.id), fields)
            .await
    })
    .await
}

/// E is `active`, absent since 2026-11-01 with two failed attempts, and has a
/// line of AED 3,000 in the `calculated` November run.
async fn world() -> (Booted, User, User, Employee, PayrollLine) {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.open_case(&e, nov(4), nov(1)).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    let run = t.payroll_run("2026-11", PayrollRunStatus::Calculated).await;
    let line = t.payroll_line(&run, &e, 300_000, 300_000).await;
    (t, hr, md, e, line)
}

/// Nothing was held, drafted or opened for `e`.
async fn assert_untouched(t: &Booted, e: &Employee, line: &PayrollLine) {
    assert_eq!(t.line(line).await.status, LineStatus::Included);
    assert!(
        FinalSettlement::of_employee(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        VisaCase::cancellations_of(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        AbsenceReport::valid_for(t.db(), e.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(t.employee(e).await.status, EmployeeStatus::Active);
}

/// AC4: when BL-HRM-07 refuses the request (7 days are not more than 7, the
/// report number is missing), no line is held and no settlement or case is
/// created.
#[renox::test]
async fn hrm30_ac4_a_refused_request_holds_nothing_and_creates_nothing() {
    let (t, hr, _md, e, line) = world().await;

    post(&t, nov(8), &hr, &e, &report())
        .await
        .assert_status(422);
    let no_number = [
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_date", "2026-11-09"),
    ];
    post(&t, nov(9), &hr, &e, &no_number)
        .await
        .assert_status(422);

    assert_untouched(&t, &e, &line).await;
}

/// AC4: when the check fails at the final approval (the worker came back and
/// the vertical closed the case), the request ends `failed` and no line is
/// held and no settlement or case is created.
#[renox::test]
async fn hrm30_ac4_a_request_that_fails_at_the_approval_holds_nothing_either() {
    let (t, hr, md, e, line) = world().await;
    post(&t, nov(9), &hr, &e, &report()).await.assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &e.subject())
        .await
        .unwrap()
        .remove(0);
    for case in wl_hr::AbsenceCase::open_of(t.db(), e.id).await.unwrap() {
        t.state()
            .emit(AbsenceCaseClosed {
                employee: e.employee_id(),
                case: Subject::new(&case.case_type, case.case_id),
                closed_on: nov(9),
                reason: "the worker returned".into(),
            })
            .await
            .unwrap();
    }

    let decided = t
        .on(nov(9), approvals::approve(t.state(), request.id, &md, ""))
        .await
        .unwrap()
        .expect("an end");
    assert_eq!(decided.decision, ApprovalEnd::Failed);

    assert_untouched(&t, &e, &line).await;
}
