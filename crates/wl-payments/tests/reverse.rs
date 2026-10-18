//! Reversing a payment and removing its allocations: the acceptance checks
//! AC5 to AC10 and AC12 of #94, with a recording `Invoicer` and `Notifier`,
//! the `FakeGateway` and a clock that travels.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_kernel::invoicing::InvoiceId;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{
    PaymentMethod, PaymentSource, PaymentStatus, RecordPayment, ReversalReason, payment_recorder,
};
use wl_kernel::{PartyId, Subject};
use wl_payments::gateway::FakeGateway;
use wl_payments::notices::{FOLLOWUP_TASK, PAYMENT_RETURNED};
use wl_payments::reverse::DISPUTE_MESSAGE;
use wl_payments::{Payment, permissions as perm};

use support::{BLUE_DUNES, Booted, boot, boot_gateway};

/// The payment of AED 3,000.00 allocated 2,000.00 to INV-1 and 1,000.00 to
/// INV-2.
async fn allocated(t: &Booted) -> (Payment, InvoiceId, InvoiceId) {
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let inv1 = t.invoices.add(BLUE_DUNES, "INV-1", 200_000);
    let inv2 = t.invoices.add(BLUE_DUNES, "INV-2", 100_000);
    t.allocate(&payment, inv1, 200_000).await.unwrap();
    t.allocate(&payment, inv2, 100_000).await.unwrap();
    (payment, inv1, inv2)
}

/// AC5: user A reverses the payment for `bank_return`: it is `reversed`,
/// both allocations are marked removed and stay, `PaymentReversed` lists
/// them once, the party gets one notice, and the invoicer is not called.
#[renox::test]
async fn a_bank_return_reverses_the_payment_and_removes_its_allocations() {
    let t = boot().await;
    let (payment, inv1, inv2) = allocated(&t).await;
    let user = t.user_a().await;
    let calls_before = t.invoices.calls().len();

    t.reverse(&user, &payment, "bank_return", "Cheque bounced")
        .await
        .assert_ok();

    let reversed = t.reload(&payment).await;
    assert_eq!(reversed.status, PaymentStatus::Reversed);
    assert_eq!(reversed.reversal_reason, Some(ReversalReason::BankReturn));
    assert_eq!(reversed.reversal_note.as_deref(), Some("Cheque bounced"));
    assert_eq!(reversed.reversed_by, Some(user.id));

    let rows = t.allocations(&payment).await;
    assert_eq!(rows.len(), 2, "removed allocations are kept");
    for row in &rows {
        assert!(row.removed_at.is_some());
        assert_eq!(row.removed_reason.as_deref(), Some("bank_return"));
        assert_eq!(row.removed_by, Some(user.id));
    }

    let events = t.reversed.all();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.payment, payment.payment_id());
    assert_eq!(event.reason, ReversalReason::BankReturn);
    let removed: Vec<_> = event
        .removed
        .iter()
        .map(|r| (r.allocation, r.target.clone(), r.amount))
        .collect();
    assert_eq!(
        removed,
        [
            (rows[0].id, inv1.subject(), 200_000),
            (rows[1].id, inv2.subject(), 100_000)
        ]
    );

    let notices = t.notices.of_type(PAYMENT_RETURNED);
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        [NotifyRecipient::Party { id: BLUE_DUNES }]
    );
    // The module never changes an invoice: no further call, no draft.
    assert_eq!(t.invoices.calls().len(), calls_before);
    assert_eq!(t.invoices.drafts(), 0);
    // The credit went with the payment.
    let credit = payment_recorder(t.state())
        .credit_balance(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(credit, 0);
}

/// AC6: for `entered_in_error` the event carries that reason and the party
/// is not told.
#[renox::test]
async fn an_entry_error_reverses_without_telling_the_party() {
    let t = boot().await;
    let (payment, ..) = allocated(&t).await;
    let user = t.user_a().await;

    t.reverse(&user, &payment, "entered_in_error", "")
        .await
        .assert_ok();

    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Reversed);
    let events = t.reversed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, ReversalReason::EnteredInError);
    assert_eq!(events[0].removed.len(), 2);
    assert!(t.notices.of_type(PAYMENT_RETURNED).is_empty());
}

