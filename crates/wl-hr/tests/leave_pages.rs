//! The leave screens: my leave (ask, cancel, what was decided) and the queue
//! of leave to decide (approve, reject with a reason, the sick note first).

mod support;

use renox::prelude::*;
use wl_hr::{LeaveRequest, LeaveStatus};

use support::{branch_scope_of, nov, other_branch};

/// The employee asks for leave through the page; wrong dates answer in words;
/// the page lists the request; the employee cancels it before it starts; a
/// user without `leave.request` is refused; an account with no employee record
/// is told so.
#[renox::test]
async fn an_employee_asks_for_leave_and_cancels_it_before_it_starts() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    account.assign_role(t.db(), "staff").await.unwrap();
    let stranger = t.user_with("Sam", "staff").await;
    let nobody = t.user("Nora").await;

    t.on(nov(1), async {
        t.app.acting_as(&nobody);
        t.app.get("/hr/leave").await.assert_forbidden();

        t.app.acting_as(&stranger);
        t.app
            .get("/hr/leave")
            .await
            .assert_ok()
            .assert_see("not linked to an employee record");
        t.app
            .htmx()
            .post(
                "/hr/leave",
                &[
                    ("leave_type", "annual"),
                    ("from_date", "2026-11-09"),
                    ("to_date", "2026-11-10"),
                ],
            )
            .await
            .assert_status(422);

        t.app.acting_as(&account);
        for (kind, from, to) in [
            ("annual", "soon", "2026-11-10"),
            ("annual", "2026-11-09", "later"),
            ("annual", "2026-11-10", "2026-11-09"),
            ("holiday", "2026-11-09", "2026-11-10"),
        ] {
            t.app
                .htmx()
                .post(
                    "/hr/leave",
                    &[("leave_type", kind), ("from_date", from), ("to_date", to)],
                )
                .await
                .assert_status(422);
        }
        assert!(LeaveRequest::query().get(t.db()).await.unwrap().is_empty());
        t.app
            .htmx()
            .post(
                "/hr/leave",
                &[
                    ("leave_type", "annual"),
                    ("from_date", "2026-11-09"),
                    ("to_date", "2026-11-12"),
                ],
            )
            .await
            .assert_ok();
        let request = LeaveRequest::query().first(t.db()).await.unwrap().unwrap();
        assert_eq!(request.employee_id, employee.id);
        assert_eq!(request.status, LeaveStatus::Requested);
        assert_eq!(request.requested_by, Some(account.id));

        t.app
            .get("/hr/leave")
            .await
            .assert_ok()
            .assert_see("Annual leave")
            .assert_see("2026-11-09 - 2026-11-12")
            .assert_see("Waiting for approval")
            .assert_see("Cancel");
        t.app
            .htmx()
            .post(
                &format!("/hr/leave/{}/cancel", request.id),
                &[("reason", "Plans changed")],
            )
            .await
            .assert_ok();
        let cancelled = LeaveRequest::find_or_404(t.db(), request.id).await.unwrap();
        assert_eq!(cancelled.status, LeaveStatus::Cancelled);
        assert_eq!(cancelled.decision_reason.as_deref(), Some("Plans changed"));
        t.app
            .get("/hr/leave")
            .await
            .assert_see("Cancelled")
            .assert_see("Plans changed");
    })
    .await;
}

