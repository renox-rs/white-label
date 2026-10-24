//! `CLN.RUL.03` (BL-CLN-11, #359) with the **real** wl-invoicing payer bill:
//! a claim still `draft` or `ready` is chased twice (the billing coordinator
//! N days after the visit, the clinic manager D days before the payer's
//! first-submission window ends), once per claim and step; a submitted claim
//! gets neither; a payer contract with N not below its window is refused; and
//! the rule is compliance-critical: a run that fails or never happens alerts
//! the owner.

mod support;

use std::time::Duration;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::compliance::service::NOTIFICATION_TYPE;
use wl_kernel::compliance::{AlertKind, ComplianceAlert, check_missed, is_critical};
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};
use wl_kernel::tickets::Assignee;

use clinics::claim_window::{self, NOTICE, RULE};
use clinics::claims;
use clinics::contracts::{self, NewContract};
use support::dubai;
use support::real::{PAYER, Real, boot_real};

fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The users a request went to.
fn users(request: &NotifyRequest) -> Vec<i64> {
    request
        .recipients
        .iter()
        .filter_map(|r| match r {
            NotifyRecipient::User { id } => Some(*id),
            _ => None,
        })
        .collect()
}

/// The open alerts of the rule.
async fn open_alerts(t: &Real) -> Vec<ComplianceAlert> {
    ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect()
}

/// Moves the clock to a second after the Asia/Dubai instant `to`.
fn go_to(t: &Real, to: renox::db::DateTime) {
    t.app.travel_back();
    let ahead = (to - renox::db::now()).to_std().expect("ahead");
    t.app.travel(Duration::from_secs(ahead.as_secs() + 1));
}

/// Done when 1: a draft claim for a visit on 2026-11-01 under a 30-day
/// window; on 2026-11-08 the coordinator is told, on 2026-11-29 the clinic
/// manager gets a task due at the end of the last day (2026-12-01). Each step
/// fires once: the same date again, and the days after, do nothing more.
#[renox::test]
async fn the_coordinator_is_told_at_n_days_and_the_clinic_manager_gets_a_task_two_days_before() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let mia = t.clinic_manager().await;
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;

    let before = claim_window::run(t.state(), d("2026-11-07")).await.unwrap();
    assert_eq!((before.notified, before.tasked), (0, 0));
    assert!(t.messages.sent().is_empty());

    let at_n = claim_window::run(t.state(), d("2026-11-08")).await.unwrap();
    assert_eq!((at_n.notified, at_n.tasked), (1, 0));
    let sent: Vec<_> = t
        .messages
        .sent()
        .into_iter()
        .filter(|r| r.type_key == NOTICE)
        .collect();
    assert_eq!(sent.len(), 1);
    assert_eq!(users(&sent[0]), [cora.id]);
    assert_eq!(sent[0].subject, claim.subject());
    let body = support::Messages::body(&sent[0]);
    assert!(body.contains(&claim.number()), "{body}");
    assert!(body.contains("2026-12-01"), "{body}");
    assert!(t.tasks.with_key("clinics.claim_window:").is_empty());

    // The same date again is the same run; the days after skip the step.
    let again = claim_window::run(t.state(), d("2026-11-08")).await.unwrap();
    assert_eq!((again.notified, again.tasked), (0, 0));
    let later = claim_window::run(t.state(), d("2026-11-28")).await.unwrap();
    assert_eq!((later.notified, later.tasked), (0, 0));
    assert!(t.tasks.with_key("clinics.claim_window:").is_empty());

    let at_d = claim_window::run(t.state(), d("2026-11-29")).await.unwrap();
    assert_eq!((at_d.notified, at_d.tasked), (0, 1));
    let tasks = t.tasks.with_key("clinics.claim_window:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].source_key, Some(claim_window::task_key(claim.id)));
    assert_eq!(tasks[0].assignee, Some(Assignee::user(mia.id)));
    assert_eq!(tasks[0].due_at, Some(dubai("2026-12-01", 23, 59)));
    assert!(tasks[0].description.contains("2026-12-01"));

    let last = claim_window::run(t.state(), d("2026-11-30")).await.unwrap();
    assert_eq!((last.notified, last.tasked), (0, 0));
    assert_eq!(t.tasks.with_key("clinics.claim_window:").len(), 1);
    let stored = t.claim_now(claim.id).await;
    assert_eq!(stored.window_notified_on, Some(d("2026-11-08")));
    assert_eq!(stored.window_task_on, Some(d("2026-11-29")));
}

