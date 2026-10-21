//! Manual points adjustments and the closing of an account by staff
//! (BL-SAL-24, #143, `SAL.ACL.07`): up to 500 points an adjustment is saved
//! with its reason; above it, and when staff close an account with points, a
//! manager other than the requester decides under "Needs your action". A
//! rejected one writes nothing and the requester is told; a returned one is
//! changed and submitted again. Every check drives the real pages, the
//! machine and the kernel's approvals.

mod support;

use renox::chrono::Months;
use renox::prelude::*;
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::notify::NotifyRecipient;
use wl_sales::notices::{ADJUSTMENT_REJECTED, ADJUSTMENT_RETURNED};
use wl_sales::permissions as perm;
use wl_sales::{
    AccountStatus, AdjustmentStatus, LoyaltyAccount, PointsAdjustment, PointsKind,
};

use support::floor::{Floor, account, movements};
use support::shop::Shop;
use support::{Booted, boot_shop};

/// The party of the member M in every test.
const M: i64 = 7;

/// The app, the programme, M with `points`, Ana who adjusts (`loyalty.adjust`)
/// and Mia (`loyalty.manage`, from the floor) who decides.
async fn world(points: i64) -> (Booted, Shop, Floor, LoyaltyAccount, User) {
    let (t, shop) = boot_shop().await;
    approvals::install(t.db()).await.unwrap();
    let floor = Floor::new(&t).await;
    floor.turn_on(&t, &[]).await;
    let m = floor.enrol(&t, M, "0501234567").await;
    if points > 0 {
        floor.earn(&t, &m, points).await;
    }
    let ana = t
        .user("Ana", &[perm::LOYALTY_VIEW, perm::LOYALTY_ADJUST])
        .await;
    t.app.acting_as(&ana);
    let m = account(&t, &m).await;
    (t, shop, floor, m, ana)
}

async fn adjust(t: &Booted, m: &LoyaltyAccount, points: &str, reason: &str, status: u16) {
    // A validation error answers a plain form post with a redirect back; the
    // 422 is what an htmx request sees.
    let request = if status == 422 { t.app.htmx() } else { t.app.request() };
    request
        .post(
            &format!("/loyalty/accounts/{}/adjust", m.id),
            &[("points", points), ("reason", reason)],
        )
        .await
        .assert_status(status);
}

async fn latest(t: &Booted, m: &LoyaltyAccount) -> PointsAdjustment {
    PointsAdjustment::of_account(t.db(), m.id).await.unwrap().remove(0)
}

#[renox::test]
async fn plus_400_is_saved_and_minus_900_waits_for_another_person_who_approves() {
    let (t, shop, floor, m, ana) = world(1000).await;

    // +400 is within the limit: saved at once, as a new lot of 12 months.
    adjust(&t, &m, "400", "Goodwill after a complaint", 303).await;
    let first = latest(&t, &m).await;
    assert_eq!(first.status, AdjustmentStatus::Applied);
    let made = movements(&t, &m, PointsKind::Adjust).await;
    assert_eq!(made.len(), 1);
    assert_eq!((made[0].points, made[0].value), (400, 400));
    assert_eq!(made[0].reason.as_deref(), Some("Goodwill after a complaint"));
    assert_eq!(made[0].user_id, Some(ana.id));
    let on = wl_kernel::documents::today();
    assert_eq!(made[0].lot_expires_on, on.checked_add_months(Months::new(12)));
    assert_eq!(account(&t, &m).await.points_balance, 1400);
    let events = shop.points.adjusted.lock().unwrap().clone();
    assert_eq!(events.len(), 1);
    assert_eq!((events[0].points, events[0].reason.as_str()), (400, "Goodwill after a complaint"));

    // -900 is above the limit: nothing is written, a request waits.
    adjust(&t, &m, "-900", "Abuse of the programme", 303).await;
    let big = latest(&t, &m).await;
    assert_eq!(big.status, AdjustmentStatus::PendingApproval);
    let request = big.approval_request_id.expect("a kernel request");
    let stored = ApprovalRequest::find(t.db(), request).await.unwrap().unwrap();
    assert_eq!(stored.status, ApprovalStatus::Pending);
    assert_eq!(movements(&t, &m, PointsKind::Adjust).await.len(), 1);
    assert_eq!(account(&t, &m).await.points_balance, 1400);

    // The requester cannot approve it.
    assert!(approvals::approve(t.state(), request, &ana, "").await.is_err());
    assert_eq!(latest(&t, &m).await.status, AdjustmentStatus::PendingApproval);

    // Another person with `loyalty.manage` can.
    approvals::approve(t.state(), request, &floor.mia, "")
        .await
        .unwrap();
    let done = latest(&t, &m).await;
    assert_eq!(done.status, AdjustmentStatus::Applied);
    assert_eq!(done.approved_by, Some(floor.mia.id));
    let all = movements(&t, &m, PointsKind::Adjust).await;
    assert_eq!(all.iter().map(|x| x.points).sum::<i64>(), 400 - 900);
    let taken: Vec<_> = all.iter().filter(|x| x.points < 0).collect();
    assert!(taken.iter().all(|x| x.approval_request_id == Some(request)));
    assert_eq!(account(&t, &m).await.points_balance, 500);
    assert_eq!(shop.points.adjusted.lock().unwrap().len(), 2);

    // Exactly 500 is still within the limit.
    adjust(&t, &m, "-500", "Points given in error", 303).await;
    assert_eq!(latest(&t, &m).await.status, AdjustmentStatus::Applied);
    assert_eq!(account(&t, &m).await.points_balance, 0);
}

