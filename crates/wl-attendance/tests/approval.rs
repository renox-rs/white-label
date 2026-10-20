//! Approving an adjustment timesheet with the separation of duties
//! (wl-attendance §4 `approve`, `approve_on_behalf`, `reject`, §5.8,
//! `HRM.ACL.08`, issue #185).

mod support;

use renox::prelude::*;
use renox::testing::TestResponse;
use wl_attendance::approval::ApprovalEvidence;
use wl_attendance::entry::TimesheetEntry;
use wl_attendance::timesheet::{AdjustmentOrigin, Affects, ApprovalBasis, ApproverKind};
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

use support::{Booted, boot, nov, project};

const COST_CODE: &str = "construction.cost_code:3";

fn deployment() -> Subject {
    Subject::new("manpower.deployment", 9)
}

/// A locked timesheet of `employee` for `context` with 8 h normal on 2
/// November against the cost code, approved by `kind`.
async fn locked(t: &Booted, employee: EmployeeId, context: &Subject, kind: ApproverKind) -> Timesheet {
    let mut new = Timesheet::regular(employee, nov(2), nov(8)).in_context(context);
    new.paid_by = Some("hr.payroll_run:7".into());
    new.approver_kind = kind;
    if kind == ApproverKind::External {
        new.approver_role = Some("client_approver".into());
    }
    let sheet = t.sheet_in(new, TimesheetStatus::Locked).await;
    TimesheetEntry::create(
        t.db(),
        TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: employee.0,
            date: nov(2),
            cost_object_type: Some("construction.cost_code".into()),
            cost_object_id: Some(3),
            normal_minutes: 480,
            billable: true,
            ..TimesheetEntry::default()
        },
    )
    .await
    .unwrap();
    sheet
}

/// User `creator` opens an adjustment of -8 h on `original` and submits it.
async fn submitted_adjustment(t: &Booted, creator: &User, original: &Timesheet) -> Timesheet {
    t.app.acting_as(creator);
    let entries = format!(
        r#"[{{"date": "2026-11-02", "category": "normal", "minutes": -480, "cost_object": "{COST_CODE}"}}]"#
    );
    t.app
        .htmx()
        .post(
            &format!("/timesheets/{}/adjustments", original.id),
            &[
                ("reason", "marked present, was absent"),
                ("origin", "attendance_error"),
                ("affects", "both"),
                ("entries", entries.as_str()),
            ],
        )
        .await
        .assert_ok();
    let adjustment = original.adjusted_by(t.db()).await.unwrap().pop().unwrap();
    let keeper = t.user_with(&format!("Keeper{}", adjustment.id), "timekeeper").await;
    t.app.acting_as(&keeper);
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/submit", adjustment.id), &[("x", "y")])
        .await
        .assert_ok();
    let sheet = t.reload_sheet(&adjustment).await;
    assert_eq!(sheet.status, TimesheetStatus::Submitted);
    sheet
}

async fn post(t: &Booted, user: &User, sheet: &Timesheet, action: &str, form: &[(&str, &str)]) -> TestResponse {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(&format!("/timesheets/{}/{action}", sheet.id), form)
        .await
}

