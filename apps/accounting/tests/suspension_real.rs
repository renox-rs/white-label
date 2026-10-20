//! #327 (BL-ACC-03) with the real wl-ticketing and wl-invoicing: a
//! suspension holds the engagement's deadline tasks and pauses its retainer
//! subscription, and a held task due 2026-11-28 still becomes `overdue` on
//! 2026-11-29 (`mark_overdue` runs on held tasks, under `ACC.RUL.04`).

mod support;

use accounting::calendar::VAT_RETURN;
use accounting::{EngagementStatus, retainer};
use wl_kernel::deadlines::{DeadlineTaskStatus, NewDeadlineTask, deadline_tasks};
use wl_kernel::subscriptions::{PauseCause, SubscriptionStatus, subscriptions};

use support::{boot_real, d};

/// DW1 and DW5 with the real modules.
#[renox::test]
async fn a_held_task_still_goes_overdue_and_the_real_retainer_is_paused() {
    let t = boot_real().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let subject = engagement.subject();
    let tasks = deadline_tasks(t.state());
    let mut ids = Vec::new();
    for (title, due) in [
        ("VAT return, period ending 2026-10-31", d(11, 28)),
        ("VAT return, period ending 2026-11-30", d(12, 28)),
        ("VAT return, period ending 2026-12-31", d(12, 30)),
    ] {
        let id = tasks
            .create(
                t.state(),
                NewDeadlineTask::new(subject.clone(), VAT_RETURN, title, due).party(client.company),
            )
            .await
            .unwrap()
            .expect("ticketing is on");
        ids.push(id);
    }
    let started = t
        .run(retainer::start(t.state(), &engagement, d(11, 1)))
        .await
        .unwrap();
    assert!(started.is_some(), "the retainer subscription started");

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);

    let held = tasks.of_subject(t.db(), &subject).await.unwrap();
    assert_eq!(held.len(), 3);
    for task in &held {
        assert!(task.held, "{} is held", task.title);
        assert_eq!(task.status, DeadlineTaskStatus::Upcoming, "state kept");
    }
    assert_eq!(held[0].due_date, d(11, 28), "due date kept");

    let retainer = subscriptions(t.state())
        .of_agreement(t.db(), &subject)
        .await
        .unwrap();
    assert_eq!(retainer.len(), 1);
    assert_eq!(retainer[0].status, SubscriptionStatus::Paused);
    assert!(retainer[0].pause_causes.contains(&PauseCause::Agreement));

    // DW5: 2026-11-29, the overdue run still marks the held task.
    t.run(wl_ticketing::deadlines::mark_overdue(t.state(), d(11, 29)))
        .await
        .unwrap();
    let task = tasks.find(t.db(), ids[0]).await.unwrap().unwrap();
    assert_eq!(task.status, DeadlineTaskStatus::Overdue);
    assert!(task.held, "still held");
    assert_eq!(task.overdue_on, Some(d(11, 29)));
    let later = tasks.find(t.db(), ids[1]).await.unwrap().unwrap();
    assert_eq!(later.status, DeadlineTaskStatus::Upcoming);
}
