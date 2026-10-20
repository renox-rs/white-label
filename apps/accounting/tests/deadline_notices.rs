//! Acceptance checks of #328 (BL-ACC-04, `ACC.RUL.09`): the statutory
//! deadlines due in the next 90 days go to the client approver on a
//! suspension that is not restricted, with an Acknowledge button in the
//! portal; a reminder after 3 days and a partner task after 7 days, both
//! counted from delivery or the recorded proof; a block or a restricted end
//! sends nothing and gives the partner or the MLRO the deadline list.

mod support;

use accounting::calendar::VAT_RETURN;
use accounting::deadline_notices::{
    self, DEADLINE_NOTICE, DEADLINE_REMINDER, DeadlineNotice, NoticeCause, follow_up,
};
use accounting::{Engagement, EngagementStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::BusinessCalendar;
use wl_kernel::deadlines::{NewDeadlineTask, deadline_tasks};
use wl_kernel::notify::{self, GiveUpReaction, NotifyRecipient};
use wl_kernel::tickets::Assignee;

use support::{Booted, Client, boot, d};

/// The VAT returns of `engagement` due 2026-11-28 and 2027-02-28 in its
/// compliance calendar.
async fn vat_returns(t: &Booted, engagement: &Engagement) {
    let tasks = deadline_tasks(t.state());
    for (title, due) in [
        ("VAT return, period ending 2026-10-31", d(11, 28)),
        (
            "VAT return, period ending 2027-01-31",
            NaiveDate::from_ymd_opt(2027, 2, 28).unwrap(),
        ),
    ] {
        tasks
            .create(
                t.state(),
                NewDeadlineTask::new(engagement.subject(), VAT_RETURN, title, due)
                    .party(engagement.client()),
            )
            .await
            .unwrap()
            .expect("the task book is on");
    }
}

/// An active engagement of a fresh client (`tag` tells clients apart) with
/// its two VAT returns, and the client's approver in the portal.
async fn served(t: &Booted, tag: &str) -> (Client, Engagement, User) {
    let client = t.client_named(tag).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    vat_returns(t, &engagement).await;
    let approver = t
        .portal_user(&format!("Omar{tag}"), client.ubo, client.company)
        .await;
    (client, engagement, approver)
}

/// The only deadline notice of `engagement`.
async fn the_notice(t: &Booted, engagement: &Engagement) -> DeadlineNotice {
    let notices = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap();
    assert_eq!(notices.len(), 1, "one deadline notice");
    notices.into_iter().next().unwrap()
}

/// DW1: suspended on 2026-11-02 with *client request*: the client approver
/// gets a notice listing 2026-11-28 only (2027-02-28 is beyond 90 days),
/// saying the firm will not act on it, and acknowledges it in the portal.
#[renox::test]
async fn a_suspension_sends_the_deadlines_of_the_next_90_days_to_acknowledge() {
    let t = boot().await;
    let manager = t.manager().await;
    let (client, engagement, approver) = served(&t, "").await;

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
    assert_eq!(notice.cause, NoticeCause::Suspend);
    assert_eq!(notice.on_date, d(11, 2));
    assert_eq!(
        notice.window_end,
        NaiveDate::from_ymd_opt(2027, 1, 31).unwrap()
    );
    assert_eq!(notice.deadline_count, 1);
    assert!(notice.lines.contains("2026-11-28"), "{}", notice.lines);
    assert!(!notice.lines.contains("2027-02-28"), "{}", notice.lines);
    assert!(notice.sent_at.is_some());

    let sent = t.notices.of_type(DEADLINE_NOTICE);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].subject, notice.subject());
    assert_eq!(
        sent[0].recipients,
        [NotifyRecipient::Party { id: client.ubo }],
        "the client approver, as a person"
    );
    let lines = sent[0].data["lines"].as_str().unwrap_or_default();
    assert!(lines.contains("2026-11-28") && !lines.contains("2027-02-28"));
    let wording = sent[0].data["wording"].as_str().unwrap_or_default();
    assert!(wording.contains("will not act on these"), "{wording}");
    assert_eq!(
        sent[0].data["link"].as_str(),
        Some(notice.portal_path().as_str())
    );

    // The portal page lists it with the Acknowledge button.
    let page = t.get(&approver, &notice.portal_path()).await;
    page.assert_ok()
        .assert_see("2026-11-28")
        .assert_see("Acknowledge")
        .assert_dont_see("2027-02-28");
    // Someone who does not act for the client gets nothing.
    let stranger = t.client_named("Other").await;
    let outsider = t.portal_user("Rami", stranger.ubo, stranger.company).await;
    t.get(&outsider, &notice.portal_path())
        .await
        .assert_not_found();
    t.post(
        &outsider,
        &format!("{}/acknowledge", notice.portal_path()),
        &[],
    )
    .await
    .assert_not_found();

    t.post(
        &approver,
        &format!("{}/acknowledge", notice.portal_path()),
        &[],
    )
    .await
    .assert_ok();
    let acknowledged = the_notice(&t, &engagement).await;
    assert_eq!(acknowledged.acknowledged_by, Some(approver.id));
    assert!(acknowledged.acknowledged_at.is_some());
    t.get(&approver, &notice.portal_path())
        .await
        .assert_ok()
        .assert_see("Acknowledged on");

    // Acknowledged: no reminder and no partner task follow.
    let later = t.run(follow_up(t.state(), d(11, 10))).await.unwrap();
    assert!(later.reminded.is_empty() && later.escalated.is_empty());
}

