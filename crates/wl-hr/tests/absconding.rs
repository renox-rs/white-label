//! BL-HRM-07 (#197): `report_absconding`. After more than 7 consecutive days
//! of absence with the worker unreachable and a MoHRE absence report filed,
//! HR requests it and the MD approves it (`HRM.ACL.07`): the pay not yet sent
//! is held outside WPS, the portal and attendance are off, the final
//! settlement is drafted, a cancellation case opens and `EmployeeAbsconded`
//! is emitted.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_hr::payroll::{self, FailedLine};
use wl_hr::{
    AbsenceReport, Employee, EmployeeStatus, FinalSettlement, LeaveRequest, LeaveStatus,
    LineStatus, PayrollLine, PayrollRun, PayrollRunStatus, SettlementStatus, VisaCase,
    VisaCaseStatus, absconding,
};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::workforce::{AbsenceCaseClosed, workforce};
use wl_kernel::{ApprovalEnd, Subject};

use support::{Booted, nov};

/// The fields of a request with the MoHRE report "MOHRE-777" of 2026-11-09.
fn report() -> Vec<(&'static str, &'static str)> {
    vec![
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_number", "MOHRE-777"),
        ("report_date", "2026-11-09"),
    ]
}

/// `user` posts a request for `employee` on `day`.
async fn post(
    t: &Booted,
    day: NaiveDate,
    user: &User,
    employee: &Employee,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    // Sign in after the clock has moved: a session made earlier would be
    // older than its lifetime.
    t.on(day, async {
        t.app.acting_as(user);
        t.app
            .htmx()
            .post(&format!("/hr/employees/{}/exits", employee.id), fields)
            .await
    })
    .await
}

/// The request pending about `employee`.
async fn pending(t: &Booted, employee: &Employee) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &employee.subject())
        .await
        .unwrap()
}

/// The Done-when world: E is `active`, the last attendance was on 2026-11-01,
/// the vertical opened the absence case on 2026-11-04 and HR could not reach
/// E on 2026-11-04 and 2026-11-06.
struct World {
    t: Booted,
    hr: User,
    md: User,
    e: Employee,
}

async fn world() -> World {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.open_case(&e, nov(4), nov(1)).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    World { t, hr, md, e }
}

/// What the database says against `report_absconding` for `e` on `day`.
async fn problem(t: &Booted, e: &Employee, day: NaiveDate) -> Option<String> {
    let mut tx = t.db().begin().await.unwrap();
    absconding::problem_in(&mut tx, e.id, day, 7).await.unwrap()
}

/// A run of November in `status` with a line of AED 3,000 for each employee.
async fn november(
    t: &Booted,
    status: PayrollRunStatus,
    employees: &[&Employee],
) -> (PayrollRun, Vec<PayrollLine>) {
    let mut run = PayrollRun {
        period: "2026-11".into(),
        run_type: "regular".into(),
        status,
        ..PayrollRun::default()
    };
    let mut tx = t.db().begin().await.unwrap();
    run.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut lines = Vec::new();
    for employee in employees {
        lines.push(
            PayrollLine::create(
                t.db(),
                PayrollLine {
                    run_id: run.id,
                    employee_id: employee.id,
                    net_amount: 300_000,
                    ..PayrollLine::default()
                },
            )
            .await
            .unwrap(),
        );
    }
    (run, lines)
}

async fn line(t: &Booted, line: &PayrollLine) -> PayrollLine {
    PayrollLine::find_or_404(t.db(), line.id).await.unwrap()
}

async fn run_of(t: &Booted, run: &PayrollRun) -> PayrollRun {
    PayrollRun::find_or_404(t.db(), run.id).await.unwrap()
}

/// Requests on 2026-11-09 and has the MD approve it.
async fn report_and_approve(w: &World) {
    post(&w.t, nov(9), &w.hr, &w.e, &report()).await.assert_ok();
    let request = pending(&w.t, &w.e).await.remove(0);
    let decided =
        w.t.on(
            nov(9),
            approvals::approve(w.t.state(), request.id, &w.md, ""),
        )
        .await
        .unwrap()
        .expect("an end");
    assert_eq!(decided.decision, ApprovalEnd::Approved);
}