/// AC7: AED 3,000.00 with 2,500.00 allocated and 500.00 refunded; the
/// reversal leaves the party's credit below 0, so the accountant gets one
/// `raise_charge` task and a notice. Nothing is drafted or charged.
#[renox::test]
async fn a_reversal_after_a_refund_asks_the_accountant_to_raise_a_charge() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let inv1 = t.invoices.add(BLUE_DUNES, "INV-1", 250_000);
    t.allocate(&payment, inv1, 250_000).await.unwrap();
    let a = t.user_a().await;
    let b = t.user_b().await;
    let zed = t.accountant("Zed").await;

    let refund = t.request_refund(&a, &payment, 50_000).await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund_action(
        &b,
        &refund,
        "pay",
        &[("method", "transfer"), ("reference", "BANK-1")],
    )
    .await
    .assert_ok();

    t.reverse(&a, &payment, "bank_return", "").await.assert_ok();

    let tasks = t.tasks(&payment).await;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.text, "Raise a charge for AED 500.00");
    assert_eq!(task.amount, 50_000);
    assert_eq!(task.role, "accountant");
    assert_eq!(task.payment_id, Some(payment.id));
    assert_eq!(task.refund_ids.0, [refund.id]);
    assert!(task.is_open());

    let notices = t.notices.of_type(FOLLOWUP_TASK);
    assert_eq!(notices.len(), 1);
    assert_eq!(
        notices[0].recipients,
        [NotifyRecipient::User { id: zed.id }]
    );
    // No charge, no invoice draft; the only invoicer call is the allocation.
    assert_eq!(t.invoices.drafts(), 0);
    assert_eq!(t.invoices.calls().len(), 1);
    let balance = payment_recorder(t.state())
        .credit_balance(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap();
    assert_eq!(balance, -50_000);
}

/// AC7: when the party's credit still covers the refund, there is no task.
#[renox::test]
async fn no_task_while_the_credit_still_covers_the_refund() {
    let t = boot().await;
    let payment = t.receive(BLUE_DUNES, 300_000).await;
    let inv1 = t.invoices.add(BLUE_DUNES, "INV-1", 250_000);
    t.allocate(&payment, inv1, 250_000).await.unwrap();
    // Other credit of the party: 1,000.00 unallocated.
    t.receive(BLUE_DUNES, 100_000).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Zed").await;

    let refund = t.request_refund(&a, &payment, 50_000).await;
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund_action(
        &b,
        &refund,
        "pay",
        &[("method", "transfer"), ("reference", "BANK-1")],
    )
    .await
    .assert_ok();

    t.reverse(&a, &payment, "bank_return", "").await.assert_ok();

    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Reversed);
    assert!(t.tasks(&payment).await.is_empty());
    assert!(t.notices.of_type(FOLLOWUP_TASK).is_empty());
}

/// AC8: no reason (or only spaces), or the chargeback reason, is refused and
/// the payment stays `received`; a second reverse is refused too, with no
/// second event.
#[renox::test]
async fn a_reason_is_required_and_a_second_reverse_is_refused() {
    let t = boot().await;
    let (payment, ..) = allocated(&t).await;
    let user = t.user_a().await;

    for reason in ["", "   ", "chargeback", "because"] {
        t.reverse(&user, &payment, reason, "")
            .await
            .assert_status(422);
    }
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Received);
    assert!(t.reversed.all().is_empty());
    assert!(t.allocations(&payment).await.iter().all(|a| a.is_active()));

    t.reverse(&user, &payment, "bank_return", "")
        .await
        .assert_ok();
    t.reverse(&user, &payment, "bank_return", "")
        .await
        .assert_status(422);
    assert_eq!(t.reversed.all().len(), 1);
    assert_eq!(t.notices.of_type(PAYMENT_RETURNED).len(), 1);
    assert!(t.tasks(&payment).await.is_empty());
}

/// A gateway payment of AED 3,000.00 with `txn`, allocated 1,000.00.
async fn gateway_payment(t: &Booted, txn: &str, dispute: Option<&str>) -> Payment {
    let entity = t.entity("PGP").await;
    let mut payment = Payment {
        number: Some(format!("RCT-PGP-0000-{txn}")),
        legal_entity_id: entity.id,
        party_id: BLUE_DUNES.0,
        method: PaymentMethod::Gateway,
        amount: 300_000,
        currency: "AED".to_owned(),
        gateway: Some("fake".to_owned()),
        gateway_transaction: Some(txn.to_owned()),
        dispute_status: dispute.map(str::to_owned),
        status: PaymentStatus::Received,
        ..Payment::default()
    };
    payment.insert(t.db()).await.unwrap();
    let invoice = t.invoices.add(BLUE_DUNES, "INV-G", 100_000);
    t.allocate(&payment, invoice, 100_000).await.unwrap();
    payment
}