/// DW2: without an acknowledgement, the reminder goes on 2026-11-05 (3 days
/// after delivery) and the partner gets a task on 2026-11-09 (7 days).
#[renox::test]
async fn without_an_acknowledgement_the_client_is_reminded_and_the_partner_told() {
    let t = boot().await;
    let manager = t.manager().await;
    let partner = t.partner().await;
    let (client, engagement, _approver) = served(&t, "").await;

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

    let on_4th = t.run(follow_up(t.state(), d(11, 4))).await.unwrap();
    assert!(on_4th.reminded.is_empty());
    assert_eq!(
        the_notice(&t, &engagement).await.delivered_on,
        Some(d(11, 2))
    );

    let on_5th = t.run(follow_up(t.state(), d(11, 5))).await.unwrap();
    assert_eq!(on_5th.reminded, [notice.id]);
    assert!(on_5th.escalated.is_empty());
    let reminders = t.notices.of_type(DEADLINE_REMINDER);
    assert_eq!(reminders.len(), 1);
    assert_eq!(
        reminders[0].recipients,
        [NotifyRecipient::Party { id: client.ubo }]
    );

    let on_8th = t.run(follow_up(t.state(), d(11, 8))).await.unwrap();
    assert!(on_8th.escalated.is_empty());
    assert!(
        t.tickets
            .with_key("accounting.deadline_notice_ack:")
            .is_empty()
    );

    let on_9th = t.run(follow_up(t.state(), d(11, 9))).await.unwrap();
    assert_eq!(on_9th.escalated, [notice.id]);
    let tasks = t.tickets.with_key("accounting.deadline_notice_ack:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(partner.id)));
    assert!(tasks[0].description.contains("2026-11-28"));
    assert_eq!(
        t.notices.of_type(DEADLINE_REMINDER).len(),
        1,
        "one reminder only"
    );
}

/// DW3: the e-mail bounces on Friday 2026-11-06: the notice is statutory
/// (wl-notify marks the contact invalid and gives staff a delivery task due
/// the next working day, Monday 2026-11-09), and no reminder goes until
/// the proof is recorded; the clocks then count from the proven day.
#[renox::test]
async fn an_undelivered_notice_starts_no_clock_until_its_proof() {
    let t = boot().await;
    let manager = t.manager().await;
    let (_client, engagement, _approver) = served(&t, "").await;

    t.set_clock(6, 10, 0);
    t.act(&manager, &engagement, "suspend", &[("reason", "other")])
        .await
        .assert_ok();
    let notice = the_notice(&t, &engagement).await;
    t.notices.bounce(&notice.subject());

    let def = notify::notification_type(DEADLINE_NOTICE).expect("registered");
    assert!(def.statutory);
    assert_eq!(
        notify::default_give_up(&def).reactions,
        [
            GiveUpReaction::MarkContactInvalid,
            GiveUpReaction::StatutoryNotice
        ]
    );
    assert_eq!(
        BusinessCalendar::default().next_working_day(d(11, 6)),
        d(11, 9),
        "the delivery task is due on Monday"
    );

    for day in [9, 10, 13, 16] {
        let run = t.run(follow_up(t.state(), d(11, day))).await.unwrap();
        assert!(run.reminded.is_empty(), "no reminder on {day} November");
        assert_eq!(run.undelivered, [notice.id]);
    }
    assert!(t.notices.of_type(DEADLINE_REMINDER).is_empty());

    // Staff record the proof: delivered by hand on Tuesday 2026-11-17.
    t.notices.prove(&notice.subject(), d(11, 17));
    let on_19th = t.run(follow_up(t.state(), d(11, 19))).await.unwrap();
    assert!(on_19th.reminded.is_empty());
    assert_eq!(
        the_notice(&t, &engagement).await.delivered_on,
        Some(d(11, 17))
    );
    let on_20th = t.run(follow_up(t.state(), d(11, 20))).await.unwrap();
    assert_eq!(on_20th.reminded, [notice.id]);
}

/// DW4: a suspension from a `sanctions` block sends nothing and opens no
/// task; from a `non_payment` block, nothing is sent and the partner has a
/// task with the deadline list and the neutral template.
#[renox::test]
async fn a_block_sends_nothing_and_the_partner_gets_the_list_unless_restricted() {
    let t = boot().await;
    let partner = t.partner().await;

    let (sanctioned, restricted, _approver) = served(&t, "A").await;
    t.block(sanctioned.company, "sanctions").await;
    let suspended = t.reload(&restricted).await;
    assert_eq!(suspended.status, EngagementStatus::Suspended);
    assert_eq!(suspended.reason_code.as_deref(), Some("on_hold"));
    assert!(
        DeadlineNotice::of_engagement(t.db(), restricted.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.tickets.with_key("accounting.deadline_notice").is_empty());

    let (unpaid, open, _approver) = served(&t, "B").await;
    t.block(unpaid.company, "non_payment").await;
    assert_eq!(t.status(&open).await, EngagementStatus::Suspended);
    let notice = the_notice(&t, &open).await;
    assert!(notice.sent_at.is_none(), "nothing sent automatically");
    assert_eq!(notice.prepared_for.as_deref(), Some("partner"));
    let tasks = t
        .tickets
        .with_key(&deadline_notices::prepared_key(notice.id));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(partner.id)));
    assert!(tasks[0].description.contains("2026-11-28"));
    assert!(
        tasks[0]
            .description
            .contains("Your engagement is on hold, please contact your partner.")
    );
    for client in [sanctioned, unpaid] {
        for party in client.parties() {
            assert!(
                t.notices.to_party(party).is_empty(),
                "a message went to {party}"
            );
        }
    }

    // The partner sends it when they decide: then it goes to the approver.
    t.get(&partner, &format!("/accounting/engagements/{}", open.id))
        .await
        .assert_ok()
        .assert_see("Send the deadline notice");
    t.post(
        &partner,
        &format!("/accounting/deadline-notices/{}/send", notice.id),
        &[],
    )
    .await
    .assert_ok();
    let sent = the_notice(&t, &open).await;
    assert_eq!(sent.sent_by, Some(partner.id));
    assert_eq!(t.notices.of_type(DEADLINE_NOTICE).len(), 1);
    assert!(
        !t.tickets
            .with_key(&deadline_notices::prepared_key(notice.id))[0]
            .open
    );
}

