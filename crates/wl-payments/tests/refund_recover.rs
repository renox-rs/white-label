//! BL-PAY-07 (#100): a failed refund is retried once after 24 hours
//! (`FIN.RUL.21`); otherwise the accountant pays it by another method (new
//! bank details need a new approval) or cancels it, leaving the amount on the
//! party's credit.

mod support;

use renox::prelude::*;
use renox::auth::User;
use renox::chrono::TimeDelta;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{PaymentMethod, RefundSource, payment_recorder};
use wl_kernel::Subject;
use wl_payments::followup::{FollowupKind, FollowupStatus};
use wl_payments::gateway::{FakeGateway, RefundOutcome};
use wl_payments::notices::{REFUND_CANCELLED, REFUND_METHOD_CHANGED, REFUND_METHOD_TASK};
use wl_payments::refund::{RefundStatus, gateway_refund_outcome};
use wl_payments::refund_attempt::AttemptOutcome;
use wl_payments::refund_recover::{failed_at, retries_of};
use wl_payments::refund_retry;
use wl_payments::{FollowupTask, Payment, Refund, RefundAttempt};

use support::{BLUE_DUNES, Booted, boot_gateway};

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

/// An approved gateway refund of `amount` fils, requested by `a`, approved
/// by `b`.
async fn approved_gateway_refund(t: &Booted, a: &User, b: &User, amount: i64) -> Refund {
    let refund = t
        .request_for_source(a, BLUE_DUNES, &credit_note(), amount, "gateway", "txn_1")
        .await;
    t.refund_action(b, &refund, "approve", &[])
        .await
        .assert_ok();
    t.refund(refund.id).await
}

/// The gateway reference of the latest attempt of `refund`.
async fn latest_ref(t: &Booted, refund: &Refund) -> String {
    RefundAttempt::latest(t.db(), refund.id)
        .await
        .unwrap()
        .expect("an attempt")
        .gateway_ref
        .expect("a gateway reference")
}

/// Sends the approved gateway `refund` and has the gateway decline it.
async fn declined_by_the_gateway(t: &Booted, a: &User, refund: &Refund) -> Refund {
    t.refund_action(a, refund, "send", &[]).await.assert_ok();
    let reference = latest_ref(t, refund).await;
    gateway_refund_outcome(t.state(), &reference, declined("card expired"))
        .await
        .unwrap()
        .expect("the outcome is applied")
}

/// A transfer refund of `amount` fils out of a payment of that size,
/// requested by `a`, approved by `b`, paid, then returned by the bank.
async fn returned_transfer(t: &Booted, a: &User, b: &User, amount: i64) -> (Payment, Refund) {
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
    t.fail_refund(a, &refund, "bank_return", "account closed")
        .await
        .assert_ok();
    let failed = t.refund(refund.id).await;
    assert_eq!(failed.status, RefundStatus::Failed);
    (payment, failed)
}

