//! The `statutory_notice` give-up reaction (wl-notify §5.4): the handler
//! side of #59 (AC1, AC5, AC6). A recording task creator stands for
//! wl-ticketing and Sara M. is the recipient.

mod support;

use renox::prelude::Model;
use wl_kernel::notify::{GiveUpReaction, MessageGaveUp, NoticeStatus, Urgency};
use wl_kernel::tickets::{Assignee, TASK_CATEGORY};
use wl_kernel::{Subject, calendar};
use wl_notify::reactions::DEFERRED;
use wl_notify::staff_task::{NO_TASK_CREATOR, TASK_CREATED};
use wl_notify::statutory_notice::{NOT_STATUTORY, StatutoryNoticeHandler, TASK_OPEN};
use wl_notify::{GiveUpContext, GiveUpHandler, Message, ReactionRun, RunStatus, StatutoryNotice};

use support::{
    COORDINATOR, SARA_EMAIL, SARA_MOBILE, SECRET_BODY, SECRET_SUBJECT, TICKET_NOT_STATUTORY,
    TICKET_NOTICE, TICKET_NOTICE_URGENT, TOWER_A_TICKET, boot_statutory,
    boot_statutory_without_tickets, tower_a,
};

const ALERT: &str = "notify-give-up-alert";

fn ticket() -> Subject {
    Subject::new("ticketing.ticket", TOWER_A_TICKET)
}

async fn record_of(t: &support::Booted, event: &MessageGaveUp) -> Option<StatutoryNotice> {
    let message = Message::find(t.db(), event.message.0)
        .await
        .unwrap()
        .expect("the message");
    StatutoryNotice::by_chain(t.db(), message.chain())
        .await
        .unwrap()
}

async fn steps(t: &support::Booted, event: &MessageGaveUp) -> Vec<wl_notify::ReactionStep> {
    let runs = ReactionRun::of_message(t.db(), event.message.0)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Succeeded);
    runs[0].steps(t.db()).await.unwrap()
}

/// AC1 and AC6: a statutory give-up keeps a record, opens one task due one
/// working day later that lists the masked channels and none of the
/// message, defers the type's `staff_task`, and the notice stays not
/// delivered.
#[renox::test]
async fn a_statutory_give_up_keeps_a_record_and_opens_one_task() {
    let t = boot_statutory().await;
    let aisha = t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_NOTICE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;

    let record = record_of(&t, &event).await.expect("a notice record");
    assert_eq!(record.type_key, TICKET_NOTICE);
    assert_eq!(record.subject(), ticket());
    assert_eq!(record.ticket_id, Some(1));
    assert!(!record.has_proof());

    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 1, "one task, not two");
    let task = &opened[0];
    assert_eq!(
        task.title,
        "Deliver TKT-2026-0042 notice to Sara M. by another means"
    );
    assert_eq!(task.category_key, TASK_CATEGORY);
    assert_eq!(task.assignee, Some(Assignee::user(aisha.id)));
    assert_eq!(
        task.source_key.as_deref(),
        Some(format!("notify.statutory:{}", record.id).as_str())
    );
    let due = calendar::calendar(t.app.state()).add_working_days(record.gave_up_at, 1);
    assert_eq!(task.due_at, Some(due));
    for line in [
        "- Email s***@example.com: 550 mailbox unknown",
        "- SMS ****4567: 504 timeout",
        "Accepted proof of delivery: courier, registered mail, hand delivery, call",
        &format!("Record the proof: /statutory-notices/{}/proof", record.id),
    ] {
        assert!(task.description.contains(line), "{}", task.description);
    }
    for text in [&task.title, &task.description] {
        for secret in [SECRET_SUBJECT, SECRET_BODY, SARA_EMAIL, SARA_MOBILE] {
            assert!(!text.contains(secret), "`{secret}` leaked into: {text}");
        }
    }

    let steps = steps(&t, &event).await;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].reaction, GiveUpReaction::StatutoryNotice);
    assert_eq!(steps[0].status, TASK_CREATED);
    assert_eq!(steps[0].detail.0["notice_id"], record.id);
    assert_eq!(steps[1].reaction, GiveUpReaction::StaffTask);
    assert_eq!(steps[1].status, DEFERRED);

    let status = wl_notify::notices::status(t.db(), &ticket(), TICKET_NOTICE)
        .await
        .unwrap();
    assert_eq!(status, NoticeStatus::NotDelivered);
    let undelivered = wl_notify::notices::undelivered(t.db(), &ticket())
        .await
        .unwrap();
    assert_eq!(undelivered.len(), 1);
    assert_eq!(undelivered[0].notice, record.id);
}

/// An urgent statutory type also alerts the holder.
#[renox::test]
async fn an_urgent_statutory_notice_alerts_the_holder() {
    let t = boot_statutory().await;
    let aisha = t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    t.give_up_on_sara(TICKET_NOTICE_URGENT, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    assert_eq!(t.tasks.opened().len(), 1);
    assert_eq!(t.notices(&aisha, ALERT).await.len(), 1);
}

/// AC5: a type that is not statutory makes no record and no task.
#[renox::test]
async fn a_type_that_is_not_statutory_makes_nothing() {
    let t = boot_statutory().await;
    t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_NOT_STATUTORY, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    assert!(record_of(&t, &event).await.is_none());
    assert!(t.tasks.opened().is_empty());
    let steps = steps(&t, &event).await;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].status, NOT_STATUTORY);
}

/// AC6: a retried run finds the record and the open task; it adds neither.
#[renox::test]
async fn a_retried_run_adds_no_second_record_or_task() {
    let t = boot_statutory().await;
    t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_NOTICE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    let record = record_of(&t, &event).await.unwrap();
    let message = Message::find(t.db(), event.message.0)
        .await
        .unwrap()
        .unwrap();

    let ctx = GiveUpContext {
        run_id: 1,
        message: event.message,
        type_key: event.type_key.clone(),
        subject: ticket(),
        subject_label: event.subject_label.clone(),
        recipient: event.recipient.clone(),
        gave_up_at: record.gave_up_at,
        contact_point: event.contact_point,
        channels_tried: event.channels_tried.clone(),
        statutory: true,
        staff_role: Some(COORDINATOR.to_owned()),
        urgency: Urgency::Normal,
        reactions: vec![GiveUpReaction::StatutoryNotice],
    };
    let step = StatutoryNoticeHandler
        .run(t.app.state(), &ctx)
        .await
        .unwrap();
    assert_eq!(step.status, TASK_OPEN);
    assert_eq!(step.detail["notice_id"], record.id);
    assert_eq!(t.tasks.opened().len(), 1);
    assert!(t.tasks.comments().is_empty(), "no comment either");
    let all = StatutoryNotice::of_subject(t.db(), &ticket())
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].chain_id, message.chain());
}

/// AC6: with ticketing off the record exists, the step says
/// `no_task_creator` and the run succeeds.
#[renox::test]
async fn without_ticketing_the_record_is_kept_and_the_run_succeeds() {
    let t = boot_statutory_without_tickets().await;
    t.holder("Aisha", COORDINATOR, Some(&tower_a())).await;
    let event = t
        .give_up_on_sara(TICKET_NOTICE, TOWER_A_TICKET, "TKT-2026-0042")
        .await;
    assert!(record_of(&t, &event).await.is_some());
    assert!(t.tasks.opened().is_empty());
    let steps = steps(&t, &event).await;
    assert_eq!(steps[0].status, NO_TASK_CREATOR);
    assert_eq!(steps[1].status, DEFERRED);
}