#[renox::test]
async fn staff_closing_an_account_with_50_points_need_approval_and_write_an_adjust_of_minus_50() {
    let (t, _shop, floor, m, ana) = world(50).await;
    t.app
        .post(
            &format!("/loyalty/accounts/{}/close", m.id),
            &[("reason", "Abuse of the programme")],
        )
        .await
        .assert_status(303);
    let closing = latest(&t, &m).await;
    assert_eq!(closing.status, AdjustmentStatus::PendingApproval);
    assert!(closing.closes_account);
    // Nothing happened yet: the member still has the account and the points.
    let waiting = account(&t, &m).await;
    assert_eq!((waiting.status, waiting.points_balance), (AccountStatus::Active, 50));
    let request = closing.approval_request_id.unwrap();
    assert!(approvals::approve(t.state(), request, &ana, "").await.is_err());

    approvals::approve(t.state(), request, &floor.mia, "")
        .await
        .unwrap();
    let closed = account(&t, &m).await;
    assert_eq!((closed.status, closed.points_balance), (AccountStatus::Closed, 0));
    assert!(closed.closed_reason.unwrap().contains("Abuse of the programme"));
    let made = movements(&t, &m, PointsKind::Adjust).await;
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].points, -50);
    assert_eq!(latest_status(&t, &m).await, AdjustmentStatus::Applied);

    // A member with nothing left is closed at once, with no manager.
    let empty = floor.enrol(&t, 8, "0509999999").await;
    t.app.acting_as(&ana);
    t.app
        .post(
            &format!("/loyalty/accounts/{}/close", empty.id),
            &[("reason", "Asked at the counter")],
        )
        .await
        .assert_status(303);
    assert_eq!(account(&t, &empty).await.status, AccountStatus::Closed);
}

async fn latest_status(t: &Booted, m: &LoyaltyAccount) -> AdjustmentStatus {
    latest(t, m).await.status
}

#[renox::test]
async fn a_rejected_adjustment_writes_nothing_and_the_requester_is_told() {
    let (t, shop, floor, m, ana) = world(1000).await;
    adjust(&t, &m, "-900", "Abuse of the programme", 303).await;
    let request = latest(&t, &m).await.approval_request_id.unwrap();
    // A comment is required to reject.
    assert!(approvals::reject(t.state(), request, &floor.mia, " ").await.is_err());
    approvals::reject(t.state(), request, &floor.mia, "No evidence of abuse")
        .await
        .unwrap();
    let rejected = latest(&t, &m).await;
    assert_eq!(rejected.status, AdjustmentStatus::Rejected);
    assert_eq!(rejected.outcome_reason.as_deref(), Some("No evidence of abuse"));
    assert!(movements(&t, &m, PointsKind::Adjust).await.is_empty());
    assert_eq!(account(&t, &m).await.points_balance, 1000);
    let told = shop.mail.of_type(ADJUSTMENT_REJECTED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, vec![NotifyRecipient::User { id: ana.id }]);
    assert_eq!(told[0].data["reason"], "No evidence of abuse");
}

#[renox::test]
async fn a_returned_adjustment_is_changed_by_its_requester_and_submitted_again() {
    let (t, shop, floor, m, ana) = world(1000).await;
    adjust(&t, &m, "-900", "Abuse of the programme", 303).await;
    let big = latest(&t, &m).await;
    approvals::return_request(
        t.state(),
        big.approval_request_id.unwrap(),
        &floor.mia,
        "Take off less",
    )
    .await
    .unwrap();
    let returned = latest(&t, &m).await;
    assert_eq!(returned.status, AdjustmentStatus::Draft);
    assert!(returned.outcome_reason.unwrap_or_default().contains("Take off less"));
    assert_eq!(shop.mail.of_type(ADJUSTMENT_RETURNED).len(), 1);
    assert!(movements(&t, &m, PointsKind::Adjust).await.is_empty());

    // Someone else cannot change it; the requester can, within the limit.
    let other = t.user("Olga", &[perm::LOYALTY_VIEW, perm::LOYALTY_ADJUST]).await;
    t.app.acting_as(&other);
    let url = format!("/loyalty/adjustments/{}/resubmit", returned.id);
    t.app
        .post(&url, &[("points", "-400"), ("reason", "Less")])
        .await
        .assert_status(422);
    t.app.acting_as(&ana);
    t.app
        .post(&url, &[("points", "-400"), ("reason", "Less")])
        .await
        .assert_status(303);
    let done = latest(&t, &m).await;
    assert_eq!(done.status, AdjustmentStatus::Applied);
    assert_eq!(account(&t, &m).await.points_balance, 600);
    // An applied adjustment cannot be submitted again.
    t.app
        .post(&url, &[("points", "-100"), ("reason", "Again")])
        .await
        .assert_status(422);
    t.app
        .get("/loyalty/adjustments")
        .await
        .assert_ok()
        .assert_see("Applied");
}

#[renox::test]
async fn an_adjustment_needs_a_reason_points_the_member_has_and_the_permission() {
    let (t, _shop, floor, m, _ana) = world(100).await;
    adjust(&t, &m, "50", "", 422).await;
    adjust(&t, &m, "0", "Nothing", 422).await;
    adjust(&t, &m, "lots", "Typed wrong", 422).await;
    // More than the member has.
    adjust(&t, &m, "-101", "Too many", 422).await;
    assert!(PointsAdjustment::of_account(t.db(), m.id).await.unwrap().is_empty());
    assert_eq!(account(&t, &m).await.points_balance, 100);

    // The cashier does not hold `loyalty.adjust`.
    t.app.acting_as(&floor.cora);
    adjust(&t, &m, "10", "Not allowed", 403).await;
}
