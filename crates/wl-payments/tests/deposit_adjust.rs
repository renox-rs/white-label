//! BL-PAY-22 (#115): a lease amendment or renewal changes the deposit while it
//! is held. The deposit does not leave `held`: an increase raises a top-up
//! charge, a decrease becomes a part refund through the normal Refund flow
//! with its approval. The held amount follows the money actually received or
//! paid back.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::deposits::{AdjustDeposit, deposits};
use wl_kernel::payments::RefundSource;
use wl_payments::deposit::DepositStatus;
use wl_payments::deposit_adjust::{AdjustmentKind, AdjustmentStatus, DepositAdjustment};
use wl_payments::followup::FollowupKind;
use wl_payments::refund::RefundStatus;
use wl_payments::{Deposit, FollowupTask, Refund};

use support::{BLUE_DUNES, Booted, boot};

fn day(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).unwrap()
}

/// A held deposit of `amount` fils paid in full.
async fn held_deposit(t: &Booted, by: &renox::auth::User, amount: i64) -> Deposit {
    let payment = t.receive(BLUE_DUNES, amount).await;
    t.hold_deposit(by, BLUE_DUNES, amount, Some(&payment)).await
}

async fn adjustments(t: &Booted, deposit: &Deposit) -> Vec<DepositAdjustment> {
    DepositAdjustment::of_deposit(t.db(), deposit.id)
        .await
        .unwrap()
}

/// The part refund of the latest adjustment.
async fn part_refund(t: &Booted, deposit: &Deposit) -> Refund {
    let adjustment = DepositAdjustment::latest(t.db(), deposit.id)
        .await
        .unwrap()
        .expect("an adjustment");
    t.refund(adjustment.refund_id.expect("a part refund")).await
}

/// AC1: a held deposit of AED 5,000 and a lease amendment to AED 6,000
/// effective 2030-05-01: `adjust` raises a top-up charge of AED 1,000 dated
/// 2030-05-01; once its payment is allocated the held amount is AED 6,000, the
/// adjustment is settled and `DepositAdjusted` was emitted. The deposit stays
/// held throughout.
#[renox::test]
async fn an_increase_raises_a_top_up_charge_and_the_held_amount_rises_with_its_payment() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = held_deposit(&t, &a, 500_000).await;

    t.adjust_deposit(&a, &deposit, "6000.00", "2", "2030-05-01")
        .await
        .assert_ok();

    let adjusted = t.deposit(deposit.id).await;
    assert_eq!(adjusted.status, DepositStatus::Held);
    assert_eq!(adjusted.amount, 600_000);
    assert_eq!(adjusted.held_amount, 500_000);
    // The charge: the difference, due on the effective date.
    let charges = t.top_ups.charges();
    assert_eq!(charges.len(), 1);
    assert_eq!(charges[0].amount, 100_000);
    assert_eq!(charges[0].due, day(2030, 5, 1));
    assert_eq!(charges[0].source_version, 2);
    assert_eq!(charges[0].party, BLUE_DUNES);
    assert_eq!(charges[0].deposit, deposit.deposit_id());
    let rows = adjustments(&t, &deposit).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, AdjustmentKind::TopUp);
    assert_eq!(rows[0].status, AdjustmentStatus::Open);
    assert_eq!((rows[0].old_amount, rows[0].new_amount), (500_000, 600_000));
    assert!(rows[0].charge().is_some());
    // The event, with the charge.
    let events = t.deposit_adjusted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].old_amount, 500_000);
    assert_eq!(events[0].new_amount, 600_000);
    assert_eq!(events[0].source_version, 2);
    assert_eq!(events[0].effective_on, day(2030, 5, 1));
    assert_eq!(events[0].top_up_charge, rows[0].charge());
    assert_eq!(events[0].part_refund, None);

    // The party pays the top-up; the payment is allocated to the deposit.
    let payment = t.receive(BLUE_DUNES, 100_000).await;
    t.allocate_to_deposit(&payment, &adjusted, 100_000)
        .await
        .unwrap();

    let held = t.deposit(deposit.id).await;
    assert_eq!(held.held_amount, 600_000);
    assert_eq!(held.status, DepositStatus::Held);
    assert_eq!(
        adjustments(&t, &deposit).await[0].status,
        AdjustmentStatus::Settled
    );
}

