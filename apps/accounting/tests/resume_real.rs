//! #329 (BL-ACC-05) DW1 with the real wl-ticketing and wl-invoicing: an
//! engagement suspended on 2026-10-01 (*client request*) with monthly
//! periods, resumed by the manager on 2026-12-05: `active`, its held tasks
//! released (the one that went overdue meanwhile listed for the manager),
//! the retainer `active` with its next invoice date recalculated, and the
//! request lists of October and November created with one client notice.
//!
//! The suspension of 2026-10-01 lies before the test's present, so the
//! engagement is a `suspended` fixture and its suspension's effects (the
//! tasks held, the retainer paused) are applied as the commit applies them,
//! on that day.

mod support;

use accounting::calendar::VAT_RETURN;
use accounting::request_lists::{self, LEAD_DAYS, REQUEST_LIST_NOTICE, RequestList};
use accounting::{EngagementStatus, retainer, suspension};
use renox::chrono::{Duration, NaiveDate};
use renox::prelude::*;
use wl_kernel::deadlines::{DeadlineTaskStatus, NewDeadlineTask, deadline_tasks};
use wl_kernel::subscriptions::{SubscriptionStatus, subscriptions};

use support::{boot_real, d};

#[renox::test]
async fn a_resume_releases_the_tasks_resumes_the_retainer_and_creates_the_skipped_lists() {
    let t = boot_real().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Suspended, Some("client_request"))
        .await;
    let subject = engagement.subject();
    let tasks = deadline_tasks(t.state());
    let past = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(
                subject.clone(),
                VAT_RETURN,
                "VAT return, period ending 2026-09-30",
                d(10, 28),
            )
            .party(client.company),
        )
        .await
        .unwrap()
        .expect("ticketing is on");
    let coming = tasks
        .create(
            t.state(),
            NewDeadlineTask::new(
                subject.clone(),
                VAT_RETURN,
                "VAT return, period ending 2026-12-31",
                NaiveDate::from_ymd_opt(2027, 1, 28).unwrap(),
            )
            .party(client.company),
        )
        .await
        .unwrap()
        .unwrap();
    t.run(retainer::start(t.state(), &engagement, d(9, 1)))
        .await
        .unwrap()
        .expect("the retainer started");

    // 2026-10-01: the suspension held the tasks and paused the retainer.
    t.run(suspension::hold(t.state(), &engagement, d(10, 1)))
        .await
        .unwrap();
    let paused = subscriptions(t.state())
        .of_agreement(t.db(), &subject)
        .await
        .unwrap();
    assert_eq!(paused[0].status, SubscriptionStatus::Paused);
    // 2026-10-29: the held task goes overdue; the month ends pass.
    t.run(wl_ticketing::deadlines::mark_overdue(t.state(), d(10, 29)))
        .await
        .unwrap();
    for end in [d(10, 31), d(11, 30)] {
        let period = t
            .run(request_lists::run_period_end(t.state(), end))
            .await
            .unwrap();
        assert!(period.created.is_empty(), "no list while suspended");
        assert_eq!(period.skipped.len(), 1);
    }

    // 2026-12-05: the manager resumes.
    let on = NaiveDate::from_ymd_opt(2026, 12, 5).unwrap();
    t.set_clock_on(on, 10, 0);
    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "The client asked us to start again")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);

    // The tasks are released; the one that went overdue is listed for the
    // manager.
    for id in [past, coming] {
        let task = tasks.find(t.db(), id).await.unwrap().unwrap();
        assert!(!task.held, "{} is released", task.title);
    }
    let overdue = tasks.find(t.db(), past).await.unwrap().unwrap();
    assert_eq!(overdue.status, DeadlineTaskStatus::Overdue);
    let listings = wl_ticketing::Ticket::query()
        .where_(wl_ticketing::Ticket::ASSIGNEE_USER_ID.eq(manager.id))
        .get(t.db())
        .await
        .unwrap();
    assert!(
        listings
            .iter()
            .any(|ticket| ticket.description.contains("(overdue since 2026-10-29)")),
        "the manager's list: {:?}",
        listings
            .iter()
            .map(|ticket| &ticket.description)
            .collect::<Vec<_>>()
    );

    // The retainer runs again from the next cycle.
    let retainer = subscriptions(t.state())
        .of_agreement(t.db(), &subject)
        .await
        .unwrap();
    assert_eq!(retainer[0].status, SubscriptionStatus::Active);
    assert!(retainer[0].pause_causes.is_empty());
    assert!(
        retainer[0].next_cycle_date >= on,
        "the next invoice date is recalculated: {}",
        retainer[0].next_cycle_date
    );

    // October and November: created now, due today plus the lead days,
    // with one notice to the client.
    let lists = RequestList::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    let periods: Vec<NaiveDate> = lists.iter().map(|l| l.period_end).collect();
    assert_eq!(periods, [d(10, 31), d(11, 30)]);
    for list in &lists {
        assert_eq!(list.requested_on, Some(on));
        assert_eq!(list.due_date, Some(on + Duration::days(LEAD_DAYS)));
        assert!(list.is_open());
    }
    let told = t.notices.of_type(REQUEST_LIST_NOTICE);
    assert_eq!(told.len(), 1, "one notice for all the periods");
    assert_eq!(
        told[0].data["periods"].as_str(),
        Some("2026-10-31, 2026-11-30")
    );
}
