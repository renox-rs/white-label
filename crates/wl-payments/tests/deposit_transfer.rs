//! BL-PAY-16 (#109): a held deposit is handed to a new holder against their
//! receipt when the source agreement passes to another holder before it ends.
//! The money passed on is an outgoing payment to the new holder (not a
//! refund), the party is told who now holds the deposit and `DepositSettled`
//! (transferred) is emitted.

mod support;

use renox::prelude::*;
use wl_kernel::deposits::DepositOutcome;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{Direction, PaymentMethod};
use wl_payments::deposit::DepositStatus;
use wl_payments::notices::DEPOSIT_TRANSFERRED;
use wl_payments::{Payment, Refund};

use support::{BLUE_DUNES, DUSK_HOLDINGS, QUIET_SANDS, QUIET_SANDS_CODE, boot};

/// AC1: a held deposit of AED 10,000.00 and a management agreement that ended:
/// `transfer` with the new holder's receipt makes it `transferred`, tells the
/// tenant, emits `DepositSettled` (transferred), and records an outgoing
/// payment of AED 10,000.00 to the new holder with the receipt as its
/// reference. No refund is made.
#[renox::test]
async fn a_held_deposit_is_handed_to_the_new_holder_against_their_receipt() {
    let t = boot().await;
    let a = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 1_000_000).await;
    let deposit = t.hold_deposit(&a, BLUE_DUNES, 1_000_000, Some(&payment)).await;
    assert_eq!(deposit.held_amount, 1_000_000);

    t.deposit_page(
        &a,
        &deposit,
        "transfer",
        &[("new_holder", QUIET_SANDS_CODE), ("receipt", "RCPT-77")],
    )
    .await
    .assert_ok();

    let done = t.deposit(deposit.id).await;
    assert_eq!(done.status, DepositStatus::Transferred);
    assert_eq!(done.transferred_to, Some(QUIET_SANDS.0));
    assert_eq!(done.transfer_receipt.as_deref(), Some("RCPT-77"));

    // An outgoing payment to the new holder, with the receipt as reference.
    let paid = Payment::find_or_404(t.db(), done.transfer_payment_id.expect("a payment"))
        .await
        .unwrap();
    assert_eq!(paid.direction, Direction::Out);
    assert_eq!(paid.party_id, QUIET_SANDS.0);
    assert_eq!(paid.amount, 1_000_000);
    assert_eq!(paid.method, PaymentMethod::Transfer);
    assert_eq!(paid.reference.as_deref(), Some("RCPT-77"));
    assert_eq!(paid.method_details.0["deposit"], deposit.id);
    // It is not a refund.
    assert_eq!(Refund::query().count(t.db()).await.unwrap(), 0);

    // The tenant is told who holds it.
    let told = t.notices.of_type(DEPOSIT_TRANSFERRED);
    assert_eq!(told.len(), 1);
    assert!(matches!(
        told[0].recipients[0],
        NotifyRecipient::Party { id } if id == BLUE_DUNES
    ));
    let data = told[0].data.to_string();
    assert!(data.contains("Quiet Sands LLC"), "{data}");
    assert!(data.contains("AED 10,000.00"), "{data}");

    // DepositSettled (transferred) names the new holder.
    let settled = t.deposit_settled.all();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].outcome, DepositOutcome::Transferred);
    assert_eq!(settled[0].new_holder, Some(QUIET_SANDS));
    assert_eq!(settled[0].refund_amount, 0);
    assert_eq!(settled[0].deposit, deposit.deposit_id());
}

/// AC2: without the new holder's receipt, `transfer` is refused and nothing
/// changes.
#[renox::test]
async fn a_transfer_without_a_receipt_is_refused() {
    let t = boot().await;
    let a = t.user_a().await;
    let payment = t.receive(BLUE_DUNES, 1_000_000).await;
    let deposit = t.hold_deposit(&a, BLUE_DUNES, 1_000_000, Some(&payment)).await;

    t.deposit_page(
        &a,
        &deposit,
        "transfer",
        &[("new_holder", QUIET_SANDS_CODE), ("receipt", "  ")],
    )
    .await
    .assert_status(422);

    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::Held);
    assert_eq!(Payment::query().count(t.db()).await.unwrap(), 1);
    assert!(t.deposit_settled.all().is_empty());
    assert!(t.notices.of_type(DEPOSIT_TRANSFERRED).is_empty());
}

/// A deposit in `settlement_pending` is not transferable: the settlement
/// completes first. An unknown holder and the deposit's own party are refused,
/// and the permission is `deposits.manage`.
#[renox::test]
async fn only_a_held_deposit_goes_to_another_party_by_a_deposit_manager() {
    let t = boot().await;
    let a = t.user_a().await;
    let viewer = t.user("Vera", &[wl_payments::permissions::DEPOSITS_VIEW]).await;
    let settling = t.settling_deposit(&a, BLUE_DUNES, 1_000_000).await;
    t.deposit_page(
        &a,
        &settling,
        "transfer",
        &[("new_holder", QUIET_SANDS_CODE), ("receipt", "RCPT-1")],
    )
    .await
    .assert_status(422);

    let payment = t.receive(BLUE_DUNES, 500_000).await;
    let held = t.hold_deposit(&a, BLUE_DUNES, 500_000, Some(&payment)).await;
    for holder in ["P-99999", "P-00001"] {
        t.deposit_page(
            &a,
            &held,
            "transfer",
            &[("new_holder", holder), ("receipt", "RCPT-2")],
        )
        .await
        .assert_status(422);
    }
    t.deposit_page(
        &viewer,
        &held,
        "transfer",
        &[("new_holder", QUIET_SANDS_CODE), ("receipt", "RCPT-2")],
    )
    .await
    .assert_forbidden();
    assert_eq!(t.deposit(held.id).await.status, DepositStatus::Held);
}

/// A deposit that holds nothing has nothing to hand over.
#[renox::test]
async fn a_deposit_that_holds_nothing_is_not_transferred() {
    let t = boot().await;
    let a = t.user_a().await;
    let deposit = t.hold_deposit(&a, BLUE_DUNES, 500_000, None).await;
    assert_eq!(deposit.held_amount, 0);
    t.deposit_page(
        &a,
        &deposit,
        "transfer",
        &[("new_holder", QUIET_SANDS_CODE), ("receipt", "RCPT-3")],
    )
    .await
    .assert_status(422);
}

/// A blocked party's deposit can be transferred: it is not money out to the
/// party. The message to the blocked party is not sent.
#[renox::test]
async fn the_deposit_of_a_blocked_party_can_be_transferred() {
    let t = boot().await;
    let a = t.user_a().await;
    let payment = t.receive(DUSK_HOLDINGS, 700_000).await;
    let deposit = t.hold_deposit(&a, DUSK_HOLDINGS, 700_000, Some(&payment)).await;

    t.deposit_page(
        &a,
        &deposit,
        "transfer",
        &[("new_holder", QUIET_SANDS_CODE), ("receipt", "RCPT-4")],
    )
    .await
    .assert_ok();

    assert_eq!(t.deposit(deposit.id).await.status, DepositStatus::Transferred);
    assert!(t.notices.of_type(DEPOSIT_TRANSFERRED).is_empty());
    assert_eq!(t.deposit_settled.all().len(), 1);
}
