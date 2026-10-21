//! BL-ATT-11 (#188): `deem_approve`, the approval by silence of an external
//! timesheet whose contract has a deemed-approval clause (wl-attendance §4,
//! §5.6 point 3, §6 `HRM.RUL.15` at N, §7).

mod support;

use renox::db::DateTime;
use renox::prelude::*;
use wl_attendance::escalation;
use wl_attendance::timesheet::{ApprovalBasis, ApproverKind};
use wl_attendance::{AttendanceStatus, Timesheet, TimesheetEntry, TimesheetStatus};
use wl_kernel::Subject;
use wl_kernel::attendance::CategoryMinutes;
use wl_kernel::workforce::EmployeeId;

use support::{
    Booted, accepted_days, at, boot, client_timesheet, nov, pay_provisionally, post_to, put_hours,
};

const DEEMED: &str = "attendance.timesheet_deemed";
const ESCALATED: &str = "attendance.timesheet_escalated";

/// The people of the contract: Carl, the client's approver, and Amal, the
/// account manager; the contract deems timesheets approved after `days`.
struct Cast {
    client: User,
    account_manager: User,
}

async fn cast(t: &Booted, days: Option<i64>) -> Cast {
    let client = t.user_with("Carl", "client_approver").await;
    let account_manager = t.user_with("Amal", "account_manager").await;
    t.approvers.client_approver(client.id);
    t.approvers.account_manager(account_manager.id);
    t.approvers.deemed_after(days);
    Cast {
        client,
        account_manager,
    }
}

/// A submitted external timesheet of a worker, submitted at `submitted`.
async fn waiting(t: &Booted, employee: EmployeeId, submitted: DateTime) -> Timesheet {
    let sheet = client_timesheet(t, employee, TimesheetStatus::Submitted).await;
    Timesheet::query()
        .where_(Timesheet::ID.eq(sheet.id))
        .update(t.db(), &[("submitted_at", &Some(submitted))])
        .await
        .unwrap();
    t.reload_sheet(&sheet).await
}

async fn run_on(t: &Booted, day: u32) -> escalation::Report {
    t.on(nov(day), async {
        escalation::run(t.state(), nov(day)).await
    })
    .await
    .unwrap()
}

/// AC1 and AC4: given a contract with deemed approval N = 10 and a timesheet
/// submitted 2026-11-01, on 2026-11-11 it is `approved` with basis `deemed`,
/// attendance is locked, and the client approver and the account manager get
/// the hours and the dispute route; `TimesheetApproved` carries the basis
/// `deemed`. The day before nothing happens.
#[renox::test]
async fn att11_after_n_days_the_timesheet_is_deemed_approved_and_the_people_are_told() {
    let t = boot().await;
    let who = cast(&t, Some(10)).await;
    let worker = t.employee(1, &[]);
    let site = t.location("Site 1").await;
    let record = t
        .fixture(
            worker,
            &site,
            at(2, 7, 0),
            Some(at(2, 15, 0)),
            AttendanceStatus::Accepted,
        )
        .await;
    let sheet = waiting(&t, worker, at(1, 9, 0)).await;
    TimesheetEntry::create(
        t.db(),
        TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: worker.0,
            date: nov(2),
            normal_minutes: 480,
            billable: true,
            attendance_record_id: Some(record.id),
            ..TimesheetEntry::default()
        },
    )
    .await
    .unwrap();

    assert_eq!(run_on(&t, 10).await.deemed, 0);
    assert_eq!(
        t.reload_sheet(&sheet).await.status,
        TimesheetStatus::Submitted
    );
    assert!(t.approved.all().is_empty());

    assert_eq!(run_on(&t, 11).await.deemed, 1);
    let stored = t.reload_sheet(&sheet).await;
    assert_eq!(stored.status, TimesheetStatus::Approved);
    assert_eq!(stored.approval_basis, Some(ApprovalBasis::Deemed));
    assert_eq!(stored.approved_by, None);
    assert!(stored.approved_at.is_some());
    assert_eq!(t.reload(&record).await.status, AttendanceStatus::Locked);
    let history = t.sheet_history(&sheet).await;
    let deemed = history.iter().find(|h| h.0 == "deem_approve").unwrap();
    assert_eq!(deemed.1, None, "the system did it");

    let told = t.notices.of_type(DEEMED);
    assert_eq!(told.len(), 1);
    let mut people = support::users_of(&told[0]);
    people.sort_unstable();
    let mut wanted = vec![who.client.id, who.account_manager.id];
    wanted.sort_unstable();
    assert_eq!(people, wanted);
    assert_eq!(told[0].subject, sheet.subject());
    assert_eq!(told[0].data["hours"], "8 h normal");
    assert_eq!(told[0].data["days"], 10);
    assert!(
        told[0].data["dispute_route"]
            .as_str()
            .unwrap()
            .contains("adjustment")
    );

    let events = t.approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].approval_basis, "deemed");
    assert_eq!(events[0].approver_kind, "external");
    assert_eq!(events[0].kind, "regular");
    assert_eq!(
        events[0].totals,
        vec![CategoryMinutes {
            category: "normal".into(),
            minutes: 480
        }]
    );

    // Run again: approved already, nothing more.
    assert_eq!(run_on(&t, 12).await.deemed, 0);
    assert_eq!(t.notices.of_type(DEEMED).len(), 1);
    assert_eq!(t.approved.all().len(), 1);
}

