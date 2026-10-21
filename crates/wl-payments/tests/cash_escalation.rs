//! BL-PAY-21 (#114), `FIN.RUL.22`, X-07, 03-N9-a: a holder who does not hand
//! over the cash at the end of the run, shift or day must not leave it
//! unaccounted for: 12 hours after the end of the period the manager gets a
//! task to reach the holder; 48 hours after the end the company admin is told
//! and the manager counts what was received, so the full difference is a
//! shortage. A vertical rule that replaces the rule (03 drivers) is not
//! escalated twice. Times are Asia/Dubai (UTC+4) in the comments.

mod support;

use renox::auth::permissions;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::custody::{CashCollection, HandoverPeriod, HandoverStatus, custody};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::tickets::{Assignee, TicketId};
use wl_payments::cash_escalation::{self, Escalated};
use wl_payments::cash_notices::CASH_TASK;
use wl_payments::gateway::FakeGateway;
use wl_payments::{CashHandover, WlPayments};

use support::cash::{at, date};
use support::{Booted, boot, boot_with};

/// The holder's manager: the holder of the default manager role, who counts.
async fn manager(t: &Booted, name: &str) -> User {
    let user = User::register(
        t.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "password123",
    )
    .await
    .unwrap();
    permissions::define_role(
        t.db(),
        "finance_manager",
        &[
            wl_payments::permissions::CASH_VIEW,
            wl_payments::permissions::CASH_COUNT,
        ],
    )
    .await
    .unwrap();
    user.assign_role(t.db(), "finance_manager").await.unwrap();
    user
}

/// The company admin.
async fn admin(t: &Booted, name: &str) -> User {
    let user = User::register(
        t.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "password123",
    )
    .await
    .unwrap();
    permissions::define_role(t.db(), wl_kernel::COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    user.assign_role(t.db(), wl_kernel::COMPANY_ADMIN_ROLE)
        .await
        .unwrap();
    user
}

fn told(recipients: &[NotifyRecipient], user: &User) -> bool {
    recipients
        .iter()
        .any(|r| matches!(r, NotifyRecipient::User { id } if *id == user.id))
}

/// A run handover of `holder` for `run_n`, planned to end at `planned_end`.
async fn run_handover(
    t: &Booted,
    holder: &User,
    run_n: i64,
    planned_end: &str,
    replaced: bool,
) -> CashHandover {
    let mut period = HandoverPeriod::run(
        Subject::new("delivery.run", run_n),
        date("2027-05-01"),
        Some(at(planned_end)),
    );
    if replaced {
        period = period.replaced_escalation();
    }
    let id = custody(t.state())
        .collect(
            t.state(),
            CashCollection::new(
                holder.id,
                period,
                90_000,
                Subject::new("delivery.delivery", run_n * 10),
            ),
        )
        .await
        .unwrap()
        .unwrap();
    t.handover(id).await
}

/// AC1: a driver's handover `open` with AED 900.00 expected and the day ending
/// 2027-05-01 23:59: the manager gets a task at 2027-05-02 11:59 (a ticket
/// assigned to them and a notice), once.
#[renox::test]
async fn the_manager_gets_a_task_12_hours_after_the_end_of_the_day() {
    let t = boot().await;
    let mona = manager(&t, "Mona").await;
    let driver = t.holder("Dani").await;
    let handover = t.collect(&driver, "2027-05-01", 90_000, 1).await;

    // 2027-05-02 11:58 in Dubai.
    let early = cash_escalation::run(t.state(), at("2027-05-02T07:58:00Z"))
        .await
        .unwrap();
    assert_eq!(early, Escalated::default());
    assert!(t.cash_tickets.opened().is_empty());

    // 2027-05-02 11:59 in Dubai.
    let done = cash_escalation::run(t.state(), at("2027-05-02T07:59:00Z"))
        .await
        .unwrap();
    assert_eq!(
        done,
        Escalated {
            tasks: 1,
            admin_notices: 0
        }
    );
    let tickets = t.cash_tickets.opened();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject, handover.subject());
    assert_eq!(tickets[0].assignee, Some(Assignee::user(mona.id)));
    assert_eq!(
        tickets[0].source_key.as_deref(),
        Some(format!("payments.cash_reach:{}", handover.id).as_str())
    );
    assert!(tickets[0].title.contains("Dani"));
    let notices = t.notices.of_type(CASH_TASK);
    assert_eq!(notices.len(), 1);
    assert!(told(&notices[0].recipients, &mona));
    assert!(
        notices[0].data["text"]
            .as_str()
            .unwrap()
            .contains("AED 900.00")
    );
    assert!(t.reloaded(&handover).await.task_at.is_some());

    // The same hour again, and the hours after, do not repeat the task.
    let again = cash_escalation::run(t.state(), at("2027-05-02T09:00:00Z"))
        .await
        .unwrap();
    assert_eq!(again, Escalated::default());
    assert_eq!(t.cash_tickets.opened().len(), 1);
}

