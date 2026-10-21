//! Online payments (wl-payments §4 `start_online`, `confirm`, `fail`, the
//! payment link and its `expire`, `FIN.RUL.19`; issue #95): what the party
//! sees on the public page of a link, what the gateway's answers and the
//! timeout do to a `pending` payment, and the link's expiry.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{PAYMENT_LINK_SUBJECT_TYPE, PaymentFailureKind, PaymentStatus};
use wl_payments::gateway::GatewayPaymentStatus;
use wl_payments::online;
use wl_payments::payment_link::{self, LinkStatus, NewLink, PaymentLink};

use support::{BLUE_DUNES, boot};

fn booking() -> Subject {
    Subject::new("scheduling.booking", 3)
}

/// The moment 18:00 in Dubai on 2030-01-07 (UTC+4, no summer time).
fn expiry() -> renox::db::DateTime {
    NaiveDate::from_ymd_opt(2030, 1, 7)
        .unwrap()
        .and_hms_opt(18, 0, 0)
        .unwrap()
        .and_utc()
        - TimeDelta::hours(4)
}

/// A decline webhook fails the `pending` payment, the link stays `open`, the
/// party sees the reason and a retry button, `PaymentFailed` carries the link
/// and the prepaid booking, the invoice is untouched and no task is made.
#[renox::test]
async fn a_declined_payment_fails_and_the_party_may_try_again() {
    let t = boot().await;
    let staff = t.user_a().await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t
        .link(BLUE_DUNES, 25_000, Some(invoice), Some(booking()), Some(&staff))
        .await;
    let payment = t.start_online(&link).await;
    assert_eq!(payment.payment_link_id, Some(link.id));
    assert_eq!(payment.collected_by, Some(staff.id));
    assert_eq!(payment.number, None);
    let txn = payment.gateway_transaction.clone().expect("a transaction");

    let failed = online::gateway_failed(
        t.state(),
        &txn,
        PaymentFailureKind::Declined,
        "Card declined",
    )
    .await
    .unwrap()
    .expect("the payment failed");
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(failed.failure_kind, Some(PaymentFailureKind::Declined));
    assert_eq!(failed.failure_reason.as_deref(), Some("Card declined"));

    // The link stays open.
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);
    // `PaymentFailed` names the link and the prepaid booking.
    let events = t.failed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payment, payment.payment_id());
    assert_eq!(events[0].kind, PaymentFailureKind::Declined);
    assert_eq!(events[0].reason, "Card declined");
    assert_eq!(
        events[0].payment_link,
        Some(Subject::new(PAYMENT_LINK_SUBJECT_TYPE, link.id))
    );
    assert_eq!(events[0].prepaid, Some(booking()));
    // The invoice is unchanged, nothing was received and no task was made.
    assert!(t.invoices.calls().is_empty());
    assert!(t.received.all().is_empty());
    assert!(t.tasks(&failed).await.is_empty());

    // The party sees the reason and a retry button.
    t.app.logout();
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("Card declined")
        .assert_see("Try again");

    // A retry is a new payment on the same link; the failed one stays.
    let response = t.press_pay(&link).await;
    response.assert_status(303);
    let retry = t.latest_on(&link).await;
    assert_ne!(retry.id, payment.id);
    assert_eq!(retry.status, PaymentStatus::Pending);
    assert_eq!(
        response.header("location"),
        Some(format!("https://pay.example/{}", retry.gateway_transaction.unwrap()).as_str())
    );
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Failed);
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);
}

