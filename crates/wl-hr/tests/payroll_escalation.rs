//! BL-HRM-14 (#204), `HRM.RUL.11`: a rejected or partly rejected run gives the
//! preparer a task per failed line and tells the approver; one that is still
//! not resubmitted (`rejected`) or not regenerated or confirmed paid
//! (`partially_rejected`) at 00:00 Asia/Dubai one day before the WPS due date
//! (the 1st of the following month) is escalated once to the MD and the
//! accountant, at once when the rejection is recorded after that point.

mod support;

use renox::prelude::*;
use wl_hr::payroll;
use wl_hr::wps_alerts;
use wl_hr::{Employee, EmployeeStatus, LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
use wl_kernel::tickets::Assignee;

use support::{Booted, nov, users};

const NET: i64 = 300_000;
const ESCALATED: &str = "hr.payroll_escalated";

/// A prepares, B approves; M is the MD and K the accountant. The November run
/// of E and F was sent before the clock moves.
struct World {
    t: Booted,
    a: User,
    b: User,
    md: User,
    acct: User,
    run: PayrollRun,
    lines: Vec<PayrollLine>,
}

async fn world() -> World {
    let t = support::boot().await;
    let a = t.user_with("Pia", "payroll_prep").await;
    let b = t.user_with("Ali", "payroll_appr").await;
    let md = t.user_with("Basel", "owner").await;
    let acct = t.user_with("Kim", "accountant").await;
    let e: Employee = t.employee_in(61, EmployeeStatus::Active).await;
    let f: Employee = t.employee_in(62, EmployeeStatus::Active).await;
    let (run, lines) = t.sent_run(&a, &b, "2026-11", &[e, f], NET).await;
    World {
        t,
        a,
        b,
        md,
        acct,
        run,
        lines,
    }
}

/// The clock to `day`, then the daily job of `HRM.RUL.11`.
async fn job_on(w: &World, day: renox::chrono::NaiveDate) {
    w.t.on(day, async {}).await;
    w.t.app.run_scheduled(wps_alerts::TASK).await.unwrap();
}

/// The preparer records a rejection on `day`.
async fn reject_on(w: &World, day: renox::chrono::NaiveDate) {
    w.t.on(day, w.t.reject(&w.a, &w.run, "funding", "BANK-REJ-1"))
        .await
        .assert_status(303);
}

fn told_md_and_accountant(w: &World) -> Vec<i64> {
    let mut ids = vec![w.md.id, w.acct.id];
    ids.sort_unstable();
    ids
}

/// AC1: a November run `rejected` on 2026-11-28: the preparer has one task per
/// line and the approver is notified. Nothing is escalated yet.
#[renox::test]
async fn hrm14_ac1_a_rejection_gives_a_task_per_line_and_tells_the_approver() {
    let w = world().await;

    reject_on(&w, nov(28)).await;

    let tasks = w.t.tasks.with_key("hr.payroll_line_failed:");
    assert_eq!(tasks.len(), 2, "one task per line");
    assert!(
        tasks
            .iter()
            .all(|t| t.assignee == Some(Assignee::user(w.a.id)))
    );
    let keys: Vec<_> = tasks.iter().filter_map(|t| t.source_key.clone()).collect();
    assert!(keys.contains(&wps_alerts::task_key(w.run.id, w.lines[0].id, 1)));
    assert!(keys.contains(&wps_alerts::task_key(w.run.id, w.lines[1].id, 1)));
    let told = w.t.notices.of_type("hr.payroll_failed");
    assert_eq!(told.len(), 1);
    assert!(
        users(&told[0]).contains(&w.b.id),
        "the approver is notified"
    );
    assert!(w.t.notices.of_type(ESCALATED).is_empty());

    // The day before the escalation day the job does nothing.
    job_on(&w, nov(29)).await;
    assert!(w.t.notices.of_type(ESCALATED).is_empty());
}

/// AC2: still `rejected` on 2026-11-30 (due 2026-12-01): the MD and the
/// accountant are notified, once, however often the job runs.
#[renox::test]
async fn hrm14_ac2_a_rejection_still_open_the_day_before_the_due_date_is_escalated_once() {
    let w = world().await;
    reject_on(&w, nov(28)).await;

    job_on(&w, nov(30)).await;
    job_on(&w, nov(30)).await;

    let escalated = w.t.notices.of_type(ESCALATED);
    assert_eq!(escalated.len(), 1);
    assert_eq!(users(&escalated[0]), told_md_and_accountant(&w));
    assert_eq!(escalated[0].data["period"], "2026-11");
    assert_eq!(escalated[0].data["count"], 2);
    assert_eq!(
        w.t.run(&w.run).await.escalated_on,
        Some(nov(30)),
        "the day it was escalated"
    );
    job_on(
        &w,
        renox::chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap(),
    )
    .await;
    assert_eq!(w.t.notices.of_type(ESCALATED).len(), 1, "once per run");
}

/// AC3: resubmitted on 2026-11-29: no escalation on the 30th.
#[renox::test]
async fn hrm14_ac3_a_run_resubmitted_in_time_is_not_escalated() {
    let w = world().await;
    reject_on(&w, nov(28)).await;

    w.t.on(nov(29), w.t.resubmit(&w.a, &w.run, "Account topped up"))
        .await
        .assert_status(303);
    assert_eq!(w.t.run(&w.run).await.status, PayrollRunStatus::Submitted);
    job_on(&w, nov(30)).await;

    assert!(w.t.notices.of_type(ESCALATED).is_empty());
    assert_eq!(w.t.run(&w.run).await.escalated_on, None);
}

/// AC4, regenerated: `partially_rejected` on 2026-11-28 and regenerated on the
/// 29th: no escalation.
#[renox::test]
async fn hrm14_ac4_a_partly_rejected_run_regenerated_in_time_is_not_escalated() {
    let w = world().await;
    w.t.on(
        nov(28),
        w.t.partial(&w.a, &w.run, "RESULT-1", &[(&w.lines[0], "invalid IBAN")]),
    )
    .await
    .assert_status(303);
    assert_eq!(w.t.tasks.with_key("hr.payroll_line_failed:").len(), 1);

    payroll::fix_line(w.t.db(), w.lines[0].id).await.unwrap();
    w.t.on(nov(29), w.t.transition(&w.a, &w.run, "regenerate", ""))
        .await
        .assert_status(303);
    job_on(&w, nov(30)).await;

    assert!(w.t.notices.of_type(ESCALATED).is_empty());
}

/// AC4, confirmed paid with the failed line held on the 29th: no escalation.
#[renox::test]
async fn hrm14_ac4_a_partly_rejected_run_paid_with_the_failed_line_held_is_not_escalated() {
    let w = world().await;
    w.t.on(
        nov(28),
        w.t.partial(&w.a, &w.run, "RESULT-1", &[(&w.lines[0], "account closed")]),
    )
    .await
    .assert_status(303);

    w.t.on(
        nov(29),
        w.t.line_action(&w.a, &w.lines[0], "hold", &[("reason", "account closed")]),
    )
    .await
    .assert_status(303);
    w.t.on(nov(29), w.t.confirm(&w.b, &w.run, ""))
        .await
        .assert_status(303);
    assert_eq!(w.t.run(&w.run).await.status, PayrollRunStatus::Paid);
    assert_eq!(w.t.line(&w.lines[0]).await.status, LineStatus::Held);
    job_on(&w, nov(30)).await;

    assert!(w.t.notices.of_type(ESCALATED).is_empty());
}

/// AC4, still `partially_rejected` on the 30th: the MD and the accountant are
/// told once.
#[renox::test]
async fn hrm14_ac4_a_partly_rejected_run_still_open_is_escalated_once() {
    let w = world().await;
    w.t.on(
        nov(28),
        w.t.partial(&w.a, &w.run, "RESULT-1", &[(&w.lines[0], "invalid IBAN")]),
    )
    .await
    .assert_status(303);

    job_on(&w, nov(29)).await;
    assert!(w.t.notices.of_type(ESCALATED).is_empty());
    job_on(&w, nov(30)).await;
    job_on(&w, nov(30)).await;

    let escalated = w.t.notices.of_type(ESCALATED);
    assert_eq!(escalated.len(), 1);
    assert_eq!(users(&escalated[0]), told_md_and_accountant(&w));
    assert_eq!(escalated[0].data["status"], "partially_rejected");
    assert_eq!(escalated[0].data["count"], 1);
}

/// A rejection recorded after the escalation day (here on the 30th) is
/// escalated at once; the job later does not tell them again, and a second
/// rejection of the same run is not escalated again.
#[renox::test]
async fn hrm14_a_rejection_recorded_late_is_escalated_at_once_and_only_once() {
    let w = world().await;

    reject_on(&w, nov(30)).await;

    assert_eq!(w.t.notices.of_type(ESCALATED).len(), 1, "at once");
    job_on(&w, nov(30)).await;
    assert_eq!(w.t.notices.of_type(ESCALATED).len(), 1);

    // Resubmitted and rejected again: the same run is not escalated twice, but
    // the new failure has its own tasks.
    w.t.on(nov(30), w.t.resubmit(&w.a, &w.run, "Topped up"))
        .await
        .assert_status(303);
    w.t.on(nov(30), w.t.reject(&w.a, &w.run, "other", "BANK-REJ-2"))
        .await
        .assert_status(303);
    assert_eq!(w.t.notices.of_type(ESCALATED).len(), 1);
    assert_eq!(w.t.tasks.with_key("hr.payroll_line_failed:").len(), 4);
}

/// The tasks of one failure are opened once: the same failure seen again opens
/// nothing.
#[renox::test]
async fn hrm14_the_tasks_are_opened_once_per_failure() {
    let w = world().await;
    reject_on(&w, nov(28)).await;
    assert_eq!(w.t.tasks.with_key("hr.payroll_line_failed:").len(), 2);

    let run = w.t.run(&w.run).await;
    let again = wps_alerts::open_tasks(w.t.state(), &run).await.unwrap();

    assert_eq!(again, 0);
    assert_eq!(w.t.tasks.with_key("hr.payroll_line_failed:").len(), 2);
}

/// HRM.RUL.11: the escalation is registered as a daily scheduled task in
/// Asia/Dubai at 00:00 local (20:00 UTC the day before), whatever the zone of
/// the app.
#[test]
fn hrm14_the_daily_task_is_registered_at_midnight_in_dubai() {
    use renox::Module;
    use renox::timezone::Zone;
    let mut registry = renox::Registry::default();
    wl_hr::WlHr::new().register(&mut registry);
    let runs = registry.schedule().upcoming(Zone::UTC);
    let run = runs
        .iter()
        .find(|run| run.name == wps_alerts::TASK)
        .expect("the escalation task is registered");
    assert_eq!(run.zone.to_string(), "Asia/Dubai");
    let at = run.at.expect("it has a next run");
    assert_eq!(at.format("%H:%M").to_string(), "20:00");
}
