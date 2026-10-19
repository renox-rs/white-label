//! BL-PRP-15 (#265): money out to a blocked party waits for unblock or an
//! approved exception (`PTY.ACL.03`). A deposit `refund` for a blocked party
//! does not run: it becomes an exception request for the role the block guard
//! names; the refund follows when the exception is approved.

mod support;

use renox::auth::permissions::define_role;
use renox::prelude::*;
use wl_kernel::BLOCK_EXCEPTION_ACTION;
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_payments::deposit::DepositStatus;

use support::{BLUE_DUNES, Booted, boot};

/// A held deposit of AED 8,000 in settlement, and its staff user.
async fn pending(t: &Booted) -> (renox::auth::User, wl_payments::Deposit) {
    let a = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 800_000).await;
    let deposit = t.hold_deposit(&a, BLUE_DUNES, 800_000, Some(&payment)).await;
    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    (a, t.deposit(deposit.id).await)
}

/// A user of the compliance role, who may decide a block exception.
async fn compliance(t: &Booted) -> renox::auth::User {
    define_role(t.db(), "compliance", &[BLOCK_EXCEPTION_ACTION])
        .await
        .unwrap();
    let user = User::register(t.db(), "Comply", "comply@example.com", "password123")
        .await
        .unwrap();
    user.assign_role(t.db(), "compliance").await.unwrap();
    user
}

/// A refund for a blocked party creates an exception request and no refund;
/// the requester cannot approve it; when compliance approves, the refund is
/// requested and the deposit is `refunded`.
#[renox::test]
async fn a_refund_for_a_blocked_party_becomes_an_exception_request() {
    let t = boot().await;
    let (a, deposit) = pending(&t).await;
    t.blocked.block(BLUE_DUNES);

    let asked = t
        .deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await;
    asked.assert_ok();
    assert!(asked.text().starts_with("exception:"), "{}", asked.text());

    let waiting = t.deposit(deposit.id).await;
    assert_eq!(waiting.status, DepositStatus::SettlementPending);
    assert_eq!(waiting.refund_id, None);
    let requests = ApprovalRequest::pending_about(t.db(), &deposit.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    assert_eq!(requests[0].transition, "refund");

    // The requester is not the approver (and holds no compliance role).
    let refused = approve(t.state(), requests[0].id, &a, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(
        t.deposit(deposit.id).await.status,
        DepositStatus::SettlementPending
    );

    let officer = compliance(&t).await;
    let decided = approve(t.state(), requests[0].id, &officer, "approved")
        .await
        .unwrap();
    assert!(decided.is_some());
    let refunded = t.deposit(deposit.id).await;
    assert_eq!(refunded.status, DepositStatus::Refunded);
    assert!(refunded.refund_id.is_some());
}

/// A party that is not blocked is refunded as before, and a second request for
/// the same blocked deposit is refused while the first is pending.
#[renox::test]
async fn an_unblocked_party_is_refunded_and_a_second_exception_waits() {
    let t = boot().await;
    let (a, deposit) = pending(&t).await;
    t.blocked.block(BLUE_DUNES);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    // Asked twice: the second is refused.
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    assert_eq!(
        ApprovalRequest::pending_about(t.db(), &deposit.subject())
            .await
            .unwrap()
            .len(),
        1
    );

    // Unblocked, the refund runs at once.
    t.blocked.free(BLUE_DUNES);
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::Refunded);
}

/// Money in is never refused: a payment from a blocked party is received.
#[renox::test]
async fn a_blocked_party_still_pays() {
    let t = boot().await;
    t.blocked.block(BLUE_DUNES);
    let payment = t.receive(BLUE_DUNES, 100_000).await;
    assert_eq!(payment.amount, 100_000);
}