/// Adjustment J (created by A) of an internally approved 02 timesheet with
/// -8 h at AED 25 an hour: A cannot approve it, B can; a cost of -AED 200 is
/// posted to the original cost object and `TimesheetApproved` carries kind
/// `adjustment`, adjusts, origin, affects and signed totals of -8 h.
#[renox::test]
async fn att08_the_creator_cannot_approve_and_another_approver_posts_the_negative_cost() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    // Amal also holds the approval permission: even so she is refused.
    a_user.assign_role(t.db(), "approver").await.unwrap();
    let b_user = t.user_with("Bilal", "approver").await;
    let worker = t.employee(1, &[]);
    let original = locked(&t, worker, &project(5), ApproverKind::Internal).await;
    let j = submitted_adjustment(&t, &a_user, &original).await;
    assert_eq!(j.created_by, Some(a_user.id));

    post(&t, &a_user, &j, "approve", &[("x", "y")]).await.assert_status(422);
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Submitted);
    assert!(t.posted.all().is_empty());
    assert!(t.approved.all().is_empty());

    post(&t, &b_user, &j, "approve", &[("x", "y")]).await.assert_ok();
    let approved = t.reload_sheet(&j).await;
    assert_eq!(approved.status, TimesheetStatus::Approved);
    assert_eq!(approved.approval_basis, Some(ApprovalBasis::Approved));
    assert_eq!(approved.approved_by, Some(b_user.id));
    assert!(approved.approved_at.is_some());

    let posts = t.posted.all();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].amount_aed, -20_000);
    assert_eq!(posts[0].cost_object, Subject::new("construction.cost_code", 3));
    assert_eq!(posts[0].source, j.subject());

    let events = t.approved.all();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.kind, "adjustment");
    assert_eq!(event.adjusts, Some(original.subject()));
    assert_eq!(event.origin.as_deref(), Some(AdjustmentOrigin::AttendanceError.as_str()));
    assert_eq!(event.affects.as_deref(), Some(Affects::Both.as_str()));
    assert_eq!(event.timesheet, j.subject());
    assert_eq!(event.owner, worker);
    assert_eq!(event.approver_kind, "internal");
    assert_eq!(event.approval_basis, "approved");
    assert_eq!(event.totals.len(), 1);
    assert_eq!(event.totals[0].category, "normal");
    assert_eq!(event.totals[0].minutes, -480);

    // The original is untouched by the approval of its adjustment.
    assert_eq!(t.reload_sheet(&original).await.status, TimesheetStatus::Locked);
}

/// An approval is posted once: a second approval is refused and the cost is
/// not counted twice.
#[renox::test]
async fn att08_an_approved_adjustment_is_not_approved_again() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let b_user = t.user_with("Bilal", "approver").await;
    let original = locked(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    let j = submitted_adjustment(&t, &a_user, &original).await;
    post(&t, &b_user, &j, "approve", &[("x", "y")]).await.assert_ok();
    post(&t, &b_user, &j, "approve", &[("x", "y")]).await.assert_status(422);
    assert_eq!(t.posted.all().len(), 1);
    assert_eq!(t.approved.all().len(), 1);
}

/// A user who may approve as nobody is forbidden, whatever the timesheet.
#[renox::test]
async fn att08_a_user_without_the_permission_or_the_role_is_forbidden() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let nobody = t.user_with("Nadia", "timekeeper").await;
    let original = locked(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    let j = submitted_adjustment(&t, &a_user, &original).await;
    post(&t, &nobody, &j, "approve", &[("x", "y")]).await.assert_status(403);
    post(&t, &nobody, &j, "reject", &[("reason", "no")]).await.assert_status(403);
    post(&t, &nobody, &j, "approve-on-behalf", &[("document_id", "5")]).await.assert_status(403);
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Submitted);
}

/// J adjusts a 09 timesheet approved by the client: internal approver B using
/// `approve` is refused; `approve_on_behalf` with a signed sheet succeeds,
/// stores the evidence and posts no labour cost (09 posts none).
#[renox::test]
async fn att08_an_externally_approved_original_needs_the_client_or_a_signed_sheet() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let b_user = t.user_with("Bilal", "approver").await;
    let c_user = t.user_with("Carla", "account_manager").await;
    let original = locked(&t, t.employee(1, &[]), &deployment(), ApproverKind::External).await;
    let j = submitted_adjustment(&t, &a_user, &original).await;
    assert_eq!(j.approver_kind, ApproverKind::External);

    // The internal approver is refused.
    post(&t, &b_user, &j, "approve", &[("x", "y")]).await.assert_status(422);
    // Without the signed sheet, nor does the account manager succeed.
    post(&t, &c_user, &j, "approve-on-behalf", &[("approver_name", "Omar")]).await.assert_status(422);
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Submitted);

    post(
        &t,
        &c_user,
        &j,
        "approve-on-behalf",
        &[("document_id", "41"), ("approver_name", "Omar Client"), ("signed_on", "2026-11-09")],
    )
    .await
    .assert_ok();
    let approved = t.reload_sheet(&j).await;
    assert_eq!(approved.status, TimesheetStatus::Approved);
    assert_eq!(approved.approval_basis, Some(ApprovalBasis::OnBehalf));
    assert_eq!(approved.approved_by, Some(c_user.id));
    let evidence = ApprovalEvidence::of_timesheet(t.db(), j.id).await.unwrap();
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].document_id, 41);
    assert_eq!(evidence[0].approver_name, "Omar Client");

    // A 09 context posts no labour cost, and the event says how it was approved.
    assert!(t.posted.all().is_empty());
    let events = t.approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].approver_kind, "external");
    assert_eq!(events[0].approval_basis, "on_behalf");
    assert_eq!(events[0].kind, "adjustment");
}

