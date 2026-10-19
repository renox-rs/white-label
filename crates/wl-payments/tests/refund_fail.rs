//! Failing a refund and restoring its source: the acceptance checks AC1 to
//! AC4 and AC6 to AC8 of #99 (AC5, the deposit, is in `deposits.rs`).

mod support;

use renox::auth::User;
use renox::prelude::Model;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{
    PaymentMethod, PaymentStatus, RefundFailedFrom, RefundFailureKind, RefundSource,
    ReversalReason, payment_recorder,
};
use wl_kernel::{PartyId, Subject};
use wl_payments::gateway::{FakeGateway, RefundOutcome};
use wl_payments::notices::{REFUND_DELAYED, REFUND_FAILED};
use wl_payments::refund::{DECLINED_BY_GATEWAY, RefundStatus, gateway_refund_outcome};
use wl_payments::refund_attempt::AttemptOutcome;
use wl_payments::{FollowupTask, Payment, Refund, RefundAttempt, permissions as perm};

use support::{BLUE_DUNES, Booted, DUSK_HOLDINGS, boot, boot_gateway, year};

fn credit_note() -> RefundSource {
    RefundSource::CreditNote(Subject::new("invoicing.credit_note", 7))
}

fn declined(reason: &str) -> RefundOutcome {
    RefundOutcome::Declined {
        reason: reason.to_owned(),
    }
}

async fn credit(t: &Booted) -> i64 {
    payment_recorder(t.state())
        .credit_balance(t.db(), BLUE_DUNES, "AED")
        .await
        .unwrap()
}

/// An approved gateway refund of `amount` fils to `party` out of the credit
/// note, requested by `a` and approved by `b`.
async fn approved_gateway_refund(
    t: &Booted,
    a: &User,
    b: &User,
    party: PartyId,
    amount: i64,
) -> Refund {
    let refund = t
        .request_for_source(a, party, &credit_note(), amount, "gateway", "txn_1")
        .await;
    t.refund_action(b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund(refund.id).await
}

/// A transfer refund of `amount` fils out of a payment of the same size,
/// requested by `a`, approved by `b` and paid by `a`.
async fn paid_refund(t: &Booted, a: &User, b: &User, amount: i64) -> (Payment, Refund) {
    let payment = t.receive(BLUE_DUNES, amount).await;
    let refund = t.request_refund(a, &payment, amount).await;
    t.refund_action(b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund_action(
        a,
        &refund,
        "pay",
        &[("method", "transfer"), ("reference", "BANK-9")],
    )
    .await
    .assert_ok();
    let paid = t.refund(refund.id).await;
    assert_eq!(paid.status, RefundStatus::Paid);
    (payment, paid)
}

fn users(recipients: &[NotifyRecipient]) -> Vec<i64> {
    recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

async fn latest_attempt(t: &Booted, refund: &Refund) -> RefundAttempt {
    RefundAttempt::latest(t.db(), refund.id)
        .await
        .unwrap()
        .expect("an attempt")
}

/// AC1: a gateway decline fails the approved refund, closes attempt 1 with
/// the gateway's reason, emits `RefundFailed` once with the credit note as
/// source, and tells the requester and the accountant (and the party that
/// the refund is delayed).
#[renox::test]
async fn a_gateway_decline_fails_the_refund_and_tells_everyone() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let accountant = t.accountant("Cara").await;
    let refund = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 50_000).await;
    assert_eq!(refund.status, RefundStatus::Approved);
    assert_eq!(refund.method, Some(PaymentMethod::Gateway));
    assert_eq!(refund.source(), Some(credit_note()));

    t.refund_action(&a, &refund, "send", &[])
        .await
        .assert_ok()
        .assert_see("attempt 1");
    assert_eq!(t.gateway.refunded(), vec![("txn_1".to_owned(), 50_000)]);
    let attempts = RefundAttempt::of_refund(t.db(), refund.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].number, 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Pending);
    let gateway_ref = attempts[0]
        .gateway_ref
        .clone()
        .expect("the gateway's reference");
    assert!(t.refund_failed.all().is_empty());

    let failed = gateway_refund_outcome(t.state(), &gateway_ref, declined("card expired"))
        .await
        .unwrap()
        .expect("the outcome is applied");
    assert_eq!(failed.status, RefundStatus::Failed);
    assert_eq!(
        failed.failure_kind,
        Some(RefundFailureKind::GatewayDeclined)
    );

    let attempts = RefundAttempt::of_refund(t.db(), refund.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Failed);
    assert_eq!(
        attempts[0].failure_kind,
        Some(RefundFailureKind::GatewayDeclined)
    );
    assert_eq!(attempts[0].failure_reason.as_deref(), Some("card expired"));
    assert_eq!(attempts[0].failed_by, None);

    let events = t.refund_failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].refund, refund.refund_id());
    assert_eq!(events[0].party, BLUE_DUNES);
    assert_eq!(events[0].source, credit_note());
    assert_eq!(events[0].amount, 50_000);
    assert_eq!(events[0].currency, "AED");
    assert_eq!(events[0].method, PaymentMethod::Gateway);
    assert_eq!(events[0].kind, RefundFailureKind::GatewayDeclined);
    assert_eq!(events[0].reason, "card expired");
    assert_eq!(events[0].attempt.number, 1);
    assert_eq!(events[0].attempt.id, attempts[0].id);
    assert_eq!(events[0].from, RefundFailedFrom::Approved);
    assert_eq!(events[0].outgoing_payment, None);

    let staff = t.notices.of_type(REFUND_FAILED);
    assert_eq!(staff.len(), 1);
    let mut told = users(&staff[0].recipients);
    told.sort_unstable();
    let mut expected = vec![a.id, accountant.id];
    expected.sort_unstable();
    assert_eq!(told, expected);
    assert_eq!(staff[0].subject, refund.refund_id().subject());
    let text = staff[0].data.to_string();
    assert!(text.contains("500.00"), "{text}");
    assert!(text.contains("card expired"), "{text}");
    assert!(text.contains(&format!("/refunds/{}", refund.id)), "{text}");
    let party = t.notices.of_type(REFUND_DELAYED);
    assert_eq!(party.len(), 1);
    assert_eq!(
        party[0].recipients,
        vec![NotifyRecipient::Party { id: BLUE_DUNES }]
    );
    // No outgoing payment was ever made.
    assert_eq!(Payment::query().count(t.db()).await.unwrap(), 0);
}