/// AC2: at 2027-05-03 23:59 the admin is notified and the manager is asked to
/// count; counting AED 0 gives `short` AED 900.00, recorded as holder absent,
/// and the manager's task is closed.
#[renox::test]
async fn after_48_hours_the_admin_is_told_and_the_manager_counts_what_was_received() {
    let t = boot().await;
    let mona = manager(&t, "Mona").await;
    let boss = admin(&t, "Amira").await;
    let driver = t.holder("Dani").await;
    let handover = t.collect(&driver, "2027-05-01", 90_000, 1).await;
    cash_escalation::run(t.state(), at("2027-05-02T07:59:00Z"))
        .await
        .unwrap();

    // 2027-05-03 23:58 in Dubai: not yet.
    let early = cash_escalation::run(t.state(), at("2027-05-03T19:58:00Z"))
        .await
        .unwrap();
    assert_eq!(early, Escalated::default());
    assert!(!told(&t.notices.recipients(CASH_TASK), &boss));

    // 2027-05-03 23:59 in Dubai.
    let done = cash_escalation::run(t.state(), at("2027-05-03T19:59:00Z"))
        .await
        .unwrap();
    assert_eq!(
        done,
        Escalated {
            tasks: 0,
            admin_notices: 1
        }
    );
    let recipients = t.notices.recipients(CASH_TASK);
    assert!(told(&recipients, &boss), "the admin is told");
    let asks = t.notices.of_type(CASH_TASK);
    let last = asks.last().unwrap();
    assert!(
        told(&last.recipients, &mona),
        "the manager is asked to count"
    );
    assert!(
        last.data["text"]
            .as_str()
            .unwrap()
            .contains("holder was absent")
    );
    assert!(t.reloaded(&handover).await.admin_notified_at.is_some());

    t.cash_post(
        &mona,
        &format!("/cash/{}/count", handover.id),
        &[("counted", "0"), ("absent", "on")],
    )
    .await
    .assert_ok();

    let after = t.reloaded(&handover).await;
    assert_eq!(after.status, HandoverStatus::Short);
    assert_eq!((after.counted, after.difference), (0, 90_000));
    assert!(after.holder_absent);
    assert_eq!(after.counted_by, Some(mona.id));
    assert_eq!(t.cash_shortage.all()[0].difference, 90_000);
    assert_eq!(t.cash_tickets.completed(), [TicketId(1)]);
    // Counted: nothing more is escalated.
    let later = cash_escalation::run(t.state(), at("2027-05-09T00:00:00Z"))
        .await
        .unwrap();
    assert_eq!(later, Escalated::default());
}

/// A rule run late does both steps at once; M counts from the end of the
/// period, not from the task.
#[renox::test]
async fn a_late_run_gives_the_task_and_tells_the_admin_together() {
    let t = boot().await;
    let mona = manager(&t, "Mona").await;
    let boss = admin(&t, "Amira").await;
    let driver = t.holder("Dani").await;
    t.collect(&driver, "2027-05-01", 90_000, 1).await;

    let done = cash_escalation::run(t.state(), at("2027-05-04T08:00:00Z"))
        .await
        .unwrap();

    assert_eq!(
        done,
        Escalated {
            tasks: 1,
            admin_notices: 1
        }
    );
    let recipients = t.notices.recipients(CASH_TASK);
    assert!(told(&recipients, &mona));
    assert!(told(&recipients, &boss));
}

/// AC3: a handover counted before 12 hours triggers nothing.
#[renox::test]
async fn a_handover_counted_in_time_triggers_nothing() {
    let t = boot().await;
    manager(&t, "Mona").await;
    admin(&t, "Amira").await;
    let driver = t.holder("Dani").await;
    let chandra = t.counter("Chandra").await;
    let handover = t.collect(&driver, "2027-05-01", 90_000, 1).await;

    t.count_cash(&chandra, &handover, "900.00")
        .await
        .assert_ok();
    let done = cash_escalation::run(t.state(), at("2027-05-09T00:00:00Z"))
        .await
        .unwrap();

    assert_eq!(done, Escalated::default());
    assert!(t.cash_tickets.opened().is_empty());
    assert!(t.notices.of_type(CASH_TASK).is_empty());
}

