//! BL-HRM-04 (#194): the five non-standard exits (`cancel_onboarding`,
//! `withdraw_notice`, `report_absconding`, `reinstate`, `record_death`) are
//! requested by HR and approved by another user with `employees.exit.approve`
//! (`HRM.ACL.07`); only then the transition runs. One request is pending per
//! employee, the guards run again at the final approval, and everything is
//! audited.

mod support;

use renox::db::sql;
use renox::prelude::*;
use renox::state_machine::history;
use wl_hr::exits;
use wl_hr::{Employee, EmployeeStatus};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::{ApprovalDecided, ApprovalEnd};

use support::Booted;

/// The fields of a `report_absconding` request; the employee has been absent
/// for long (`Booted::make_absent`).
fn absconding<'a>() -> Vec<(&'a str, &'a str)> {
    vec![
        ("transition", "report_absconding"),
        ("reason", "absent since 2026-11-01"),
        ("report_number", "MOHRE-777"),
        ("report_date", "2026-11-09"),
        ("report_file", "12"),
    ]
}

/// `user` posts a request for `employee`.
async fn post(
    t: &Booted,
    user: &User,
    employee: &Employee,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app.acting_as(user);
    t.app
        .htmx()
        .post(&format!("/hr/employees/{}/exits", employee.id), fields)
        .await
}

/// The requests about `employee` that are pending.
async fn pending(t: &Booted, employee: &Employee) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &employee.subject())
        .await
        .unwrap()
}

/// Every request about `employee`, pending or not.
async fn all_requests(t: &Booted, employee: &Employee) -> Vec<ApprovalRequest> {
    ApprovalRequest::query()
        .where_(ApprovalRequest::SUBJECT_TYPE.eq("hr.employee"))
        .where_(ApprovalRequest::SUBJECT_ID.eq(employee.id))
        .get(t.db())
        .await
        .unwrap()
}

/// The notices of `kind` that `user` received.
async fn notices(t: &Booted, user: &User, kind: &str) -> Vec<String> {
    user.notifications(t.db(), 50)
        .await
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == kind)
        .map(|n| n.data.to_string())
        .collect()
}

/// HR user A requests `report_absconding` with the reason and the report
/// number: E keeps its state and an approval request exists, and the MD is
/// told.
#[renox::test]
async fn hrm04_a_request_keeps_the_state_and_creates_an_approval_request() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;

    post(&t, &a, &e, &absconding()).await.assert_ok();

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Active);
    let requests = pending(&t, &e).await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.action, "hr.report_absconding");
    assert_eq!(request.transition, "report_absconding");
    assert_eq!(request.requester_id, a.id);
    assert_eq!(request.payload.0["reason"], "absent since 2026-11-01");
    assert_eq!(request.payload.0["report_number"], "MOHRE-777");
    assert_eq!(notices(&t, &b, "approval-requested").await.len(), 1);
    assert!(notices(&t, &a, "approval-requested").await.is_empty());
}

/// The requester cannot approve; the MD can, and then E moves and the history
/// shows the requester and the approver.
#[renox::test]
async fn hrm04_the_requester_cannot_approve_and_the_md_can() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    post(&t, &a, &e, &absconding()).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);

    let refused = approvals::approve(t.state(), request.id, &a, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Active);

    let decided = approvals::approve(t.state(), request.id, &b, "approved")
        .await
        .unwrap()
        .expect("the last step ends the request");
    assert_eq!(decided.decision, ApprovalEnd::Approved);
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Absconded);

    let last = history(t.db(), Employee::TABLE, e.id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(last.transition, "report_absconding_approved");
    assert_eq!(last.actor_id, None, "run as the system");
    assert_eq!(last.metadata["requester_id"], a.id);
    assert_eq!(last.metadata["actor_id"], b.id);
    assert_eq!(last.reason.as_deref(), Some("absent since 2026-11-01"));
}

/// A rejection leaves E unchanged and tells the requester, with the comment.
#[renox::test]
async fn hrm04_a_rejection_changes_nothing_and_tells_the_requester() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    post(&t, &a, &e, &absconding()).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);

    approvals::reject(t.state(), request.id, &b, "wait for the second attempt")
        .await
        .unwrap();

    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Active);
    assert!(pending(&t, &e).await.is_empty());
    let told = notices(&t, &a, "approval-decided").await;
    assert_eq!(told.len(), 1);
    assert!(
        told[0].contains("wait for the second attempt"),
        "{}",
        told[0]
    );
    // The request ended, so a new one is accepted.
    post(&t, &a, &e, &absconding()).await.assert_ok();
    assert_eq!(pending(&t, &e).await.len(), 1);
}

/// A second request for E while the first is pending is refused, for the same
/// action and for another of the five.
#[renox::test]
async fn hrm04_one_pending_request_per_employee() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    post(&t, &a, &e, &absconding()).await.assert_ok();

    post(&t, &a, &e, &absconding()).await.assert_status(422);
    let death = [
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("death_certificate", "DC-9"),
    ];
    post(&t, &a, &e, &death).await.assert_status(422);
    assert_eq!(all_requests(&t, &e).await.len(), 1);
}

