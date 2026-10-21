//! Refunding a payer credit balance (wl-payments §3 *Refund source*, §5.5a
//! *Balances owed between the business and a supplier or payer*,
//! `FIN.ACL.04`, `FIN.ACL.16`; issue #101): what the module does with the
//! balance wl-invoicing holds: the refundable balance guard, the second
//! person's approval and the `RefundApproved` that lets wl-invoicing record
//! the movement. The movement itself is wl-invoicing's and is tested with the
//! real modules (`payer_credit_refund_real` in wl-invoicing).

mod support;

use renox::prelude::User;
use wl_kernel::PartyId;
use wl_kernel::payer_bills::payer_credit_subject;
use wl_kernel::payments::RefundSource;
use wl_payments::refund::RefundStatus;

use support::{QUIET_SANDS, boot};

/// The payer: Quiet Sands, an insurer.
const PAYER: PartyId = QUIET_SANDS;

fn source() -> RefundSource {
    RefundSource::PayerCredit(payer_credit_subject(PAYER))
}

/// Posts the request of a refund of `amount` fils out of the payer credit as
/// `by`.
async fn ask(
    t: &support::Booted,
    by: &User,
    source: &RefundSource,
    party: PartyId,
    amount: i64,
) -> renox::testing::TestResponse {
    let entity = t.entity("PGP").await;
    t.app.acting_as(by);
    let subject = source.subject();
    let party = party.0.to_string();
    let entity = entity.id.to_string();
    let source_id = subject.subject_id.to_string();
    let amount = amount.to_string();
    t.app
        .htmx()
        .post(
            "/test/refunds/source",
            &[
                ("source_kind", source.kind()),
                ("source_type", subject.subject_type.as_str()),
                ("source_id", source_id.as_str()),
                ("party", party.as_str()),
                ("entity", entity.as_str()),
                ("amount", amount.as_str()),
                ("method", "transfer"),
                ("reason", "Overpayment not set off"),
            ],
        )
        .await
}

/// Given a payer credit balance of AED 1,200.00, user A requests a refund of
/// AED 1,200.00; user A cannot approve it; user B approves it, and that
/// approval is the movement's (`RefundApproved` names the approver and the
/// source; there is no second approval request).
#[renox::test]
async fn the_refund_of_a_payer_credit_is_approved_by_a_second_person() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    t.payer_credits.set(PAYER, entity.id, 120_000);
    let a = t.user_a().await;
    let b = t.user_b().await;

    let response = ask(&t, &a, &source(), PAYER, 120_000).await;
    response.assert_ok();
    let id: i64 = response.text().trim().parse().expect("a refund id");
    let refund = t.refund(id).await;
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.source_kind, "payer_credit");
    assert_eq!(refund.source_subject(), payer_credit_subject(PAYER));
    assert_eq!(refund.requested_by, Some(a.id));
    // Nothing is taken from the balance before the approval.
    assert!(t.refund_approved.all().is_empty());

    // The requester cannot approve.
    t.refund_action(&a, &refund, "approve", &[])
        .await
        .assert_status(422)
        .assert_see("FIN.ACL.04");
    assert_eq!(t.refund(id).await.status, RefundStatus::Requested);

    // Another user can; the approval is the one approval.
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    let approved = t.refund(id).await;
    assert_eq!(approved.status, RefundStatus::Approved);
    assert_eq!(approved.approved_by, Some(b.id));
    let events = t.refund_approved.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].refund.0, id);
    assert_eq!(events[0].party, PAYER);
    assert_eq!(events[0].legal_entity_id, entity.id);
    assert_eq!(events[0].amount, 120_000);
    assert_eq!(events[0].currency, "AED");
    assert_eq!(events[0].source, source());
    assert_eq!(events[0].approved_by, Some(b.id));
}

/// A refund of AED 1,300.00 is refused when the balance is AED 1,200.00;
/// refunds still waiting for approval count against the balance too, and a
/// rejected one gives it back.
#[renox::test]
async fn a_refund_above_the_balance_is_refused() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    t.payer_credits.set(PAYER, entity.id, 120_000);
    let a = t.user_a().await;
    let b = t.user_b().await;

    ask(&t, &a, &source(), PAYER, 130_000)
        .await
        .assert_status(422)
        .assert_see("above the refundable balance");

    // AED 1,000.00 waits; only AED 200.00 is left to ask for.
    let first = ask(&t, &a, &source(), PAYER, 100_000).await;
    first.assert_ok();
    let first = t.refund(first.text().trim().parse().unwrap()).await;
    ask(&t, &a, &source(), PAYER, 30_000)
        .await
        .assert_status(422)
        .assert_see("above the refundable balance");
    ask(&t, &a, &source(), PAYER, 20_000).await.assert_ok();

    // Rejecting the first gives its amount back.
    t.refund_action(&b, &first, "reject", &[("reason", "Asked twice")])
        .await
        .assert_ok();
    ask(&t, &a, &source(), PAYER, 100_000).await.assert_ok();
    assert!(t.refund_approved.all().is_empty());
}

/// A payer with no credit balance (wl-invoicing off, or nothing overpaid),
/// and a credit that is not the party's own, cannot be refunded.
#[renox::test]
async fn nothing_is_refunded_where_there_is_no_balance_or_it_is_not_the_partys() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let a = t.user_a().await;
    // No balance at all.
    ask(&t, &a, &source(), PAYER, 100)
        .await
        .assert_status(422);
    // The balance of another payer.
    t.payer_credits.set(PAYER, entity.id, 120_000);
    let other = RefundSource::PayerCredit(payer_credit_subject(support::BLUE_DUNES));
    ask(&t, &a, &other, PAYER, 100)
        .await
        .assert_status(422)
        .assert_see("does not belong");
    // The balance of the right payer is refundable.
    ask(&t, &a, &source(), PAYER, 100).await.assert_ok();
}

/// Money out to a payer on hold waits (`PTY.ACL.03`): the refund is approved,
/// the payment is not made until the payer is released.
#[renox::test]
async fn the_refund_of_a_payer_credit_to_a_blocked_payer_is_not_paid() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    t.payer_credits.set(PAYER, entity.id, 120_000);
    let a = t.user_a().await;
    let b = t.user_b().await;
    let response = ask(&t, &a, &source(), PAYER, 120_000).await;
    let refund = t.refund(response.text().trim().parse().unwrap()).await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();

    t.blocked.block(PAYER);
    let pay = [("method", "transfer"), ("reference", "BANK-1")];
    t.refund_action(&a, &t.refund(refund.id).await, "pay", &pay)
        .await
        .assert_status(422)
        .assert_see("hold");
    let waiting = t.refund(refund.id).await;
    assert_eq!(waiting.status, RefundStatus::Approved);
    assert_eq!(waiting.payment_id, None, "no voucher is written");
    assert!(t.refund_paid.all().is_empty());

    t.blocked.free(PAYER);
    t.refund_action(&a, &t.refund(refund.id).await, "pay", &pay)
        .await
        .assert_ok();
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Paid);
    let paid = t.refund_paid.all();
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0].source, source());
}
