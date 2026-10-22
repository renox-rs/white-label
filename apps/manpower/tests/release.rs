//! BL-MPW-03 (#417): releasing a credit hold (`release`, `MPW.RUL.10`,
//! `MPW.ACL.14`). The system releases a contract back to the state it had
//! before the hold when payment, `ChequeBounceResolved` or the 02:00 run find
//! nothing left that holds it; the account manager may release earlier, with
//! the MD's approval while money is owed, and the requester cannot approve.

mod support;

use manpower::notices::{CLIENT_NOTICE, NOTICE};
use manpower::{CREDIT_HOLD_TASK, ClientContract, ContractHold, ContractStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve, reject};
use wl_kernel::invoicing::InvoiceId;
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_kernel::payments::{PaymentAllocation, PaymentId, PaymentMethod, PaymentReceived};

use support::{Booted, TOWER, boot, date, users_of};

/// A contract of Tower in `before`, held on `day` for INV-1 of AED 42,000 due
/// 2026-10-01 through the 02:00 run.
async fn held(
    t: &Booted,
    manager: &User,
    before: ContractStatus,
    day: NaiveDate,
) -> (ClientContract, InvoiceId) {
    let contract = t.contract(TOWER, before, Some(manager)).await;
    let invoice = t
        .invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    run_rule(t, day).await;
    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(before));
    (held, invoice)
}

async fn run_rule(t: &Booted, day: NaiveDate) {
    t.on(day, async {
        t.app.run_scheduled(CREDIT_HOLD_TASK).await.unwrap();
    })
    .await;
}

/// The client pays `amount` fils of `invoice`: the invoicing module allocates
/// and wl-payments emits the event.
async fn pay(t: &Booted, invoice: InvoiceId, amount: i64) {
    t.invoices.pay(invoice, amount);
    t.state()
        .emit(PaymentReceived {
            payment: PaymentId(1),
            party: TOWER,
            amount,
            method: PaymentMethod::Transfer,
            allocations: vec![PaymentAllocation {
                target: invoice.subject(),
                amount,
            }],
        })
        .await
        .unwrap();
}

/// The notices to staff whose title starts with `start`.
fn notices_titled(t: &Booted, start: &str) -> Vec<NotifyRequest> {
    t.notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|request| {
            request.data["title"]
                .as_str()
                .is_some_and(|title| title.starts_with(start))
        })
        .collect()
}

async fn pending(t: &Booted, contract: &ClientContract) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &contract.subject())
        .await
        .unwrap()
}

/// AC1: K `on_hold` (before `active`, reason `overdue` for INV-1 AED 42,000);
/// a payment of AED 42,000 is allocated to INV-1: K is `active`, the reason is
/// closed, the account manager is told and the client is not.
#[renox::test]
async fn paying_the_invoice_releases_the_contract_to_its_state_before() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;

    pay(&t, invoice, 4_200_000).await;

    let released = t.reload_contract(contract.id).await;
    assert_eq!(released.status, ContractStatus::Active);
    assert_eq!(released.status_before_hold, None);
    assert_eq!(released.held_at, None);
    let reasons = ContractHold::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(reasons.len(), 1);
    assert!(!reasons[0].is_open());
    assert_eq!(reasons[0].resolution.as_deref(), Some("payment"));
    assert!(!reasons[0].covered);
    let told = notices_titled(&t, "Credit hold released");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), vec![amal.id]);
    assert!(t.notices.of_type(CLIENT_NOTICE).is_empty());
    assert!(
        t.notices.all().iter().all(|request| request
            .recipients
            .iter()
            .all(|recipient| matches!(recipient, NotifyRecipient::User { .. }))),
        "nothing went to the client company"
    );
}

/// A part payment leaves the hold; the rest releases it.
#[renox::test]
async fn a_part_payment_does_not_release_the_contract() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;

    pay(&t, invoice, 1_000_000).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    pay(&t, invoice, 3_200_000).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
}