/// E `active`, the case open since the last attendance on 2026-11-01, contact
/// attempts on 2026-11-04 and 2026-11-06, today 2026-11-09 and the report
/// "MOHRE-777": when approved, E is `absconded`, its line in the `calculated`
/// November run is held with *absence report*, the portal and attendance are
/// disabled, a draft settlement and a *cancellation* case exist and
/// `EmployeeAbsconded` is emitted.
#[renox::test]
async fn hrm07_an_approved_report_holds_the_pay_drafts_the_settlement_and_opens_the_case() {
    let w = world().await;
    let other = w.t.employee_in(61, EmployeeStatus::Active).await;
    let (run, lines) = november(&w.t, PayrollRunStatus::Calculated, &[&w.e, &other]).await;

    report_and_approve(&w).await;

    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Absconded);
    let held = line(&w.t, &lines[0]).await;
    assert_eq!(held.status, LineStatus::Held);
    assert_eq!(held.held_reason.as_deref(), Some("absence report"));
    let untouched = line(&w.t, &lines[1]).await;
    assert_eq!(untouched.status, LineStatus::Included);
    assert_eq!(
        run_of(&w.t, &run).await.status,
        PayrollRunStatus::Calculated
    );

    // The report is on the employee, and the portal and attendance are off.
    let report = AbsenceReport::valid_for(w.t.db(), w.e.id)
        .await
        .unwrap()
        .expect("the report");
    assert_eq!(report.report_number, "MOHRE-777");
    assert_eq!(report.report_date, nov(9));
    assert!(report.access_disabled_at.is_some());
    let profile = workforce(w.t.state())
        .staff(w.t.state(), w.e.employee_id())
        .await
        .unwrap()
        .expect("the profile");
    assert!(!profile.listed, "an absconded worker is no longer listed");

    // A draft settlement and a cancellation case from the absconding variant.
    let settlements = FinalSettlement::of_employee(w.t.db(), w.e.id)
        .await
        .unwrap();
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].status, SettlementStatus::Draft);
    assert_eq!(settlements[0].reason, "absconding");
    assert_eq!(settlements[0].payee, "employee");
    let cases = VisaCase::cancellations_of(w.t.db(), w.e.id).await.unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].status, VisaCaseStatus::Open);
    assert_eq!(cases[0].variant, "absconding");
    assert_eq!(cases[0].report_number.as_deref(), Some("MOHRE-777"));

    // `EmployeeAbsconded` with the report number and date.
    let events = w.t.absconded.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, w.e.employee_id());
    assert_eq!(events[0].report_number, "MOHRE-777");
    assert_eq!(events[0].report_date, nov(9));
}

/// On 2026-11-08 (7 days) the request is refused; on 2026-11-09 (8 days) it
/// is allowed.
#[renox::test]
async fn hrm07_seven_days_are_not_enough_and_eight_are() {
    let w = world().await;
    post(&w.t, nov(8), &w.hr, &w.e, &report())
        .await
        .assert_status(422);
    assert!(pending(&w.t, &w.e).await.is_empty());
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Active);

    post(&w.t, nov(9), &w.hr, &w.e, &report()).await.assert_ok();
    assert_eq!(pending(&w.t, &w.e).await.len(), 1);
}