/// DW5: `end` with a restricted reason (after a CDD exit) sends nothing and
/// gives the MLRO a task with the deadline list and the neutral "ended"
/// template; the partner may not send it, the MLRO may.
#[renox::test]
async fn a_restricted_end_sends_nothing_and_the_mlro_gets_the_list() {
    let t = boot().await;
    let partner = t.partner().await;
    let mlro = t.mlro().await;
    let client = t.client().await;
    let file = t.cdd_approved(&client, &mlro).await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    vat_returns(&t, &engagement).await;
    t.portal_user("Omar", client.ubo, client.company).await;
    t.post(
        &mlro,
        &format!("/accounting/cdd/{}/exit", file.id),
        &[("reason", "AML concern: unexplained transfers")],
    )
    .await
    .assert_ok();

    t.act(
        &partner,
        &engagement,
        "end",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    let notice = the_notice(&t, &engagement).await;
    assert_eq!(notice.cause, NoticeCause::End);
    assert!(notice.sent_at.is_none());
    assert_eq!(notice.prepared_for.as_deref(), Some("compliance"));
    assert!(t.notices.of_type(DEADLINE_NOTICE).is_empty());
    for party in client.parties() {
        assert!(t.notices.to_party(party).is_empty());
    }
    let tasks = t
        .tickets
        .with_key(&deadline_notices::prepared_key(notice.id));
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee, Some(Assignee::user(mlro.id)));
    assert!(tasks[0].description.contains("Our engagement has ended"));

    let path = format!("/accounting/deadline-notices/{}/send", notice.id);
    t.post(&partner, &path, &[]).await.assert_forbidden();
    t.post(&mlro, &path, &[]).await.assert_ok();
    assert_eq!(t.notices.of_type(DEADLINE_NOTICE).len(), 1);
}