/// A payment still `pending` after 30 minutes that the gateway reports paid
/// is `received` and allocated; before the 30 minutes nothing happens.
#[renox::test]
async fn a_pending_payment_the_gateway_reports_paid_is_received_after_the_timeout() {
    let t = boot().await;
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", 25_000);
    let link = t.link(BLUE_DUNES, 25_000, Some(invoice), None, None).await;
    let payment = t.start_online(&link).await;
    let txn = payment.gateway_transaction.clone().unwrap();
    t.gateway.set_status(&txn, GatewayPaymentStatus::Paid);
    let since = payment.created_at.expect("a creation time");

    let early = online::check_pending(t.state(), since + TimeDelta::minutes(29))
        .await
        .unwrap();
    assert_eq!((early.confirmed, early.failed), (0, 0));
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Pending);

    let report = online::check_pending(t.state(), since + TimeDelta::minutes(30))
        .await
        .unwrap();
    assert_eq!((report.confirmed, report.failed), (1, 0));
    let received = t.reload(&payment).await;
    assert_eq!(received.status, PaymentStatus::Received);
    assert!(received.number.is_some(), "a receipt number");
    // Allocated to the invoice, and the link is paid.
    let calls = t.invoices.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].invoice, calls[0].amount), (invoice, 25_000));
    assert_eq!(calls[0].source, payment.subject());
    assert_eq!(t.allocations(&received).await.len(), 1);
    let link = t.link_now(&link).await;
    assert_eq!(link.status, LinkStatus::Paid);
    assert_eq!(link.paid_payment_id, Some(payment.id));
    // `PaymentReceived` lists the allocation.
    let events = t.received.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].amount, 25_000);
    assert_eq!(events[0].allocations.len(), 1);
    // A second run finds nothing pending.
    let again = online::check_pending(t.state(), since + TimeDelta::minutes(45))
        .await
        .unwrap();
    assert_eq!((again.confirmed, again.failed), (0, 0));
}

/// A payer who closes the page and no webhook: 30 minutes after the payment
/// became `pending` it is `failed` with the reason timeout, whether the
/// gateway gives no final answer (still processing) or its status query errs.
#[renox::test]
async fn a_payment_without_a_final_answer_fails_with_the_reason_timeout() {
    let t = boot().await;
    let link = t.link(BLUE_DUNES, 25_000, None, Some(booking()), None).await;
    // Still processing at the gateway.
    let slow = t.start_online(&link).await;
    let since = slow.created_at.unwrap();
    let report = online::check_pending(t.state(), since + TimeDelta::minutes(29))
        .await
        .unwrap();
    assert_eq!((report.confirmed, report.failed), (0, 0));
    assert_eq!(t.reload(&slow).await.status, PaymentStatus::Pending);
    let report = online::check_pending(t.state(), since + TimeDelta::minutes(30))
        .await
        .unwrap();
    assert_eq!((report.confirmed, report.failed), (0, 1));
    let slow = t.reload(&slow).await;
    assert_eq!(slow.status, PaymentStatus::Failed);
    assert_eq!(slow.failure_kind, Some(PaymentFailureKind::Timeout));
    // The link is still open for another try.
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);

    // The gateway is down: the status query errs.
    let down = t.start_online(&link).await;
    t.gateway
        .fail_status_query(down.gateway_transaction.as_deref().unwrap());
    let since = down.created_at.unwrap();
    online::check_pending(t.state(), since + TimeDelta::minutes(31))
        .await
        .unwrap();
    let down = t.reload(&down).await;
    assert_eq!(down.status, PaymentStatus::Failed);
    assert_eq!(down.failure_kind, Some(PaymentFailureKind::Timeout));

    let kinds: Vec<_> = t.failed.all().iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [PaymentFailureKind::Timeout, PaymentFailureKind::Timeout]);
}

/// The gateway declining the payment when asked fails it as declined, with
/// its reason.
#[renox::test]
async fn a_status_query_that_says_declined_fails_the_payment_as_declined() {
    let t = boot().await;
    let link = t.link(BLUE_DUNES, 25_000, None, None, None).await;
    let payment = t.start_online(&link).await;
    t.gateway.set_status(
        payment.gateway_transaction.as_deref().unwrap(),
        GatewayPaymentStatus::Declined {
            reason: "Insufficient funds".to_owned(),
        },
    );
    online::check_pending(t.state(), payment.created_at.unwrap() + TimeDelta::minutes(30))
        .await
        .unwrap();
    let failed = t.reload(&payment).await;
    assert_eq!(failed.failure_kind, Some(PaymentFailureKind::Declined));
    assert_eq!(failed.failure_reason.as_deref(), Some("Insufficient funds"));
}