/// The day count and the contact attempts, without a clock: strictly more
/// than the setting, two failed attempts on different days after the last day
/// present.
#[renox::test]
async fn hrm07_the_database_checks_in_days_and_attempts() {
    let t = support::boot().await;
    macro_rules! check {
        ($e:expr, $day:expr) => {
            problem(&t, $e, $day)
        };
    }

    // One attempt only (2026-11-05): refused.
    let one = t.employee_in(61, EmployeeStatus::Active).await;
    t.open_case(&one, nov(4), nov(1)).await;
    t.attempt(&one, nov(5)).await;
    assert!(check!(&one, nov(9)).await.is_some());

    // Two attempts on the same day: refused. On two days: allowed.
    let same = t.employee_in(62, EmployeeStatus::Active).await;
    t.open_case(&same, nov(4), nov(1)).await;
    t.attempt(&same, nov(5)).await;
    t.attempt(&same, nov(5)).await;
    assert!(check!(&same, nov(9)).await.is_some());
    t.attempt(&same, nov(6)).await;
    assert_eq!(check!(&same, nov(9)).await, None);

    // 7 days (2026-11-08) are not more than 7; 8 are.
    assert!(check!(&same, nov(8)).await.is_some());
    assert_eq!(check!(&same, nov(9)).await, None);

    // An attempt that reached the worker, or came on or before the last day
    // present, does not count.
    let reached = t.employee_in(63, EmployeeStatus::Active).await;
    t.open_case(&reached, nov(4), nov(1)).await;
    t.attempt(&reached, nov(1)).await;
    t.attempt(&reached, nov(6)).await;
    wl_hr::ContactAttempt::create(
        t.db(),
        wl_hr::ContactAttempt {
            employee_id: reached.id,
            attempted_on: nov(7),
            method: "phone".into(),
            outcome: wl_hr::ContactOutcome::Reached,
            ..wl_hr::ContactAttempt::default()
        },
    )
    .await
    .unwrap();
    assert!(check!(&reached, nov(9)).await.is_some());
}

/// With the case opened late (2026-11-09) and the last attendance on
/// 2026-11-01, the request on 2026-11-09 is allowed: only that a case is open
/// matters, not its age.
#[renox::test]
async fn hrm07_a_case_opened_late_still_allows_it() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.open_case(&e, nov(9), nov(1)).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    post(&t, nov(9), &hr, &e, &report()).await.assert_ok();
    assert_eq!(pending(&t, &e).await.len(), 1);
}

/// No open absence case, or a closed one, refuses; so does approved leave
/// that keeps the last day present near.
#[renox::test]
async fn hrm07_it_needs_an_open_case_and_counts_approved_leave() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    // No case at all.
    post(&t, nov(9), &hr, &e, &report())
        .await
        .assert_status(422);

    t.open_case(&e, nov(4), nov(1)).await;
    // Approved leave until 2026-11-06 moves the last day present to the 6th:
    // three days on the 9th.
    LeaveRequest::create(
        t.db(),
        LeaveRequest {
            employee_id: e.id,
            leave_type: "annual".into(),
            from_date: nov(5),
            to_date: nov(6),
            status: LeaveStatus::Approved,
            ..LeaveRequest::default()
        },
    )
    .await
    .unwrap();
    post(&t, nov(9), &hr, &e, &report())
        .await
        .assert_status(422);
    assert!(pending(&t, &e).await.is_empty());
}

/// Without the report number, or without its date, it is refused.
#[renox::test]
async fn hrm07_it_needs_the_report_number_and_date() {
    let w = world().await;
    let no_number = [
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_date", "2026-11-09"),
    ];
    post(&w.t, nov(9), &w.hr, &w.e, &no_number)
        .await
        .assert_status(422);
    let no_date = [
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_number", "MOHRE-777"),
    ];
    post(&w.t, nov(9), &w.hr, &w.e, &no_date)
        .await
        .assert_status(422);
    assert!(pending(&w.t, &w.e).await.is_empty());
}

