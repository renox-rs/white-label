//! More acceptance checks of #328 (BL-ACC-04, `ACC.RUL.09`): several client
//! approvers all get the notice and one acknowledgement is enough; a
//! suspension with no deadline in the next 90 days still sends the notice; a
//! client with no approver in the portal is written to as the company; an end
//! says the firm no longer acts; the partner's task for want of an
//! acknowledgement is closed by the resume; and a failed daily run alerts the
//! partner and the company admin.

mod support;

use std::time::Duration;

use accounting::calendar::VAT_RETURN;
use accounting::deadline_notices::{DEADLINE_NOTICE, DEADLINE_REMINDER, DeadlineNotice, follow_up};
use accounting::{Engagement, EngagementStatus};
use renox::auth::permissions;
use wl_kernel::COMPANY_ADMIN_ROLE;
use wl_kernel::deadlines::{NewDeadlineTask, deadline_tasks};
use wl_kernel::notify::NotifyRecipient;

use support::{Booted, boot, d};

/// A VAT return of `engagement` due 2026-11-28.
async fn vat_return(t: &Booted, engagement: &Engagement) {
    deadline_tasks(t.state())
        .create(
            t.state(),
            NewDeadlineTask::new(
                engagement.subject(),
                VAT_RETURN,
                "VAT return, period ending 2026-10-31",
                d(11, 28),
            )
            .party(engagement.client()),
        )
        .await
        .unwrap()
        .expect("the task book is on");
}

/// The only notice of `engagement`.
async fn the_notice(t: &Booted, engagement: &Engagement) -> DeadlineNotice {
    let mut notices = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    notices.remove(0)
}

/// Two client approvers (the UBO and the director) both get the notice; one
/// acknowledgement is enough, a second changes nothing, and the reminder and
/// the partner's task are not needed any more.
#[renox::test]
async fn every_approver_is_told_and_one_acknowledgement_is_enough() {
    let t = boot().await;
    let manager = t.manager().await;
    t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    vat_return(&t, &engagement).await;
    let omar = t.portal_user("Omar", client.ubo, client.company).await;
    let lina = t.portal_user("Lina", client.director, client.company).await;

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let notice = the_notice(&t, &engagement).await;
    let sent = t.notices.of_type(DEADLINE_NOTICE);
    assert_eq!(sent.len(), 1);
    let mut told = sent[0].recipients.clone();
    told.sort_by_key(|r| format!("{r:?}"));
    let mut expected = vec![
        NotifyRecipient::Party { id: client.ubo },
        NotifyRecipient::Party {
            id: client.director,
        },
    ];
    expected.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(told, expected, "both approvers");

    // Lina acknowledges; Omar's later click changes nothing.
    let path = format!("{}/acknowledge", notice.portal_path());
    t.post(&lina, &path, &[]).await.assert_ok();
    let first = the_notice(&t, &engagement).await;
    assert_eq!(first.acknowledged_by, Some(lina.id));
    t.post(&omar, &path, &[]).await.assert_ok();
    let second = the_notice(&t, &engagement).await;
    assert_eq!(second.acknowledged_by, Some(lina.id));
    assert_eq!(second.acknowledged_at, first.acknowledged_at);

    // Acknowledged: no reminder on the 5th, no partner task on the 9th.
    let on_5th = t.run(follow_up(t.state(), d(11, 5))).await.unwrap();
    let on_9th = t.run(follow_up(t.state(), d(11, 9))).await.unwrap();
    assert!(on_5th.reminded.is_empty() && on_9th.escalated.is_empty());
    assert!(t.notices.of_type(DEADLINE_REMINDER).is_empty());
}

/// No deadline in the next 90 days: the notice is still sent, and says so.
#[renox::test]
async fn a_suspension_with_no_deadline_in_the_window_still_sends_the_notice() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.portal_user("Omar", client.ubo, client.company).await;

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let notice = the_notice(&t, &engagement).await;
    assert_eq!(notice.deadline_count, 0);
    assert!(notice.sent_at.is_some());
    assert!(
        notice.lines.contains("no statutory deadlines"),
        "{}",
        notice.lines
    );
    assert_eq!(t.notices.of_type(DEADLINE_NOTICE).len(), 1);
}