/// AC2: the same outcome delivered again changes nothing; a `Confirmed`
/// outcome for a pending attempt pays the refund and emits `RefundPaid`.
#[renox::test]
async fn a_repeated_outcome_changes_nothing_and_a_confirmation_pays() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;

    let first = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 50_000).await;
    t.refund_action(&a, &first, "send", &[]).await.assert_ok();
    let first_ref = latest_attempt(&t, &first)
        .await
        .gateway_ref
        .expect("a reference");
    assert!(
        gateway_refund_outcome(t.state(), &first_ref, declined("card expired"))
            .await
            .unwrap()
            .is_some()
    );
    let notices = t.notices.all().len();
    assert_eq!(t.refund_failed.all().len(), 1);

    // The webhook is delivered again, in both senses: nothing happens.
    for outcome in [declined("card expired"), RefundOutcome::Confirmed] {
        assert!(
            gateway_refund_outcome(t.state(), &first_ref, outcome)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        gateway_refund_outcome(t.state(), "re_nobody", RefundOutcome::Confirmed)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(t.refund(first.id).await.status, RefundStatus::Failed);
    assert_eq!(t.refund_failed.all().len(), 1);
    assert_eq!(t.notices.all().len(), notices);
    assert!(t.refund_paid.all().is_empty());

    // A second refund is confirmed by the gateway.
    let second = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 20_000).await;
    t.refund_action(&a, &second, "send", &[]).await.assert_ok();
    let attempt = latest_attempt(&t, &second).await;
    assert_eq!(attempt.number, 1);
    assert_eq!(attempt.gateway_ref.as_deref(), Some("re_fake_2"));
    let paid = gateway_refund_outcome(t.state(), "re_fake_2", RefundOutcome::Confirmed)
        .await
        .unwrap()
        .expect("applied");
    assert_eq!(paid.status, RefundStatus::Paid);
    assert_eq!(paid.payment_id, None);
    assert_eq!(
        latest_attempt(&t, &second).await.outcome,
        AttemptOutcome::Paid
    );
    let events = t.refund_paid.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].refund, second.refund_id());
    assert_eq!(events[0].amount, 20_000);
    assert_eq!(events[0].source, credit_note());
    assert_eq!(t.refund_failed.all().len(), 1);
    // Confirmed twice: ignored.
    assert!(
        gateway_refund_outcome(t.state(), "re_fake_2", RefundOutcome::Confirmed)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(t.refund_paid.all().len(), 1);
}

