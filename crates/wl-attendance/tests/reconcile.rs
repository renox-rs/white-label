//! BL-ATT-13 (#190): when a timesheet that payroll paid on provisional hours
//! is approved, the approved hours are compared with the hours paid and a
//! difference becomes an adjustment of origin `provisional_reconciliation`
//! that affects payroll, with `TimesheetReconciliationNeeded` (wl-attendance
//! §5.7 point 3, §6 *Events emitted*).

mod support;

use renox::state_machine::StateMachine;
use wl_attendance::timesheet::{AdjustmentOrigin, Affects, ApprovalBasis, TimesheetKind};
use wl_attendance::{Timesheet, TimesheetEntry, TimesheetStatus};
use wl_kernel::Subject;
use wl_kernel::attendance::CategoryMinutes;
use wl_kernel::workforce::EmployeeId;

use support::{
    Booted, accepted_days, boot, client_timesheet, nov, pay_provisionally, post_to, put_hours,
};

fn minutes(category: &str, minutes: i64) -> CategoryMinutes {
    CategoryMinutes {
        category: category.into(),
        minutes,
    }
}

/// T of a worker with 160 accepted attendance hours (2 to 21 November),
/// submitted, paid provisionally by run 7 and approved by the client's
/// approver for `approved_days` days of 8 h normal hours.
async fn paid_and_submitted(t: &Booted, approved_days: &[u32]) -> (Timesheet, EmployeeId) {
    let worker = t.employee(1, &[]);
    let sheet = client_timesheet(t, worker, TimesheetStatus::Submitted).await;
    accepted_days(t, worker, &(2..=21).collect::<Vec<_>>()).await;
    pay_provisionally(t).await;
    for day in approved_days {
        put_hours(t, &sheet, *day, "normal", 480).await;
    }
    (sheet, worker)
}

async fn adjustments_of(t: &Booted, sheet: &Timesheet) -> Vec<Timesheet> {
    sheet.adjusted_by(t.db()).await.unwrap()
}

/// AC1 and AC4: T paid provisionally with 160 h normal by run 7; the client
/// approves 152 h normal. An adjustment of -8 h normal, origin
/// `provisional_reconciliation`, affects `payroll` exists, and
/// `TimesheetReconciliationNeeded` carries provisional 160, approved 152,
/// delta -8 normal and the adjustment. The adjustment is `approved` with the
/// approver and basis of T's approval, the approver got no new request and no
/// `TimesheetApproved` is emitted for it.
#[renox::test]
async fn att13_a_smaller_approval_makes_a_payroll_adjustment_approved_in_the_same_step() {
    let t = boot().await;
    let client = t.user_with("Carl", "client_approver").await;
    let (sheet, worker) = paid_and_submitted(&t, &(2..=20).collect::<Vec<_>>()).await;

    post_to(&t, &client, &sheet, "approve", &[("x", "y")])
        .await
        .assert_ok();

    let original = t.reload_sheet(&sheet).await;
    assert_eq!(original.status, TimesheetStatus::Locked);
    assert_eq!(
        original.paid_provisionally_by.as_deref(),
        Some("hr.payroll_run:7")
    );
    let list = adjustments_of(&t, &sheet).await;
    assert_eq!(list.len(), 1);
    let adjustment = &list[0];
    assert_eq!(adjustment.kind, TimesheetKind::Adjustment);
    assert_eq!(
        adjustment.origin,
        Some(AdjustmentOrigin::ProvisionalReconciliation)
    );
    assert_eq!(adjustment.affects, Some(Affects::Payroll));
    assert_eq!(adjustment.adjusts_id, Some(sheet.id));
    assert_eq!(adjustment.linked(), Some(Subject::new("hr.payroll_run", 7)));
    assert_eq!(adjustment.created_by, None);
    assert_eq!(adjustment.owner(), worker);

    // Approved in the same step, with the approver and basis of T's.
    assert_eq!(adjustment.status, TimesheetStatus::Approved);
    assert_eq!(adjustment.approved_by, Some(client.id));
    assert_eq!(adjustment.approval_basis, Some(ApprovalBasis::Approved));
    assert!(adjustment.approved_at.is_some());
    assert_eq!(adjustment.approver_kind, original.approver_kind);
    let history = t.sheet_history(adjustment).await;
    let steps: Vec<&str> = history.iter().map(|h| h.0.as_str()).collect();
    assert!(
        steps.ends_with(&["submit_by_system", "approve_by_system"]),
        "{steps:?}"
    );

    // -8 h normal on the day the client did not approve.
    let rows = TimesheetEntry::of_timesheet(t.db(), adjustment.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].date, rows[0].normal_minutes), (nov(21), -480));
    assert_eq!(rows[0].cost_object(), None);

    // The event of the difference, and only that one.
    let needed = t.reconciled.all();
    assert_eq!(needed.len(), 1);
    assert_eq!(needed[0].timesheet, sheet.subject());
    assert_eq!(needed[0].owner, worker);
    assert_eq!(needed[0].payroll_run, Subject::new("hr.payroll_run", 7));
    assert_eq!(needed[0].provisional, vec![minutes("normal", 160 * 60)]);
    assert_eq!(needed[0].approved, vec![minutes("normal", 152 * 60)]);
    assert_eq!(needed[0].delta, vec![minutes("normal", -8 * 60)]);
    assert_eq!(needed[0].adjustment, adjustment.subject());
    let approvals = t.approved.all();
    assert_eq!(
        approvals.len(),
        1,
        "no TimesheetApproved for the adjustment"
    );
    assert_eq!(approvals[0].timesheet, sheet.subject());
    assert_eq!(approvals[0].kind, "regular");

    // The approver was not asked again: nobody was sent a request.
    assert!(t.notices.all().is_empty());
    assert!(t.tasks.opened().is_empty());
    assert!(t.posted.all().is_empty());
}

