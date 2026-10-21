//! A late gateway confirmation (wl-payments §4 `late_confirm`, the refund
//! `request` of a late payment, §7 *Late payment refunds*; issue #96): a
//! payment the gateway confirms after it failed is allocated when its record
//! is still active and otherwise kept as credit and refunded to the same card.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{PaymentFailureKind, PaymentMethod, PaymentStatus, RecordPayment};
use wl_payments::gateway::FakeGateway;
use wl_payments::late::REASON;
use wl_payments::online;
use wl_payments::payment_link::{self, LinkStatus, NewLink, PaymentLink};
use wl_payments::refund::{Refund, RefundStatus};
use wl_payments::{Payment, WlPayments};

use support::{BLUE_DUNES, Booted, boot, boot_with};

fn booking() -> Subject {
    Subject::new("scheduling.booking", 3)
}

/// The moment 18:00 in Dubai on 2030-01-07.
fn expiry() -> renox::db::DateTime {
    NaiveDate::from_ymd_opt(2030, 1, 7)
        .unwrap()
        .and_hms_opt(18, 0, 0)
        .unwrap()
        .and_utc()
        - renox::chrono::TimeDelta::hours(4)
}

/// An AED 250.00 payment of Blue Dunes on a link for the booking, started and
/// failed (timeout), waiting for a late confirmation.
async fn failed_booking_payment(t: &Booted, link: &PaymentLink) -> Payment {
    let payment = t.start_online(link).await;
    let txn = payment.gateway_transaction.clone().unwrap();
    online::gateway_failed(t.state(), &txn, PaymentFailureKind::Timeout, "")
        .await
        .unwrap()
        .expect("failed");
    t.reload(&payment).await
}

/// The gateway confirms the failed `payment` now.
async fn confirm_late(t: &Booted, payment: &Payment) -> Option<Payment> {
    online::gateway_succeeded(
        t.state(),
        payment.gateway_transaction.as_deref().unwrap(),
        payment.amount,
    )
    .await
    .unwrap()
}

async fn refunds_of(t: &Booted, payment: &Payment) -> Vec<Refund> {
    Refund::query()
        .where_(Refund::SOURCE_ID.eq(payment.id))
        .where_(Refund::SOURCE_KIND.eq("payment"))
        .order_by(Refund::ID)
        .get(t.db())
        .await
        .unwrap()
}

fn told(t: &Booted, user: i64) -> bool {
    t.notices
        .recipients(wl_payments::notices::ONLINE_TASK)
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user))
}

/// A failed AED 250.00 payment for a booking still on hold with its invoice
/// open, and a verified late webhook: `received`, a receipt number,
/// allocated, the link paid, `PaymentReceived`; no refund.
#[renox::test]
async fn a_late_confirmation_for_a_booking_still_on_hold_is_allocated() {
    let t = boot().await;
    let accountant = t.accountant("Rita").await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t
        .link(BLUE_DUNES, 25_000, Some(invoice), Some(booking()), None)
        .await;
    let payment = failed_booking_payment(&t, &link).await;
    assert_eq!(payment.status, PaymentStatus::Failed);

    let late = confirm_late(&t, &payment).await.expect("confirmed late");
    assert_eq!(late.status, PaymentStatus::Received);
    assert!(late.number.is_some(), "a receipt number");
    assert!(late.late_confirmed_at.is_some());
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].invoice, calls[0].amount), (invoice, 25_000));
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Paid);
    let events = t.received.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].allocations.len(), 1);
    assert!(refunds_of(&t, &payment).await.is_empty());
    assert!(told(&t, accountant.id), "the accountant is told");
}