/// AC2 and AC3: with no clause, a timesheet is still `submitted` on
/// 2026-11-11; an internal approver's timesheet is never deemed approved.
#[renox::test]
async fn att11_without_a_clause_or_with_an_internal_approver_nothing_is_deemed() {
    let t = boot().await;
    cast(&t, None).await;
    let worker = t.employee(1, &[]);
    let sheet = waiting(&t, worker, at(1, 9, 0)).await;
    assert_eq!(run_on(&t, 11).await.deemed, 0);
    assert_eq!(
        t.reload_sheet(&sheet).await.status,
        TimesheetStatus::Submitted
    );
    // A value below 1 is no clause either.
    t.approvers.deemed_after(Some(0));
    assert_eq!(run_on(&t, 11).await.deemed, 0);

    // The contract has a clause, but this approver is internal.
    t.approvers.deemed_after(Some(10));
    let mut internal = Timesheet::regular(t.employee(2, &[]), nov(1), nov(30))
        .in_context(&Subject::new("manpower.deployment", 9));
    internal.submitted_at = Some(at(1, 9, 0));
    assert_eq!(internal.approver_kind, ApproverKind::Internal);
    let internal = t.sheet_in(internal, TimesheetStatus::Submitted).await;
    assert_eq!(run_on(&t, 12).await.deemed, 1, "only the external one");
    assert_eq!(
        t.reload_sheet(&internal).await.status,
        TimesheetStatus::Submitted
    );
    assert_eq!(
        t.reload_sheet(&sheet).await.status,
        TimesheetStatus::Approved
    );
    // The machine refuses it too, whoever asks.
    use renox::state_machine::StateMachine;
    let refused = internal
        .transition(wl_attendance::approval::DEEM_APPROVE)
        .by_system()
        .input(&wl_attendance::approval::DeemInput {
            days: 1,
            on: nov(30),
        })
        .run(t.state())
        .await;
    assert!(refused.is_err());
}

/// AC5: with N = 6 and a timesheet submitted 2026-11-01, on 2026-11-07 it is
/// deemed approved and no escalation (BL-ATT-10) is sent; the task to collect
/// a signed sheet that an earlier escalation opened closes with the approval.
#[renox::test]
async fn att11_at_n_not_above_e_the_deemed_approval_comes_first_and_no_escalation_is_sent() {
    let t = boot().await;
    let who = cast(&t, Some(6)).await;
    t.approvers.escalation_contact(who.client.id);
    let sheet = waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;

    let report = run_on(&t, 7).await;
    assert_eq!((report.deemed, report.escalated), (1, 0));
    assert_eq!(
        t.reload_sheet(&sheet).await.approval_basis,
        Some(ApprovalBasis::Deemed)
    );
    assert!(t.notices.of_type(ESCALATED).is_empty());
    assert!(t.tasks.opened().is_empty());
    assert_eq!(t.notices.of_type(DEEMED).len(), 1);
}

