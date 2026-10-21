//! Closing a chargeback (wl-payments §4 `win_chargeback`, `lose_chargeback`,
//! `accept_chargeback`, §5.2a steps 3 and 4, `FIN.ACL.18`; issue #98): won
//! re-applies the allocations where it can, lost leaves the invoices open,
//! accepted needs another person's approval and tells the gateway.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};
use wl_kernel::invoicing::InvoiceId;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{ChargebackOutcome, PaymentStatus, ReversalReason};
use wl_payments::chargeback::{self, DisputeOpened};
use wl_payments::chargeback_close;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::gateway::FakeGateway;
use wl_payments::{Payment, WlPayments, permissions as perm};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot_with};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

async fn boot_with_gateway(gateway: &FakeGateway) -> Booted {
    boot_with(
        WlPayments::new()
            .gateway(gateway.clone())
            .gateway_party(QUIET_SANDS),
        gateway.clone(),
    )
    .await
}

/// An AED 800.00 online payment allocated to INV-9, charged back by the
/// dispute `dp_1` whose evidence is due on 2027-02-10; the invoice is
/// reopened as wl-invoicing does on `PaymentReversed`.
async fn charged_back(t: &Booted) -> (Payment, InvoiceId) {
    let staff = t.user_a().await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 80_000);
    let link = t
        .link(BLUE_DUNES, 80_000, Some(invoice), None, Some(&staff))
        .await;
    let payment = t.start_online(&link).await;
    let txn = payment.gateway_transaction.clone().unwrap();
    wl_payments::online::gateway_succeeded(t.state(), &txn, 80_000)
        .await
        .unwrap()
        .expect("confirmed");
    let charged = chargeback::gateway_dispute_opened(
        t.state(),
        &txn,
        &DisputeOpened::new("dp_1").evidence_due(day(2027, 2, 10)),
    )
    .await
    .unwrap()
    .expect("charged back");
    t.invoices.set_balance(invoice, 80_000);
    (charged, invoice)
}

fn told(t: &Booted, user: i64) -> bool {
    t.notices
        .recipients(wl_payments::notices::ONLINE_TASK)
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user))
}

fn said(t: &Booted, part: &str) -> bool {
    t.notices
        .of_type(wl_payments::notices::ONLINE_TASK)
        .iter()
        .any(|n| n.data.to_string().contains(part))
}

/// A won dispute: received again, the AED 800.00 re-allocated to INV-9,
/// `PaymentReceived`; a replayed webhook does nothing.
#[renox::test]
async fn a_won_dispute_applies_the_allocation_again() {
    let t = boot_with_gateway(&FakeGateway::new()).await;
    let (payment, invoice) = charged_back(&t).await;
    assert_eq!(t.invoices.calls().len(), 1);

    let won = chargeback_close::gateway_dispute_won(t.state(), "dp_1")
        .await
        .unwrap()
        .expect("won");
    assert_eq!(won.status, PaymentStatus::Received);
    assert_eq!(won.dispute_outcome.as_deref(), Some("won"));
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!((calls[1].invoice, calls[1].amount), (invoice, 80_000));
    let rows = t.allocations(&won).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].removed_reason.as_deref(), Some("chargeback_won"));
    assert!(rows[1].is_active());
    assert_eq!(t.summary(&won).await.unallocated, 0);
    let events = t.received.all();
    assert_eq!(events.last().unwrap().allocations.len(), 1);
    assert_eq!(events.last().unwrap().allocations[0].amount, 80_000);
    // The evidence task is done with the dispute.
    let tasks = t.tasks(&payment).await;
    assert_eq!(tasks[0].kind, FollowupKind::ChargebackEvidence);
    assert_eq!(tasks[0].status, FollowupStatus::Done);

    // The webhook again.
    assert!(
        chargeback_close::gateway_dispute_won(t.state(), "dp_1")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(t.invoices.calls().len(), 2);
}