/// A `record_death` request without the certificate or the authority's notice
/// is refused at request: no approval request exists.
#[renox::test]
async fn hrm04_a_request_without_its_documents_is_refused_at_request() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;

    let death = [
        ("transition", "record_death"),
        ("reason", "died in service"),
    ];
    post(&t, &a, &e, &death).await.assert_status(422);
    assert!(all_requests(&t, &e).await.is_empty());

    // The same for the other exits that name documents, and for a missing
    // reason; each employee is in a state its action leaves.
    let cases: [(EmployeeStatus, &[(&str, &str)]); 4] = [
        (
            EmployeeStatus::Notice,
            &[("transition", "withdraw_notice"), ("reason", "agreed")],
        ),
        (
            EmployeeStatus::Active,
            &[
                ("transition", "report_absconding"),
                ("reason", "absent"),
                ("report_date", "2026-11-09"),
            ],
        ),
        (
            EmployeeStatus::Absconded,
            &[("transition", "reinstate"), ("reason", "returned")],
        ),
        (
            EmployeeStatus::Active,
            &[
                ("transition", "record_death"),
                ("reason", "  "),
                ("authority_notice", "N-1"),
            ],
        ),
    ];
    for (n, (status, fields)) in cases.into_iter().enumerate() {
        let other = t.employee_in(80 + n as i64, status).await;
        post(&t, &a, &other, fields).await.assert_status(422);
        assert!(all_requests(&t, &other).await.is_empty(), "{fields:?}");
    }
    assert!(all_requests(&t, &e).await.is_empty());
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Active);

    // With the notice of the authority it is accepted.
    let with_notice = [
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("authority_notice", "N-1"),
    ];
    post(&t, &a, &e, &with_notice).await.assert_ok();
    assert_eq!(pending(&t, &e).await.len(), 1);
}

/// Only HR asks, only from a state the action leaves, and only for the five.
#[renox::test]
async fn hrm04_who_may_ask_and_from_which_state() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let viewer = t.user_with("Vera", "manager").await;
    let md = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;

    post(&t, &viewer, &e, &absconding())
        .await
        .assert_forbidden();
    post(&t, &md, &e, &absconding()).await.assert_forbidden();
    // `reinstate` leaves `absconded` only; `activate` is not one of the five.
    let reinstate = [
        ("transition", "reinstate"),
        ("reason", "returned"),
        ("reference", "WD-55"),
        ("return_date", "2026-01-10"),
    ];
    post(&t, &a, &e, &reinstate).await.assert_status(422);
    let activate = [("transition", "activate"), ("reason", "now")];
    post(&t, &a, &e, &activate).await.assert_status(422);
    assert!(all_requests(&t, &e).await.is_empty());
}

/// After B approves, the audit log has three entries (request by A, approval
/// by B, transition), each with the reason and the report file.
#[renox::test]
async fn hrm04_the_request_the_approval_and_the_transition_are_audited() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    post(&t, &a, &e, &absconding()).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();

    let log = renox::audit::for_subject(t.db(), Employee::TABLE, e.id, 20)
        .await
        .unwrap();
    let entry = |action: &str| {
        let found: Vec<_> = log.iter().filter(|l| l.action == action).collect();
        assert_eq!(found.len(), 1, "{action}: {log:?}");
        found[0]
    };
    let requested = entry(exits::AUDIT_REQUESTED);
    let approved = entry(exits::AUDIT_APPROVED);
    let transition = entry(exits::AUDIT_TRANSITION);
    assert_eq!(requested.user_id, Some(a.id));
    assert_eq!(approved.user_id, Some(b.id));
    assert_eq!(transition.user_id, Some(b.id));
    for logged in [requested, approved, transition] {
        assert_eq!(logged.data["reason"], "absent since 2026-11-01");
        assert_eq!(logged.data["report_file"], 12);
        assert_eq!(logged.data["report_number"], "MOHRE-777");
        assert_eq!(logged.data["requested_by"], a.id);
        assert_eq!(logged.data["transition"], "report_absconding");
    }
}

/// A rejected request is audited with the comment and no transition entry.
#[renox::test]
async fn hrm04_a_rejection_is_audited_without_a_transition() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    post(&t, &a, &e, &absconding()).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);
    approvals::reject(t.state(), request.id, &b, "not enough attempts")
        .await
        .unwrap();

    let log = renox::audit::for_subject(t.db(), Employee::TABLE, e.id, 20)
        .await
        .unwrap();
    let actions: Vec<&str> = log.iter().map(|l| l.action.as_str()).collect();
    assert!(actions.contains(&exits::AUDIT_REQUESTED), "{actions:?}");
    assert!(actions.contains(&"hr.exit_rejected"), "{actions:?}");
    assert!(!actions.contains(&exits::AUDIT_TRANSITION), "{actions:?}");
    let rejected = log.iter().find(|l| l.action == "hr.exit_rejected").unwrap();
    assert_eq!(rejected.data["comment"], "not enough attempts");
    assert_eq!(rejected.user_id, Some(b.id));
}