/// AC2: a held deposit of AED 6,000 changed to AED 5,500: a refund request of
/// AED 500 is created with the deposit as its source; user A requests, user B
/// approves, it is paid, and the held amount is AED 5,500. Until it is paid the
/// held amount does not fall.
#[renox::test]
async fn a_decrease_asks_for_a_part_refund_and_the_held_amount_falls_when_it_is_paid() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let deposit = held_deposit(&t, &a, 600_000).await;

    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();

    let refund = part_refund(&t, &deposit).await;
    assert_eq!(refund.amount, 50_000);
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.requested_by, Some(a.id));
    assert_eq!(
        refund.source(),
        Some(RefundSource::Deposit(deposit.deposit_id()))
    );
    let adjusted = t.deposit(deposit.id).await;
    assert_eq!(adjusted.status, DepositStatus::Held);
    assert_eq!(adjusted.amount, 550_000);
    assert_eq!(
        adjusted.held_amount, 600_000,
        "not reduced before the refund is paid"
    );
    assert!(t.top_ups.charges().is_empty());
    let events = t.deposit_adjusted.all();
    assert_eq!(events[0].part_refund, Some(refund.refund_id()));
    assert_eq!(events[0].top_up_charge, None);

    // The requester cannot approve it (FIN.ACL.04).
    t.refund_action(&a, &refund, "approve", &[])
        .await
        .assert_status(422);
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    assert_eq!(t.deposit(deposit.id).await.held_amount, 600_000);
    t.refund_action(
        &b,
        &t.refund(refund.id).await,
        "pay",
        &[("method", "transfer"), ("reference", "TRF-55")],
    )
    .await
    .assert_ok();

    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Paid);
    let paid = t.deposit(deposit.id).await;
    assert_eq!(paid.held_amount, 550_000);
    assert_eq!(paid.status, DepositStatus::Held);
    assert_eq!(
        adjustments(&t, &deposit).await[0].status,
        AdjustmentStatus::Settled
    );
}

/// AC3: when that refund `fail`s after payment, the held amount goes back to
/// AED 6,000; when the failed refund is cancelled, the amount moves to the
/// party's credit and the held amount stays reduced.
#[renox::test]
async fn a_failed_part_refund_restores_the_held_amount_and_a_cancelled_one_keeps_it_reduced() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let deposit = held_deposit(&t, &a, 600_000).await;
    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();
    let refund = part_refund(&t, &deposit).await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund_action(
        &b,
        &t.refund(refund.id).await,
        "pay",
        &[("method", "transfer"), ("reference", "TRF-56")],
    )
    .await
    .assert_ok();
    assert_eq!(t.deposit(deposit.id).await.held_amount, 550_000);

    t.fail_refund(
        &b,
        &t.refund(refund.id).await,
        "bank_return",
        "Account closed",
    )
    .await
    .assert_ok();

    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
    assert_eq!(t.deposit(deposit.id).await.held_amount, 600_000);
    // The adjustment is not settled while the refund is.
    assert_eq!(
        adjustments(&t, &deposit).await[0].status,
        AdjustmentStatus::Open
    );
    // A new adjustment is still refused.
    t.adjust_deposit(&a, &deposit, "5000.00", "4", "2030-07-01")
        .await
        .assert_status(422);

    t.refund_action(
        &b,
        &t.refund(refund.id).await,
        "cancel",
        &[("reason", "The party left")],
    )
    .await
    .assert_ok();
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Cancelled);
    assert_eq!(t.deposit(deposit.id).await.held_amount, 550_000);
    let rows = adjustments(&t, &deposit).await;
    assert_eq!(rows[0].status, AdjustmentStatus::Closed);
    assert_eq!(rows[0].closed_reason.as_deref(), Some("cancelled"));
    // The adjustment is closed, so the next one is allowed.
    t.adjust_deposit(&a, &deposit, "5000.00", "4", "2030-07-01")
        .await
        .assert_ok();
}

/// AC4: a second `adjust` while the AED 500 refund is `requested` is refused;
/// once the refund is settled the next one is allowed.
#[renox::test]
async fn a_second_adjustment_waits_until_the_first_is_settled() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let deposit = held_deposit(&t, &a, 600_000).await;
    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();

    t.adjust_deposit(&a, &deposit, "5000.00", "4", "2030-07-01")
        .await
        .assert_status(422);
    assert_eq!(adjustments(&t, &deposit).await.len(), 1);
    assert_eq!(t.deposit(deposit.id).await.amount, 550_000);

    let refund = part_refund(&t, &deposit).await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund_action(
        &b,
        &t.refund(refund.id).await,
        "pay",
        &[("method", "transfer"), ("reference", "TRF-57")],
    )
    .await
    .assert_ok();
    t.adjust_deposit(&a, &deposit, "5000.00", "4", "2030-07-01")
        .await
        .assert_ok();
    assert_eq!(adjustments(&t, &deposit).await.len(), 2);
}

/// A top-up unpaid also blocks the next adjustment; paying it frees it.
#[renox::test]
async fn an_unpaid_top_up_blocks_the_next_adjustment() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = held_deposit(&t, &a, 500_000).await;
    t.adjust_deposit(&a, &deposit, "6000.00", "2", "2030-05-01")
        .await
        .assert_ok();
    t.adjust_deposit(&a, &deposit, "7000.00", "3", "2030-06-01")
        .await
        .assert_status(422);
    let payment = t.receive(BLUE_DUNES, 100_000).await;
    t.allocate_to_deposit(&payment, &deposit, 100_000)
        .await
        .unwrap();
    t.adjust_deposit(&a, &deposit, "7000.00", "3", "2030-06-01")
        .await
        .assert_ok();
}

