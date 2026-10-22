//! BL-HRM-18 (#208), `HRM.RUL.12`: a held line is released when its reason
//! clears (wl-hr §3 *Payroll line*, §4 *Run types*, §5.6 step 2): a draft
//! off-cycle run is proposed for the lines of a paid run, carrying a line into
//! the next regular run needs a reason and warns when it pays after the WPS
//! due date (not for an absence-report hold), and a line is released once.

mod support;

use renox::prelude::*;
use wl_hr::release;
use wl_hr::{Employee, EmployeeStatus, LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use wl_kernel::Subject;
use wl_kernel::workforce::AbsenceCaseClosed;

use support::{Booted, nov};

/// A wage of AED 3,000 in fils.
const WAGE: i64 = 300_000;

/// A preparer and an approver, employees E and F, and the paid November run.
struct Desk {
    t: Booted,
    prep: User,
    appr: User,
    e: Employee,
    f: Employee,
    november: PayrollRun,
}

async fn desk() -> Desk {
    let t = support::boot().await;
    let prep = t.user_with("Pia", "payroll_prep").await;
    let appr = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(91, EmployeeStatus::Active).await;
    let f = t.employee_in(92, EmployeeStatus::Active).await;
    let november = t.payroll_run("2026-11", PayrollRunStatus::Paid).await;
    Desk {
        t,
        prep,
        appr,
        e,
        f,
        november,
    }
}

impl Desk {
    /// The November pay of `employee` in `run`, held for `reason`.
    async fn held(&self, run: &PayrollRun, employee: &Employee, reason: &str) -> PayrollLine {
        let line = self.t.payroll_line(run, employee, WAGE, WAGE).await;
        PayrollLine::query()
            .where_(PayrollLine::ID.eq(line.id))
            .update(
                self.t.db(),
                &[
                    ("status", &LineStatus::Held.as_str().to_owned()),
                    ("held_reason", &Some(reason.to_owned())),
                ],
            )
            .await
            .unwrap();
        self.t.line(&line).await
    }

    /// The absence case of `employee` was closed on `day`.
    async fn close_case(&self, employee: &Employee, case: i64, day: renox::chrono::NaiveDate) {
        self.t
            .state()
            .emit(AbsenceCaseClosed {
                employee: employee.employee_id(),
                case: Subject::new("ticketing.ticket", case),
                closed_on: day,
                reason: "returned".into(),
            })
            .await
            .unwrap();
    }

    /// The preparer carries `line` into the regular `run` with `reason`.
    async fn carry(
        &self,
        run: &PayrollRun,
        line: &PayrollLine,
        reason: &str,
    ) -> renox::testing::TestResponse {
        self.t.app.acting_as(&self.prep);
        self.t
            .app
            .post(
                &format!("/hr/payroll/{}/carry/{}", run.id, line.id),
                &[("reason", reason)],
            )
            .await
    }

    async fn runs_of_type(&self, run_type: &str) -> Vec<PayrollRun> {
        PayrollRun::query()
            .where_(PayrollRun::RUN_TYPE.eq(run_type))
            .order_by(PayrollRun::ID)
            .get(self.t.db())
            .await
            .unwrap()
    }
}

fn dec(day: u32) -> renox::chrono::NaiveDate {
    renox::chrono::NaiveDate::from_ymd_opt(2026, 12, day).unwrap()
}

/// AC1: E's November line is held in the paid run R and the absence case is
/// closed on 2026-12-03: a draft off-cycle run with E's line is proposed and
/// the payroll officer is told. R is not edited.
#[renox::test]
async fn hrm18_ac1_a_closed_absence_case_proposes_a_draft_off_cycle_run() {
    let d = desk().await;
    d.t.open_case(&d.e, nov(10), nov(1)).await;
    let held = d.held(&d.november, &d.e, "absence case").await;

    d.close_case(&d.e, 1, dec(3)).await;

    let off = d.runs_of_type("off_cycle").await;
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].status, PayrollRunStatus::Draft);
    assert_eq!(off[0].period, "2026-11");
    let lines = off[0].lines(d.t.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].employee_id, d.e.id);
    assert_eq!(lines[0].net_amount, WAGE);
    assert_eq!(lines[0].status, LineStatus::Included);
    assert_eq!(lines[0].source_line_id, Some(held.id));
    // The paid run keeps its line; only the release is recorded on it.
    let held = d.t.line(&held).await;
    assert_eq!(held.status, LineStatus::Held);
    assert_eq!(held.net_amount, WAGE);
    assert_eq!(held.released_into, Some(off[0].id));
    assert_eq!(d.t.run(&d.november).await.status, PayrollRunStatus::Paid);
    // Proposed, not calculated or submitted.
    assert_eq!(off[0].sif_version, 0);
    assert_eq!(off[0].prepared_by, None);
    // The payroll officer is told.
    let told = d.t.notices.of_type("hr.payroll_released");
    assert_eq!(told.len(), 1);
    assert_eq!(support::users(&told[0]), vec![d.prep.id]);
}