/// AC2: K held for INV-1 and a bounced cheque CHQ-9: paying INV-1 alone leaves
/// K `on_hold`; after `ChequeBounceResolved` for CHQ-9 K is released.
#[renox::test]
async fn two_reasons_need_both_cleared() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    t.state()
        .emit(t.register.bounce(9, TOWER, "CHQ-9", 1_500_000))
        .await
        .unwrap();
    assert_eq!(
        ContractHold::open_of(t.db(), contract.id)
            .await
            .unwrap()
            .len(),
        2
    );

    pay(&t, invoice, 4_200_000).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    assert_eq!(
        ContractHold::open_of(t.db(), contract.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let resolved = t.register.clear(9);
    t.state().emit(resolved).await.unwrap();

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert_eq!(notices_titled(&t, "Credit hold released").len(), 1);
}

/// AC3: K held for INV-1 while INV-2 of the same contract is `disputed` and 45
/// days overdue: paying INV-1 releases K (INV-2 does not count).
#[renox::test]
async fn a_disputed_invoice_does_not_keep_the_contract_held() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let first = t
        .invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    let second = t
        .invoices
        .issue(&contract, "INV-2", 900_000, date(2026, 9, 17));
    t.invoices.dispute(second);
    run_rule(&t, date(2026, 11, 1)).await;
    let reasons = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    assert_eq!(reasons.len(), 1, "the disputed invoice is no reason");

    pay(&t, first, 4_200_000).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
}

/// A dispute raised after the hold does not release it.
#[renox::test]
async fn a_dispute_raised_after_the_hold_does_not_release_it() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;

    t.invoices.dispute(invoice);
    run_rule(&t, date(2026, 11, 2)).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
}

/// AC4: K held while `terminating` is `terminating` after the release, never
/// `active`.
#[renox::test]
async fn a_terminating_contract_goes_back_to_terminating() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Terminating, date(2026, 11, 1)).await;

    pay(&t, invoice, 4_200_000).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Terminating
    );
}

/// AC5: account manager A requests release while INV-1 is unpaid: an approval
/// request goes to the MD, K stays `on_hold`; A cannot approve it, not even
/// with the MD role; when MD user B approves, K returns to its state before,
/// the account manager is told and the reasons are covered.
#[renox::test]
async fn the_md_approves_a_release_the_account_manager_asked_for() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let (contract, _invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;

    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    let waiting = pending(&t, &contract).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].requester_id, amal.id);
    assert_eq!(waiting[0].action, "manpower.release");
    let refused = approve(t.state(), waiting[0].id, &amal, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");

    approve(t.state(), waiting[0].id, &bader, "payment plan agreed")
        .await
        .unwrap();

    let released = t.reload_contract(contract.id).await;
    assert_eq!(released.status, ContractStatus::Active);
    assert_eq!(released.status_before_hold, None);
    let reasons = ContractHold::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    assert!(reasons.iter().all(|row| !row.is_open() && row.covered));
    assert_eq!(notices_titled(&t, "Credit hold released").len(), 1);
}

/// A held `terminating` contract released by the MD's approval is
/// `terminating` again.
#[renox::test]
async fn an_approved_release_of_a_terminating_contract_goes_back_to_terminating() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let (contract, _invoice) =
        held(&t, &amal, ContractStatus::Terminating, date(2026, 11, 1)).await;

    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);
    let waiting = pending(&t, &contract).await;
    assert_eq!(waiting.len(), 1);
    approve(t.state(), waiting[0].id, &bader, "").await.unwrap();

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Terminating
    );
}

/// AC6: when B rejects with a comment, K stays `on_hold` and A is notified
/// with the comment.
#[renox::test]
async fn a_rejected_release_leaves_the_contract_held_and_tells_the_account_manager() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let (contract, _invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);
    let waiting = pending(&t, &contract).await;

    reject(
        t.state(),
        waiting[0].id,
        &bader,
        "no payment plan in writing",
    )
    .await
    .unwrap();

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    let told = notices_titled(&t, "Release of the credit hold refused");
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), vec![amal.id]);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("no payment plan in writing")
    );
    assert!(
        ContractHold::open_of(t.db(), contract.id)
            .await
            .unwrap()
            .len()
            == 1
    );
}