/// A decline without a text is stored with the default one.
#[renox::test]
async fn a_decline_without_a_text_gets_the_default_reason() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let refund = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 50_000).await;
    t.refund_action(&a, &refund, "send", &[]).await.assert_ok();
    gateway_refund_outcome(t.state(), "re_fake_1", declined("   "))
        .await
        .unwrap();
    let attempt = latest_attempt(&t, &refund).await;
    assert_eq!(attempt.failure_reason.as_deref(), Some(DECLINED_BY_GATEWAY));
    assert_eq!(t.refund_failed.all()[0].reason, DECLINED_BY_GATEWAY);
}

/// A gateway refund goes to the gateway only when approved, with its
/// transaction, once at a time; the gateway may refuse the call.
#[renox::test]
async fn sending_to_the_gateway_is_guarded() {
    let gateway = FakeGateway::new();
    gateway.refuse_refund("txn_bad", "unknown transaction");
    let t = boot_gateway(gateway).await;
    let a = t.user_a().await;
    let b = t.user_b().await;

    // A gateway refund needs its transaction.
    t.app.acting_as(&a);
    let entity = t.entity("PGP").await.id.to_string();
    t.app
        .htmx()
        .post(
            "/test/refunds/source",
            &[
                ("source_kind", "credit_note"),
                ("source_type", "invoicing.credit_note"),
                ("source_id", "7"),
                ("party", "1"),
                ("entity", entity.as_str()),
                ("amount", "100"),
                ("method", "gateway"),
                ("reason", "x"),
            ],
        )
        .await
        .assert_status(422);

    let refund = t
        .request_for_source(&a, BLUE_DUNES, &credit_note(), 50_000, "gateway", "txn_bad")
        .await;
    // Not approved yet.
    t.refund_action(&a, &refund, "send", &[])
        .await
        .assert_status(422);
    t.refund_action(&b, &refund, "approve", &[])
        .await
        .assert_ok();
    // The gateway refuses the call: nothing is written.
    t.refund_action(&a, &refund, "send", &[])
        .await
        .assert_status(500);
    assert!(
        RefundAttempt::of_refund(t.db(), refund.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Approved);

    let good = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 50_000).await;
    t.refund_action(&a, &good, "send", &[]).await.assert_ok();
    // Already with the gateway.
    t.refund_action(&a, &good, "send", &[])
        .await
        .assert_status(422);
    // And it cannot be paid by hand meanwhile.
    t.refund_action(
        &a,
        &good,
        "pay",
        &[("method", "transfer"), ("reference", "X")],
    )
    .await
    .assert_status(422);
    assert_eq!(
        RefundAttempt::of_refund(t.db(), good.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// AC3: a bank return of a paid transfer refund of AED 1,000.00 fails the
/// refund, reverses the outgoing payment `PV-PGP-<year>-0001` with
/// `bank_return` and gives the credit back; no returned-payment notice and
/// no follow-up task for an outgoing payment.
#[renox::test]
async fn a_bank_return_fails_a_paid_refund_and_restores_the_credit() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let accountant = t.accountant("Cara").await;
    assert_eq!(credit(&t).await, 0);
    let (payment, paid) = paid_refund(&t, &a, &b, 100_000).await;
    assert_eq!(credit(&t).await, 0);
    let voucher_id = paid.payment_id.expect("the voucher");
    let voucher = t.payment(voucher_id).await;
    assert_eq!(
        voucher.number.as_deref(),
        Some(&*format!("PV-PGP-{}-0001", year()))
    );
    assert_eq!(voucher.status, PaymentStatus::Received);

    t.fail_refund(&a, &paid, "bank_return", "account closed")
        .await
        .assert_ok();

    let failed = t.refund(paid.id).await;
    assert_eq!(failed.status, RefundStatus::Failed);
    assert_eq!(failed.failure_kind, Some(RefundFailureKind::BankReturn));
    assert_eq!(failed.failure_reason.as_deref(), Some("account closed"));
    let voucher = t.payment(voucher_id).await;
    assert_eq!(voucher.status, PaymentStatus::Reversed);
    assert_eq!(voucher.reversal_reason, Some(ReversalReason::BankReturn));
    assert_eq!(credit(&t).await, 100_000);
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Received);

    let attempts = RefundAttempt::of_refund(t.db(), paid.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Failed);
    assert_eq!(attempts[0].payment_id, Some(voucher_id));
    assert_eq!(attempts[0].failed_by, Some(a.id));

    let events = t.refund_failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from, RefundFailedFrom::Paid);
    assert_eq!(events[0].kind, RefundFailureKind::BankReturn);
    assert_eq!(events[0].outgoing_payment, Some(voucher.payment_id()));
    assert_eq!(
        events[0].source,
        RefundSource::Payment(payment.payment_id())
    );

    // The people concerned are told by the refund notices only.
    assert!(t.notices.of_type("payments.payment_returned").is_empty());
    assert_eq!(FollowupTask::query().count(t.db()).await.unwrap(), 0);
    let staff = t.notices.of_type(REFUND_FAILED);
    assert_eq!(staff.len(), 1);
    assert!(users(&staff[0].recipients).contains(&accountant.id));
    assert_eq!(t.notices.of_type(REFUND_DELAYED).len(), 1);
}