/// INV-9 paid by another payment meanwhile: the AED 800.00 stays as credit
/// and the accountant is told; paid in part: what is open is applied.
#[renox::test]
async fn a_won_dispute_keeps_what_cannot_be_applied_as_credit() {
    let t = boot_with_gateway(&FakeGateway::new()).await;
    let accountant = t.accountant("Rita").await;
    let (payment, invoice) = charged_back(&t).await;
    // Paid by another payment meanwhile.
    t.invoices.set_balance(invoice, 0);

    let won = chargeback_close::gateway_dispute_won(t.state(), "dp_1")
        .await
        .unwrap()
        .expect("won");
    assert_eq!(won.status, PaymentStatus::Received);
    assert_eq!(t.invoices.calls().len(), 1, "nothing allocated again");
    assert_eq!(t.summary(&won).await.unallocated, 80_000);
    assert!(t.received.all().last().unwrap().allocations.is_empty());
    assert!(told(&t, accountant.id));
    assert!(said(&t, "stays as the party's credit"));
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Received);
}

/// Edge: a target partly paid by another payment takes up to its open
/// balance; the rest stays as credit.
#[renox::test]
async fn a_won_dispute_applies_up_to_the_open_balance() {
    let t = boot_with_gateway(&FakeGateway::new()).await;
    let (_payment, invoice) = charged_back(&t).await;
    t.invoices.set_balance(invoice, 30_000);
    let won = chargeback_close::gateway_dispute_won(t.state(), "dp_1")
        .await
        .unwrap()
        .expect("won");
    let calls = t.invoices.calls();
    assert_eq!(calls.last().unwrap().amount, 30_000);
    let summary = t.summary(&won).await;
    assert_eq!((summary.allocated, summary.unallocated), (30_000, 50_000));
}

/// The evidence deadline passed with no evidence submitted: `reversed`,
/// INV-9 stays open, the accountant is told, `ChargebackClosed` (lost). With
/// the evidence submitted the deadline closes nothing.
#[renox::test]
async fn the_deadline_without_evidence_loses_the_dispute() {
    let t = boot_with_gateway(&FakeGateway::new()).await;
    let accountant = t.accountant("Rita").await;
    let cara = t.user("Cara", &[perm::VIEW, perm::CHARGEBACK]).await;
    let (payment, _invoice) = charged_back(&t).await;
    let calls_before = t.invoices.calls().len();

    // On the deadline itself nothing is lost.
    assert_eq!(
        chargeback_close::run_deadlines(t.state(), day(2027, 2, 10))
            .await
            .unwrap(),
        0
    );
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::ChargedBack);
    // The evidence was noted submitted: the deadline leaves it alone.
    t.app.acting_as(&cara);
    t.app
        .htmx()
        .post(&format!("/payments/{}/evidence", payment.id), &[])
        .await
        .assert_ok();
    assert_eq!(
        chargeback_close::run_deadlines(t.state(), day(2027, 2, 11))
            .await
            .unwrap(),
        0
    );
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::ChargedBack);

    // A second payment whose evidence was never submitted.
    let (other, _) = charged_back_again(&t).await;
    assert_eq!(
        chargeback_close::run_deadlines(t.state(), day(2027, 2, 11))
            .await
            .unwrap(),
        1
    );
    let lost = t.reload(&other).await;
    assert_eq!(lost.status, PaymentStatus::Reversed);
    assert_eq!(lost.reversal_reason, Some(ReversalReason::Chargeback));
    assert_eq!(lost.dispute_outcome.as_deref(), Some("lost"));
    assert!(lost.reversed_on.is_some());
    // The invoices are not closed or credited.
    assert_eq!(t.invoices.calls().len(), calls_before + 1);
    let closed = t.chargeback_closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].outcome, ChargebackOutcome::Lost);
    assert_eq!(closed[0].payment, other.payment_id());
    assert_eq!(closed[0].reopened.len(), 1);
    assert_eq!(closed[0].reopened[0].amount, 80_000);
    assert!(told(&t, accountant.id));
    assert!(said(&t, "stay open"));
}

