//! #328 (BL-ACC-04, `ACC.RUL.09`) point 3 with the real wl-notify: the
//! engagement is suspended on Friday 2026-11-06 and the notice bounces. The
//! client approver's contact point is marked invalid, the partner gets a task
//! (due Monday 2026-11-09) to deliver the notice another way, and no
//! acknowledgement reminder or partner task for want of one runs until the
//! proof of delivery is recorded; the clock then starts from the proof.

mod support;

use accounting::EngagementStatus;
use accounting::deadline_notices::{DEADLINE_NOTICE, DeadlineNotice, follow_up};
use renox::prelude::*;
use wl_kernel::notify::{NoticeStatus, NotifyChannel};
use wl_notify::{HardError, SendError, StatutoryNotice, notices};
use wl_parties::{ContactPoint, ContactStatus};
use wl_ticketing::Ticket;

use support::{boot_notify, d, dubai};

#[renox::test]
async fn a_bounced_notice_marks_the_contact_invalid_and_waits_for_the_proof() {
    let t = boot_notify().await;
    let (manager, partner) = (t.manager().await, t.partner().await);
    let client = t.client().await;
    let engagement = t
        .engagement_in(&client, EngagementStatus::Active, None)
        .await;
    t.portal_user("Omar", client.ubo, client.company).await;
    let point = t.email_for(client.ubo, "omar@gulf-trading.example").await;
    t.mail.then([Err(SendError::hard(
        HardError::HardBounce,
        "550",
        "mailbox does not exist",
    ))]);

    // Friday 2026-11-06: the suspension sends the notice, and it bounces.
    t.set_clock(6, 10, 0);
    t.act(
        &manager,
        &engagement,
        "suspend",
        &[("reason", "client_request")],
    )
    .await
    .assert_ok();
    t.app.run_all_jobs().await;
    // (the partner's task e-mail goes to the partner, not to the client)
    let to_client = || {
        t.mail
            .attempts()
            .into_iter()
            .filter(|mail| mail.address == "omar@gulf-trading.example")
            .collect::<Vec<_>>()
    };
    let attempts = to_client();
    assert_eq!(attempts.len(), 1, "{attempts:?}");
    assert_eq!(attempts[0].address, "omar@gulf-trading.example");
    assert_eq!(attempts[0].channel, NotifyChannel::Email);

    // The contact point is invalid.
    let point = ContactPoint::find(t.db(), point.id).await.unwrap().unwrap();
    assert_eq!(point.status, ContactStatus::Invalid);

    // The partner has one task, due Monday 2026-11-09, to deliver it another
    // way.
    let notice = DeadlineNotice::of_engagement(t.db(), engagement.id)
        .await
        .unwrap()
        .remove(0);
    let record = StatutoryNotice::query()
        .first(t.db())
        .await
        .unwrap()
        .expect("a statutory notice record");
    assert_eq!(record.type_key, DEADLINE_NOTICE);
    let tasks = Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(format!("notify.statutory:{}", record.id).as_str()))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].assignee_user_id, Some(partner.id));
    let late = (tasks[0].due_at.expect("a due time") - dubai(9, 10, 0))
        .num_seconds()
        .abs();
    assert!(late < 5, "due Monday 2026-11-09 10:00, off by {late}s");
    assert_eq!(
        notices::status(t.db(), &notice.subject(), DEADLINE_NOTICE)
            .await
            .unwrap(),
        NoticeStatus::NotDelivered
    );

    // No clock runs without a delivery: nothing on the 9th, 10th or 20th.
    for day in [9, 10, 20] {
        t.set_clock(day, 10, 0);
        let run = t.run(follow_up(t.state(), d(11, day))).await.unwrap();
        assert!(run.reminded.is_empty() && run.escalated.is_empty(), "{day}");
    }
    assert_eq!(to_client().len(), 1, "no reminder was sent");

    // The partner records the proof (a call on the 12th); the clock starts
    // from that day: nothing on the 14th, the reminder on the 15th.
    let clerk = t
        .user_with_grants("Clerk", &[wl_notify::permissions::RECORD_PROOF])
        .await;
    t.set_clock(12, 10, 0);
    t.app.acting_as(&clerk);
    t.app
        .htmx()
        .post_multipart(
            &format!("/statutory-notices/{}/proof", record.id),
            &[
                ("method", "call"),
                ("delivered_on", "2026-11-12"),
                ("note", "Phoned Omar, who confirmed he read it"),
            ],
            &[],
        )
        .await
        .assert_ok();
    assert!(matches!(
        notices::status(t.db(), &notice.subject(), DEADLINE_NOTICE)
            .await
            .unwrap(),
        NoticeStatus::ProofRecorded { .. }
    ));
    t.set_clock(14, 10, 0);
    let early = t.run(follow_up(t.state(), d(11, 14))).await.unwrap();
    assert!(early.reminded.is_empty());
    t.set_clock(15, 10, 0);
    let due = t.run(follow_up(t.state(), d(11, 15))).await.unwrap();
    assert_eq!(
        due.reminded,
        [notice.id],
        "the reminder runs from the proof"
    );
    assert!(due.escalated.is_empty());
}