/// One draft off-cycle run per entity collects the released lines, and a line
/// is added once however often the rule runs.
#[renox::test]
async fn hrm18_one_draft_off_cycle_run_collects_the_lines_and_a_line_is_added_once() {
    let d = desk().await;
    d.t.open_case(&d.e, nov(10), nov(1)).await;
    d.t.open_case(&d.f, nov(11), nov(2)).await;
    d.held(&d.november, &d.e, "absence case").await;
    d.held(&d.november, &d.f, "absence case").await;

    d.close_case(&d.e, 1, dec(3)).await;
    d.close_case(&d.f, 2, dec(4)).await;
    d.t.app.run_scheduled(release::TASK).await.unwrap();
    d.t.app.run_scheduled(release::TASK).await.unwrap();

    let off = d.runs_of_type("off_cycle").await;
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].lines(d.t.db()).await.unwrap().len(), 2);
}

/// A hold by a valid absence report is not cleared by the case closing; it is
/// cleared when the report is withdrawn.
#[renox::test]
async fn hrm18_an_absence_report_hold_clears_when_the_report_is_withdrawn() {
    let d = desk().await;
    d.t.open_case(&d.e, nov(10), nov(1)).await;
    let held = d
        .held(&d.november, &d.e, wl_hr::payroll::HELD_ABSENCE_REPORT)
        .await;

    d.close_case(&d.e, 1, dec(3)).await;
    assert_eq!(d.t.line(&held).await.hold_cleared_on, None);
    assert!(d.runs_of_type("off_cycle").await.is_empty());

    let cleared = release::report_withdrawn(d.t.state(), d.e.id)
        .await
        .unwrap();

    assert_eq!(cleared.waiting, 1);
    assert_eq!(d.runs_of_type("off_cycle").await.len(), 1);
    assert!(d.t.line(&held).await.released_into.is_some());
}

/// AC1, end to end: the held line is not an UPDATE of the fixture but what
/// `calculate` makes of an open absence case; the run goes through approve,
/// submit and the bank's confirmation, and closing the case on 2026-12-03
/// proposes the off-cycle run with E's line.
#[renox::test]
async fn hrm18_ac1_the_line_held_by_calculate_is_proposed_when_the_case_is_closed() {
    let t = support::boot().await;
    let prep = t.user_with("Pia", "payroll_prep").await;
    let appr = t.user_with("Ali", "payroll_appr").await;
    let e = t.employee_in(91, EmployeeStatus::Active).await;
    let f = t.employee_in(92, EmployeeStatus::Active).await;
    for (employee, timesheet) in [(&e, 3), (&f, 4)] {
        t.salary(employee, WAGE, 1_500).await;
        t.timesheets.give(support::hours_of(
            employee,
            timesheet,
            false,
            &[(2, "normal", 9_600)],
        ));
    }
    t.open_case(&e, nov(10), nov(1)).await;
    let november = t.payroll_run("2026-11", PayrollRunStatus::Draft).await;

    t.transition(&prep, &november, "calculate", "")
        .await
        .assert_status(303);
    let held = t.line_of(&november, &e).await;
    assert_eq!(held.status, LineStatus::Held);
    assert_eq!(held.held_reason.as_deref(), Some("absence case"));
    t.transition(&appr, &november, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(t.state(), november.id)
        .await
        .unwrap();
    t.confirm(&appr, &november, "BANK-1")
        .await
        .assert_status(303);
    assert_eq!(t.run(&november).await.status, PayrollRunStatus::Paid);
    assert_eq!(t.line_of(&november, &f).await.status, LineStatus::Paid);
    assert_eq!(t.line(&held).await.status, LineStatus::Held);

    t.state()
        .emit(AbsenceCaseClosed {
            employee: e.employee_id(),
            case: Subject::new("ticketing.ticket", 1),
            closed_on: dec(3),
            reason: "returned".into(),
        })
        .await
        .unwrap();

    let off = PayrollRun::query()
        .where_(PayrollRun::RUN_TYPE.eq("off_cycle"))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].status, PayrollRunStatus::Draft);
    let lines = off[0].lines(t.db()).await.unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].employee_id, e.id);
    assert_eq!(lines[0].source_line_id, Some(held.id));
    assert_eq!(t.line(&held).await.released_into, Some(off[0].id));
}