/// A line in a run that was `submitted` stays in the sent SIF file; when the
/// bank fails it, it is held with *absence report*, and `regenerate` leaves it
/// out.
#[renox::test]
async fn hrm07_a_line_already_sent_stays_and_is_held_if_the_bank_fails_it() {
    let w = world().await;
    let other = w.t.employee_in(61, EmployeeStatus::Active).await;
    let (run, lines) = november(&w.t, PayrollRunStatus::Approved, &[&w.e, &other]).await;
    payroll::submit(w.t.state(), run.id).await.unwrap();
    assert_eq!(run_of(&w.t, &run).await.status, PayrollRunStatus::Submitted);

    report_and_approve(&w).await;

    // It stays in the file already sent.
    let sent = line(&w.t, &lines[0]).await;
    assert_eq!(sent.status, LineStatus::Included);
    assert_eq!(sent.sif_version, Some(1));
    assert_eq!(sent.held_reason, None);

    // The bank fails both lines: E's is held, not failed; the other fails.
    payroll::record_partial(
        w.t.state(),
        run.id,
        vec![
            FailedLine {
                line_id: lines[0].id,
                reason: "account closed".into(),
            },
            FailedLine {
                line_id: lines[1].id,
                reason: "invalid IBAN".into(),
            },
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        run_of(&w.t, &run).await.status,
        PayrollRunStatus::PartiallyRejected
    );
    let held = line(&w.t, &lines[0]).await;
    assert_eq!(held.status, LineStatus::Held);
    assert_eq!(held.held_reason.as_deref(), Some("absence report"));
    assert_eq!(line(&w.t, &lines[1]).await.status, LineStatus::Failed);

    // A failed line must be fixed or moved out first; then `regenerate`
    // makes a second SIF version with the fixed line only.
    assert!(payroll::regenerate(w.t.state(), run.id).await.is_err());
    payroll::fix_line(w.t.db(), lines[1].id).await.unwrap();
    let again = payroll::regenerate(w.t.state(), run.id).await.unwrap();
    assert_eq!(again.status, PayrollRunStatus::Submitted);
    assert_eq!(again.sif_version, 2);
    let fixed = line(&w.t, &lines[1]).await;
    assert_eq!(fixed.sif_version, Some(2));
    let left_out = line(&w.t, &lines[0]).await;
    assert_eq!(left_out.status, LineStatus::Held);
    assert_ne!(left_out.sif_version, Some(2), "left out of the new file");
}

/// A line held for the absence report is left out of the SIF file of
/// `submit`.
#[renox::test]
async fn hrm07_a_held_line_is_left_out_of_a_new_sif_file() {
    let w = world().await;
    let other = w.t.employee_in(61, EmployeeStatus::Active).await;
    let (run, lines) = november(&w.t, PayrollRunStatus::Approved, &[&w.e, &other]).await;
    report_and_approve(&w).await;
    assert_eq!(line(&w.t, &lines[0]).await.status, LineStatus::Held);

    payroll::submit(w.t.state(), run.id).await.unwrap();
    assert_eq!(line(&w.t, &lines[0]).await.sif_version, None);
    assert_eq!(line(&w.t, &lines[1]).await.sif_version, Some(1));
}

/// From `notice`, the settlement the notice drafted gets the reason
/// *absconding* and no second one is drafted; the open cancellation case is
/// reused.
#[renox::test]
async fn hrm07_from_notice_the_existing_settlement_and_case_are_kept() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Notice).await;
    t.open_case(&e, nov(4), nov(1)).await;
    t.attempt(&e, nov(4)).await;
    t.attempt(&e, nov(6)).await;
    let drafted = FinalSettlement::create(
        t.db(),
        FinalSettlement {
            employee_id: e.id,
            reason: "resignation".into(),
            payee: "employee".into(),
            ..FinalSettlement::default()
        },
    )
    .await
    .unwrap();
    let case = VisaCase::create(
        t.db(),
        VisaCase {
            employee_id: e.id,
            case_type: "cancellation".into(),
            variant: "standard".into(),
            ..VisaCase::default()
        },
    )
    .await
    .unwrap();

    post(&t, nov(9), &hr, &e, &report()).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);
    t.on(nov(9), approvals::approve(t.state(), request.id, &md, ""))
        .await
        .unwrap();

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Absconded);
    let settlements = FinalSettlement::of_employee(t.db(), e.id).await.unwrap();
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].id, drafted.id);
    assert_eq!(settlements[0].reason, "absconding");
    let cases = VisaCase::cancellations_of(t.db(), e.id).await.unwrap();
    assert_eq!(cases.len(), 1);
    assert_eq!(cases[0].id, case.id);
    assert_eq!(cases[0].variant, "absconding");
}

