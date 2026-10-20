//! #330 (BL-ACC-06) with the real wl-ticketing, wl-invoicing and wl-portal:
//! ending an engagement mid-period cancels the retainer with a pro rata
//! credit, waives the upcoming tasks and lists the started one for the
//! manager, and `ACC.RUL.09` sends the deadline notice (listing the
//! deadlines before the waiver takes them out); for a blocked client the
//! credit is still issued; after the end the client's users have read-only
//! access for 90 days; a scheduled end keeps the scheduled date as its end
//! date and the pro rata with it; an end after a CDD exit sends the client
//! nothing and gives the MLRO a task.

mod support;

use accounting::deadline_notices::{DEADLINE_NOTICE, DeadlineNotice, prepared_key};
use accounting::ending::{run_scheduled, scheduled_key};
use accounting::{EngagementStatus, ReturnStatus, portal};
use renox::chrono::Duration;
use renox::prelude::*;
use wl_invoicing::CreditNote;
use wl_kernel::deadlines::DeadlineTaskStatus;
use wl_kernel::subscriptions::{SubscriptionStatus, subscriptions};
use wl_ticketing::Ticket;

use support::{Booted, boot_billing, d};

/// Starts the retainer of `engagement` on `start` and bills its first cycle
/// (issued at once).
async fn retainer_billed(
    t: &Booted,
    engagement: &accounting::Engagement,
    start: renox::chrono::NaiveDate,
) {
    t.run(accounting::retainer::start(t.state(), engagement, start))
        .await
        .unwrap()
        .expect("the retainer started");
    let billed = t
        .run(wl_invoicing::subscriptions::bill_due(t.state(), start))
        .await
        .unwrap();
    assert_eq!(billed, 1, "the first cycle is invoiced");
}