/// AC9: a gateway payment with an open dispute at the gateway, or a stored
/// open dispute, cannot be reversed; it keeps its allocations. Without a
/// dispute it reverses.
#[renox::test]
async fn a_payment_with_an_open_dispute_is_not_reversed() {
    let gateway = FakeGateway::new();
    gateway.dispute("txn_gw", "dp_1");
    let t = boot_gateway(gateway.clone()).await;
    let user = t.user_a().await;

    // The gateway reports the dispute.
    let at_gateway = gateway_payment(&t, "txn_gw", None).await;
    t.reverse(&user, &at_gateway, "bank_return", "")
        .await
        .assert_status(422);
    let refusal = wl_payments::reverse::reverse(
        t.state(),
        None,
        Some(user.id),
        &at_gateway,
        Some(ReversalReason::BankReturn),
        None,
    )
    .await;
    assert!(
        matches!(&refusal, Err(TransitionError::Guard(m)) if m == DISPUTE_MESSAGE),
        "{refusal:?}"
    );
    assert!(gateway.asked().contains(&"txn_gw".to_owned()));
    assert_eq!(t.reload(&at_gateway).await.status, PaymentStatus::Received);
    assert!(
        t.allocations(&at_gateway)
            .await
            .iter()
            .all(|a| a.is_active())
    );

    // A stored dispute is enough, whatever the gateway says.
    let stored = gateway_payment(&t, "txn_stored", Some("open")).await;
    t.reverse(&user, &stored, "bank_return", "")
        .await
        .assert_status(422);
    assert_eq!(t.reload(&stored).await.status, PaymentStatus::Received);

    // No dispute anywhere: it reverses.
    let clean = gateway_payment(&t, "txn_clean", None).await;
    t.reverse(&user, &clean, "entered_in_error", "")
        .await
        .assert_ok();
    assert_eq!(t.reload(&clean).await.status, PaymentStatus::Reversed);
    assert_eq!(t.reversed.all().len(), 1);
}

/// AC10: the reversal date is today in Asia/Dubai (2026-11-03 22:30 UTC is
/// 2026-11-04 02:30 there) and the payment date is unchanged; the source,
/// the statement line and the prepaid record travel in `PaymentReversed`.
#[renox::test]
async fn the_reversal_date_is_today_in_dubai_and_the_event_carries_the_origin() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let mut request = RecordPayment::new(BLUE_DUNES, entity.id, PaymentMethod::Transfer, 300_000);
    request.reference = Some("STMT-1".to_owned());
    request.date = NaiveDate::from_ymd_opt(2026, 11, 1);
    request.source = PaymentSource::BankStatement;
    request.statement_line_id = Some(77);
    request.prepaid = Some(Subject::new("property.deposit", 3));
    let payment = t.receive_request(request).await;
    assert_eq!(payment.payment_date.to_string(), "2026-11-01");
    let user = t.user_a().await;

    t.travel_to("2026-11-03", 22, 30);
    t.reverse(&user, &payment, "bank_return", "")
        .await
        .assert_ok();

    let reversed = t.reload(&payment).await;
    assert_eq!(
        reversed.reversed_on.map(|d| d.to_string()).as_deref(),
        Some("2026-11-04")
    );
    assert_eq!(reversed.payment_date.to_string(), "2026-11-01");
    let events = t.reversed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reversed_on.to_string(), "2026-11-04");
    assert_eq!(events[0].source, PaymentSource::BankStatement);
    assert_eq!(events[0].statement_line, Some(77));
    assert_eq!(events[0].prepaid, Some(Subject::new("property.deposit", 3)));
    assert_eq!(events[0].party, PartyId(1));
}

/// AC12: a user without `payments.reverse` gets 403 from the reverse route,
/// and the payment page shows removed allocations with their reason.
#[renox::test]
async fn reversing_needs_its_permission_and_the_page_shows_removed_allocations() {
    let t = boot().await;
    let (payment, ..) = allocated(&t).await;
    let clerk = t.user("Cora", &[perm::VIEW, perm::RECORD]).await;
    let user = t.user_a().await;

    t.reverse(&clerk, &payment, "bank_return", "")
        .await
        .assert_forbidden();
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Received);
    assert!(t.reversed.all().is_empty());

    t.reverse(&user, &payment, "bank_return", "")
        .await
        .assert_ok();
    t.app.acting_as(&clerk);
    t.app
        .get(&format!("/payments/{}", payment.id))
        .await
        .assert_ok()
        .assert_see("Returned by the bank")
        .assert_see("2,000.00")
        .assert_see("1,000.00");
}

/// The kernel contract reverses too: by the system or by a user id, with the
/// same effects; a second call is refused.
#[renox::test]
async fn the_recorder_reverses_for_another_module() {
    let t = boot().await;
    let (payment, ..) = allocated(&t).await;
    let recorder = payment_recorder(t.state());

    recorder
        .reverse(
            t.state(),
            payment.payment_id(),
            ReversalReason::EnteredInError,
            Some("Wrong party".to_owned()),
            None,
        )
        .await
        .unwrap();
    let reversed = t.reload(&payment).await;
    assert_eq!(reversed.status, PaymentStatus::Reversed);
    assert_eq!(reversed.reversed_by, None);
    assert_eq!(t.reversed.all()[0].removed.len(), 2);

    let again = recorder
        .reverse(
            t.state(),
            payment.payment_id(),
            ReversalReason::EnteredInError,
            None,
            None,
        )
        .await;
    assert!(again.is_err());
    assert_eq!(t.reversed.all().len(), 1);
}
