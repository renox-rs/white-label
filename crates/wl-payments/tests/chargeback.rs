//! Recording a chargeback (wl-payments §4 `charge_back`, §5.2a steps 1 and 2,
//! `FIN.RUL.20`; issue #97): the gateway's `DisputeOpened` and the
//! accountant's hand-made record for N-Genius, the removed allocations, the
//! dispute fee, the evidence task and its reminder.

mod support;

use renox::chrono::{NaiveDate, TimeDelta};
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payments::{Direction, PaymentStatus, ReversalReason};
use wl_payments::chargeback::{self, BANK_CHARGES, DisputeOpened};
use wl_payments::followup::FollowupKind;
use wl_payments::gateway::FakeGateway;
use wl_payments::{Payment, WlPayments, permissions as perm};

use support::{BLUE_DUNES, Booted, QUIET_SANDS, boot_with};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// An app whose gateway fees are paid to Quiet Sands (the gateway provider).
async fn boot() -> Booted {
    let gateway = FakeGateway::new();
    boot_with(
        WlPayments::new()
            .gateway(gateway.clone())
            .gateway_party(QUIET_SANDS),
        gateway,
    )
    .await
}

/// An online payment of `amount` fils, taken through a link sent by `by`,
/// confirmed and allocated to a new invoice of the same amount.
async fn paid_online(t: &Booted, amount: i64, by: &User) -> Payment {
    let invoice = t.invoices.add(BLUE_DUNES, "INV-9", amount);
    let link = t
        .link(BLUE_DUNES, amount, Some(invoice), None, Some(by))
        .await;
    let payment = t.start_online(&link).await;
    wl_payments::online::gateway_succeeded(
        t.state(),
        payment.gateway_transaction.as_deref().unwrap(),
        amount,
    )
    .await
    .unwrap()
    .expect("confirmed")
}

fn told(t: &Booted, user: i64) -> bool {
    t.notices
        .recipients(wl_payments::notices::ONLINE_TASK)
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user))
}