/// AC4: no reason or a blank one is refused and the refund stays paid; a
/// second `fail` is refused; a user cannot give `gateway_declined`; the fail
/// route needs `refunds.pay`.
#[renox::test]
async fn failing_is_guarded() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let clerk = t.user("Cora", &[perm::VIEW, perm::REFUNDS_REQUEST]).await;
    let (_payment, paid) = paid_refund(&t, &a, &b, 100_000).await;

    for reason in ["", "   "] {
        t.fail_refund(&a, &paid, "bank_return", reason)
            .await
            .assert_status(422);
    }
    t.fail_refund(&a, &paid, "gateway_declined", "declined")
        .await
        .assert_status(422);
    assert_eq!(t.refund(paid.id).await.status, RefundStatus::Paid);
    assert!(t.refund_failed.all().is_empty());
    assert!(t.notices.of_type(REFUND_FAILED).is_empty());
    assert_eq!(
        latest_attempt(&t, &paid).await.outcome,
        AttemptOutcome::Paid
    );

    // The module's own route (the fail sheet) needs `refunds.pay`.
    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post(
            &format!("/refunds/{}/fail", paid.id),
            &[("kind", "bank_return"), ("reason", "account closed")],
        )
        .await
        .assert_forbidden();
    t.fail_refund(&clerk, &paid, "bank_return", "account closed")
        .await
        .assert_forbidden();
    assert_eq!(t.refund(paid.id).await.status, RefundStatus::Paid);

    // The route refuses a blank reason (422) and a chosen decline, then fails.
    t.app.acting_as(&a);
    let route = format!("/refunds/{}/fail", paid.id);
    t.app
        .htmx()
        .post(&route, &[("kind", "bank_return"), ("reason", " ")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&route, &[("kind", "gateway_declined"), ("reason", "x")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &route,
            &[("kind", "card_unavailable"), ("reason", "card closed")],
        )
        .await
        .assert_ok();
    assert_eq!(t.refund(paid.id).await.status, RefundStatus::Failed);

    // A second `fail`, by either route, is refused and sends nothing.
    t.fail_refund(&a, &paid, "bank_return", "again")
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&route, &[("kind", "bank_return"), ("reason", "again")])
        .await
        .assert_status(422);
    assert_eq!(t.refund_failed.all().len(), 1);
    assert_eq!(t.notices.of_type(REFUND_FAILED).len(), 1);
    assert_eq!(
        RefundAttempt::of_refund(t.db(), paid.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// An approved gateway refund can be failed by a user too once it was sent:
/// the card is unavailable. There is no outgoing payment to reverse.
#[renox::test]
async fn an_approved_gateway_refund_fails_by_a_user_with_its_pending_attempt() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let refund = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 50_000).await;
    // Nothing was tried yet: there is no attempt to fail.
    t.fail_refund(&a, &refund, "card_unavailable", "card closed")
        .await
        .assert_status(422);
    t.refund_action(&a, &refund, "send", &[]).await.assert_ok();
    t.fail_refund(&a, &refund, "card_unavailable", "card closed")
        .await
        .assert_ok();
    let attempt = latest_attempt(&t, &refund).await;
    assert_eq!(attempt.outcome, AttemptOutcome::Failed);
    assert_eq!(
        attempt.failure_kind,
        Some(RefundFailureKind::CardUnavailable)
    );
    assert_eq!(attempt.failed_by, Some(a.id));
    let events = t.refund_failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].from, RefundFailedFrom::Approved);
    // The gateway's late answer for the failed attempt is ignored.
    assert!(
        gateway_refund_outcome(t.state(), "re_fake_1", RefundOutcome::Confirmed)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
}