/// A client with nobody in the portal is written to as the company.
#[renox::test]
async fn without_an_approver_the_company_is_written_to() {
    let t = boot().await;
    let manager = t.manager().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    vat_return(&t, &engagement).await;

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let sent = t.notices.of_type(DEADLINE_NOTICE);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].recipients,
        [NotifyRecipient::Party { id: client.company }]
    );
}

/// An end sends the same list worded for an end: the firm no longer acts.
#[renox::test]
async fn an_end_says_the_firm_no_longer_acts() {
    let t = boot().await;
    let partner = t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    vat_return(&t, &engagement).await;
    t.portal_user("Omar", client.ubo, client.company).await;

    t.set_clock(10, 10, 0);
    t.act(
        &partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let sent = t.notices.of_type(DEADLINE_NOTICE);
    assert_eq!(sent.len(), 1);
    let wording = sent[0].data["wording"].as_str().unwrap_or_default();
    assert!(wording.contains("no longer acts"), "{wording}");
    assert!(!wording.contains("will not act"), "{wording}");
}

/// The partner's task for want of an acknowledgement is closed when the
/// engagement resumes, and no more reminders follow.
#[renox::test]
async fn the_resume_closes_the_partners_task() {
    let t = boot().await;
    let manager = t.manager().await;
    t.partner().await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    vat_return(&t, &engagement).await;
    t.portal_user("Omar", client.ubo, client.company).await;

    t.set_clock(2, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    t.run(follow_up(t.state(), d(11, 9))).await.unwrap();
    let tasks = t.tickets.with_key("accounting.deadline_notice_ack:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].open, "open for want of an acknowledgement");

    t.set_clock(10, 10, 0);
    t.act(
        &manager,
        &engagement,
        "resume",
        &[("note", "The client asked to start again")],
    )
    .await
    .assert_ok();
    let tasks = t.tickets.with_key("accounting.deadline_notice_ack:");
    assert!(!tasks[0].open, "closed by the resume");
    assert!(the_notice(&t, &engagement).await.cancelled_on.is_some());
}

/// `ACC.RUL.09` is compliance-critical: a daily run that fails on all five
/// attempts alerts the partner role and the company admin, once.
#[renox::test]
async fn a_failed_run_alerts_the_partner_and_the_admin() {
    let t = boot().await;
    let partner = t.partner().await;
    permissions::define_role(t.db(), COMPANY_ADMIN_ROLE, &[])
        .await
        .unwrap();
    let admin = t.user_with("Ada", COMPANY_ADMIN_ROLE).await;
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    let manager = t.manager().await;
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    renox::db::sql(
        "ALTER TABLE \"accounting_deadline_notices\" RENAME TO \"accounting_deadline_notices_off\"",
    )
    .execute(t.db())
    .await
    .unwrap();

    let alerted = |user: i64| {
        t.notices.all().iter().any(|r| {
            r.type_key == "kernel.compliance_alert"
                && r.recipients.contains(&NotifyRecipient::User { id: user })
        })
    };
    t.set_clock(3, 8, 0);
    for wait in [60_u64, 300, 900, 3600] {
        // The run retries on a failure; the answer is not the point here.
        let _retrying = t.run(follow_up(t.state(), d(11, 3))).await;
        assert!(!alerted(partner.id), "not before the last attempt");
        t.app.travel(Duration::from_secs(wait));
    }
    let _last = t.run(follow_up(t.state(), d(11, 3))).await;
    let alerts: Vec<_> = wl_kernel::compliance::ComplianceAlert::open(t.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|a| a.rule == accounting::deadline_notices::RULE)
        .collect();
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0].owner_role, support::PARTNER);
    assert!(alerted(partner.id), "the partner role is told");
    assert!(alerted(admin.id), "the company admin is told");
}