/// A received AED 800.00 gateway payment allocated to INV-9, a dispute
/// webhook with the evidence deadline 2027-02-10 and the fee AED 60.00: the
/// payment is `charged_back`, the allocation is removed (`PaymentReversed`
/// reason `chargeback`, so INV-9 reopens), the fee of AED 60.00 is an
/// outgoing payment to the gateway provider, the accountant has a task due
/// 2027-02-10 with the evidence links, and the staff member who took the
/// payment is told. A replayed webhook does nothing.
#[renox::test]
async fn a_dispute_webhook_charges_the_payment_back_and_opens_the_evidence_task() {
    let t = boot().await;
    let staff = t.user_a().await;
    let accountant = t.accountant("Rita").await;
    let payment = paid_online(&t, 80_000, &staff).await;
    assert_eq!(t.invoices.calls().len(), 1);
    let txn = payment.gateway_transaction.clone().unwrap();

    let dispute = DisputeOpened::new("dp_1")
        .evidence_due(day(2027, 2, 10))
        .fee(6_000);
    let charged = chargeback::gateway_dispute_opened(t.state(), &txn, &dispute)
        .await
        .unwrap()
        .expect("charged back");
    assert_eq!(charged.status, PaymentStatus::ChargedBack);
    assert_eq!(charged.dispute_reference.as_deref(), Some("dp_1"));
    assert_eq!(charged.dispute_status.as_deref(), Some("open"));
    assert_eq!(charged.dispute_evidence_due, Some(day(2027, 2, 10)));
    assert_eq!(charged.dispute_fee, Some(6_000));

    // Every allocation is removed, and the row stays.
    let allocations = t.allocations(&charged).await;
    assert_eq!(allocations.len(), 1);
    assert!(!allocations[0].is_active());
    assert_eq!(allocations[0].removed_reason.as_deref(), Some("chargeback"));
    // `PaymentReversed` names the chargeback and the allocation to unallocate.
    let reversed = t.reversed.all();
    assert_eq!(reversed.len(), 1);
    assert_eq!(reversed[0].reason, ReversalReason::Chargeback);
    assert_eq!(reversed[0].removed.len(), 1);
    assert_eq!(reversed[0].removed[0].amount, 80_000);
    assert_eq!(
        reversed[0].removed[0].target.subject_type,
        "invoicing.invoice"
    );

    // The fee: an outgoing payment to the gateway provider, bank charges,
    // allocated to nothing.
    let fee = Payment::query()
        .where_(Payment::PURPOSE.eq(BANK_CHARGES))
        .first(t.db())
        .await
        .unwrap()
        .expect("the fee is a payment");
    assert_eq!(fee.direction, Direction::Out);
    assert_eq!(fee.amount, 6_000);
    assert_eq!(fee.party_id, QUIET_SANDS.0);
    assert_eq!(fee.status, PaymentStatus::Received);
    assert!(fee.number.as_deref().is_some_and(|n| n.starts_with("PV-")));
    assert!(t.allocations(&fee).await.is_empty());
    assert_eq!(
        t.payment(charged.id).await.dispute_fee_payment_id,
        Some(fee.id)
    );

    // The accountant's evidence task, with the deadline and the links.
    let tasks = t.tasks(&charged).await;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.kind, FollowupKind::ChargebackEvidence);
    assert_eq!(task.due_on, Some(day(2027, 2, 10)));
    assert_eq!(task.role, "accountant");
    assert!(task.text.contains("10 Feb 2027"), "{}", task.text);
    assert!(
        !task.text.contains("Enter the dispute fee"),
        "{}",
        task.text
    );
    let links = task.evidence.0["links"].to_string();
    assert!(links.contains("Invoice INV-9"), "{links}");
    assert!(
        links.contains(&format!("/payments/{}", charged.id)),
        "{links}"
    );
    // The accountant and the staff member who took the payment are told.
    assert!(told(&t, accountant.id));
    assert!(told(&t, staff.id));
    // Nothing is undone and nothing goes to the gateway.
    assert!(t.failed.all().is_empty());
    assert!(t.gateway.accepted().is_empty());

    // A replayed webhook does nothing.
    let again = chargeback::gateway_dispute_opened(t.state(), &txn, &dispute)
        .await
        .unwrap();
    assert!(again.is_none());
    assert_eq!(t.reversed.all().len(), 1);
    assert_eq!(t.tasks(&charged).await.len(), 1);
    let fees = Payment::query()
        .where_(Payment::PURPOSE.eq(BANK_CHARGES))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(fees, 1);
}

/// With the deadline 2027-02-10 and K = 2 the reminder is sent on 2027-02-08
/// at 09:00 Asia/Dubai, once, and not when the evidence was already submitted.
#[renox::test]
async fn the_reminder_comes_two_days_before_the_deadline_once_and_not_after_the_evidence() {
    let t = boot().await;
    let staff = t.user_a().await;
    let accountant = t.accountant("Rita").await;
    let chargeback_user = t.user("Cara", &[perm::VIEW, perm::CHARGEBACK]).await;
    let waiting = paid_online(&t, 80_000, &staff).await;
    let submitted = paid_online(&t, 50_000, &staff).await;
    for (payment, reference) in [(&waiting, "dp_1"), (&submitted, "dp_2")] {
        chargeback::gateway_dispute_opened(
            t.state(),
            payment.gateway_transaction.as_deref().unwrap(),
            &DisputeOpened::new(reference).evidence_due(day(2027, 2, 10)),
        )
        .await
        .unwrap()
        .expect("charged back");
    }
    // The second one's evidence was submitted in the gateway.
    t.app.acting_as(&chargeback_user);
    t.app
        .htmx()
        .post(&format!("/payments/{}/evidence", submitted.id), &[])
        .await
        .assert_ok();
    assert_eq!(
        t.payment(submitted.id).await.dispute_evidence_submitted_on,
        Some(wl_kernel::documents::today())
    );
    let before = t.notices.of_type(wl_payments::notices::ONLINE_TASK).len();

    let at = chargeback::reminder_at(day(2027, 2, 10), 2).unwrap();
    // 09:00 in Dubai on 8 Feb is 05:00 UTC; a minute earlier is too soon.
    assert_eq!(at.to_rfc3339(), "2027-02-08T05:00:00+00:00");
    let sent = chargeback::run_reminders(t.state(), at - TimeDelta::minutes(1))
        .await
        .unwrap();
    assert_eq!(sent, 0);
    let sent = chargeback::run_reminders(t.state(), at).await.unwrap();
    assert_eq!(
        sent, 1,
        "only the dispute whose evidence is still to submit"
    );
    let reminders: Vec<String> = t
        .notices
        .of_type(wl_payments::notices::ONLINE_TASK)
        .iter()
        .skip(before)
        .map(|n| n.data.to_string())
        .collect();
    assert_eq!(reminders.len(), 1);
    assert!(reminders[0].contains("dp_1"), "{reminders:?}");
    assert!(told(&t, accountant.id));
    // Once.
    let again = chargeback::run_reminders(t.state(), at + TimeDelta::hours(5))
        .await
        .unwrap();
    assert_eq!(again, 0);
}