/// With N above E the escalation comes first, at E, and the task it opened
/// closes when the timesheet is deemed approved at N.
#[renox::test]
async fn att11_with_n_above_e_the_task_of_the_escalation_closes_at_the_deemed_approval() {
    let t = boot().await;
    cast(&t, Some(10)).await;
    let sheet = waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;

    let report = run_on(&t, 7).await;
    assert_eq!((report.deemed, report.escalated), (0, 1));
    assert_eq!(t.tasks.opened().len(), 1);
    let key = escalation::task_key(sheet.id);
    assert_eq!(t.tasks.closed_with_key(&key), 0);

    let report = run_on(&t, 11).await;
    assert_eq!(report.deemed, 1);
    assert_eq!(t.tasks.closed_with_key(&key), 1);
}

/// N is read when the rule runs, so a contract changed while the timesheet
/// waits applies; and after a `reject` and a new `submit` the days count from
/// the latest `submit`.
#[renox::test]
async fn att11_n_is_read_at_run_time_and_counts_from_the_latest_submit() {
    let t = boot().await;
    let who = cast(&t, None).await;
    let keeper = t.user_with("Tariq", "timekeeper").await;
    let early = waiting(&t, t.employee(1, &[]), at(1, 9, 0)).await;
    let rejected = waiting(&t, t.employee(2, &[]), at(1, 9, 0)).await;

    // No clause yet: nothing. Then the contract gets one of 3 days, which
    // the first timesheet has passed already.
    assert_eq!(run_on(&t, 5).await.deemed, 0);
    t.approvers.deemed_after(Some(3));
    // The second one is sent back and submitted again on the 5th.
    t.on(nov(5), async {
        post_to(
            &t,
            &who.client,
            &rejected,
            "reject",
            &[("reason", "wrong site")],
        )
        .await
        .assert_ok();
        post_to(&t, &keeper, &rejected, "submit", &[("x", "y")])
            .await
            .assert_ok();
    })
    .await;
    assert_eq!(run_on(&t, 5).await.deemed, 1);
    assert_eq!(
        t.reload_sheet(&early).await.status,
        TimesheetStatus::Approved
    );
    assert_eq!(
        t.reload_sheet(&rejected).await.status,
        TimesheetStatus::Submitted
    );
    assert_eq!(run_on(&t, 7).await.deemed, 0);
    assert_eq!(run_on(&t, 8).await.deemed, 1);
    assert_eq!(
        t.reload_sheet(&rejected).await.approval_basis,
        Some(ApprovalBasis::Deemed)
    );
}

/// AC6: given the timesheet was paid provisionally with 160 h and is deemed
/// approved with 152 h, the reconciliation of BL-ATT-13 runs (adjustment -8
/// h), with the basis `deemed` copied and no approver.
#[renox::test]
async fn att11_a_deemed_approval_reconciles_the_provisional_hours() {
    let t = boot().await;
    cast(&t, Some(10)).await;
    let worker = t.employee(1, &[]);
    let sheet = waiting(&t, worker, at(1, 9, 0)).await;
    accepted_days(&t, worker, &(2..=21).collect::<Vec<_>>()).await;
    pay_provisionally(&t).await;
    for day in 2..=20 {
        put_hours(&t, &sheet, day, "normal", 480).await;
    }

    assert_eq!(run_on(&t, 11).await.deemed, 1);
    let original = t.reload_sheet(&sheet).await;
    assert_eq!(original.status, TimesheetStatus::Locked);
    assert_eq!(original.approval_basis, Some(ApprovalBasis::Deemed));
    assert_eq!(
        original.paid_provisionally_by.as_deref(),
        Some("hr.payroll_run:7")
    );
    let list = original.adjusted_by(t.db()).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].status, TimesheetStatus::Approved);
    assert_eq!(list[0].approval_basis, Some(ApprovalBasis::Deemed));
    assert_eq!(list[0].approved_by, None);
    let needed = t.reconciled.all();
    assert_eq!(needed.len(), 1);
    assert_eq!(
        needed[0].delta,
        vec![CategoryMinutes {
            category: "normal".into(),
            minutes: -480
        }]
    );
    // The original's approval is the only `TimesheetApproved`.
    let events = t.approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].approval_basis, "deemed");
}