async fn open_tasks(t: &Booted, refund: &Refund) -> Vec<FollowupTask> {
    FollowupTask::open_of_refund(t.db(), refund.id).await.unwrap()
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

/// AC1: a gateway refund failed at some time is retried 24 hours later; if it
/// fails again, the accountant gets a "change method" task.
#[renox::test]
async fn a_gateway_refund_is_retried_after_24_hours_and_then_the_accountant_gets_a_task() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let accountant = t.accountant("Cara").await;
    let refund = approved_gateway_refund(&t, &a, &b, 50_000).await;

    // 2027-01-10 10:00: the gateway declines the first attempt.
    t.travel_to("2027-01-10", 10, 0);
    t.refund_action(&a, &refund, "send", &[]).await.assert_ok();
    let first_ref = latest_ref(&t, &refund).await;
    let failed = t
        .app
        .at_travelled_time(gateway_refund_outcome(
            t.state(),
            &first_ref,
            declined("card expired"),
        ))
        .await
        .unwrap()
        .expect("applied");
    assert_eq!(failed.status, RefundStatus::Failed);
    let at = failed_at(t.db(), refund.id).await.unwrap().expect("a failure time");
    assert_eq!(at.format("%Y-%m-%d").to_string(), "2027-01-10");

    // A retry is waiting: no task yet.
    assert!(open_tasks(&t, &refund).await.is_empty());
    assert!(t.notices.of_type(REFUND_METHOD_TASK).is_empty());

    let early = refund_retry::run(t.state(), at + TimeDelta::hours(24) - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(early.retried, 0);
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);

    // 24 hours later it is retried: approved, with a new attempt at the gateway.
    let report = refund_retry::run(t.state(), at + TimeDelta::hours(24))
        .await
        .unwrap();
    assert_eq!((report.retried, report.tasks), (1, 0));
    let retried = t.refund(refund.id).await;
    assert_eq!(retried.status, RefundStatus::Approved);
    assert_eq!(retries_of(t.db(), refund.id).await.unwrap(), 1);
    let attempts = RefundAttempt::of_refund(t.db(), refund.id).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].number, 2);
    assert_eq!(attempts[1].outcome, AttemptOutcome::Pending);
    assert_eq!(t.gateway.refunded().len(), 2);

    // It fails again: now the accountant gets the task, once.
    let second_ref = latest_ref(&t, &refund).await;
    gateway_refund_outcome(t.state(), &second_ref, declined("card closed"))
        .await
        .unwrap();
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
    let tasks = open_tasks(&t, &refund).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].kind, FollowupKind::ChangeMethod);
    assert_eq!(tasks[0].role, "accountant");
    assert_eq!(tasks[0].refund_id, Some(refund.id));
    assert!(tasks[0].text.contains(&format!("#{}", refund.id)));
    let told = t.notices.of_type(REFUND_METHOD_TASK);
    assert_eq!(told.len(), 1);
    assert_eq!(users(&told[0].recipients), vec![accountant.id]);
    assert!(told[0].data.to_string().contains(&format!("/refunds/{}", refund.id)));

    // A third attempt is refused: by the rule and by hand.
    let later = refund_retry::run(t.state(), at + TimeDelta::hours(200))
        .await
        .unwrap();
    assert_eq!(later.retried, 0);
    t.refund_action(&b, &refund, "retry", &[])
        .await
        .assert_status(422);
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
    assert_eq!(RefundAttempt::of_refund(t.db(), refund.id).await.unwrap().len(), 2);
    assert_eq!(open_tasks(&t, &refund).await.len(), 1, "no second task");
}

/// A transfer returned by the bank is not retried by the system: the
/// accountant gets the task at once.
#[renox::test]
async fn a_returned_transfer_gives_the_accountant_the_task_at_once() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let (_payment, failed) = returned_transfer(&t, &a, &b, 50_000).await;

    assert_eq!(open_tasks(&t, &failed).await.len(), 1);
    let report = refund_retry::run(t.state(), renox::db::now() + TimeDelta::hours(100))
        .await
        .unwrap();
    assert_eq!(report.retried, 0);
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Failed);
}

/// Manual retries count toward the same maximum.
#[renox::test]
async fn a_manual_retry_counts_toward_the_maximum() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let refund = approved_gateway_refund(&t, &a, &b, 50_000).await;
    let failed = declined_by_the_gateway(&t, &a, &refund).await;
    assert_eq!(failed.status, RefundStatus::Failed);

    t.refund_action(&a, &refund, "retry", &[]).await.assert_ok();
    let retried = t.refund(refund.id).await;
    assert_eq!(retried.status, RefundStatus::Approved);
    assert_eq!(
        RefundAttempt::latest(t.db(), refund.id).await.unwrap().unwrap().outcome,
        AttemptOutcome::Pending
    );

    let reference = latest_ref(&t, &refund).await;
    gateway_refund_outcome(t.state(), &reference, declined("card closed"))
        .await
        .unwrap();
    t.refund_action(&a, &refund, "retry", &[])
        .await
        .assert_status(422);
    // The system does not retry it either.
    let report = refund_retry::run(t.state(), renox::db::now() + TimeDelta::hours(100))
        .await
        .unwrap();
    assert_eq!(report.retried, 0);
    assert_eq!(open_tasks(&t, &refund).await.len(), 1);
}

