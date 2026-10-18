//! The Refund base: the acceptance check AC11 of #94.

mod support;

use renox::prelude::Model;
use wl_kernel::payments::{Direction, PaymentStatus, payment_recorder};
use wl_payments::refund::RefundStatus;
use wl_payments::{Payment, Refund};

use support::{BLUE_DUNES, boot, year};

async fn credit(t: &support::Booted) -> i64 {
    payment_recorder(t.state())
        .credit_balance(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap()
}

/// AC11: the requester cannot approve (`FIN.ACL.04`); another user can;
/// `pay` records the outgoing payment `PV-PGP-<year>-0001`, emits
/// `RefundPaid` and lowers the credit by the amount.
#[renox::test]
async fn a_refund_is_approved_by_another_user_and_paid_with_a_voucher() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    assert_eq!(credit(&t).await, 300_000);

    let refund = t.request_refund(&a, &payment, 100_000).await;
    assert_eq!(refund.status, RefundStatus::Requested);

    // The requester cannot approve it.
    t.refund_action(&a, &refund, "approve", &[])
        .await
        .assert_status(422)
        .assert_see("FIN.ACL.04");
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Requested);
    // Nor can it be paid before it is approved.
    t.refund_action(
        &b,
        &refund,
        "pay",
        &[("method", "transfer"), ("reference", "X")],
    )
    .await
    .assert_status(422);

    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    let approved = t.refund(refund.id).await;
    assert_eq!(approved.status, RefundStatus::Approved);
    assert_eq!(approved.approved_by, Some(b.id));

    t.refund_action(
        &a,
        &approved,
        "pay",
        &[("method", "transfer"), ("reference", "BANK-9")],
    )
    .await
    .assert_ok();

    let paid = t.refund(refund.id).await;
    assert_eq!(paid.status, RefundStatus::Paid);
    let voucher = Payment::find_or_404(t.db(), paid.payment_id.expect("the voucher"))
        .await
        .unwrap();
    assert_eq!(voucher.number.unwrap(), format!("PV-PGP-{}-0001", year()));
    assert_eq!(voucher.direction, Direction::Out);
    assert_eq!(voucher.amount, 100_000);
    assert_eq!(voucher.status, PaymentStatus::Received);

    let events = t.refund_paid.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].refund, paid.refund_id());
    assert_eq!(events[0].party, BLUE_DUNES);
    assert_eq!(events[0].amount, 100_000);
    assert_eq!(
        Some(events[0].source.clone()),
        wl_kernel::payments::RefundSource::from_subject(&payment.subject())
    );
    // Money out is not a payment received.
    assert_eq!(t.received.all().len(), 1);
    assert_eq!(credit(&t).await, 200_000);
}

/// AC11: a refund above the refundable balance is refused and nothing is
/// written; the balance already claimed by a pending refund counts.
#[renox::test]
async fn a_refund_above_the_refundable_balance_is_refused() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let a = t.user_a().await;
    let amount = |fils: &str| fils.to_owned();

    t.app.acting_as(&a);
    let payment_id = payment.id.to_string();
    for fils in [amount("300001"), amount("0"), amount("-5")] {
        t.app
            .htmx()
            .post(
                "/test/refunds",
                &[
                    ("payment", payment_id.as_str()),
                    ("amount", &fils),
                    ("reason", "x"),
                ],
            )
            .await
            .assert_status(422);
    }
    assert_eq!(Refund::query().count(t.db()).await.unwrap(), 0);

    t.request_refund(&a, &payment, 250_000).await;
    t.app
        .htmx()
        .post(
            "/test/refunds",
            &[
                ("payment", payment_id.as_str()),
                ("amount", "50001"),
                ("reason", "x"),
            ],
        )
        .await
        .assert_status(422);
    assert_eq!(Refund::query().count(t.db()).await.unwrap(), 1);
}

/// The kernel's `request_refund` asks a refund of a credit note: it starts
/// `requested` for the user, still needs another user's approval, and is
/// refused for a blank reason, an amount of 0 or an unknown party.
#[renox::test]
async fn a_credit_note_refund_is_requested_through_the_kernel() {
    use wl_kernel::payments::{PaymentMethod, RefundRequest, RefundSource};
    use wl_kernel::{PartyId, Subject};

    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let a = t.user_a().await;
    let note = Subject::new("invoicing.credit_note", 7);
    let request = |amount: i64, reason: &str, party: PartyId| RefundRequest {
        source: RefundSource::CreditNote(note.clone()),
        party,
        legal_entity_id: payment.legal_entity_id,
        amount,
        currency: "AED".into(),
        method: Some(PaymentMethod::Transfer),
        gateway_txn: None,
        reason: reason.into(),
    };
    let recorder = payment_recorder(t.state());
    for bad in [
        request(50_000, "  ", BLUE_DUNES),
        request(0, "Credit note", BLUE_DUNES),
        request(50_000, "Credit note", PartyId(999)),
    ] {
        assert!(
            recorder
                .request_refund(t.state(), bad, Some(a.id))
                .await
                .is_err()
        );
    }
    assert_eq!(Refund::query().count(t.db()).await.unwrap(), 0);

    let id = recorder
        .request_refund(
            t.state(),
            request(50_000, "Credit note CN-1", BLUE_DUNES),
            Some(a.id),
        )
        .await
        .unwrap();
    let refund = t.refund(id.0).await;
    assert_eq!(refund.status, RefundStatus::Requested);
    assert_eq!(refund.requested_by, Some(a.id));
    assert_eq!(refund.amount, 50_000);
    assert_eq!(
        (refund.source_type.as_str(), refund.source_id),
        ("invoicing.credit_note", 7)
    );
    // The requester still cannot approve it (`FIN.ACL.04`).
    t.refund_action(&a, &refund, "approve", &[])
        .await
        .assert_status(422)
        .assert_see("FIN.ACL.04");
}