/// AC7: after B's approval the 02:00 run the next day does not hold K again
/// for INV-1; when INV-2 becomes 31 days overdue, K is held again.
#[renox::test]
async fn an_approved_release_is_not_held_again_for_the_same_invoice() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let (contract, _invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    let later = t
        .invoices
        .issue(&contract, "INV-2", 900_000, date(2026, 11, 10));
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);
    let waiting = pending(&t, &contract).await;
    t.on(date(2026, 11, 1), async {
        approve(t.state(), waiting[0].id, &bader, "").await.unwrap();
    })
    .await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );

    run_rule(&t, date(2026, 11, 2)).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active,
        "INV-1 is covered"
    );
    // INV-2 is 30 days overdue on 10 December: not yet.
    run_rule(&t, date(2026, 12, 10)).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    run_rule(&t, date(2026, 12, 11)).await;

    let again = t.reload_contract(contract.id).await;
    assert_eq!(again.status, ContractStatus::OnHold);
    let open = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].invoice_id, Some(later.0));
}

/// A manual release when no hold condition remains needs no approval: the
/// reason was the account manager's own.
#[renox::test]
async fn a_release_with_nothing_owed_needs_no_approval() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", contract.id),
        &[("reason", "other"), ("text", "client in liquidation")],
    )
    .await
    .assert_status(303);

    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert!(pending(&t, &contract).await.is_empty());
}

/// Payment does not close the account manager's own reason: only a release
/// does.
#[renox::test]
async fn the_system_never_closes_a_reason_the_account_manager_wrote() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", contract.id),
        &[("reason", "other"), ("text", "client in liquidation")],
    )
    .await
    .assert_status(303);

    pay(&t, invoice, 4_200_000).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    let open = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(
        open[0].reason_text.as_deref(),
        Some("client in liquidation")
    );
}

/// A system release wins over a pending manual request: the approval request
/// is withdrawn with the reason *released by payment*.
#[renox::test]
async fn a_release_by_payment_withdraws_the_pending_request() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    t.md("Bader").await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);
    let waiting = pending(&t, &contract).await;
    assert_eq!(waiting.len(), 1);

    pay(&t, invoice, 4_200_000).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert!(pending(&t, &contract).await.is_empty());
    let request = ApprovalRequest::find_or_404(t.db(), waiting[0].id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Withdrawn);
}

/// The 02:00 run releases a hold whose invoice was paid without an event, and
/// does so once.
#[renox::test]
async fn the_daily_run_releases_a_contract_that_was_paid() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    t.invoices.pay(invoice, 4_200_000);

    run_rule(&t, date(2026, 11, 2)).await;
    run_rule(&t, date(2026, 11, 2)).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert_eq!(notices_titled(&t, "Credit hold released").len(), 1);
}

/// Only a contract on hold can be released, and only by a manager of the
/// client's contracts.
#[renox::test]
async fn a_release_needs_a_held_contract_and_the_permission() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let stranger = t.account_manager("Sam", support::FOODS).await;
    let active = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", active.id),
        &[],
    )
    .await
    .assert_status(422);
    let (contract, _invoice) = held(&t, &amal, ContractStatus::Active, date(2026, 11, 1)).await;
    t.post(
        &stranger,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_forbidden();
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
}

/// Segregation of duties: an account manager who also holds the MD role asks
/// for the release and still cannot approve it; another MD can.
#[renox::test]
async fn the_requester_cannot_approve_even_with_the_md_role() {
    let t = boot().await;
    let bea = t.account_manager("Bea", TOWER).await;
    bea.assign_role(t.db(), manpower::permissions::MD_ROLE)
        .await
        .unwrap();
    let bader = t.md("Bader").await;
    let (contract, _invoice) = held(&t, &bea, ContractStatus::Active, date(2026, 11, 1)).await;

    t.post(
        &bea,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);

    let waiting = pending(&t, &contract).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].requester_id, bea.id);
    let refused = approve(t.state(), waiting[0].id, &bea, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    approve(t.state(), waiting[0].id, &bader, "").await.unwrap();
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
}