/// A gateway that refuses the retry leaves the refund failed and gives the
/// accountant the task.
#[renox::test]
async fn a_retry_the_gateway_refuses_leaves_the_refund_failed_with_a_task() {
    let gateway = FakeGateway::new();
    let t = boot_gateway(gateway.clone()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let refund = approved_gateway_refund(&t, &a, &b, 50_000).await;
    declined_by_the_gateway(&t, &a, &refund).await;
    gateway.refuse_refund("txn_1", "card closed");
    let at = failed_at(t.db(), refund.id).await.unwrap().unwrap();

    let report = refund_retry::run(t.state(), at + TimeDelta::hours(24))
        .await
        .unwrap();
    assert_eq!((report.retried, report.tasks), (0, 1));
    assert_eq!(t.refund(refund.id).await.status, RefundStatus::Failed);
    assert_eq!(retries_of(t.db(), refund.id).await.unwrap(), 0, "nothing was retried");
    assert_eq!(RefundAttempt::of_refund(t.db(), refund.id).await.unwrap().len(), 1);
    assert_eq!(open_tasks(&t, &refund).await.len(), 1);
}

/// A retry scheduled while the accountant changes the method first finds the
/// refund no longer failed and does nothing.
#[renox::test]
async fn a_scheduled_retry_after_a_change_of_method_does_nothing() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let refund = approved_gateway_refund(&t, &a, &b, 50_000).await;
    declined_by_the_gateway(&t, &a, &refund).await;
    let at = failed_at(t.db(), refund.id).await.unwrap().unwrap();

    t.refund_action(&a, &refund, "change_method", &[("method", "cash")])
        .await
        .assert_ok();
    let report = refund_retry::run(t.state(), at + TimeDelta::hours(24))
        .await
        .unwrap();
    assert_eq!(report.retried, 0);
    let after = t.refund(refund.id).await;
    assert_eq!(after.status, RefundStatus::Approved);
    assert_eq!(after.method, Some(PaymentMethod::Cash));
    assert_eq!(t.gateway.refunded().len(), 1, "the gateway heard nothing more");
}

/// AC2: `change_method` to a cheque in the party's name goes to `approved`;
/// to a new IBAN it goes to `requested`, and the user who set it cannot
/// approve it.
#[renox::test]
async fn changing_the_method_goes_to_approved_or_back_to_requested() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let c = t.staff("Carl").await;
    t.accountant("Cara").await;

    // A cheque in the party's name: approved at once; the party is told.
    let (_p, failed) = returned_transfer(&t, &a, &b, 50_000).await;
    t.refund_action(&c, &failed, "change_method", &[("method", "cheque")])
        .await
        .assert_ok();
    let cheque = t.refund(failed.id).await;
    assert_eq!(cheque.status, RefundStatus::Approved);
    assert_eq!(cheque.method, Some(PaymentMethod::Cheque));
    assert_eq!(cheque.payee_details, None);
    assert!(open_tasks(&t, &failed).await.is_empty(), "the task closes");
    let told = t.notices.of_type(REFUND_METHOD_CHANGED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, vec![NotifyRecipient::Party { id: BLUE_DUNES }]);
    assert!(told[0].data.to_string().contains("Cheque"));

    // A new IBAN: back to requested, for someone else to approve.
    let (_p, failed) = returned_transfer(&t, &a, &b, 70_000).await;
    t.refund_action(
        &c,
        &failed,
        "change_method",
        &[("method", "transfer"), ("details", "AE07 0331 2345 6789 0123 456")],
    )
    .await
    .assert_ok();
    let requested = t.refund(failed.id).await;
    assert_eq!(requested.status, RefundStatus::Requested);
    assert_eq!(
        requested.payee_details.as_deref(),
        Some("AE07 0331 2345 6789 0123 456")
    );
    assert_eq!(requested.details_by, Some(c.id));
    assert_eq!(requested.approved_by, None);

    // Neither the user who set the details nor the requester approves it.
    t.refund_action(&c, &requested, "approve", &[])
        .await
        .assert_status(422);
    t.refund_action(&a, &requested, "approve", &[])
        .await
        .assert_status(422);
    t.refund_action(&b, &requested, "approve", &[])
        .await
        .assert_ok();
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Approved);
}

/// The method change is checked: a transfer needs bank details, only four
/// methods are accepted, and nothing changes when it is refused.
#[renox::test]
async fn a_method_change_that_is_not_valid_is_refused() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let (_p, failed) = returned_transfer(&t, &a, &b, 50_000).await;

    t.refund_action(&a, &failed, "change_method", &[("method", "transfer")])
        .await
        .assert_status(422);
    t.refund_action(&a, &failed, "change_method", &[("method", "card")])
        .await
        .assert_status(422);
    // A transfer to no details at all.
    t.refund_action(
        &a,
        &failed,
        "change_method",
        &[("method", "transfer"), ("details", "")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Failed);

    // Not from any other state.
    let payment = t.receive(BLUE_DUNES, 10_000).await;
    let requested = t.request_refund(&a, &payment, 10_000).await;
    t.refund_action(&a, &requested, "change_method", &[("method", "cash")])
        .await
        .assert_status(422);
}