/// When the MD is the requester, another holder of the permission must
/// approve; the MD cannot approve their own request, even holding both
/// permissions.
#[renox::test]
async fn hrm04_the_md_who_requests_cannot_approve_their_own_request() {
    let t = support::boot().await;
    let md = t.user_with("Mona", "hr_md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    post(&t, &md, &e, &absconding()).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);

    let refused = approvals::approve(t.state(), request.id, &md, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Active);
    assert_eq!(pending(&t, &e).await.len(), 1, "the request waits");

    let other = t.user_with("Omar", "md").await;
    approvals::approve(t.state(), request.id, &other, "")
        .await
        .unwrap();
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Absconded);
}

/// Guards run again at the final approval: when E is no longer in a state the
/// action leaves, the request ends `failed` and nothing runs.
#[renox::test]
async fn hrm04_a_failed_check_at_approval_ends_the_request_failed() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    let death = [
        ("transition", "record_death"),
        ("reason", "died in service"),
        ("death_certificate", "DC-9"),
    ];
    post(&t, &a, &e, &death).await.assert_ok();
    let request = pending(&t, &e).await.remove(0);

    // E left every state `record_death` leaves, behind the machine's back.
    sql("UPDATE wl_hr_employees SET status = 'exited' WHERE id = ?")
        .bind(e.id)
        .execute(t.db())
        .await
        .unwrap();

    let decided: Option<ApprovalDecided> = approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    assert_eq!(decided.expect("an end").decision, ApprovalEnd::Failed);
    let after = ApprovalRequest::find_or_404(t.db(), request.id)
        .await
        .unwrap();
    assert_eq!(after.status, ApprovalStatus::Failed);
    assert_eq!(t.employee(&e).await.status, EmployeeStatus::Exited);
    let told = notices(&t, &a, "approval-decided").await;
    assert_eq!(told.len(), 1);
}

/// Every one of the five exits is a transition that only the approval runs:
/// each, approved, takes the employee to its state.
#[renox::test]
async fn hrm04_each_exit_moves_the_employee_once_approved() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let b = t.user_with("Basel", "md").await;
    type Case<'a> = (EmployeeStatus, Vec<(&'a str, &'a str)>, EmployeeStatus);
    let cases: [Case; 4] = [
        (
            EmployeeStatus::Onboarding,
            vec![
                ("transition", "cancel_onboarding"),
                ("reason", "visa refused"),
            ],
            EmployeeStatus::OnboardingCancelled,
        ),
        (
            EmployeeStatus::Notice,
            vec![
                ("transition", "withdraw_notice"),
                ("reason", "agreed"),
                ("agreement", "AGR-1"),
            ],
            EmployeeStatus::Active,
        ),
        (
            EmployeeStatus::Absconded,
            vec![
                ("transition", "reinstate"),
                ("reason", "came back"),
                ("reference", "WD-55"),
                ("return_date", "2026-01-10"),
            ],
            EmployeeStatus::Active,
        ),
        (
            EmployeeStatus::OnLeave,
            vec![
                ("transition", "record_death"),
                ("reason", "died"),
                ("death_certificate", "DC-1"),
            ],
            EmployeeStatus::Deceased,
        ),
    ];
    for (n, (from, fields, to)) in cases.into_iter().enumerate() {
        let e = t.employee_in(70 + n as i64, from).await;
        post(&t, &a, &e, &fields).await.assert_ok();
        assert_eq!(t.employee(&e).await.status, from, "kept until approved");
        let request = pending(&t, &e).await.remove(0);
        approvals::approve(t.state(), request.id, &b, "")
            .await
            .unwrap();
        assert_eq!(t.employee(&e).await.status, to, "{fields:?}");
    }
}

/// The employee page shows the pending request as a badge, and the form to
/// ask only to the holders of `employees.manage`.
#[renox::test]
async fn hrm04_the_page_shows_the_request_and_the_form() {
    let t = support::boot().await;
    let a = t.user_with("Hana", "hr").await;
    let viewer = t.user_with("Vera", "manager").await;
    let e = t.employee_in(60, EmployeeStatus::Active).await;
    t.make_absent(&e).await;
    let path = format!("/hr/employees/{}", e.id);

    t.app.acting_as(&a);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Request an exit action");
    post(&t, &a, &e, &absconding()).await.assert_ok();
    t.app.acting_as(&a);
    t.app
        .get(&path)
        .await
        .assert_ok()
        .assert_see("Waiting for approval")
        .assert_see("Report absconding")
        .assert_see("absent since 2026-11-01");

    t.app.acting_as(&viewer);
    let page = t.app.get(&path).await;
    assert!(!page.text().contains("Request an exit action"));
    page.assert_ok().assert_see("Waiting for approval");
}