/// AC6: at 2026-12-01 21:00 UTC a bank return of a refund paid on 2026-11-03
/// is dated 2026-12-02 (Asia/Dubai); the voucher keeps its date and the
/// failed attempt stays stored.
#[renox::test]
async fn a_late_bank_return_is_dated_today_in_dubai() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.travel_to("2026-11-03", 10, 0);
    let (_payment, paid) = paid_refund(&t, &a, &b, 100_000).await;
    let voucher_id = paid.payment_id.expect("the voucher");
    assert_eq!(
        t.payment(voucher_id).await.payment_date.to_string(),
        "2026-11-03"
    );

    t.travel_to("2026-12-01", 21, 0);
    t.fail_refund(&a, &paid, "bank_return", "account closed")
        .await
        .assert_ok();

    let voucher = t.payment(voucher_id).await;
    assert_eq!(
        voucher.reversed_on.map(|d| d.to_string()).as_deref(),
        Some("2026-12-02")
    );
    assert_eq!(voucher.payment_date.to_string(), "2026-11-03");
    let attempts = RefundAttempt::of_refund(t.db(), paid.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Failed);
    assert_eq!(attempts[0].started_on.to_string(), "2026-11-03");
    assert_eq!(
        attempts[0].failed_on.map(|d| d.to_string()).as_deref(),
        Some("2026-12-02")
    );
}

/// AC7: a blocked party is not told that its refund is delayed; the
/// requester and the accountant still are.
#[renox::test]
async fn a_blocked_party_is_not_told_but_the_staff_is() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let accountant = t.accountant("Cara").await;
    let refund = approved_gateway_refund(&t, &a, &b, DUSK_HOLDINGS, 50_000).await;
    t.refund_action(&a, &refund, "send", &[]).await.assert_ok();
    gateway_refund_outcome(t.state(), "re_fake_1", declined("card expired"))
        .await
        .unwrap();

    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
    assert_eq!(t.refund_failed.all().len(), 1);
    assert!(t.notices.of_type(REFUND_DELAYED).is_empty());
    let staff = t.notices.of_type(REFUND_FAILED);
    assert_eq!(staff.len(), 1);
    let told = users(&staff[0].recipients);
    assert!(told.contains(&a.id));
    assert!(told.contains(&accountant.id));
}

/// AC8: the refund pages answer 200 with `payments.view` and 403 without;
/// the refund page lists the attempts with their failure reason.
#[renox::test]
async fn the_refund_pages_need_the_view_permission_and_list_the_attempts() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    let nobody = t.user("Nora", &[]).await;
    let refund = approved_gateway_refund(&t, &a, &b, BLUE_DUNES, 50_000).await;
    t.refund_action(&a, &refund, "send", &[]).await.assert_ok();
    gateway_refund_outcome(t.state(), "re_fake_1", declined("card expired"))
        .await
        .unwrap();

    t.app.acting_as(&viewer);
    t.app.get("/refunds").await.assert_ok();
    t.app
        .get(&format!("/refunds/{}", refund.id))
        .await
        .assert_ok()
        .assert_see("card expired")
        .assert_see("re_fake_1")
        .assert_see("500.00");
    t.app.acting_as(&nobody);
    t.app.get("/refunds").await.assert_forbidden();
    t.app
        .get(&format!("/refunds/{}", refund.id))
        .await
        .assert_forbidden();
}