/// The creator of an adjustment is refused even with `approve_on_behalf`.
#[renox::test]
async fn att08_the_creator_with_approve_on_behalf_is_still_refused() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    a_user.assign_role(t.db(), "account_manager").await.unwrap();
    let original = locked(&t, t.employee(1, &[]), &deployment(), ApproverKind::External).await;
    let j = submitted_adjustment(&t, &a_user, &original).await;
    post(
        &t,
        &a_user,
        &j,
        "approve-on-behalf",
        &[("document_id", "41"), ("approver_name", "Omar"), ("signed_on", "2026-11-09")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Submitted);
    assert!(ApprovalEvidence::of_timesheet(t.db(), j.id).await.unwrap().is_empty());
}

/// The client's approver (a user holding the timesheet's approver role)
/// approves an external timesheet with `approve`; `approve_on_behalf` is not
/// for an internal timesheet.
#[renox::test]
async fn att08_the_client_approver_approves_and_on_behalf_is_for_external_only() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let client = t.user_with("Omar", "client_approver").await;
    let c_user = t.user_with("Carla", "account_manager").await;
    let external = locked(&t, t.employee(1, &[]), &deployment(), ApproverKind::External).await;
    let j = submitted_adjustment(&t, &a_user, &external).await;
    post(&t, &client, &j, "approve", &[("x", "y")]).await.assert_ok();
    let approved = t.reload_sheet(&j).await;
    assert_eq!(approved.status, TimesheetStatus::Approved);
    assert_eq!(approved.approval_basis, Some(ApprovalBasis::Approved));
    assert_eq!(approved.approved_by, Some(client.id));
    assert_eq!(t.approved.all()[0].approver_kind, "external");

    let internal = locked(&t, t.employee(2, &[]), &project(5), ApproverKind::Internal).await;
    let k = submitted_adjustment(&t, &a_user, &internal).await;
    post(
        &t,
        &c_user,
        &k,
        "approve-on-behalf",
        &[("document_id", "41"), ("approver_name", "Omar"), ("signed_on", "2026-11-09")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload_sheet(&k).await.status, TimesheetStatus::Submitted);
}

/// `reject` returns J to `open` with the reason, tells the submitter, and
/// needs a reason.
#[renox::test]
async fn att08_reject_returns_the_adjustment_to_open_with_the_reason() {
    let t = boot().await;
    let a_user = t.user_with("Amal", "adjuster").await;
    let b_user = t.user_with("Bilal", "approver").await;
    let original = locked(&t, t.employee(1, &[]), &project(5), ApproverKind::Internal).await;
    let j = submitted_adjustment(&t, &a_user, &original).await;
    assert!(j.submitted_by.is_some());

    post(&t, &b_user, &j, "reject", &[("reason", "")]).await.assert_status(422);
    assert_eq!(t.reload_sheet(&j).await.status, TimesheetStatus::Submitted);

    post(&t, &b_user, &j, "reject", &[("reason", "hours do not match the roster")])
        .await
        .assert_ok();
    let sheet = t.reload_sheet(&j).await;
    assert_eq!(sheet.status, TimesheetStatus::Open);
    let history = t.sheet_history(&sheet).await;
    let last = history.last().unwrap();
    assert_eq!(last.0, "reject");
    assert_eq!(last.1, Some(b_user.id));
    assert_eq!(last.2.as_deref(), Some("hours do not match the roster"));

    let told = t.notices.of_type("attendance.timesheet_rejected");
    assert_eq!(told.len(), 1);
    assert_eq!(support::users_of(&told[0]), vec![j.submitted_by.unwrap()]);
    assert_eq!(told[0].data["reason"], "hours do not match the roster");
    assert!(t.posted.all().is_empty());
    assert!(t.approved.all().is_empty());
}