/// A `ready` claim is chased too; a claim past the last day of its window is
/// not (it is closed by `write_off`, `CLN.ACL.16`).
#[renox::test]
async fn a_ready_claim_is_chased_and_a_claim_past_its_window_is_not() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    t.clinic_manager().await;
    let ready = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    claims::validate(t.state(), &cora, ready.id, None, dubai("2026-11-02", 9, 0))
        .await
        .unwrap();
    let old = t.claim(d("2026-09-01"), &[("consultation", 40_000)]).await;

    let report = claim_window::run(t.state(), d("2026-11-29")).await.unwrap();
    assert_eq!((report.notified, report.tasked), (1, 1));
    let tasks = t.tasks.with_key("clinics.claim_window:");
    assert_eq!(tasks[0].source_key, Some(claim_window::task_key(ready.id)));
    assert_eq!(t.claim_now(old.id).await.window_notified_on, None);
}

/// Done when 2: a claim submitted on 2026-11-05 gets neither.
#[renox::test]
async fn a_submitted_claim_gets_neither() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    t.clinic_manager().await;
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    claims::validate(t.state(), &cora, claim.id, None, dubai("2026-11-04", 9, 0))
        .await
        .unwrap();
    claims::submit(
        t.state(),
        &cora,
        claim.id,
        Some("CH-1"),
        dubai("2026-11-05", 9, 0),
    )
    .await
    .unwrap();

    for day in ["2026-11-08", "2026-11-29"] {
        let report = claim_window::run(t.state(), d(day)).await.unwrap();
        assert_eq!((report.notified, report.tasked), (0, 0), "{day}");
    }
    assert!(t.messages.sent().iter().all(|r| r.type_key != NOTICE));
    assert!(t.tasks.with_key("clinics.claim_window:").is_empty());
}

/// Posts a contract with a 30-day window and the reminder `reminder` through
/// the page.
async fn new_contract(
    t: &support::real::Real,
    by: &User,
    reminder: &str,
) -> renox::testing::TestResponse {
    let agreement = t.agreement.id.to_string();
    t.post(
        by,
        "/clinics/contracts",
        &[
            ("payer", "900"),
            ("agreement", agreement.as_str()),
            ("first_submission_days", "30"),
            ("reminder_days", reminder),
        ],
    )
    .await
}

/// Done when 3: saving a payer contract with a 30-day window and N = 30 is
/// refused, through the page and the service; N = 29 is saved; the default N
/// of a 5-day window is below it.
#[renox::test]
async fn a_contract_with_n_not_below_its_window_is_refused() {
    let t = boot_real().await;
    let bea = t
        .user("Bea", &[clinics::permissions::AUTHORISATIONS_MANAGE])
        .await;
    let refused = new_contract(&t, &bea, "30").await;
    refused.assert_status(422);
    assert!(
        refused
            .text()
            .contains("N must be below the submission window"),
        "{}",
        refused.text()
    );
    new_contract(&t, &bea, "29").await.assert_ok();

    let by_service = contracts::create(
        t.state(),
        None,
        NewContract::new(PAYER, d("2026-01-01"))
            .windows(30, 30, 2)
            .reminder_days(30),
    )
    .await;
    assert!(by_service.is_err());
    let short = contracts::create(
        t.state(),
        None,
        NewContract::new(PAYER, d("2026-01-01")).windows(5, 30, 2),
    )
    .await
    .expect("the default N is below a short window");
    assert_eq!(short.reminder_days, 4);
}