/// A webhook that says the payer cancelled the hosted session fails the
/// payment with the reason abandoned; the link stays `open`.
#[renox::test]
async fn a_session_the_payer_cancelled_fails_the_payment_as_abandoned() {
    let t = boot().await;
    let link = t.link(BLUE_DUNES, 25_000, None, None, None).await;
    let payment = t.start_online(&link).await;
    let txn = payment.gateway_transaction.clone().unwrap();
    let failed = online::gateway_failed(t.state(), &txn, PaymentFailureKind::Abandoned, "")
        .await
        .unwrap()
        .expect("failed");
    assert_eq!(failed.status, PaymentStatus::Failed);
    assert_eq!(failed.failure_kind, Some(PaymentFailureKind::Abandoned));
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);
    // The party is told in words, not in codes.
    t.app.logout();
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("Cancelled by the payer");
}

/// Edge: a webhook and the timeout at the same moment: the first wins, the
/// second finds the payment `failed` and does nothing.
#[renox::test]
async fn the_second_of_two_answers_does_nothing() {
    let t = boot().await;
    let link = t.link(BLUE_DUNES, 25_000, None, None, None).await;
    let payment = t.start_online(&link).await;
    let txn = payment.gateway_transaction.clone().unwrap();
    online::gateway_failed(t.state(), &txn, PaymentFailureKind::Declined, "No")
        .await
        .unwrap()
        .expect("the first wins");
    let second = online::gateway_failed(t.state(), &txn, PaymentFailureKind::Abandoned, "")
        .await
        .unwrap();
    assert!(second.is_none());
    online::check_pending(t.state(), payment.created_at.unwrap() + TimeDelta::hours(1))
        .await
        .unwrap();
    assert_eq!(t.failed.all().len(), 1);
    let stored = t.reload(&payment).await;
    assert_eq!(stored.failure_kind, Some(PaymentFailureKind::Declined));
    // An unknown transaction is ignored too.
    assert!(
        online::gateway_failed(t.state(), "nobody", PaymentFailureKind::Declined, "")
            .await
            .unwrap()
            .is_none()
    );
}

/// A replayed success webhook does nothing; an amount that is not the
/// payment's is not confirmed and gives the accountant a task.
#[renox::test]
async fn a_confirmation_is_idempotent_and_its_amount_must_match() {
    let t = boot().await;
    let accountant = t.accountant("Rita").await;
    let link = t.link(BLUE_DUNES, 25_000, None, None, None).await;
    let payment = t.start_online(&link).await;
    let txn = payment.gateway_transaction.clone().unwrap();

    let wrong = online::gateway_succeeded(t.state(), &txn, 24_000).await;
    assert!(wrong.is_err(), "the amount does not match");
    assert_eq!(t.reload(&payment).await.status, PaymentStatus::Pending);
    let tasks = t.tasks(&payment).await;
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].text.contains("AED 240.00"), "{}", tasks[0].text);
    assert!(
        t.notices
            .recipients(wl_payments::notices::ONLINE_TASK)
            .iter()
            .any(|r| matches!(r, NotifyRecipient::User { id } if *id == accountant.id))
    );

    let ok = online::gateway_succeeded(t.state(), &txn, 25_000)
        .await
        .unwrap()
        .expect("confirmed");
    assert_eq!(ok.status, PaymentStatus::Received);
    // The webhook again.
    let again = online::gateway_succeeded(t.state(), &txn, 25_000)
        .await
        .unwrap();
    assert!(again.is_none());
    assert_eq!(t.received.all().len(), 1);
}

