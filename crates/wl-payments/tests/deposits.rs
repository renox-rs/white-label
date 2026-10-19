//! The minimal deposit base and "refund failed" on a refunded deposit: the
//! acceptance check AC5 of #99.

mod support;

use renox::prelude::Model;
use wl_kernel::PartyId;
use wl_kernel::payments::{
    Direction, PaymentMethod, RefundFailedFrom, RefundFailureKind, RefundSource, payment_recorder,
};
use wl_payments::deposit::DepositStatus;
use wl_payments::refund::RefundStatus;
use wl_payments::{Deposit, Payment, Refund, RefundAttempt, permissions as perm};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot};

async fn credit(t: &Booted, party: PartyId) -> i64 {
    payment_recorder(t.state())
        .credit_balance(t.db(), party, "AED")
        .await
        .unwrap()
}

/// AC5: a received payment of AED 8,000.00 is held as a deposit (the credit
/// drops to 0); settled and refunded by transfer, approved by B and paid by
/// A; the bank returns it and A fails it. The deposit stays `refunded`, its
/// page says "Refund failed" with a link to the refund, the credit is AED
/// 8,000.00 again and `RefundFailed` names the deposit.
#[renox::test]
async fn a_failed_deposit_refund_leaves_the_deposit_refunded_and_restores_the_credit() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let viewer = t.user("Vera", &[perm::VIEW, perm::DEPOSITS_VIEW]).await;
    t.accountant("Cara").await;
    let payment = t.receive(BLUE_DUNES, 800_000).await;
    assert_eq!(credit(&t, BLUE_DUNES).await, 800_000);

    let deposit = t
        .hold_deposit(&a, BLUE_DUNES, 800_000, Some(&payment))
        .await;
    assert_eq!(deposit.status, DepositStatus::Held);
    assert_eq!(deposit.held_amount, 800_000);
    assert_eq!(deposit.received_payment_id, Some(payment.id));
    assert_eq!(credit(&t, BLUE_DUNES).await, 0);

    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    // Cash is not a way to pay a deposit back; a method is required.
    let pending = t.deposit(deposit.id).await;
    assert_eq!(pending.status, DepositStatus::SettlementPending);
    t.deposit_action(&a, &pending, "refund", &[("method", "cash")])
        .await
        .assert_status(422);
    t.deposit_action(&a, &pending, "refund", &[])
        .await
        .assert_status(422);
    assert_eq!(
        t.deposit(deposit.id).await.status,
        DepositStatus::SettlementPending
    );
    t.deposit_action(&a, &pending, "refund", &[("method", "transfer")])
        .await
        .assert_ok();

    let refunded = t.deposit(deposit.id).await;
    assert_eq!(refunded.status, DepositStatus::Refunded);
    assert_eq!(refunded.refund_method, Some(PaymentMethod::Transfer));
    let refund = Refund::find_or_404(t.db(), refunded.refund_id.expect("a refund"))
        .await
        .unwrap();
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.amount, 800_000);
    assert_eq!(
        refund.source(),
        Some(RefundSource::Deposit(deposit.deposit_id()))
    );
    assert_eq!(refund.method, Some(PaymentMethod::Transfer));
    assert_eq!(credit(&t, BLUE_DUNES).await, 0);

    // The requester of the deposit refund cannot approve it (FIN.ACL.04).
    t.refund_action(&a, &refund, "approve", &[])
        .await
        .assert_status(422)
        .assert_see("FIN.ACL.04");
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund_action(
        &a,
        &refund,
        "pay",
        &[("method", "transfer"), ("reference", "BANK-8")],
    )
    .await
    .assert_ok();
    let paid = t.refund(refund.id).await;
    assert_eq!(paid.status, RefundStatus::Paid);
    let voucher = Payment::find_or_404(t.db(), paid.payment_id.expect("a voucher"))
        .await
        .unwrap();
    assert_eq!(voucher.direction, Direction::Out);
    assert_eq!(voucher.amount, 800_000);
    // The deposit's money was never in the credit: paying it takes nothing.
    assert_eq!(credit(&t, BLUE_DUNES).await, 0);
    assert_eq!(
        t.refund_paid.all()[0].source,
        RefundSource::Deposit(deposit.deposit_id())
    );

    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/deposits/{}", deposit.id))
        .await
        .assert_ok()
        .assert_dont_see("Refund failed");

    t.fail_refund(&a, &paid, "bank_return", "account closed")
        .await
        .assert_ok();

    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
    let after = t.deposit(deposit.id).await;
    assert_eq!(after.status, DepositStatus::Refunded);
    assert!(after.refund_failed(t.db()).await.unwrap());
    assert_eq!(credit(&t, BLUE_DUNES).await, 800_000);
    let events = t.refund_failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].source,
        RefundSource::Deposit(deposit.deposit_id())
    );
    assert_eq!(events[0].kind, RefundFailureKind::BankReturn);
    assert_eq!(events[0].from, RefundFailedFrom::Paid);
    assert_eq!(events[0].amount, 800_000);
    assert_eq!(
        RefundAttempt::of_refund(t.db(), refund.id)
            .await
            .unwrap()
            .len(),
        1
    );

    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/deposits/{}", deposit.id))
        .await
        .assert_ok()
        .assert_see("Refund failed")
        .assert_see(&format!("/refunds/{}", refund.id));
    t.app.get("/deposits").await.assert_ok();
    let nobody = t.user("Nora", &[perm::VIEW]).await;
    t.app.acting_as(&nobody);
    t.app.get("/deposits").await.assert_forbidden();
}

/// Holding a deposit: the guards, the allocation of what is in, and the
/// permissions of the actions.
#[renox::test]
async fn holding_a_deposit_is_guarded_and_allocates_what_is_in() {
    let t = boot().await;
    let a = t.user_a().await;
    let clerk = t.user("Cora", &[perm::VIEW, perm::DEPOSITS_VIEW]).await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let other = t.receive(QUIET_SANDS, 300_000).await;

    t.hold_deposit_response(&clerk, BLUE_DUNES, 500_000, None)
        .await
        .assert_forbidden();
    t.hold_deposit_response(&a, BLUE_DUNES, 0, None)
        .await
        .assert_status(422);
    // Another party's payment.
    t.hold_deposit_response(&a, BLUE_DUNES, 500_000, Some(&other))
        .await
        .assert_status(422);
    assert_eq!(Deposit::query().count(t.db()).await.unwrap(), 0);

    // A deposit of AED 5,000.00 with AED 3,000.00 in: held amount 3,000.00.
    let deposit = t
        .hold_deposit(&a, BLUE_DUNES, 500_000, Some(&payment))
        .await;
    assert_eq!(deposit.amount, 500_000);
    assert_eq!(deposit.held_amount, 300_000);
    assert_eq!(credit(&t, BLUE_DUNES).await, 0);

    // Only a settled deposit is refunded; a clerk may not settle.
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
    t.deposit_action(&clerk, &deposit, "start_settlement", &[])
        .await
        .assert_forbidden();
    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_status(422);
    // A deposit that holds nothing is not refunded.
    let empty = t.hold_deposit(&a, QUIET_SANDS, 100_000, None).await;
    assert_eq!(empty.held_amount, 0);
    t.deposit_action(&a, &empty, "start_settlement", &[])
        .await
        .assert_ok();
    t.deposit_action(&a, &empty, "refund", &[("method", "cheque")])
        .await
        .assert_status(422);
    assert_eq!(Refund::query().count(t.db()).await.unwrap(), 0);
}
