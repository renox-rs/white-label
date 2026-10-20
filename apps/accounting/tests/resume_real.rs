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

use support::{Booted, boot_billing, boot_real, d};

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

/// Starts the retainer of `engagement` on `start` and bills its first cycle
/// (issued at once, due on `start`).
async fn retainer_billed(t: &Booted, engagement: &accounting::Engagement, start: NaiveDate) {
    t.run(retainer::start(t.state(), engagement, start))
        .await
        .unwrap()
        .expect("the retainer started");
    let billed = t
        .run(wl_invoicing::subscriptions::bill_due(t.state(), start))
        .await
        .unwrap();
    assert_eq!(billed, 1, "the first cycle is invoiced");
}

/// DW2 with the real wl-invoicing, through the suspend route: suspended for
/// *non-payment* with the retainer invoice overdue, the page shows what is
/// overdue from wl-invoicing itself, the resume needs a note, and releases
/// the real tasks and resumes the real retainer.
#[renox::test]
async fn a_non_payment_suspension_shows_the_real_overdue_invoice_and_resumes() {
    let t = boot_billing().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t.engagement_billed(&client, 600_000).await;
    let task = t
        .calendar_task(
            &engagement,
            "vat_return",
            "VAT return, period ending 2026-10-31",
            d(11, 28),
        )
        .await;
    retainer_billed(&t, &engagement, d(11, 1)).await;

    // The invoice falls due on 2026-11-01 and is overdue on the 2nd.
    t.set_clock(2, 10, 0);
    let moved = t.run(wl_invoicing::overdue::run(t.state())).await.unwrap();
    assert_eq!(moved, 1, "the invoice is overdue");
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "non_payment")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);
    let held = deadline_tasks(t.state())
        .find(t.db(), task)
        .await
        .unwrap()
        .unwrap();
    assert!(held.held, "the real task is held by the suspension");

    let invoice = wl_invoicing::Invoice::query()
        .where_(wl_invoicing::Invoice::PARTY_ID.eq(client.company.0))
        .first(t.db())
        .await
        .unwrap()
        .expect("the retainer invoice");
    let number = invoice.number.clone().expect("an issued invoice has a number");
    let page = t
        .get(
            &manager,
            &format!("/accounting/engagements/{}", engagement.id),
        )
        .await;
    page.assert_ok()
        .assert_see("Overdue invoices")
        .assert_see(&number)
        .assert_see(&accounting::money_text(invoice.balance));

    t.set_clock(4, 10, 0);
    t.act(&manager, &engagement, "resume", &[("note", " ")])
        .await
        .assert_status(422);
    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "Payment plan agreed")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    let released = deadline_tasks(t.state())
        .find(t.db(), task)
        .await
        .unwrap()
        .unwrap();
    assert!(!released.held, "released");
    let retainer = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    // The overdue invoice keeps it past due (wl-invoicing's own dunning);
    // the suspension no longer pauses it.
    assert_eq!(retainer[0].status, SubscriptionStatus::PastDue);
    assert!(retainer[0].pause_causes.is_empty());
}

/// DW3 and DW4 with the real modules: a block suspends the engagement
/// (*on hold*, tasks held, retainer paused); after the unblock the resume
/// becomes a request to the MLRO; the deputy sends it back (the engagement
/// stays suspended and nothing is released), and the deputy approves the
/// second request: active, tasks released, retainer resumed.
#[renox::test]
async fn after_a_block_the_deputy_decides_and_the_real_modules_follow() {
    use wl_kernel::approvals;

    let t = boot_billing().await;
    let (manager, mlro, deputy) = (t.manager().await, t.mlro().await, t.deputy().await);
    let client = t.client().await;
    let engagement = t.engagement_billed(&client, 300_000).await;
    let task = t
        .calendar_task(
            &engagement,
            "vat_return",
            "VAT return, period ending 2026-10-31",
            d(11, 28),
        )
        .await;
    retainer_billed(&t, &engagement, d(11, 1)).await;
    let tasks = deadline_tasks(t.state());

    t.set_clock(2, 10, 0);
    t.block(client.company, "non_payment").await;
    let suspended = t.reload(&engagement).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    assert!(tasks.find(t.db(), task).await.unwrap().unwrap().held);

    // Still blocked: refused. Unblocked: asked of the MLRO, nothing moves.
    t.act(&manager, &engagement, "resume", &[("note", "Paid")])
        .await
        .assert_status(422);
    t.unblock(&mlro, client.company, "Arrears settled in full")
        .await;
    t.set_clock(4, 10, 0);
    t.act(&manager, &engagement, "resume", &[("note", "Paid")])
        .await
        .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);
    assert!(tasks.find(t.db(), task).await.unwrap().unwrap().held);
    let request = approvals::pending_for(t.db(), &engagement.subject())
        .await
        .unwrap()
        .remove(0);

    // The deputy sends it back: still suspended, nothing released.
    t.run(approvals::return_request(
        t.state(),
        request.id,
        &deputy,
        "Wait for the second instalment",
    ))
    .await
    .unwrap();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);
    assert!(tasks.find(t.db(), task).await.unwrap().unwrap().held);
    assert!(
        approvals::pending_for(t.db(), &engagement.subject())
            .await
            .unwrap()
            .is_empty()
    );

    // Asked again, the deputy approves.
    t.act(&manager, &engagement, "resume", &[("note", "Second paid")])
        .await
        .assert_ok();
    let again = approvals::pending_for(t.db(), &engagement.subject())
        .await
        .unwrap()
        .remove(0);
    t.run(approvals::approve(t.state(), again.id, &deputy, "Cleared"))
        .await
        .unwrap();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    assert!(!tasks.find(t.db(), task).await.unwrap().unwrap().held);
    let retainer = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(retainer[0].status, SubscriptionStatus::Active);
    assert!(retainer[0].pause_causes.is_empty(), "{:?}", retainer[0]);
}