/// The booking hold was released on `PaymentLinkExpired` (its slot is gone):
/// the AED 250.00 stays as credit, a refund is `approved` at once with the
/// reason "late confirmation", paid to the same card, and the accountant is
/// told. A replayed webhook does nothing.
#[renox::test]
async fn a_late_confirmation_after_the_hold_was_released_is_refunded_at_once() {
    let gateway = FakeGateway::new();
    let t = boot_with(WlPayments::new().gateway(gateway.clone()), gateway.clone()).await;
    let accountant = t.accountant("Rita").await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let entity = t.entity("PGP").await;
    let new = NewLink::new(BLUE_DUNES, entity.id, 25_000, "AED")
        .target(&invoice.subject(), 25_000)
        .prepaid(booking())
        .expires_at(expiry());
    let link = payment_link::create(t.state(), new).await.unwrap();
    let payment = failed_booking_payment(&t, &link).await;
    // The link expires; wl-scheduling reacts by releasing the hold.
    payment_link::expire_due(t.state(), expiry()).await.unwrap();
    let released = t.link_expired.all();
    assert_eq!(released.len(), 1);
    t.prepaid.release(released[0].prepaid.as_ref().unwrap());

    let late = confirm_late(&t, &payment).await.expect("confirmed late");
    assert_eq!(late.status, PaymentStatus::Received);
    // Not allocated; the money is the party's credit.
    assert!(t.invoices.calls().is_empty());
    let summary = t.summary(&late).await;
    assert_eq!((summary.allocated, summary.unallocated), (0, 25_000));
    // The refund: approved at once, nobody requested or approved it.
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    let refund = &refunds[0];
    assert_eq!(refund.status, RefundStatus::Approved);
    assert_eq!(refund.reason, REASON);
    assert_eq!(refund.amount, 25_000);
    assert_eq!(refund.requested_by, None);
    assert_eq!(refund.approved_by, None);
    assert!(refund.approved_at.is_some());
    assert_eq!(refund.method, Some(PaymentMethod::Gateway));
    assert_eq!(refund.gateway_txn, payment.gateway_transaction);
    // Paid to the same card.
    assert_eq!(
        gateway.refunded(),
        [(payment.gateway_transaction.clone().unwrap(), 25_000)]
    );
    assert!(told(&t, accountant.id), "the accountant is told");
    let texts: Vec<String> = t
        .notices
        .of_type(wl_payments::notices::ONLINE_TASK)
        .iter()
        .map(|n| n.data.to_string())
        .collect();
    assert!(
        texts.iter().any(|d| d.contains("refunded to the same card")),
        "{texts:?}"
    );

    // The webhook again changes nothing.
    assert!(confirm_late(&t, &late).await.is_none());
    assert_eq!(refunds_of(&t, &payment).await.len(), 1);
    assert_eq!(t.received.all().len(), 1);
    assert_eq!(gateway.refunded().len(), 1);
}

/// The same when the booking was cancelled with its link still open.
#[renox::test]
async fn a_late_confirmation_for_a_cancelled_booking_is_refunded_at_once() {
    let gateway = FakeGateway::new();
    let t = boot_with(WlPayments::new().gateway(gateway.clone()), gateway.clone()).await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t
        .link(BLUE_DUNES, 25_000, Some(invoice), Some(booking()), None)
        .await;
    let payment = failed_booking_payment(&t, &link).await;
    t.prepaid.release(&booking());

    confirm_late(&t, &payment).await.expect("confirmed late");
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Approved);
    assert_eq!(gateway.refunded().len(), 1);
    assert!(t.invoices.calls().is_empty());
}

/// The same when another payment already paid the booking.
#[renox::test]
async fn a_late_confirmation_for_a_booking_paid_by_another_payment_is_refunded_at_once() {
    let gateway = FakeGateway::new();
    let t = boot_with(WlPayments::new().gateway(gateway.clone()), gateway.clone()).await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t
        .link(BLUE_DUNES, 25_000, Some(invoice), Some(booking()), None)
        .await;
    let payment = failed_booking_payment(&t, &link).await;
    // The party paid at the desk meanwhile.
    let entity = t.entity("PGP").await;
    t.receive_request(
        RecordPayment::new(BLUE_DUNES, entity.id, PaymentMethod::Cash, 25_000)
            .prepaid(booking()),
    )
    .await;

    confirm_late(&t, &payment).await.expect("confirmed late");
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Approved);
    assert_eq!(refunds[0].reason, REASON);
    assert_eq!(gateway.refunded().len(), 1);
    assert!(t.invoices.calls().is_empty());
}

/// The same when the invoice of the booking was paid by another payment.
#[renox::test]
async fn a_late_confirmation_for_an_invoice_paid_meanwhile_is_refunded_at_once() {
    let gateway = FakeGateway::new();
    let t = boot_with(WlPayments::new().gateway(gateway.clone()), gateway.clone()).await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t
        .link(BLUE_DUNES, 25_000, Some(invoice), Some(booking()), None)
        .await;
    let payment = failed_booking_payment(&t, &link).await;
    t.invoices.set_balance(invoice, 0);

    confirm_late(&t, &payment).await.expect("confirmed late");
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Approved);
    assert!(t.invoices.calls().is_empty());
    // The link is not paid: nothing was allocated.
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);
}

