//! The leave request lifecycle, the base of BL-HRM-02 (#192): `request`,
//! `approve`, `reject`, `cancel` and `complete` through the services, with
//! `LeaveApproved` and `LeaveCancelled` after the commit, and the unpaid
//! days that `approve` marks.

mod support;
use renox::prelude::*;

use wl_hr::leave_service as leave;
use wl_hr::{LeaveRequest, LeaveStatus};

use support::{Booted, branch_scope_of, nov, other_branch};

/// Ravi's leave from `from` to `to` of `kind`, requested on his own account.
async fn requested(
    t: &Booted,
    employee: &wl_hr::Employee,
    account: i64,
    kind: &str,
    from: u32,
    to: u32,
) -> LeaveRequest {
    leave::request(t.state(), account, employee.id, kind, nov(from), nov(to))
        .await
        .unwrap()
}

/// The request as stored now.
async fn stored(t: &Booted, request: &LeaveRequest) -> LeaveRequest {
    LeaveRequest::find_or_404(t.db(), request.id).await.unwrap()
}

#[renox::test]
async fn an_employee_asks_hr_approves_and_the_event_goes_out() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    let request = requested(&t, &employee, account.id, "annual", 9, 13).await;
    assert_eq!(request.status, LeaveStatus::Requested);
    assert_eq!(request.requested_by, Some(account.id));
    assert!(t.leave_approved.all().is_empty());

    // The employee cannot approve their own leave.
    assert!(
        leave::approve(t.state(), account.id, request.id)
            .await
            .is_err()
    );
    assert_eq!(stored(&t, &request).await.status, LeaveStatus::Requested);

    let approved = leave::approve(t.state(), hr.id, request.id).await.unwrap();
    assert_eq!(approved.status, LeaveStatus::Approved);
    assert_eq!(approved.decided_by, Some(hr.id));
    assert_eq!(approved.unpaid_days, 0);
    let events = t.leave_approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, employee.employee_id());
    assert_eq!(events[0].request, request.id);
    assert_eq!((events[0].from, events[0].to), (nov(9), nov(13)));
}

#[renox::test]
async fn hr_of_another_branch_cannot_approve() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let request = requested(&t, &employee, account.id, "annual", 9, 13).await;
    let elsewhere = t.user("Omar").await;
    elsewhere
        .assign_role_in(t.db(), "hr", &branch_scope_of(&other_branch()))
        .await
        .unwrap();
    let refused = leave::approve(t.state(), elsewhere.id, request.id).await;
    assert!(matches!(refused, Err(renox::Error::Forbidden)));
    assert!(t.leave_approved.all().is_empty());
}

#[renox::test]
async fn a_rejection_needs_a_reason_and_keeps_it() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    let request = requested(&t, &employee, account.id, "annual", 9, 13).await;
    assert!(
        leave::reject(t.state(), hr.id, request.id, "  ")
            .await
            .is_err()
    );
    let rejected = leave::reject(t.state(), hr.id, request.id, "peak week")
        .await
        .unwrap();
    assert_eq!(rejected.status, LeaveStatus::Rejected);
    assert_eq!(rejected.decision_reason.as_deref(), Some("peak week"));
    // A rejected request is settled.
    assert!(leave::approve(t.state(), hr.id, request.id).await.is_err());
}

#[renox::test]
async fn cancel_works_until_the_start_date_and_releases_the_calendars() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    let request = requested(&t, &employee, account.id, "annual", 9, 13).await;
    leave::approve(t.state(), hr.id, request.id).await.unwrap();

    // On 10 Nov the leave has started.
    let late = t
        .on(
            nov(10),
            leave::cancel(t.state(), account.id, request.id, ""),
        )
        .await;
    assert!(late.is_err());
    assert_eq!(stored(&t, &request).await.status, LeaveStatus::Approved);

    let other = requested(&t, &employee, account.id, "annual", 23, 24).await;
    leave::approve(t.state(), hr.id, other.id).await.unwrap();
    // Before it starts the employee may cancel it.
    let cancelled = t
        .on(
            nov(10),
            leave::cancel(t.state(), account.id, other.id, "plans changed"),
        )
        .await
        .unwrap();
    assert_eq!(cancelled.status, LeaveStatus::Cancelled);
    let events = t.leave_cancelled.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].request, other.id);
    // Someone else without `leave.approve` may not.
    let stranger = t.user("Sam").await;
    let again = requested(&t, &employee, account.id, "annual", 25, 26).await;
    assert!(matches!(
        leave::cancel(t.state(), stranger.id, again.id, "").await,
        Err(renox::Error::Forbidden)
    ));
}

#[renox::test]
async fn unpaid_leave_is_all_unpaid_and_the_period_completes_after_its_end() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    // Mon 9 to Tue 17 Nov: seven working days.
    let request = requested(&t, &employee, account.id, "unpaid", 9, 17).await;
    let approved = leave::approve(t.state(), hr.id, request.id).await.unwrap();
    assert_eq!(approved.unpaid_days, 7);

    assert_eq!(leave::complete_ended(t.state(), nov(17)).await.unwrap(), 0);
    assert_eq!(leave::complete_ended(t.state(), nov(18)).await.unwrap(), 1);
    assert_eq!(stored(&t, &request).await.status, LeaveStatus::Completed);
    assert_eq!(leave::complete_ended(t.state(), nov(18)).await.unwrap(), 0);
}

#[renox::test]
async fn a_sick_leave_requested_ahead_needs_its_note_to_be_approved() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let hr = t.user_in_branch("Hana", "hr").await;
    let request = requested(&t, &employee, account.id, "sick", 9, 10).await;
    assert!(leave::approve(t.state(), hr.id, request.id).await.is_err());
    LeaveRequest::query()
        .where_(LeaveRequest::ID.eq(request.id))
        .update(t.db(), &[("sick_note_document_id", &Some(77_i64))])
        .await
        .unwrap();
    let approved = leave::approve(t.state(), hr.id, request.id).await.unwrap();
    // Hired on 2 Nov: still in probation, so the sick days are unpaid.
    assert_eq!(approved.unpaid_days, 2);
}

#[renox::test]
async fn bad_requests_are_refused() {
    let t = support::boot().await;
    let (employee, account) = t.ravi().await;
    let backwards = leave::request(
        t.state(),
        account.id,
        employee.id,
        "annual",
        nov(13),
        nov(9),
    )
    .await;
    assert!(backwards.is_err());
    let unknown = leave::request(
        t.state(),
        account.id,
        employee.id,
        "sabbatical",
        nov(9),
        nov(13),
    )
    .await;
    assert!(unknown.is_err());
    let stranger = t.user("Sam").await;
    let not_theirs = leave::request(
        t.state(),
        stranger.id,
        employee.id,
        "annual",
        nov(9),
        nov(13),
    )
    .await;
    assert!(matches!(not_theirs, Err(renox::Error::Forbidden)));
}