/// AC4: a run planned 08:00 with an estimated duration of 10 h (planned end
/// 18:00) that never returns to the depot: the task is created at 06:00 the
/// next day.
#[renox::test]
async fn a_run_that_never_returns_starts_the_clock_at_its_planned_end() {
    let t = boot().await;
    manager(&t, "Mona").await;
    let driver = t.holder("Dani").await;
    // Planned end 2027-05-01 18:00 in Dubai.
    run_handover(&t, &driver, 5, "2027-05-01T14:00:00Z", false).await;

    // 2027-05-02 05:59 in Dubai.
    let early = cash_escalation::run(t.state(), at("2027-05-02T01:59:00Z"))
        .await
        .unwrap();
    assert_eq!(early, Escalated::default());
    // 2027-05-02 06:00 in Dubai.
    let done = cash_escalation::run(t.state(), at("2027-05-02T02:00:00Z"))
        .await
        .unwrap();
    assert_eq!(done.tasks, 1);
}

/// AC5: the same run returning to the depot at 15:00 with its handover left
/// `open`: the task is created at 03:00 the next day (the earlier of the
/// return and the planned end).
#[renox::test]
async fn a_run_that_returns_earlier_starts_the_clock_at_its_return() {
    let t = boot().await;
    manager(&t, "Mona").await;
    let driver = t.holder("Dani").await;
    run_handover(&t, &driver, 5, "2027-05-01T14:00:00Z", false).await;
    // Back at the depot at 15:00 in Dubai.
    custody(t.state())
        .end_period(
            t.state(),
            driver.id,
            Subject::new("delivery.run", 5),
            at("2027-05-01T11:00:00Z"),
        )
        .await
        .unwrap();

    // 2027-05-02 02:59 in Dubai.
    let early = cash_escalation::run(t.state(), at("2027-04-30T00:00:00Z"))
        .await
        .unwrap();
    assert_eq!(early, Escalated::default());
    let still = cash_escalation::run(t.state(), at("2027-05-01T22:59:00Z"))
        .await
        .unwrap();
    assert_eq!(still, Escalated::default());
    // 2027-05-02 03:00 in Dubai.
    let done = cash_escalation::run(t.state(), at("2027-05-01T23:00:00Z"))
        .await
        .unwrap();
    assert_eq!(done.tasks, 1);
}

/// AC6: for a 03 client a driver's handover is escalated by `LOG.RUL.13`
/// only: `FIN.RUL.22` creates no task for it, while another holder's handover
/// (counter staff, a day) still uses `FIN.RUL.22`.
#[renox::test]
async fn a_driver_handover_whose_vertical_rule_replaces_the_rule_is_not_escalated_twice() {
    let t = boot().await;
    manager(&t, "Mona").await;
    let driver = t.holder("Dani").await;
    let clerk = t.holder("Cleo").await;
    run_handover(&t, &driver, 5, "2027-05-01T14:00:00Z", true).await;
    let counter = t.collect(&clerk, "2027-05-01", 20_000, 7).await;

    let done = cash_escalation::run(t.state(), at("2027-05-09T00:00:00Z"))
        .await
        .unwrap();

    assert_eq!(
        done,
        Escalated {
            tasks: 1,
            admin_notices: 1
        }
    );
    let tickets = t.cash_tickets.opened();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].subject, counter.subject());
}

/// The hours are settings (§7): a client that sets 2 and 4 hours escalates
/// after those.
#[renox::test]
async fn the_hours_are_the_clients_settings() {
    let t = boot_with(WlPayments::new().cash_escalation(2, 4), FakeGateway::new()).await;
    manager(&t, "Mona").await;
    let driver = t.holder("Dani").await;
    t.collect(&driver, "2027-05-01", 90_000, 1).await;

    // 2027-05-02 01:58 in Dubai: 2 hours after 23:59 is 01:59.
    let early = cash_escalation::run(t.state(), at("2027-05-01T21:58:00Z"))
        .await
        .unwrap();
    assert_eq!(early, Escalated::default());
    let task = cash_escalation::run(t.state(), at("2027-05-01T21:59:00Z"))
        .await
        .unwrap();
    assert_eq!(
        task,
        Escalated {
            tasks: 1,
            admin_notices: 0
        }
    );
    let admin_notice = cash_escalation::run(t.state(), at("2027-05-01T23:59:00Z"))
        .await
        .unwrap();
    assert_eq!(
        admin_notice,
        Escalated {
            tasks: 0,
            admin_notices: 1
        }
    );
}