/// Done when 4: a payer contract with a 1-day window and N = 0, a visit on
/// 2026-11-01: the 2026-11-02 run tells the coordinator and creates the
/// clinic manager's task.
#[renox::test]
async fn a_one_day_window_with_n_zero_is_chased_on_the_first_run() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let mia = t.clinic_manager().await;
    let contract = contracts::create(
        t.state(),
        None,
        NewContract::new(PAYER, d("2026-01-01"))
            .agreement(t.agreement.id)
            .windows(1, 30, 2)
            .reminder_days(0),
    )
    .await
    .expect("N = 0 is below a 1-day window");
    let claim = t
        .claim_under(&contract, d("2026-11-01"), &[("consultation", 40_000)])
        .await;

    let report = claim_window::run(t.state(), d("2026-11-02")).await.unwrap();
    assert_eq!((report.notified, report.tasked), (1, 1));
    let sent: Vec<_> = t
        .messages
        .sent()
        .into_iter()
        .filter(|r| r.type_key == NOTICE)
        .collect();
    assert_eq!(users(&sent[0]), [cora.id]);
    let tasks = t.tasks.with_key("clinics.claim_window:");
    assert_eq!(tasks[0].assignee, Some(Assignee::user(mia.id)));
    assert_eq!(tasks[0].due_at, Some(dubai("2026-11-02", 23, 59)));
    assert_eq!(
        t.claim_now(claim.id).await.window_task_on,
        Some(d("2026-11-02"))
    );
}

/// Done when 5 (failed): the rule is compliance-critical and set by us; a run
/// that fails (here the notifier is down for the claim) is retried and, after
/// its fifth failure, alerts the owner / medical director at once. Nothing was
/// recorded for the claim, so the notice is sent once it works again.
#[renox::test]
async fn a_failed_run_alerts_the_owner() {
    let t = boot_real().await;
    t.coordinator().await;
    t.clinic_manager().await;
    let owner = t.owner().await;
    assert!(is_critical(RULE));
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    t.messages.fail_for(NOTICE, claim.subject());

    // Four failed attempts: the owner is not alerted yet.
    for wait in [60_u64, 300, 900, 3600] {
        t.app
            .at_travelled_time(claim_window::run(t.state(), d("2026-11-08")))
            .await
            .unwrap();
        assert!(
            open_alerts(&t).await.is_empty(),
            "no alert before the fifth failure"
        );
        assert!(
            !t.messages
                .sent()
                .iter()
                .any(|r| r.type_key == NOTIFICATION_TYPE),
            "and the owner is not told"
        );
        t.app.travel(Duration::from_secs(wait));
    }
    t.app
        .at_travelled_time(claim_window::run(t.state(), d("2026-11-08")))
        .await
        .unwrap();

    let alerts: Vec<_> = ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].kind, AlertKind::Failed);
    assert_eq!(alerts[0].owner_role, clinics::permissions::OWNER_ROLE);
    assert!(
        t.messages
            .sent()
            .iter()
            .any(|r| r.type_key == NOTIFICATION_TYPE && users(r).contains(&owner.id)),
        "the owner is told"
    );
    assert_eq!(t.claim_now(claim.id).await.window_notified_on, None);
}

/// Done when 5 (missed): no run exists for the 02:00 occurrence of a day. At
/// 02:30 (30 minutes late) the owner is not alerted; from 03:00 on (the
/// kernel's missed-run check, 60 minutes after 02:00) the alert for that
/// occurrence is raised and the owner is told.
#[renox::test]
async fn a_missed_run_alerts_the_owner_from_the_03_00_check() {
    let t = boot_real().await;
    let owner = t.owner().await;
    // The rule is watched, daily at 02:00 Asia/Dubai.
    assert_eq!(
        (claim_window::TASK_AT, claim_window::TASK_ZONE),
        ("02:00", "Asia/Dubai")
    );
    let tomorrow = wl_kernel::documents::dubai_date(renox::db::now()) + renox::chrono::Days::new(1);
    let occurrence = wl_kernel::compliance::occurrence_key(RULE, tomorrow);
    let missed = |alerts: &[ComplianceAlert]| {
        alerts
            .iter()
            .filter(|a| a.occurrence_key == occurrence)
            .count()
    };

    // 01:00: the test may run in the small hours, when the rule's 02:00 run of
    // *today* is planned after the rule was watched and is missed by now. It
    // is settled here, so that only what the 02:30 check adds is looked at.
    go_to(&t, dubai(&tomorrow.to_string(), 1, 0));
    t.app
        .at_travelled_time(check_missed(t.state()))
        .await
        .unwrap();
    let told_before = t.messages.sent().len();

    // 02:30: only 30 minutes late.
    go_to(&t, dubai(&tomorrow.to_string(), 2, 30));
    t.app
        .at_travelled_time(check_missed(t.state()))
        .await
        .unwrap();
    assert_eq!(missed(&open_alerts(&t).await), 0, "not yet at 02:30");
    assert!(
        !t.messages
            .sent()
            .iter()
            .skip(told_before)
            .any(|r| r.type_key == NOTIFICATION_TYPE && users(r).contains(&owner.id)),
        "the owner is not told at 02:30"
    );

    // 03:00 and a minute: 60 minutes late.
    go_to(&t, dubai(&tomorrow.to_string(), 3, 0));
    t.app
        .at_travelled_time(check_missed(t.state()))
        .await
        .unwrap();
    let alerts = open_alerts(&t).await;
    assert_eq!(missed(&alerts), 1, "{alerts:?}");
    assert!(alerts.iter().all(|a| a.kind == AlertKind::Missed));
    assert_eq!(alerts[0].owner_role, clinics::permissions::OWNER_ROLE);
    assert!(
        t.messages
            .sent()
            .iter()
            .any(|r| r.type_key == NOTIFICATION_TYPE && users(r).contains(&owner.id)),
        "the owner is told"
    );
}

