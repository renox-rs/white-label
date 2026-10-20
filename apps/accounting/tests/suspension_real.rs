//! #327 (BL-ACC-03) with the real wl-ticketing and wl-invoicing: a
//! suspension holds the engagement's deadline tasks and pauses its retainer
//! subscription, and a held task due 2026-11-28 still becomes `overdue` on
//! 2026-11-29 (`mark_overdue` runs on held tasks, under `ACC.RUL.04`).

mod support;

use accounting::calendar::VAT_RETURN;
use accounting::deadline_notices::{DeadlineNotice, prepared_key};
use accounting::{EngagementStatus, retainer};
use renox::prelude::*;
use renox::state_machine::history;
use wl_kernel::deadlines::{DeadlineTaskStatus, NewDeadlineTask, deadline_tasks};
use wl_kernel::subscriptions::{PauseCause, SubscriptionStatus, subscriptions};
use wl_ticketing::Ticket;

use support::{Booted, boot_real, d};

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

/// An `active` engagement of a fresh client with two real tasks, the real
/// retainer started and a portal user for the client.
async fn served(t: &Booted) -> (support::Client, accounting::Engagement) {
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let tasks = deadline_tasks(t.state());
    for (title, due) in [
        ("VAT return, period ending 2026-10-31", d(11, 28)),
        ("Corporate tax registration", d(12, 31)),
    ] {
        tasks
            .create(
                t.state(),
                NewDeadlineTask::new(engagement.subject(), VAT_RETURN, title, due)
                    .party(client.company),
            )
            .await
            .unwrap()
            .expect("ticketing is on");
    }
    t.run(retainer::start(t.state(), &engagement, d(11, 1)))
        .await
        .unwrap()
        .expect("the retainer started");
    t.portal_user("Omar", client.ubo, client.company).await;
    (client, engagement)
}

/// The open pause rows of the engagement's real subscription as
/// (cause, reason).
async fn pauses(t: &Booted, engagement: &accounting::Engagement) -> Vec<(PauseCause, String)> {
    let found = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    wl_invoicing::SubscriptionPause::open_of(t.db(), found[0].id.0)
        .await
        .unwrap()
        .into_iter()
        .map(|pause| (pause.cause, pause.reason))
        .collect()
}

/// DW3 with the real modules: a `sanctions` block suspends the engagement
/// with *on hold*, holds the real tasks, pauses the real retainer with the
/// reason `on_hold`, and sends no message and opens no task.
#[renox::test]
async fn a_sanctions_block_pauses_the_real_retainer_with_on_hold_and_opens_nothing() {
    let t = boot_real().await;
    t.partner().await;
    let (client, engagement) = served(&t).await;

    t.set_clock(2, 10, 0);
    t.block(client.company, "sanctions").await;

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    let held = deadline_tasks(t.state())
        .of_subject(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(held.len(), 2);
    assert!(held.iter().all(|task| task.held));
    let retainer = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(retainer[0].status, SubscriptionStatus::Paused);
    let rows = pauses(&t, &engagement).await;
    assert!(
        rows.contains(&(PauseCause::Agreement, "on_hold".to_owned())),
        "{rows:?}"
    );
    for party in client.parties() {
        assert!(t.notices.to_party(party).is_empty(), "a message to {party}");
    }
    // The two tasks above are deadline tasks; no staff task was opened and
    // no deadline notice prepared.
    assert!(
        DeadlineNotice::of_engagement(t.db(), engagement.id)
            .await
            .unwrap()
            .iter()
            .all(|notice| notice.task_id.is_none())
    );
    assert_eq!(
        Ticket::query()
            .where_(Ticket::SOURCE_KEY.like("accounting.%"))
            .count(t.db())
            .await
            .unwrap(),
        0,
        "no staff task"
    );
}

/// DW4 with the real modules: a `legal_dispute` block sends the client
/// nothing and gives the partner a real ticket with the deadline list and the
/// neutral template.
#[renox::test]
async fn a_legal_dispute_block_opens_the_partners_real_task_with_the_deadline_list() {
    let t = boot_real().await;
    let partner = t.partner().await;
    let (client, engagement) = served(&t).await;

    t.set_clock(2, 10, 0);
    t.block(client.company, "legal_dispute").await;

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    for party in client.parties() {
        assert!(t.notices.to_party(party).is_empty(), "a message to {party}");
    }
    let notice = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap()
        .remove(0);
    let task = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(prepared_key(notice.id).as_str()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the partner's task");
    assert_eq!(task.assignee_user_id, Some(partner.id));
    assert!(!task.confidential, "a partner task is an ordinary one");
    for line in [
        "2026-11-28 — VAT return, period ending 2026-10-31",
        "2026-12-31 — Corporate tax registration",
        "Your engagement is on hold, please contact your partner.",
    ] {
        assert!(
            task.description.contains(line),
            "{line}: {}",
            task.description
        );
    }
}

/// Edge case: a block on an engagement that is already suspended runs no
/// second transition and pauses nothing twice: the shown reason becomes *on
/// hold*, client messages stay off, and the original reason stays in the
/// history for the staff who may read it.
#[renox::test]
async fn a_block_on_a_suspended_engagement_only_changes_the_shown_reason() {
    let t = boot_real().await;
    let manager = t.manager().await;
    let (client, engagement) = served(&t).await;

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let before = history(t.db(), accounting::Engagement::TABLE, engagement.id)
        .await
        .unwrap();
    let pauses_before = pauses(&t, &engagement).await;
    let sent_before = t.notices.all().len();

    t.set_clock(3, 10, 0);
    t.block(client.company, "sanctions").await;

    let after = history(t.db(), accounting::Engagement::TABLE, engagement.id)
        .await
        .unwrap();
    assert_eq!(after.len(), before.len(), "no second transition");
    assert!(
        after
            .iter()
            .any(|row| row.transition == "suspend"
                && row.reason.as_deref() == Some("client_request")),
        "the original reason stays in the history: {after:?}"
    );
    let shown = t.reload(&engagement).await;
    assert_eq!(shown.status, EngagementStatus::Suspended);
    assert_eq!(shown.reason_code.as_deref(), Some("on_hold"));
    assert!(shown.client_messages_suppressed, "restricted: no messages");
    // The engagement pauses nothing a second time; wl-invoicing adds the
    // party-block cause itself, with the neutral reason.
    let rows = pauses(&t, &engagement).await;
    assert_eq!(
        rows.iter()
            .filter(|(cause, _)| *cause == PauseCause::Agreement)
            .count(),
        1,
        "{rows:?}"
    );
    assert!(rows.contains(&(PauseCause::PartyBlock, "on_hold".to_owned())));
    assert!(pauses_before.contains(&(PauseCause::Agreement, "client_request".to_owned())));
    assert_eq!(t.notices.all().len(), sent_before, "nothing new is sent");
    let page = t
        .get(
            &manager,
            &format!("/accounting/engagements/{}", engagement.id),
        )
        .await;
    page.assert_ok().assert_see("On hold");
}