/// AC2: the client approves 160 h, the hours paid: nothing is created and
/// nothing is emitted. The timesheet is still locked at once.
#[renox::test]
async fn att13_an_approval_of_the_hours_paid_creates_nothing() {
    let t = boot().await;
    let client = t.user_with("Carl", "client_approver").await;
    let (sheet, _) = paid_and_submitted(&t, &(2..=21).collect::<Vec<_>>()).await;

    post_to(&t, &client, &sheet, "approve", &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(t.reload_sheet(&sheet).await.status, TimesheetStatus::Locked);
    assert!(adjustments_of(&t, &sheet).await.is_empty());
    assert!(t.reconciled.all().is_empty());
}

/// AC3: 150 normal + 10 overtime approved against 160 normal paid gives a
/// delta of -10 normal and +10 overtime; the entries of the adjustment follow
/// the days.
#[renox::test]
async fn att13_a_move_between_categories_counts() {
    let t = boot().await;
    let client = t.user_with("Carl", "client_approver").await;
    let (sheet, _) = paid_and_submitted(&t, &(2..=19).collect::<Vec<_>>()).await;
    // 18 days of 8 h, 6 h on the 20th: 150 h normal; 10 h overtime on the
    // 21st.
    put_hours(&t, &sheet, 20, "normal", 360).await;
    put_hours(&t, &sheet, 21, "overtime", 600).await;

    post_to(&t, &client, &sheet, "approve", &[("x", "y")])
        .await
        .assert_ok();
    let needed = t.reconciled.all();
    assert_eq!(needed.len(), 1);
    assert_eq!(needed[0].provisional, vec![minutes("normal", 9_600)]);
    assert_eq!(
        needed[0].approved,
        vec![minutes("normal", 9_000), minutes("overtime", 600)]
    );
    assert_eq!(
        needed[0].delta,
        vec![minutes("normal", -600), minutes("overtime", 600)]
    );

    let adjustment = adjustments_of(&t, &sheet).await.pop().unwrap();
    let rows = TimesheetEntry::of_timesheet(t.db(), adjustment.id)
        .await
        .unwrap();
    let by_day: Vec<(u32, i64, i64)> = rows
        .iter()
        .map(|r| {
            use renox::chrono::Datelike;
            (r.date.day(), r.normal_minutes, r.overtime_minutes)
        })
        .collect();
    assert_eq!(by_day, vec![(20, -120, 0), (21, -480, 600)]);
}

/// AC5: handling the approval event twice creates one adjustment and says
/// nothing the second time.
#[renox::test]
async fn att13_handling_the_approval_event_twice_creates_one_adjustment() {
    let t = boot().await;
    let client = t.user_with("Carl", "client_approver").await;
    let (sheet, _) = paid_and_submitted(&t, &(2..=20).collect::<Vec<_>>()).await;
    post_to(&t, &client, &sheet, "approve", &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(adjustments_of(&t, &sheet).await.len(), 1);

    let event = t.approved.all().pop().unwrap();
    for _ in 0..2 {
        t.state().emit(event.clone()).await.unwrap();
    }
    assert_eq!(adjustments_of(&t, &sheet).await.len(), 1);
    assert_eq!(t.reconciled.all().len(), 1);
    assert_eq!(
        wl_attendance::reconcile::reconcile(t.state(), sheet.id)
            .await
            .unwrap()
            .map(|a| a.id),
        None
    );
}

/// An approval on behalf of the client reconciles the same way, with the
/// basis `on_behalf` copied; and 0 h approved against the hours paid takes
/// them all back, though the original has no hours to take them from.
#[renox::test]
async fn att13_an_approval_on_behalf_reconciles_too_and_nothing_approved_takes_it_all_back() {
    let t = boot().await;
    let staff = t.user_with("Amal", "account_manager").await;
    let (sheet, _) = paid_and_submitted(&t, &[]).await;

    post_to(
        &t,
        &staff,
        &sheet,
        "approve-on-behalf",
        &[
            ("document_id", "31"),
            ("approver_name", "Carl Client"),
            ("signed_on", "2026-11-09"),
        ],
    )
    .await
    .assert_ok();
    let original = t.reload_sheet(&sheet).await;
    assert_eq!(original.status, TimesheetStatus::Locked);
    assert_eq!(original.approval_basis, Some(ApprovalBasis::OnBehalf));
    let adjustment = adjustments_of(&t, &sheet).await.pop().unwrap();
    assert_eq!(adjustment.approval_basis, Some(ApprovalBasis::OnBehalf));
    assert_eq!(adjustment.approved_by, Some(staff.id));
    assert_eq!(adjustment.status, TimesheetStatus::Approved);
    let needed = t.reconciled.all();
    assert_eq!(needed.len(), 1);
    assert_eq!(needed[0].approved, Vec::<CategoryMinutes>::new());
    assert_eq!(needed[0].delta, vec![minutes("normal", -160 * 60)]);
    let rows = TimesheetEntry::of_timesheet(t.db(), adjustment.id)
        .await
        .unwrap();
    assert_eq!(rows.len(), 20);
    assert!(rows.iter().all(|r| r.normal_minutes == -480));
}

/// A timesheet payroll did not pay provisionally is not reconciled, and a
/// user cannot approve the system's adjustment: it is `approved` already, and
/// the system transition refuses any other timesheet.
#[renox::test]
async fn att13_only_a_provisionally_paid_timesheet_is_reconciled() {
    let t = boot().await;
    let client = t.user_with("Carl", "client_approver").await;
    let worker = t.employee(1, &[]);
    let sheet = client_timesheet(&t, worker, TimesheetStatus::Submitted).await;
    put_hours(&t, &sheet, 2, "normal", 480).await;
    post_to(&t, &client, &sheet, "approve", &[("x", "y")])
        .await
        .assert_ok();
    assert_eq!(
        t.reload_sheet(&sheet).await.status,
        TimesheetStatus::Approved
    );
    assert!(adjustments_of(&t, &sheet).await.is_empty());
    assert!(t.reconciled.all().is_empty());

    // The system approval is for the reconciliation adjustment only.
    let submitted = t
        .sheet_in(
            Timesheet::regular(t.employee(2, &[]), nov(1), nov(30)),
            TimesheetStatus::Submitted,
        )
        .await;
    let refused = submitted
        .transition(wl_attendance::approval::APPROVE_BY_SYSTEM)
        .by_system()
        .run(t.state())
        .await;
    assert!(refused.is_err());
    assert_eq!(
        t.reload_sheet(&submitted).await.status,
        TimesheetStatus::Submitted
    );
}