/// The manager's task (D) is not given up when ticketing is off: the step is
/// open for the next run, and the task is opened once ticketing is on, once.
#[renox::test]
async fn the_clinic_managers_task_is_opened_once_ticketing_is_on() {
    let t = boot_real().await;
    t.coordinator().await;
    let mia = t.clinic_manager().await;
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    t.tasks.set_off(true);

    let off = claim_window::run(t.state(), d("2026-11-29")).await.unwrap();
    assert_eq!(
        (off.notified, off.tasked),
        (1, 0),
        "no task without ticketing"
    );
    assert!(t.tasks.with_key("clinics.claim_window:").is_empty());
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.window_task_on, None, "the step is not marked done");
    assert_eq!(after.window_notified_on, Some(d("2026-11-29")));

    t.tasks.set_off(false);
    let on = claim_window::run(t.state(), d("2026-11-30")).await.unwrap();
    assert_eq!((on.notified, on.tasked), (0, 1));
    let tasks = t.tasks.with_key("clinics.claim_window:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(mia.id)));
    assert_eq!(tasks[0].due_at, Some(dubai("2026-12-01", 23, 59)));
    assert_eq!(
        t.claim_now(claim.id).await.window_task_on,
        Some(d("2026-11-30"))
    );
    let again = claim_window::run(t.state(), d("2026-12-01")).await.unwrap();
    assert_eq!((again.notified, again.tasked), (0, 0));
    assert_eq!(t.tasks.with_key("clinics.claim_window:").len(), 1);
}

/// A failure while telling leaves the step open, and the run that works
/// again tells everybody once, in one message: nobody is told twice.
#[renox::test]
async fn a_notice_that_failed_is_sent_once_to_every_coordinator_when_it_works_again() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    // A second coordinator.
    let carl = t.user("Carl", &[clinics::permissions::VISITS_VIEW]).await;
    User::find_or_404(t.db(), carl.id)
        .await
        .unwrap()
        .assign_role(t.db(), clinics::permissions::BILLING_COORDINATOR_ROLE)
        .await
        .unwrap();
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    t.messages.fail_for(NOTICE, claim.subject());

    claim_window::run(t.state(), d("2026-11-08")).await.unwrap();
    assert_eq!(t.claim_now(claim.id).await.window_notified_on, None);
    assert!(t.messages.sent().iter().all(|r| r.type_key != NOTICE));

    t.messages.recover();
    let ok = claim_window::run(t.state(), d("2026-11-09")).await.unwrap();
    assert_eq!(ok.notified, 1);
    let sent: Vec<_> = t
        .messages
        .sent()
        .into_iter()
        .filter(|r| r.type_key == NOTICE)
        .collect();
    assert_eq!(sent.len(), 1, "one message for all");
    let mut told = users(&sent[0]);
    told.sort_unstable();
    let mut expected = vec![cora.id, carl.id];
    expected.sort_unstable();
    assert_eq!(told, expected);
}