/// AC2: carrying E's line into the December regular run asks for a reason and
/// warns "pays after the WPS due date 2026-12-01" for a closed-account hold,
/// but not for an absence-report hold.
#[renox::test]
async fn hrm18_ac2_carrying_a_line_needs_a_reason_and_warns_except_for_an_absence_report() {
    let d = desk().await;
    let account = d.held(&d.november, &d.e, "closed account").await;
    let report = d
        .held(&d.november, &d.f, wl_hr::payroll::HELD_ABSENCE_REPORT)
        .await;
    release::line_fixed(d.t.state(), account.id).await.unwrap();
    release::report_withdrawn(d.t.state(), d.f.id)
        .await
        .unwrap();
    let december = d.t.payroll_run("2026-12", PayrollRunStatus::Draft).await;

    // The page offers both lines and warns for the closed account only.
    d.t.app.acting_as(&d.prep);
    d.t.app
        .get(&format!("/hr/payroll/{}", december.id))
        .await
        .assert_ok()
        .assert_see("Held lines that can be carried here")
        .assert_see("pays after the WPS due date 2026-12-01");

    d.carry(&december, &account, "  ").await.assert_status(422);
    assert!(december.lines(d.t.db()).await.unwrap().is_empty());
    d.carry(&december, &account, "worker asked to wait")
        .await
        .assert_status(303);
    d.carry(&december, &report, "worker is back")
        .await
        .assert_status(303);

    let lines = december.lines(d.t.db()).await.unwrap();
    assert_eq!(lines.len(), 2);
    let carried_account = lines.iter().find(|l| l.employee_id == d.e.id).unwrap();
    assert_eq!(carried_account.source_line_id, Some(account.id));
    assert_eq!(
        carried_account.carry_reason.as_deref(),
        Some("worker asked to wait")
    );
    assert_eq!(
        carried_account.late_warning.as_deref(),
        Some("This pays after the WPS due date 2026-12-01.")
    );
    let carried_report = lines.iter().find(|l| l.employee_id == d.f.id).unwrap();
    assert_eq!(carried_report.late_warning, None);
    // Both held lines now point at the regular run; the off-cycle run lost
    // them.
    assert_eq!(d.t.line(&account).await.released_into, Some(december.id));
    assert_eq!(d.t.line(&report).await.released_into, Some(december.id));
    let off = d.runs_of_type("off_cycle").await;
    assert!(off[0].lines(d.t.db()).await.unwrap().is_empty());
    // And the run page shows the warning on the carried line.
    d.t.app.acting_as(&d.prep);
    d.t.app
        .get(&format!("/hr/payroll/{}", december.id))
        .await
        .assert_ok()
        .assert_see("worker asked to wait");
}

