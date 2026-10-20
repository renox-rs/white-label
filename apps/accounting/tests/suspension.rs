//! Acceptance checks of #327 (BL-ACC-03): suspending an engagement holds
//! its tasks, pauses its retainer and runs `ACC.RUL.09`; no request list is
//! created and open requests remind nobody while it is suspended; a client
//! block suspends it with *on hold*, sends nothing and gives the partner the
//! deadline list only when the block reason is not restricted; time can
//! still be logged. The held task going overdue is in `suspension_real.rs`.

mod support;

use accounting::calendar::VAT_RETURN;
use accounting::deadline_notices::{self, DEADLINE_NOTICE, DeadlineNotice};
use accounting::request_lists::{self, REQUEST_REMINDER_NOTICE, RequestList};
use accounting::{Engagement, EngagementStatus};
use renox::prelude::*;
use wl_kernel::deadlines::{NewDeadlineTask, deadline_tasks};
use wl_kernel::subscriptions::PauseCause;
use wl_kernel::tickets::Assignee;

use support::{Booted, boot, d};

/// Three open tasks of `engagement`: VAT return, VAT payment, CT
/// registration.
async fn three_tasks(t: &Booted, engagement: &Engagement) {
    let tasks = deadline_tasks(t.state());
    for (kind, title, due) in [
        (
            VAT_RETURN,
            "VAT return, period ending 2026-10-31",
            d(11, 28),
        ),
        (
            "vat_payment",
            "VAT payment, period ending 2026-10-31",
            d(11, 28),
        ),
        ("ct_registration", "Corporate tax registration", d(12, 31)),
    ] {
        tasks
            .create(
                t.state(),
                NewDeadlineTask::new(engagement.subject(), kind, title, due)
                    .party(engagement.client()),
            )
            .await
            .unwrap()
            .expect("the task book is on");
    }
}

/// An open request list of `engagement` for October, due 2026-11-05.
async fn open_request(t: &Booted, engagement: &Engagement) -> RequestList {
    RequestList::create(
        t.db(),
        RequestList {
            engagement_id: engagement.id,
            client_party_id: engagement.client_party_id,
            period_end: d(10, 31),
            requested_on: Some(d(10, 26)),
            due_date: Some(d(11, 5)),
            ..RequestList::default()
        },
    )
    .await
    .unwrap()
}

/// DW1: an active engagement with 3 open tasks and a monthly retainer,
/// suspended by the manager with *client request*: `suspended`, the 3 tasks
/// held (the page shows it), the retainer paused, and `ACC.RUL.09` ran.
#[renox::test]
async fn a_suspension_holds_the_tasks_pauses_the_retainer_and_runs_the_deadline_rule() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    three_tasks(&t, &engagement).await;

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("client_request"));

    let book = t.tasks.book(&engagement.subject());
    assert_eq!(book.len(), 3);
    assert!(book.iter().all(|task| task.held), "the 3 tasks are held");
    assert_eq!(t.tasks.holds(), [(engagement.subject(), d(11, 2))]);

    let pauses = t.subscriptions.pauses();
    assert_eq!(pauses.len(), 1);
    assert_eq!(pauses[0].0, engagement.subject());
    assert_eq!(pauses[0].1.cause, PauseCause::Agreement);
    assert_eq!(pauses[0].1.on, d(11, 2));
    assert_eq!(pauses[0].1.reason.as_deref(), Some("client_request"));

    // `ACC.RUL.09` ran: the notice went to the client.
    let notices = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].sent_at.is_some());
    assert_eq!(notices[0].deadline_count, 3);
    assert_eq!(t.notices.of_type(DEADLINE_NOTICE).len(), 1);

    t.get(
        &manager,
        &format!("/accounting/engagements/{}", engagement.id),
    )
    .await
    .assert_ok()
    .assert_see("Held")
    .assert_see("Corporate tax registration");
}