/// A second charged-back payment of AED 800.00 with the dispute `dp_2`
/// (the first test fixture, again).
async fn charged_back_again(t: &Booted) -> (Payment, InvoiceId) {
    let staff = t.user("Gus", &[perm::VIEW]).await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-10", 80_000);
    let link = t
        .link(BLUE_DUNES, 80_000, Some(invoice), None, Some(&staff))
        .await;
    let payment = t.start_online(&link).await;
    let txn = payment.gateway_transaction.clone().unwrap();
    wl_payments::online::gateway_succeeded(t.state(), &txn, 80_000)
        .await
        .unwrap();
    let charged = chargeback::gateway_dispute_opened(
        t.state(),
        &txn,
        &DisputeOpened::new("dp_2").evidence_due(day(2027, 2, 10)),
    )
    .await
    .unwrap()
    .expect("charged back");
    (charged, invoice)
}

/// One charged-back payment that errors when the deadline closes it does not
/// stop the others: the second is lost, the error is returned after the batch.
#[renox::test]
async fn a_payment_that_errors_does_not_stop_the_deadline_run() {
    let t = boot_with_gateway(&FakeGateway::new()).await;
    let (bad, _) = charged_back(&t).await;
    let (good, _) = charged_back_again(&t).await;
    assert!(bad.id < good.id, "the failing one is handled first");
    renox::db::sql(format!(
        "CREATE TRIGGER poison_payment BEFORE UPDATE ON wl_payments_payments \
         WHEN NEW.id = {} BEGIN SELECT RAISE(ABORT, 'poisoned for the test'); END",
        bad.id
    ))
    .execute(t.db())
    .await
    .unwrap();

    let ran = chargeback_close::run_deadlines(t.state(), day(2027, 2, 11)).await;
    assert!(ran.is_err(), "the error is reported after the batch");
    assert_eq!(t.reload(&bad).await.status, PaymentStatus::ChargedBack);
    assert_eq!(t.reload(&good).await.status, PaymentStatus::Reversed);
}

/// Lost by the gateway's webhook, and by hand for N-Genius.
#[renox::test]
async fn a_lost_dispute_reverses_the_payment_whoever_records_it() {
    let t = boot_with_gateway(&FakeGateway::new()).await;
    let nina = t.user("Nina", &[perm::VIEW, perm::CHARGEBACK]).await;
    let nobody = t.user("Omar", &[perm::VIEW]).await;
    let (first, _) = charged_back(&t).await;
    let (second, _) = charged_back_again(&t).await;

    let lost = chargeback_close::gateway_dispute_lost(t.state(), "dp_1")
        .await
        .unwrap()
        .expect("lost");
    assert_eq!(lost.status, PaymentStatus::Reversed);
    // A replay does nothing.
    assert!(
        chargeback_close::gateway_dispute_lost(t.state(), "dp_1")
            .await
            .unwrap()
            .is_none()
    );

    let path = format!("/payments/{}/chargeback-lost", second.id);
    t.app.acting_as(&nobody);
    t.app.htmx().post(&path, &[]).await.assert_forbidden();
    t.app.acting_as(&nina);
    t.app.htmx().post(&path, &[]).await.assert_ok();
    let by_hand = t.reload(&second).await;
    assert_eq!(by_hand.status, PaymentStatus::Reversed);
    assert_eq!(by_hand.reversed_by, Some(nina.id));
    assert_eq!(t.reload(&first).await.reversed_by, None);
    assert_eq!(t.chargeback_closed.all().len(), 2);
}