/// `FIN.RUL.20` through the scheduler: the registered hourly task
/// (`payments:chargeback_reminders`) run at 09:00 Dubai two days before the
/// evidence deadline sends the reminder once; an hour earlier it sends none.
#[renox::test]
async fn the_scheduled_task_sends_the_reminder_two_days_before_the_deadline() {
    let t = boot().await;
    let staff = t.user_a().await;
    t.accountant("Rita").await;
    let payment = paid_online(&t, 80_000, &staff).await;
    let due = wl_kernel::documents::today() + chrono_days(10);
    chargeback::gateway_dispute_opened(
        t.state(),
        payment.gateway_transaction.as_deref().unwrap(),
        &DisputeOpened::new("dp_1").evidence_due(due),
    )
    .await
    .unwrap()
    .expect("charged back");
    let before = t.notices.of_type(wl_payments::notices::ONLINE_TASK).len();
    let at = chargeback::reminder_at(due, 2).unwrap();
    let wait = |at: renox::db::DateTime| {
        (at - renox::db::now())
            .to_std()
            .expect("the reminder time is ahead")
    };

    t.app.travel(wait(at - TimeDelta::hours(1)));
    t.app
        .run_scheduled(chargeback::REMINDER_TASK)
        .await
        .expect("the task runs");
    assert_eq!(
        t.notices.of_type(wl_payments::notices::ONLINE_TASK).len(),
        before
    );

    t.app.travel(std::time::Duration::from_secs(61 * 60));
    t.app
        .run_scheduled(chargeback::REMINDER_TASK)
        .await
        .expect("the task runs");
    let sent = t.notices.of_type(wl_payments::notices::ONLINE_TASK);
    assert_eq!(sent.len(), before + 1);
    assert!(sent[before].data.to_string().contains("dp_1"));

    // The next hour sends nothing more.
    t.app.travel(std::time::Duration::from_secs(60 * 60));
    t.app
        .run_scheduled(chargeback::REMINDER_TASK)
        .await
        .expect("the task runs");
    assert_eq!(
        t.notices.of_type(wl_payments::notices::ONLINE_TASK).len(),
        before + 1
    );
}

fn chrono_days(n: u64) -> renox::chrono::Days {
    renox::chrono::Days::new(n)
}