/// DW2: on the next period end no request list is created for the
/// suspended engagement (the period is recorded as skipped), and its open
/// request due 2026-11-05 sends no client reminder; a running engagement's
/// list is created and its open request reminded.
#[renox::test]
async fn while_suspended_no_request_list_is_created_and_no_request_reminds() {
    let t = boot().await;
    let manager = t.manager().await;
    let paused_client = t.client_named("P").await;
    let paused = t
        .engagement_in(&paused_client, EngagementStatus::Active, None)
        .await;
    let running_client = t.client_named("R").await;
    let running = t
        .engagement_in(&running_client, EngagementStatus::Active, None)
        .await;
    let held_request = open_request(&t, &paused).await;
    let running_request = open_request(&t, &running).await;

    t.set_clock(2, 10, 0);
    t.act(&manager, &paused, "suspend", &[("reason", "non_payment")])
        .await
        .assert_ok();

    // 2026-11-08: three days after the due date.
    let reminded = t
        .run(request_lists::remind(t.state(), d(11, 8)))
        .await
        .unwrap();
    assert_eq!(reminded.reminded, [running_request.id]);
    assert_eq!(reminded.held, [held_request.id]);
    let reminders = t.notices.of_type(REQUEST_REMINDER_NOTICE);
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].subject, running_request.subject());
    for party in paused_client.parties() {
        assert!(
            t.notices
                .to_party(party)
                .iter()
                .all(|r| r.type_key != REQUEST_REMINDER_NOTICE),
            "the suspended client is not reminded"
        );
    }
    let kept = RequestList::find(t.db(), held_request.id)
        .await
        .unwrap()
        .unwrap();
    assert!(kept.is_open(), "the open request keeps its state");
    assert_eq!(kept.reminders_sent, 0);

    // 2026-11-30: the period ends.
    let period = t
        .run(request_lists::run_period_end(t.state(), d(11, 30)))
        .await
        .unwrap();
    assert_eq!(period.created.len(), 1);
    assert_eq!(period.skipped.len(), 1);
    let lists = RequestList::of_engagement(t.db(), paused.id).await.unwrap();
    let november = lists
        .iter()
        .find(|l| l.period_end == d(11, 30))
        .expect("the skipped period is recorded");
    assert!(november.is_skipped());
    assert_eq!(november.skipped_on, Some(d(11, 30)));
    let running_lists = RequestList::of_engagement(t.db(), running.id)
        .await
        .unwrap();
    assert!(
        running_lists
            .iter()
            .any(|l| l.period_end == d(11, 30) && l.is_open())
    );
}

/// DW3: `PartyBlocked` with `sanctions`: the engagement is `suspended`
/// with *on hold*, its tasks held and its retainer paused with `on_hold`;
/// no message goes to the client and no task is created.
#[renox::test]
async fn a_sanctions_block_suspends_on_hold_silently() {
    let t = boot().await;
    t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    three_tasks(&t, &engagement).await;
    t.portal_user("Omar", client.ubo, client.company).await;

    t.block(client.company, "sanctions").await;

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    assert!(
        t.tasks
            .book(&engagement.subject())
            .iter()
            .all(|task| task.held)
    );
    let pauses = t.subscriptions.pauses();
    assert_eq!(pauses.len(), 1);
    assert_eq!(pauses[0].1.cause, PauseCause::Agreement);
    assert_eq!(pauses[0].1.reason.as_deref(), Some("on_hold"));
    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}"
        );
    }
    assert!(
        t.tickets.with_key("accounting.").is_empty(),
        "no task is created"
    );
}

/// DW4: `PartyBlocked` with `legal_dispute`: `suspended` with *on hold*,
/// no message to the client, and the partner has a task with the deadline
/// list and the neutral template.
#[renox::test]
async fn a_legal_dispute_block_gives_the_partner_the_deadline_list() {
    let t = boot().await;
    let partner = t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    three_tasks(&t, &engagement).await;
    t.portal_user("Omar", client.ubo, client.company).await;

    t.set_clock(2, 10, 0);
    t.block(client.company, "legal_dispute").await;

    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}"
        );
    }
    let notices = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    let tasks = t
        .tickets
        .with_key(&deadline_notices::prepared_key(notices[0].id));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(partner.id)));
    for line in [
        "2026-11-28 — VAT return, period ending 2026-10-31",
        "2026-12-31 — Corporate tax registration",
        "Your engagement is on hold, please contact your partner.",
    ] {
        assert!(
            tasks[0].description.contains(line),
            "{line}: {}",
            tasks[0].description
        );
    }
}

/// DW6: time can still be logged on a suspended engagement (not on an
/// ended or onboarding one).
#[renox::test]
async fn time_can_still_be_logged_on_a_suspended_engagement() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.act(&manager, &engagement, "suspend", &[("reason", "other")])
        .await
        .assert_ok();
    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert!(suspended.accepts_time_entries());
    let ended = t
        .engagement_in(&client, EngagementStatus::Ended, Some("client_request"))
        .await;
    assert!(!ended.accepts_time_entries());
    let onboarding = t.engagement(&client).await;
    assert!(!onboarding.accepts_time_entries());
}