/// Done when 3: a monthly retainer of AED 3,000 invoiced in advance for
/// November and an end date of 2026-11-15: `end` cancels the retainer with a
/// credit of AED 1,500.00 (15 of 30 days), waives the 2 upcoming tasks, lists
/// the 1 started task for the manager, and `ACC.RUL.09` sends the deadline
/// notice.
#[renox::test]
async fn ending_mid_period_credits_the_retainer_waives_the_tasks_and_sends_the_notice() {
    let t = boot_billing().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let engagement = t.engagement_billed(&client, 300_000).await;
    retainer_billed(&t, &engagement, d(11, 1)).await;
    let started = t
        .calendar_task(
            &engagement,
            "vat_return",
            "VAT return, period ending 2026-10-31",
            d(11, 28),
        )
        .await;
    let q4 = t
        .calendar_task(
            &engagement,
            "vat_return",
            "VAT return, period ending 2026-12-31",
            "2027-01-28".parse().unwrap(),
        )
        .await;
    let ct = t
        .calendar_task(
            &engagement,
            "ct_return",
            "Corporate tax return, year ending 2026-12-31",
            "2027-09-30".parse().unwrap(),
        )
        .await;
    t.task_act(&crew.reviewer, started, "start", &[])
        .await
        .assert_ok();

    t.set_clock(15, 10, 0);
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let ended = t.reload(&engagement).await;
    assert_eq!(ended.status, EngagementStatus::Ended);
    assert_eq!(ended.ended_on, Some(d(11, 15)));

    // The retainer: cancelled on the end date with a credit of 15 of 30 days.
    let subs = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].status, SubscriptionStatus::Cancelled);
    assert_eq!(subs[0].cancelled_on, Some(d(11, 15)));
    let note = CreditNote::find(t.db(), subs[0].final_credit_note.expect("a credit note"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(note.net_total, 150_000, "AED 1,500.00 of AED 3,000.00");
    assert_eq!(note.vat_total, 7_500);

    // The tasks: two upcoming waived, the started one left and listed.
    for task in [q4, ct] {
        let waived = t.task_now(task).await;
        assert_eq!(waived.status, DeadlineTaskStatus::NotRequired);
        assert_eq!(
            waived.closed_reason.as_deref(),
            Some("source agreement ended")
        );
    }
    assert_eq!(
        t.task_now(started).await.status,
        DeadlineTaskStatus::InProgress
    );
    let key = format!("ticketing.source_ended:{}", engagement.subject());
    let listing = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the manager's task");
    assert_eq!(listing.assignee_user_id, Some(crew.reviewer.id));
    assert!(
        listing
            .description
            .contains("VAT return, period ending 2026-10-31"),
        "{}",
        listing.description
    );

    // ACC.RUL.09: the notice went to the client approver and lists the
    // deadlines of the next 90 days, also the two just waived.
    let notices = t.notices.of_type(DEADLINE_NOTICE);
    assert_eq!(notices.len(), 1);
    let record = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(record.len(), 1);
    assert_eq!(record[0].on_date, d(11, 15));
    assert!(record[0].sent_at.is_some());
    assert_eq!(record[0].deadline_count, 2);
    assert!(
        record[0].lines.contains("2026-10-31"),
        "{}",
        record[0].lines
    );
    assert!(
        record[0].lines.contains("2026-12-31"),
        "{}",
        record[0].lines
    );
}

/// Done when 4: with the client blocked the credit for the unused part is
/// still issued (a blocked client's engagement is suspended, and ends from
/// there); wl-invoicing pays no refund (`PTY.ACL.03`).
#[renox::test]
async fn a_blocked_clients_engagement_ends_and_the_credit_is_still_issued() {
    let t = boot_billing().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let engagement = t.engagement_billed(&client, 300_000).await;
    retainer_billed(&t, &engagement, d(11, 1)).await;

    t.set_clock(15, 9, 0);
    t.block(client.company, "sanctions").await;
    assert_eq!(t.status(&engagement).await, EngagementStatus::Suspended);
    t.set_clock(15, 10, 0);
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    assert_eq!(t.status(&engagement).await, EngagementStatus::Ended);

    let subs = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(subs[0].status, SubscriptionStatus::Cancelled);
    let note = CreditNote::find(t.db(), subs[0].final_credit_note.expect("a credit note"))
        .await
        .unwrap()
        .unwrap();
    // Paused from the 15th, the unused part runs from the pause's first day.
    assert_eq!(note.net_total, 160_000);
    assert_ne!(note.status, wl_kernel::invoicing::CreditNoteStatus::Draft);
}

/// Done when 5: after the end the client users keep read-only access to the
/// returns: they read the filed return, nothing can be approved, sent back or
/// withdrawn, and the portal closes 90 days after the end.
#[renox::test]
async fn after_the_end_the_clients_users_have_read_only_access_for_90_days() {
    support::register_portal_section();
    let t = boot_billing().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let engagement = t.engagement_billed(&client, 300_000).await;
    let ret = t.open_return(&engagement, d(11, 28)).await;
    t.return_at(&ret, &crew, ReturnStatus::Approved).await;
    t.ret_act(&crew.partner, &ret, "file", &[("acknowledgement", "FTA-1")])
        .await
        .assert_ok();
    let third = t.open_return(&engagement, d(12, 28)).await;
    t.return_at(&third, &crew, ReturnStatus::ClientApproval)
        .await;
    t.ret_act(
        &crew.reviewer,
        &third,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();

    t.set_clock(15, 10, 0);
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();

    let page = t
        .get(
            &crew.approver,
            &format!("/accounting/portal/returns/{}", ret.id),
        )
        .await;
    page.assert_ok()
        .assert_see("The engagement has ended")
        .assert_dont_see("Withdraw approval")
        .assert_dont_see("Approve for filing");
    t.get(
        &crew.approver,
        &format!("/portal/{}/returns", client.company.0),
    )
    .await
    .assert_ok()
    .assert_see("RET-0001");
    // Nothing can be changed any more.
    t.portal_act(&crew.approver, &third, "approve", &[])
        .await
        .assert_status(422);
    t.portal_act(&crew.approver, &ret, "withdraw", &[("comment", "no")])
        .await
        .assert_status(422);
    t.portal_act(&crew.approver, &third, "send-back", &[("comment", "no")])
        .await
        .assert_status(422);

    // 90 days after the end the portal is still open; the day after, closed.
    let ended_on = d(11, 15);
    let last = ended_on + Duration::days(portal::CLOSES_AFTER_DAYS);
    assert!(
        portal::is_open(t.state(), client.company, last)
            .await
            .unwrap()
    );
    assert!(
        !portal::is_open(t.state(), client.company, last + Duration::days(1))
            .await
            .unwrap()
    );
    assert!(
        !t.run(portal::returns(t.state(), client.company))
            .await
            .unwrap()
            .is_empty()
    );
    t.set_clock_on(last + Duration::days(1), 10, 0);
    assert!(
        t.run(portal::returns(t.state(), client.company))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        t.run(portal::rows(t.state(), client.company))
            .await
            .unwrap()
            .is_empty()
    );
    t.get(
        &crew.approver,
        &format!("/accounting/portal/returns/{}", ret.id),
    )
    .await
    .assert_not_found();
}

/// Done when 6 with the real modules: an end scheduled for 2026-12-31 and a
/// VAT return still `preparing` that day: the engagement stays `active` and
/// the partner has a task naming the return; after the return is cancelled on
/// 2027-01-04 the next daily try ends it with end date 2026-12-31, the retainer
/// is cancelled from that date, and the task closes.
#[renox::test]
async fn a_scheduled_end_keeps_its_date_and_cancels_the_retainer_from_it() {
    let t = boot_billing().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let engagement = t.engagement_billed(&client, 300_000).await;
    retainer_billed(&t, &engagement, d(12, 1)).await;
    let ret = t.open_return(&engagement, d(12, 28)).await;

    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request"), ("end_date", "2026-12-31")],
    )
    .await
    .assert_ok();
    let day = t.run(run_scheduled(t.state(), d(12, 31))).await.unwrap();
    assert_eq!(day.refused, [engagement.id]);
    assert_eq!(t.status(&engagement).await, EngagementStatus::Active);
    let key = scheduled_key(engagement.id);
    let task = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the partner's task");
    assert_eq!(task.assignee_user_id, Some(crew.partner.id));
    assert!(
        task.description.contains("RET-0001"),
        "{}",
        task.description
    );

    t.ret_act(
        &crew.reviewer,
        &ret,
        "cancel",
        &[("reason", "engagement_ended")],
    )
    .await
    .assert_ok();
    let jan4 = "2027-01-04".parse().unwrap();
    let retry = t.run(run_scheduled(t.state(), jan4)).await.unwrap();
    assert_eq!(retry.ended, [engagement.id]);
    let ended = t.reload(&engagement).await;
    assert_eq!(ended.status, EngagementStatus::Ended);
    assert_eq!(ended.ended_on, Some(d(12, 31)));
    let subs = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(subs[0].status, SubscriptionStatus::Cancelled);
    assert_eq!(
        subs[0].cancelled_on,
        Some(d(12, 31)),
        "pro rata from the scheduled date"
    );
    let open = wl_kernel::tickets::task_creator(t.state())
        .find_open(t.state(), &key)
        .await
        .unwrap();
    assert!(open.is_none(), "the partner's task closed");
}

/// ACC.ACL.14: an end after a CDD exit sends the client nothing; the MLRO
/// gets the task with the deadline list and the neutral template; the
/// retainer and the tasks are still settled.
#[renox::test]
async fn an_end_after_a_cdd_exit_tells_the_mlro_and_still_settles_the_engagement() {
    let t = boot_billing().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let crew = t.crew(&client).await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t.engagement_billed(&client, 300_000).await;
    retainer_billed(&t, &engagement, d(11, 1)).await;
    let task = t
        .calendar_task(
            &engagement,
            "vat_return",
            "VAT return, period ending 2026-12-31",
            "2027-01-28".parse().unwrap(),
        )
        .await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", "AML concern")],
    )
    .await
    .assert_ok();

    t.set_clock(15, 10, 0);
    t.act(
        &crew.partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let ended = t.reload(&engagement).await;
    assert_eq!(ended.reason_code.as_deref(), Some("relationship_ended"));
    assert!(
        t.notices.of_type(DEADLINE_NOTICE).is_empty(),
        "nothing is sent"
    );
    for party in client.parties() {
        assert!(
            t.notices.to_party(party).is_empty(),
            "a message went to {party}"
        );
    }
    let notice = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(notice.prepared_for.as_deref(), Some("compliance"));
    let key = prepared_key(notice.id);
    let todo = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key.as_str()))
        .first(t.db())
        .await
        .unwrap()
        .expect("the MLRO's task");
    assert_eq!(todo.assignee_user_id, Some(mlro.id));
    assert!(
        todo.description.contains("2027-01-28"),
        "{}",
        todo.description
    );

    let subs = subscriptions(t.state())
        .of_agreement(t.db(), &engagement.subject())
        .await
        .unwrap();
    assert_eq!(subs[0].status, SubscriptionStatus::Cancelled);
    assert_eq!(
        t.task_now(task).await.status,
        DeadlineTaskStatus::NotRequired
    );
}