/// A part refund that the approver rejects leaves the held amount, and the
/// difference is paid back at settlement.
#[renox::test]
async fn a_rejected_part_refund_leaves_the_held_amount() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let deposit = held_deposit(&t, &a, 600_000).await;
    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();
    let refund = part_refund(&t, &deposit).await;

    t.refund_action(
        &b,
        &refund,
        "reject",
        &[("reason", "Not agreed with the tenant")],
    )
    .await
    .assert_ok();

    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Rejected);
    assert_eq!(t.deposit(deposit.id).await.held_amount, 600_000);
    let rows = adjustments(&t, &deposit).await;
    assert_eq!(rows[0].status, AdjustmentStatus::Closed);
    assert_eq!(rows[0].closed_reason.as_deref(), Some("rejected"));

    // The difference is paid back at settlement: the refund is for the held
    // amount.
    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    let settled = t.deposit(deposit.id).await;
    let final_refund = t.refund(settled.refund_id.expect("a refund")).await;
    assert_eq!(final_refund.amount, 600_000);
}

/// Edge: the same amount records no adjustment.
#[renox::test]
async fn the_same_amount_records_nothing() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = held_deposit(&t, &a, 500_000).await;
    t.adjust_deposit(&a, &deposit, "5000.00", "2", "2030-05-01")
        .await
        .assert_ok();
    assert!(adjustments(&t, &deposit).await.is_empty());
    assert!(t.deposit_adjusted.all().is_empty());
    assert!(t.top_ups.charges().is_empty());
    assert_eq!(t.deposit(deposit.id).await.amount, 500_000);
}

/// Edge: settlement before the top-up is paid uses the held amount; the
/// unpaid top-up stays.
#[renox::test]
async fn settlement_before_the_top_up_is_paid_uses_the_held_amount() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = held_deposit(&t, &a, 500_000).await;
    t.adjust_deposit(&a, &deposit, "6000.00", "2", "2030-05-01")
        .await
        .assert_ok();
    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_ok();
    let refunded = t.deposit(deposit.id).await;
    let refund = t.refund(refunded.refund_id.expect("a refund")).await;
    assert_eq!(refund.amount, 500_000);
    assert_eq!(
        adjustments(&t, &deposit).await[0].status,
        AdjustmentStatus::Open
    );
}

/// Edge: a part refund that is not settled holds back the refund at
/// settlement and the transfer.
#[renox::test]
async fn an_unsettled_part_refund_holds_back_the_settlement_refund() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = held_deposit(&t, &a, 600_000).await;
    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();
    t.deposit_page(
        &a,
        &deposit,
        "transfer",
        &[("new_holder", "P-00002"), ("receipt", "RCPT-1")],
    )
    .await
    .assert_status(422);
    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    t.deposit_action(&a, &deposit, "refund", &[("method", "transfer")])
        .await
        .assert_status(422);
}

/// Edge: when nothing was paid in above the new amount there is nothing to
/// refund: the amount is lowered and the adjustment is settled at once.
#[renox::test]
async fn a_decrease_below_what_was_paid_in_needs_no_refund() {
    let t = boot().await;
    let a = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let deposit = t
        .hold_deposit(&a, BLUE_DUNES, 600_000, Some(&payment))
        .await;
    assert_eq!(deposit.held_amount, 300_000);

    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();

    let rows = adjustments(&t, &deposit).await;
    assert_eq!(rows[0].kind, AdjustmentKind::PartRefund);
    assert_eq!(rows[0].difference, 0);
    assert_eq!(rows[0].refund_id, None);
    assert_eq!(rows[0].status, AdjustmentStatus::Settled);
    assert_eq!(t.deposit(deposit.id).await.amount, 550_000);
    assert_eq!(wl_payments::Refund::query().count(t.db()).await.unwrap(), 0);
}