/// AC3: after the off-cycle run is paid, E's line has `released_into` = that
/// run, and it cannot be added to another run.
#[renox::test]
async fn hrm18_ac3_a_paid_off_cycle_run_is_where_the_line_was_released_and_it_is_released_once() {
    let d = desk().await;
    d.t.open_case(&d.e, nov(10), nov(1)).await;
    let held = d.held(&d.november, &d.e, "absence case").await;
    d.close_case(&d.e, 1, dec(3)).await;
    let off = d.runs_of_type("off_cycle").await.remove(0);

    // The same approval as any run, with its own SIF file.
    d.t.transition(&d.prep, &off, "calculate", "")
        .await
        .assert_status(303);
    d.t.transition(&d.appr, &off, "approve", "")
        .await
        .assert_status(303);
    wl_hr::payroll::submit(d.t.state(), off.id).await.unwrap();
    assert_eq!(d.t.run(&off).await.sif_version, 1);
    d.t.confirm(&d.appr, &off, "BANK-1")
        .await
        .assert_status(303);

    assert_eq!(d.t.run(&off).await.status, PayrollRunStatus::Paid);
    assert_eq!(d.t.line(&held).await.released_into, Some(off.id));
    let paid = d.t.line_of(&off, &d.e).await;
    assert_eq!(paid.status, LineStatus::Paid);
    assert!(paid.payslip_published_at.is_some());

    // It cannot be added again: not by the rule, not by carrying it.
    d.t.app.run_scheduled(release::TASK).await.unwrap();
    assert_eq!(d.runs_of_type("off_cycle").await.len(), 1);
    let december = d.t.payroll_run("2026-12", PayrollRunStatus::Draft).await;
    d.carry(&december, &held, "again").await.assert_status(422);
    assert!(december.lines(d.t.db()).await.unwrap().is_empty());
}

/// When the reason clears before the run is sent, the line is `included`
/// again in that run and no off-cycle run is proposed.
#[renox::test]
async fn hrm18_a_reason_cleared_before_the_run_is_sent_unholds_the_line_in_it() {
    let d = desk().await;
    d.t.open_case(&d.e, nov(10), nov(1)).await;
    let run = d.t.payroll_run("2026-12", PayrollRunStatus::Approved).await;
    let held = d.held(&run, &d.e, "absence case").await;

    d.close_case(&d.e, 1, dec(3)).await;

    let line = d.t.line(&held).await;
    assert_eq!(line.status, LineStatus::Included);
    assert_eq!(line.held_reason, None);
    assert!(d.runs_of_type("off_cycle").await.is_empty());
}

/// When the run is sent but not paid, the line waits; it is proposed once the
/// run is `paid`.
#[renox::test]
async fn hrm18_a_reason_cleared_while_the_run_is_sent_waits_until_it_is_paid() {
    let d = desk().await;
    d.t.open_case(&d.e, nov(10), nov(1)).await;
    let run =
        d.t.payroll_run("2026-12", PayrollRunStatus::Submitted)
            .await;
    let held = d.held(&run, &d.e, "absence case").await;

    d.close_case(&d.e, 1, dec(3)).await;

    let line = d.t.line(&held).await;
    assert_eq!(line.status, LineStatus::Held);
    assert!(line.hold_cleared_on.is_some());
    assert!(d.runs_of_type("off_cycle").await.is_empty());

    d.t.confirm(&d.appr, &run, "").await.assert_status(303);

    let off = d.runs_of_type("off_cycle").await;
    assert_eq!(off.len(), 1);
    assert_eq!(off[0].period, "2026-12");
    assert_eq!(off[0].lines(d.t.db()).await.unwrap().len(), 1);
    assert_eq!(d.t.line(&held).await.released_into, Some(off[0].id));
}

/// Only a held line whose reason cleared can be released or carried.
#[renox::test]
async fn hrm18_a_line_that_is_still_held_or_not_held_is_refused() {
    let d = desk().await;
    let still_held = d.held(&d.november, &d.e, "absence case").await;
    let included = d.t.payroll_line(&d.november, &d.f, WAGE, WAGE).await;
    let december = d.t.payroll_run("2026-12", PayrollRunStatus::Draft).await;

    assert!(release::line_fixed(d.t.state(), included.id).await.is_err());
    d.carry(&december, &still_held, "wants it")
        .await
        .assert_status(422);
    assert!(d.runs_of_type("off_cycle").await.is_empty());
}