/// AC3: `cancel` with a reason leaves the amount on the party's credit and
/// notifies the party.
#[renox::test]
async fn cancelling_leaves_the_amount_on_the_credit_and_tells_the_party() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let (_payment, failed) = returned_transfer(&t, &a, &b, 50_000).await;
    assert_eq!(credit(&t).await, 50_000, "restored by the failure");

    // A reason is required.
    t.refund_action(&b, &failed, "cancel", &[("reason", "  ")])
        .await
        .assert_status(422);
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Failed);

    t.refund_action(
        &b,
        &failed,
        "cancel",
        &[("reason", "party asks to keep the credit")],
    )
    .await
    .assert_ok();
    let cancelled = t.refund(failed.id).await;
    assert_eq!(cancelled.status, RefundStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("party asks to keep the credit")
    );
    assert_eq!(credit(&t).await, 50_000);
    let told = t.notices.of_type(REFUND_CANCELLED);
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].recipients, vec![NotifyRecipient::Party { id: BLUE_DUNES }]);
    assert!(told[0].data.to_string().contains("500.00"));
    assert!(open_tasks(&t, &failed).await.is_empty(), "the task closes");
    let task = FollowupTask::query()
        .where_eq("refund_id", failed.id)
        .first(t.db())
        .await
        .unwrap()
        .expect("the task");
    assert_eq!(task.status, FollowupStatus::Done);

    // A cancelled refund is final.
    t.refund_action(&b, &cancelled, "retry", &[])
        .await
        .assert_status(422);
    t.refund_action(&b, &cancelled, "cancel", &[("reason", "again")])
        .await
        .assert_status(422);
}

/// Cancelling needs `refunds.approve`; retrying and changing the method need
/// `refunds.pay`.
#[renox::test]
async fn the_recovery_actions_need_their_permissions() {
    use wl_payments::permissions as perm;
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let (_p, failed) = returned_transfer(&t, &a, &b, 50_000).await;
    let viewer = t.user("Vera", &[perm::VIEW]).await;
    let payer = t.user("Pavel", &[perm::VIEW, perm::REFUNDS_PAY]).await;

    t.refund_action(&viewer, &failed, "retry", &[])
        .await
        .assert_forbidden();
    t.refund_action(&viewer, &failed, "change_method", &[("method", "cash")])
        .await
        .assert_forbidden();
    t.refund_action(&payer, &failed, "cancel", &[("reason", "x")])
        .await
        .assert_forbidden();
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Failed);
}

/// A source that other refunds claimed meanwhile has no room: the retry and
/// the change of method are refused.
#[renox::test]
async fn a_source_without_room_refuses_the_retry() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let (payment, failed) = returned_transfer(&t, &a, &b, 50_000).await;
    // Another refund claims the whole payment while the first one is failed.
    let other = t.request_refund(&a, &payment, 50_000).await;
    assert_eq!(other.status, RefundStatus::Requested);

    t.refund_action(&b, &failed, "change_method", &[("method", "cash")])
        .await
        .assert_status(422);
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Failed);
}

/// The refund page offers the recovery actions of a failed refund only, and
/// the tasks page links the refund.
#[renox::test]
async fn the_pages_offer_retry_change_method_and_cancel_on_a_failed_refund() {
    let t = boot_gateway(FakeGateway::new()).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    t.accountant("Cara").await;
    let (_p, failed) = returned_transfer(&t, &a, &b, 50_000).await;

    t.app.acting_as(&a);
    t.app
        .get(&format!("/refunds/{}", failed.id))
        .await
        .assert_ok()
        .assert_see(&format!("change-method-refund-{}", failed.id))
        .assert_see(&format!("cancel-refund-{}", failed.id))
        .assert_see(&format!("retry-refund-{}", failed.id));
    t.app
        .get("/payments/tasks")
        .await
        .assert_ok()
        .assert_see("Change the method of refund")
        .assert_see(&format!("/refunds/{}", failed.id));

    // The real routes: change to cash, then the page offers nothing more.
    t.app
        .htmx()
        .post(
            &format!("/refunds/{}/change-method", failed.id),
            &[("method", "cash")],
        )
        .await
        .assert_ok();
    assert_eq!(t.refund(failed.id).await.status, RefundStatus::Approved);
    t.app
        .get(&format!("/refunds/{}", failed.id))
        .await
        .assert_ok()
        .assert_dont_see(&format!("change-method-refund-{}", failed.id))
        .assert_dont_see(&format!("cancel-refund-{}", failed.id));
}