/// A received AED 500.00 payment, a chargeback recorded by hand with the
/// reference NG-778, the deadline 2027-03-05 and the fee AED 50.00: the same
/// effects as a webhook; recording NG-778 a second time is refused.
#[renox::test]
async fn an_accountant_records_a_chargeback_by_hand() {
    let t = boot().await;
    let staff = t.user_a().await;
    let nina = t.user("Nina", &[perm::VIEW, perm::CHARGEBACK]).await;
    let first = paid_online(&t, 50_000, &staff).await;
    let second = paid_online(&t, 30_000, &staff).await;

    t.app.acting_as(&nina);
    t.app
        .htmx()
        .post(
            &format!("/payments/{}/chargeback", first.id),
            &[
                ("reference", "NG-778"),
                ("deadline", "2027-03-05"),
                ("fee", "50.00"),
            ],
        )
        .await
        .assert_ok();
    let charged = t.payment(first.id).await;
    assert_eq!(charged.status, PaymentStatus::ChargedBack);
    assert_eq!(charged.dispute_reference.as_deref(), Some("NG-778"));
    assert_eq!(charged.dispute_evidence_due, Some(day(2027, 3, 5)));
    assert_eq!(charged.dispute_fee, Some(5_000));
    assert!(t.allocations(&charged).await.iter().all(|a| !a.is_active()));
    assert_eq!(t.reversed.all().len(), 1);
    let tasks = t.tasks(&charged).await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].due_on, Some(day(2027, 3, 5)));
    let fee = Payment::query()
        .where_(Payment::PURPOSE.eq(BANK_CHARGES))
        .first(t.db())
        .await
        .unwrap()
        .expect("the fee is a payment");
    assert_eq!(fee.amount, 5_000);

    // The payment page shows the dispute.
    t.app
        .get(&format!("/payments/{}", first.id))
        .await
        .assert_ok()
        .assert_see("NG-778");

    // The same reference again: on the same payment and on another one.
    t.app
        .htmx()
        .post(
            &format!("/payments/{}/chargeback", first.id),
            &[("reference", "NG-778"), ("deadline", "2027-03-05")],
        )
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &format!("/payments/{}/chargeback", second.id),
            &[("reference", "NG-778"), ("deadline", "2027-03-05")],
        )
        .await
        .assert_status(422);
    assert_eq!(t.payment(second.id).await.status, PaymentStatus::Received);
}

/// Edge: a chargeback needs a reference and a deadline, a payment that went
/// through the gateway and the permission; the fee is optional and then the
/// evidence task asks for it.
#[renox::test]
async fn a_chargeback_is_refused_without_what_it_needs() {
    let t = boot().await;
    let staff = t.user_a().await;
    let nina = t.user("Nina", &[perm::VIEW, perm::CHARGEBACK]).await;
    let nobody = t.user("Omar", &[perm::VIEW]).await;
    let online = paid_online(&t, 50_000, &staff).await;
    let path = format!("/payments/{}/chargeback", online.id);

    t.app.acting_as(&nobody);
    t.app
        .htmx()
        .post(&path, &[("reference", "NG-1"), ("deadline", "2027-03-05")])
        .await
        .assert_forbidden();

    t.app.acting_as(&nina);
    t.app
        .htmx()
        .post(&path, &[("reference", "NG-1")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(&path, &[("deadline", "2027-03-05")])
        .await
        .assert_status(422);
    t.app
        .htmx()
        .post(
            &path,
            &[
                ("reference", "NG-1"),
                ("deadline", "2027-03-05"),
                ("fee", "abc"),
            ],
        )
        .await
        .assert_status(422);
    assert_eq!(t.payment(online.id).await.status, PaymentStatus::Received);

    // A payment that did not go through the gateway is not charged back.
    let cash = t.receive(BLUE_DUNES, 10_000).await;
    t.app
        .htmx()
        .post(
            &format!("/payments/{}/chargeback", cash.id),
            &[("reference", "NG-2"), ("deadline", "2027-03-05")],
        )
        .await
        .assert_status(422);

    // No fee given: a task that asks for it and no outgoing payment.
    t.app
        .htmx()
        .post(&path, &[("reference", "NG-1"), ("deadline", "2027-03-05")])
        .await
        .assert_ok();
    let tasks = t.tasks(&t.payment(online.id).await).await;
    assert_eq!(tasks.len(), 1);
    assert!(
        tasks[0].text.contains("Enter the dispute fee"),
        "{}",
        tasks[0].text
    );
    let fees = Payment::query()
        .where_(Payment::PURPOSE.eq(BANK_CHARGES))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(fees, 0);
}

/// Edge: a webhook that gives no deadline still records the chargeback, with
/// a deadline a week ahead.
#[renox::test]
async fn a_dispute_without_a_deadline_gets_one_a_week_ahead() {
    let t = boot().await;
    let staff = t.user_a().await;
    let payment = paid_online(&t, 80_000, &staff).await;
    let charged = chargeback::gateway_dispute_opened(
        t.state(),
        payment.gateway_transaction.as_deref().unwrap(),
        &DisputeOpened::new("dp_7"),
    )
    .await
    .unwrap()
    .expect("charged back");
    assert_eq!(
        charged.dispute_evidence_due,
        Some(wl_kernel::documents::today() + TimeDelta::days(chargeback::DEFAULT_DEADLINE_DAYS))
    );
}