/// With the setting off the refund is `requested` and follows `FIN.ACL.04`:
/// another user approves it.
#[renox::test]
async fn with_the_setting_off_the_refund_waits_for_a_second_person() {
    let gateway = FakeGateway::new();
    let t = boot_with(
        WlPayments::new()
            .gateway(gateway.clone())
            .late_refunds_auto(false),
        gateway.clone(),
    )
    .await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t
        .link(BLUE_DUNES, 25_000, Some(invoice), Some(booking()), None)
        .await;
    let payment = failed_booking_payment(&t, &link).await;
    t.prepaid.release(&booking());

    confirm_late(&t, &payment).await.expect("confirmed late");
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert_eq!(refunds[0].reason, REASON);
    assert!(gateway.refunded().is_empty());

    // Another user approves it; it is not sent until a user pays it.
    let b = t.user_b().await;
    t.refund_action(&b, &refunds[0], "approve", &[])
        .await
        .assert_ok();
    assert_eq!(t.refund(refunds[0].id).await.status, RefundStatus::Approved);
}

/// A booking still active but its link partly open: the open invoice is
/// allocated, the rest stays as credit and a refund of the rest is
/// `requested` (it does not equal the payment).
#[renox::test]
async fn a_partly_open_link_allocates_what_is_open_and_requests_the_rest() {
    let gateway = FakeGateway::new();
    let t = boot_with(WlPayments::new().gateway(gateway.clone()), gateway.clone()).await;
    let first = t.invoices.add(BLUE_DUNES, "INV-1", 10_000);
    let second = t.invoices.add(BLUE_DUNES, "INV-2", 15_000);
    let entity = t.entity("PGP").await;
    let new = NewLink::new(BLUE_DUNES, entity.id, 25_000, "AED")
        .target(&first.subject(), 10_000)
        .target(&second.subject(), 15_000)
        .prepaid(booking());
    let link = payment_link::create(t.state(), new).await.unwrap();
    let payment = failed_booking_payment(&t, &link).await;
    // The first invoice was paid by another payment meanwhile.
    t.invoices.set_balance(first, 0);

    confirm_late(&t, &payment).await.expect("confirmed late");
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].invoice, calls[0].amount), (second, 15_000));
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].amount, 10_000);
    assert_eq!(refunds[0].status, RefundStatus::Requested);
    assert!(gateway.refunded().is_empty());
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Paid);
}

/// A confirmation of another amount is not confirmed: the payment stays
/// `failed`, the accountant gets a task.
#[renox::test]
async fn a_late_confirmation_of_another_amount_gives_the_accountant_a_task() {
    let t = boot().await;
    let accountant = t.accountant("Rita").await;
    let link = t.link(BLUE_DUNES, 25_000, None, Some(booking()), None).await;
    let payment = failed_booking_payment(&t, &link).await;

    let refused = online::gateway_succeeded(
        t.state(),
        payment.gateway_transaction.as_deref().unwrap(),
        20_000,
    )
    .await;
    assert!(refused.is_err());
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Failed);
    assert_eq!(t.tasks(&payment).await.len(), 1);
    assert!(told(&t, accountant.id));
    assert!(t.received.all().is_empty());
}

/// A blocked party's refund is approved but not sent: money out waits
/// (`PTY.ACL.03`).
#[renox::test]
async fn the_refund_of_a_late_payment_to_a_blocked_party_waits() {
    let gateway = FakeGateway::new();
    let t = boot_with(WlPayments::new().gateway(gateway.clone()), gateway.clone()).await;
    let link = t.link(BLUE_DUNES, 25_000, None, Some(booking()), None).await;
    let payment = failed_booking_payment(&t, &link).await;
    t.prepaid.release(&booking());
    t.blocked.block(BLUE_DUNES);

    confirm_late(&t, &payment).await.expect("confirmed late");
    let refunds = refunds_of(&t, &payment).await;
    assert_eq!(refunds.len(), 1);
    assert_eq!(refunds[0].status, RefundStatus::Approved);
    assert!(gateway.refunded().is_empty(), "nothing goes out to a blocked party");
}