/// The manager decides from the queue: it lists the requests of the people in
/// the manager's scopes only; approve works; reject needs a reason; a sick
/// leave is approved only after its sick note; a user with no
/// `leave.approve` has no queue; a request already decided leaves it.
#[renox::test]
async fn the_manager_decides_the_queue_and_a_sick_leave_needs_its_note() {
    let t = support::boot().await;
    let (_employee, account) = t.ravi().await;
    account.assign_role(t.db(), "staff").await.unwrap();
    let hana = t.user_in_branch("Hana", "hr").await;
    let elsewhere = t.user("Mark").await;
    elsewhere
        .assign_role_in(t.db(), "hr", &branch_scope_of(&other_branch()))
        .await
        .unwrap();

    t.on(nov(1), async {
        t.app.acting_as(&account);
        for (kind, from, to) in [
            ("annual", "2026-11-09", "2026-11-10"),
            ("unpaid", "2026-11-16", "2026-11-17"),
            ("sick", "2026-11-03", "2026-11-04"),
        ] {
            t.app
                .htmx()
                .post(
                    "/hr/leave",
                    &[("leave_type", kind), ("from_date", from), ("to_date", to)],
                )
                .await
                .assert_ok();
        }
        let all = LeaveRequest::query()
            .order_by(LeaveRequest::ID)
            .get(t.db())
            .await
            .unwrap();
        let (annual, unpaid, sick) = (&all[0], &all[1], &all[2]);

        t.app.acting_as(&account);
        t.app.get("/hr/leave/queue").await.assert_forbidden();
        t.app.acting_as(&elsewhere);
        t.app
            .get("/hr/leave/queue")
            .await
            .assert_ok()
            .assert_see("Nothing to decide")
            .assert_dont_see("2026-11-09 - 2026-11-10");

        t.app.acting_as(&hana);
        t.app
            .get("/hr/leave/queue")
            .await
            .assert_ok()
            .assert_see("Annual leave")
            .assert_see("Unpaid leave")
            .assert_see("Sick leave")
            .assert_see("Approve")
            .assert_see("Reject")
            .assert_see("Attach sick note");

        // Approve one.
        t.app
            .htmx()
            .post(&format!("/hr/leave/{}/approve", annual.id), &[("x", "y")])
            .await
            .assert_ok();
        assert_eq!(
            LeaveRequest::find_or_404(t.db(), annual.id)
                .await
                .unwrap()
                .status,
            LeaveStatus::Approved
        );
        // Reject needs a reason.
        t.app
            .htmx()
            .post(
                &format!("/hr/leave/{}/reject", unpaid.id),
                &[("reason", " ")],
            )
            .await
            .assert_status(422);
        t.app
            .htmx()
            .post(
                &format!("/hr/leave/{}/reject", unpaid.id),
                &[("reason", "Too many are away that week")],
            )
            .await
            .assert_ok();
        let rejected = LeaveRequest::find_or_404(t.db(), unpaid.id).await.unwrap();
        assert_eq!(rejected.status, LeaveStatus::Rejected);

        // The sick leave waits for its note.
        t.app
            .htmx()
            .post(&format!("/hr/leave/{}/approve", sick.id), &[("x", "y")])
            .await
            .assert_status(422);
        t.app
            .htmx()
            .post(
                &format!("/hr/leave/{}/sick-note", sick.id),
                &[("document", "77")],
            )
            .await
            .assert_ok();
        t.app
            .htmx()
            .post(&format!("/hr/leave/{}/approve", sick.id), &[("x", "y")])
            .await
            .assert_ok();
        assert_eq!(
            LeaveRequest::find_or_404(t.db(), sick.id)
                .await
                .unwrap()
                .status,
            LeaveStatus::Approved
        );

        // Nothing waits any more; the decisions are listed in the same grid, and the
        // approved leave that has not started can still be cancelled.
        t.app
            .get("/hr/leave/queue")
            .await
            .assert_ok()
            .assert_see("Too many are away that week")
            .assert_see("Rejected")
            .assert_see("Approved")
            .assert_see("Cancel");
        // The employee sees the answer, with the reason.
        t.app.acting_as(&account);
        t.app
            .get("/hr/leave")
            .await
            .assert_see("Too many are away that week");
        // Already decided: a second approve is refused.
        t.app.acting_as(&hana);
        t.app
            .htmx()
            .post(&format!("/hr/leave/{}/approve", annual.id), &[("x", "y")])
            .await
            .assert_status(422);
    })
    .await;
}