/// User A requests `accept_chargeback`; user A cannot approve; user B
/// approves; the gateway is told; `ChargebackClosed` (accepted).
#[renox::test]
async fn accepting_a_chargeback_needs_another_persons_approval_and_tells_the_gateway() {
    let gateway = FakeGateway::new();
    let t = boot_with_gateway(&gateway).await;
    let a = t.user("Cara", &[perm::VIEW, perm::CHARGEBACK]).await;
    let b = t.user("Dina", &[perm::VIEW, perm::CHARGEBACK]).await;
    let (payment, _invoice) = charged_back(&t).await;
    let path = format!("/payments/{}/accept-chargeback", payment.id);

    // A reason is required.
    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(&path, &[("reason", " ")])
        .await
        .assert_status(422);
    assert!(
        ApprovalRequest::pending_about(t.db(), &payment.subject())
            .await
            .unwrap()
            .is_empty()
    );

    t.app
        .htmx()
        .post(&path, &[("reason", "Not worth contesting")])
        .await
        .assert_ok();
    // It waits: the payment is still charged back, nothing went to the gateway.
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::ChargedBack);
    let pending = ApprovalRequest::pending_about(t.db(), &payment.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    let request = &pending[0];
    assert_eq!(request.action, chargeback_close::ACCEPT_ACTION);
    assert_eq!(request.requester_id, a.id);
    assert!(gateway.accepted().is_empty());
    assert!(t.chargeback_closed.all().is_empty());

    // The requester cannot approve it.
    let approve = format!("/approvals/{}/approve", request.id);
    t.app
        .htmx()
        .post(&approve, &[("comment", "")])
        .await
        .assert_forbidden();
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::ChargedBack);

    // Another user does.
    t.app.acting_as(&b);
    t.app
        .htmx()
        .post(&approve, &[("comment", "")])
        .await
        .assert_ok();
    let accepted = t.reload(&payment).await;
    assert_eq!(accepted.status, PaymentStatus::Reversed);
    assert_eq!(accepted.dispute_outcome.as_deref(), Some("accepted"));
    assert_eq!(accepted.reversal_reason, Some(ReversalReason::Chargeback));
    assert_eq!(
        accepted.reversal_note.as_deref(),
        Some("Not worth contesting")
    );
    assert_eq!(
        gateway.accepted(),
        [(
            payment.gateway_transaction.clone().unwrap(),
            "dp_1".to_owned()
        )]
    );
    let closed = t.chargeback_closed.all();
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].outcome, ChargebackOutcome::Accepted);
    assert_eq!(closed[0].reopened.len(), 1);
    let request = ApprovalRequest::find_or_404(t.db(), request.id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Approved);
}

/// The gateway refuses the acceptance: the request fails, the payment stays
/// `charged_back` and the accountant is told.
#[renox::test]
async fn a_gateway_that_refuses_the_acceptance_fails_the_request() {
    let gateway = FakeGateway::new();
    let t = boot_with_gateway(&gateway).await;
    let accountant = t.accountant("Rita").await;
    let a = t.user("Cara", &[perm::VIEW, perm::CHARGEBACK]).await;
    let b = t.user("Dina", &[perm::VIEW, perm::CHARGEBACK]).await;
    let (payment, _invoice) = charged_back(&t).await;
    gateway.refuse_accept(
        payment.gateway_transaction.as_deref().unwrap(),
        "dispute closed",
    );

    t.app.acting_as(&a);
    t.app
        .htmx()
        .post(
            &format!("/payments/{}/accept-chargeback", payment.id),
            &[("reason", "Not worth contesting")],
        )
        .await
        .assert_ok();
    let request = ApprovalRequest::pending_about(t.db(), &payment.subject())
        .await
        .unwrap()
        .remove(0);
    t.app.acting_as(&b);
    t.app
        .htmx()
        .post(
            &format!("/approvals/{}/approve", request.id),
            &[("comment", "")],
        )
        .await
        .assert_ok();

    let request = ApprovalRequest::find_or_404(t.db(), request.id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Failed);
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::ChargedBack);
    assert!(t.chargeback_closed.all().is_empty());
    assert!(told(&t, accountant.id));
    assert!(said(&t, "acceptance of chargeback dp_1"));
}