/// The system adjusts through the kernel contract, as a lease amendment or
/// renewal `activate` does; a source without a held deposit and a deposit
/// being settled are refused.
#[renox::test]
async fn the_source_agreement_adjusts_the_deposit_through_the_kernel_contract() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = held_deposit(&t, &a, 500_000).await;
    let request = AdjustDeposit {
        source: Subject::new("property.lease", 9),
        new_amount: 650_000,
        source_version: 5,
        effective_on: day(2030, 8, 1),
    };

    let adjusted = deposits(t.state())
        .adjust(t.state(), request.clone(), None)
        .await
        .unwrap()
        .expect("adjusted");
    assert_eq!(adjusted.new_amount, 650_000);
    assert_eq!(adjusted.source_version, 5);
    assert!(adjusted.top_up_charge.is_some());
    assert_eq!(t.deposit(deposit.id).await.amount, 650_000);
    let rows = adjustments(&t, &deposit).await;
    assert_eq!(rows[0].adjusted_by, None);

    // The same amount again: nothing is recorded.
    let same = AdjustDeposit {
        new_amount: 650_000,
        ..request.clone()
    };
    assert!(
        deposits(t.state())
            .adjust(t.state(), same, None)
            .await
            .unwrap()
            .is_none()
    );

    // No held deposit for another source.
    let other = AdjustDeposit {
        source: Subject::new("property.lease", 77),
        ..request
    };
    assert!(
        deposits(t.state())
            .adjust(t.state(), other, None)
            .await
            .is_err()
    );
}

/// When nothing can schedule the top-up charge, the accountant gets a task to
/// raise it.
#[renox::test]
async fn the_accountant_raises_the_top_up_when_nothing_can_schedule_it() {
    let t = boot().await;
    let a = t.user_a().await;
    t.accountant("Cara").await;
    t.top_ups.make_charges(false);
    let deposit = held_deposit(&t, &a, 500_000).await;

    t.adjust_deposit(&a, &deposit, "6000.00", "2", "2030-05-01")
        .await
        .assert_ok();

    let tasks = FollowupTask::open_for(t.db(), "accountant").await.unwrap();
    let task = tasks
        .iter()
        .find(|task| task.kind == FollowupKind::RaiseTopUp)
        .expect("a task");
    assert_eq!(task.amount, 100_000);
    assert!(task.text.contains("AED 1,000.00"), "{}", task.text);
    assert!(task.text.contains("2030-05-01"), "{}", task.text);
    assert_eq!(adjustments(&t, &deposit).await[0].charge(), None);
    assert_eq!(t.deposit_adjusted.all().len(), 1);
}

/// Only a held deposit is adjusted, by a deposit manager, with a version and a
/// valid amount.
#[renox::test]
async fn only_a_held_deposit_is_adjusted_by_a_deposit_manager_with_a_version() {
    let t = boot().await;
    let a = t.user_a().await;
    let viewer = t
        .user("Vera", &[wl_payments::permissions::DEPOSITS_VIEW])
        .await;
    let deposit = held_deposit(&t, &a, 500_000).await;

    t.adjust_deposit(&viewer, &deposit, "6000.00", "2", "2030-05-01")
        .await
        .assert_forbidden();
    // No version, no date.
    t.deposit_page(
        &a,
        &deposit,
        "adjust",
        &[("new_amount", "6000.00"), ("effective_on", "2030-05-01")],
    )
    .await
    .assert_status(422);
    t.deposit_page(
        &a,
        &deposit,
        "adjust",
        &[("new_amount", "6000.00"), ("source_version", "2")],
    )
    .await
    .assert_status(422);
    t.adjust_deposit(&a, &deposit, "lots", "2", "2030-05-01")
        .await
        .assert_status(422);
    assert!(adjustments(&t, &deposit).await.is_empty());

    t.deposit_action(&a, &deposit, "start_settlement", &[])
        .await
        .assert_ok();
    t.adjust_deposit(&a, &deposit, "6000.00", "2", "2030-05-01")
        .await
        .assert_status(422);
    assert_eq!(
        t.deposit(deposit.id).await.status,
        DepositStatus::SettlementPending
    );
}

/// Edge case: the part refund is money out and waits under `PTY.ACL.03`: it
/// is requested and approved, but not paid while the party is blocked (the
/// held amount stays); once the party is free it is paid and the held amount
/// falls.
#[renox::test]
async fn a_part_refund_for_a_blocked_party_waits_until_it_is_unblocked() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let deposit = held_deposit(&t, &a, 600_000).await;
    t.adjust_deposit(&a, &deposit, "5500.00", "3", "2030-06-01")
        .await
        .assert_ok();
    let refund = part_refund(&t, &deposit).await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();

    t.blocked.block(BLUE_DUNES);
    let pay = [("method", "transfer"), ("reference", "TRF-56")];
    t.refund_action(&b, &t.refund(refund.id).await, "pay", &pay)
        .await
        .assert_status(422);
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Approved);
    assert_eq!(t.deposit(deposit.id).await.held_amount, 600_000);
    assert_eq!(
        adjustments(&t, &deposit).await[0].status,
        AdjustmentStatus::Open
    );

    t.blocked.free(BLUE_DUNES);
    t.refund_action(&b, &t.refund(refund.id).await, "pay", &pay)
        .await
        .assert_ok();
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Paid);
    assert_eq!(t.deposit(deposit.id).await.held_amount, 550_000);
}