/// The checks run again at the final approval: when the case was closed in
/// between (the worker came back), the request ends `failed` and nothing runs.
#[renox::test]
async fn hrm07_the_checks_run_again_at_the_final_approval() {
    let w = world().await;
    let (_run, lines) = november(&w.t, PayrollRunStatus::Calculated, &[&w.e]).await;
    post(&w.t, nov(9), &w.hr, &w.e, &report()).await.assert_ok();
    let request = pending(&w.t, &w.e).await.remove(0);

    // The vertical closes the case: the worker returned.
    for case in wl_hr::AbsenceCase::open_of(w.t.db(), w.e.id).await.unwrap() {
        w.t.state()
            .emit(AbsenceCaseClosed {
                employee: w.e.employee_id(),
                case: Subject::new(&case.case_type, case.case_id),
                closed_on: nov(9),
                reason: "the worker returned".into(),
            })
            .await
            .unwrap();
    }
    let decided =
        w.t.on(
            nov(9),
            approvals::approve(w.t.state(), request.id, &w.md, ""),
        )
        .await
        .unwrap()
        .expect("an end");
    assert_eq!(decided.decision, ApprovalEnd::Failed);
    let after = ApprovalRequest::find_or_404(w.t.db(), request.id)
        .await
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Failed);
    assert_eq!(w.t.employee(&w.e).await.status, EmployeeStatus::Active);
    assert_eq!(line(&w.t, &lines[0]).await.status, LineStatus::Included);
    assert!(
        AbsenceReport::valid_for(w.t.db(), w.e.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(w.t.absconded.all().is_empty());
}

/// HR records contact attempts on the employee page; they need a method, no
/// future day, and the right to manage.
#[renox::test]
async fn hrm07_hr_records_the_contact_attempts_on_the_employee() {
    let t = support::boot().await;
    let hr = t.user_with("Hana", "hr").await;
    let viewer = t.user_with("Vera", "manager").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    let path = format!("/hr/employees/{}/contact-attempts", e.id);

    t.app.acting_as(&hr);
    t.app
        .htmx()
        .post(&path, &[("method", "  "), ("attempted_on", "2026-01-05")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &path,
            &[("method", "phone"), ("attempted_on", "2099-01-01")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &path,
            &[
                ("method", "phone"),
                ("attempted_on", "2026-01-05"),
                ("outcome", "no_answer"),
                ("note", "rang six times"),
            ],
        )
        .await
        .assert_ok();
    let attempts = wl_hr::ContactAttempt::of_employee(t.db(), e.id)
        .await
        .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].method, "phone");
    assert_eq!(attempts[0].outcome, wl_hr::ContactOutcome::NoAnswer);
    assert_eq!(attempts[0].by_user_id, Some(hr.id));
    assert_eq!(attempts[0].note.as_deref(), Some("rang six times"));

    t.app
        .get(&format!("/hr/employees/{}", e.id))
        .await
        .assert_ok()
        .assert_see("Attempts to reach the worker")
        .assert_see("rang six times");

    t.app.acting_as(&viewer);
    t.app
        .htmx()
        .post(
            &path,
            &[("method", "phone"), ("attempted_on", "2026-01-06")],
        )
        .await
        .assert_forbidden();
    // Somebody who is not working has nobody to reach.
    let gone = t.employee_in(61, EmployeeStatus::Exited).await;
    t.app.acting_as(&hr);
    t.app
        .htmx()
        .post(
            &format!("/hr/employees/{}/contact-attempts", gone.id),
            &[("method", "phone")],
        )
        .await
        .assert_status(422);
}

/// The vertical's events are kept: a case opened twice is one, and closing it
/// ends it.
#[renox::test]
async fn hrm07_absence_cases_of_the_verticals_are_kept_once() {
    let t = support::boot().await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    let case = Subject::new("ticketing.ticket", 5);
    let opened = wl_kernel::workforce::AbsenceCaseOpened {
        employee: e.employee_id(),
        case: case.clone(),
        opened_on: nov(4),
        last_present_on: nov(1),
    };
    t.state().emit(opened.clone()).await.unwrap();
    t.state().emit(opened).await.unwrap();
    assert_eq!(
        wl_hr::AbsenceCase::open_of(t.db(), e.id)
            .await
            .unwrap()
            .len(),
        1
    );
    t.state()
        .emit(AbsenceCaseClosed {
            employee: e.employee_id(),
            case,
            closed_on: nov(10),
            reason: "returned".into(),
        })
        .await
        .unwrap();
    assert!(
        wl_hr::AbsenceCase::open_of(t.db(), e.id)
            .await
            .unwrap()
            .is_empty()
    );
}