/// At the expiry of a link (18:00 in Dubai) it is `expired` and
/// `PaymentLinkExpired` carries the booking; before it nothing happens.
#[renox::test]
async fn a_link_expires_at_its_expiry_and_says_which_booking_was_waiting() {
    let t = boot().await;
    let entity = t.entity("PGP").await;
    let new = NewLink::new(BLUE_DUNES, entity.id, 25_000, "AED")
        .prepaid(booking())
        .expires_at(expiry());
    let link = payment_link::create(t.state(), new).await.unwrap();
    assert_eq!(link.status, LinkStatus::Open);

    let early = payment_link::expire_due(t.state(), expiry() - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(early, 0);
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);
    assert!(t.link_expired.all().is_empty());

    let expired = payment_link::expire_due(t.state(), expiry()).await.unwrap();
    assert_eq!(expired, 1);
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Expired);
    let events = t.link_expired.all();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].payment_link,
        Subject::new(PAYMENT_LINK_SUBJECT_TYPE, link.id)
    );
    assert_eq!(events[0].party, BLUE_DUNES);
    assert_eq!(events[0].amount, 25_000);
    assert_eq!(events[0].prepaid, Some(booking()));
    // Once only.
    assert_eq!(
        payment_link::expire_due(t.state(), expiry() + TimeDelta::hours(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(t.link_expired.all().len(), 1);
}

/// The scheduled task `payments:expire_links` expires a link whose expiry
/// has passed; the party's page then says so and offers no payment.
#[renox::test]
async fn the_expiry_task_expires_a_link_and_the_party_cannot_pay_it() {
    let t = boot().await;
    let link = t.link(BLUE_DUNES, 25_000, None, Some(booking()), None).await;
    let past = renox::db::now() - TimeDelta::minutes(5);
    PaymentLink::query()
        .where_(PaymentLink::ID.eq(link.id))
        .update(t.db(), &[("expires_at", &past)])
        .await
        .unwrap();

    t.app
        .kernel()
        .call(payment_link::TASK, Vec::<String>::new())
        .await
        .expect("the task runs");
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Expired);
    assert_eq!(t.link_expired.all().len(), 1);

    t.app.logout();
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("This payment link has expired")
        .assert_dont_see("Pay now");
    // The button's route refuses too, and starts nothing.
    t.press_pay(&link).await.assert_ok().assert_see("expired");
    assert!(t.gateway.checkouts().is_empty());
}

/// The public page: a token that opens nothing shows a neutral page; a
/// waiting payment and a paid link say so.
#[renox::test]
async fn the_public_page_answers_for_every_state_of_the_link() {
    let t = boot().await;
    t.app.logout();
    t.app
        .get("/pay/not-a-token")
        .await
        .assert_ok()
        .assert_see("This link does not open");

    let link = t.link(BLUE_DUNES, 25_000, None, None, None).await;
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("AED 250.00")
        .assert_see("Pay now");
    let payment = t.start_online(&link).await;
    // A second press while one is waiting starts nothing new.
    t.press_pay(&link).await.assert_ok().assert_see("already in progress");
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("being processed");
    assert_eq!(t.gateway.checkouts().len(), 1);

    online::gateway_succeeded(t.state(), payment.gateway_transaction.as_deref().unwrap(), 25_000)
        .await
        .unwrap();
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("this payment has been received");
}

/// Staff with `payments.links` cancel an open link; a cancelled link cannot be
/// paid, and a user without the permission is refused.
#[renox::test]
async fn a_cancelled_link_cannot_be_paid() {
    let t = boot().await;
    let staff = t.user("Linda", &[wl_payments::permissions::LINKS]).await;
    let nobody = t.user("Omar", &[wl_payments::permissions::VIEW]).await;
    let link = t.link(BLUE_DUNES, 25_000, None, None, None).await;
    let path = format!("/payments/links/{}/cancel", link.id);

    t.app.acting_as(&nobody);
    t.app.htmx().post(&path, &[]).await.assert_forbidden();
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Open);

    t.app.acting_as(&staff);
    t.app.htmx().post(&path, &[]).await.assert_status(303);
    assert_eq!(t.link_now(&link).await.status, LinkStatus::Cancelled);

    t.app.logout();
    t.app
        .get(&t.link_path(&link))
        .await
        .assert_ok()
        .assert_see("withdrawn");
    t.press_pay(&link).await.assert_ok().assert_see("can no longer be paid");
    assert!(t.gateway.checkouts().is_empty());
}
