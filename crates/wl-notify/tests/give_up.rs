//! Falling back, giving up and the give-up rule `NTF.RUL.01` (wl-notify
//! §4.1, §5.4): the acceptance checks AC1, AC2, AC7, AC8 and AC10 of #55.

mod support;

use std::time::Duration;

use renox::db::Json;
use renox::prelude::*;
use wl_kernel::notify::{GiveUpReaction, MessageId, NotifyChannel, NotifyRecipient};
use wl_notify::give_up::{Attempt, RULE, RunOutcome, run_reaction};
use wl_notify::{
    MessageStatus, NotificationType, ReactionRun, RunGiveUpReaction, RunStatus, SendError,
};

use support::{BLUE_DUNES, BLUE_DUNES_EMAIL, BLUE_DUNES_MOBILE, Booted, REMINDER, boot};

use GiveUpReaction::{AlternativeRecipient, MarkContactInvalid, StaffTask};

/// Blue Dunes' e-mail fails for good (`550`), its SMS three times
/// (transient), with the backoff waited out between the SMS attempts.
/// Returns the e-mail and the SMS message.
async fn give_up_on_blue_dunes(t: &Booted) -> (MessageId, MessageId) {
    t.email
        .then([Err(SendError::permanent("550", "mailbox unknown"))]);
    t.sms
        .then((0..3).map(|_| Err(SendError::transient("504", "gateway timeout"))));
    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    assert_eq!(ids.len(), 1);
    let email = ids[0];

    // The e-mail fails and falls back; the SMS goes out at once and fails.
    t.run_jobs().await;
    let messages = t.messages().await;
    assert_eq!(messages.len(), 2, "the fallback queued an SMS");
    let sms = messages[1].message_id();
    assert_eq!(t.sms.sent().len(), 1);
    // Retried after 1 minute, then after 5 more.
    assert_eq!(t.after(Duration::from_secs(30)).await, 0);
    t.after(Duration::from_secs(30)).await;
    assert_eq!(t.sms.sent().len(), 2);
    assert_eq!(t.after(Duration::from_secs(4 * 60)).await, 0);
    t.after(Duration::from_secs(60)).await;
    assert_eq!(t.sms.sent().len(), 3);
    (email, sms)
}

/// AC1: the e-mail ends `abandoned` through `fall_back`, the SMS through
/// `give_up`, and one `MessageGaveUp` lists both channels with their last
/// errors; neither message reads as sent.
#[renox::test]
async fn the_last_channel_gives_up_and_the_event_lists_the_chain() {
    let t = boot().await;
    let (email_id, sms_id) = give_up_on_blue_dunes(&t).await;

    let email = t.message(email_id).await;
    assert_eq!(email.status, MessageStatus::Abandoned);
    assert_eq!(email.channel, NotifyChannel::Email);
    assert_eq!(email.address.as_deref(), Some(BLUE_DUNES_EMAIL.0));
    assert_eq!(email.attempts, 1);
    assert_eq!(email.last_error_code.as_deref(), Some("550"));
    assert_eq!(email.error_kind.as_deref(), Some("permanent"));
    assert_eq!(
        t.history(email_id).await.last().map(String::as_str),
        Some("fall_back")
    );

    let sms = t.message(sms_id).await;
    assert_eq!(sms.status, MessageStatus::Abandoned);
    assert_eq!(sms.channel, NotifyChannel::Sms);
    assert_eq!(sms.address.as_deref(), Some(BLUE_DUNES_MOBILE.0));
    assert_eq!(sms.contact_point_id, Some(BLUE_DUNES_MOBILE.1));
    assert_eq!(sms.attempts, 3);
    assert_eq!(sms.chain_id, email.chain_id);
    assert_eq!(sms.chain_id, Some(email.id));
    assert_eq!(sms.fallback_of, Some(email.id));
    assert_eq!(sms.rendered_body, "Invoice INV-PGP-2026-0007 is overdue.");
    assert_eq!(
        t.history(sms_id).await,
        ["fail", "retry", "fail", "retry", "fail", "give_up"]
    );
    for message in [&email, &sms] {
        assert_eq!(message.sent_at, None);
        assert_eq!(message.delivered_at, None);
        assert_eq!(message.provider_message_id, None);
    }

    let events = t.gave_up.all();
    assert_eq!(events.len(), 1, "one MessageGaveUp");
    let event = &events[0];
    assert_eq!(event.message, sms_id);
    assert_eq!(event.type_key, REMINDER);
    assert_eq!(event.recipient, NotifyRecipient::Party { id: BLUE_DUNES });
    assert_eq!(event.contact_point, Some(BLUE_DUNES_MOBILE.1));
    assert!(!event.statutory);
    assert_eq!(
        event.reactions,
        [MarkContactInvalid, AlternativeRecipient, StaffTask]
    );
    let tried: Vec<_> = event
        .channels_tried
        .iter()
        .map(|c| {
            (
                c.message,
                c.channel,
                c.last_error_code.as_deref(),
                c.last_error.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        tried,
        [
            (
                email_id,
                NotifyChannel::Email,
                Some("550"),
                Some("mailbox unknown")
            ),
            (
                sms_id,
                NotifyChannel::Sms,
                Some("504"),
                Some("gateway timeout")
            ),
        ]
    );

    // Four failures were told: the e-mail's (final), three on SMS.
    let finals: Vec<_> = t.failed.all().iter().map(|f| f.is_final).collect();
    assert_eq!(finals, [true, false, false, true]);
}

/// AC2: one run `succeeded` with the steps in run order, each handler
/// called once; the rule run again for the message is `skipped`; the
/// fallback ran nothing.
#[renox::test]
async fn the_reaction_runs_once_per_message() {
    let t = boot().await;
    let (email_id, sms_id) = give_up_on_blue_dunes(&t).await;

    let runs = ReactionRun::of_message(t.db(), sms_id.0).await.unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.rule, RULE);
    assert_eq!(run.status, RunStatus::Succeeded);
    assert_eq!(run.attempts, 1);
    assert!(run.finished_at.is_some());
    let steps: Vec<_> = run
        .steps(t.db())
        .await
        .unwrap()
        .iter()
        .map(|s| (s.position, s.reaction, s.status.clone()))
        .collect();
    assert_eq!(
        steps,
        [
            (1, MarkContactInvalid, "tested".to_owned()),
            (2, AlternativeRecipient, "tested".to_owned()),
            (3, StaffTask, "tested".to_owned()),
        ]
    );
    assert_eq!(
        t.calls(),
        [
            (MarkContactInvalid, sms_id),
            (AlternativeRecipient, sms_id),
            (StaffTask, sms_id)
        ]
    );
    let step = &run.steps(t.db()).await.unwrap()[2];
    assert_eq!(step.detail.0["staff_role"], "collector");
    assert_eq!(step.detail.0["channels_tried"].as_array().unwrap().len(), 2);

    // The job dispatched again: a `skipped` run, no handler, no step.
    t.app
        .state()
        .queue
        .dispatch(RunGiveUpReaction {
            message_id: sms_id.0,
        })
        .await
        .unwrap();
    t.run_jobs().await;
    let runs = ReactionRun::of_message(t.db(), sms_id.0).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, RunStatus::Succeeded);
    assert_eq!(runs[1].status, RunStatus::Skipped);
    assert!(runs[1].steps(t.db()).await.unwrap().is_empty());
    assert_eq!(t.calls().len(), 3);
    assert_eq!(runs[0].steps(t.db()).await.unwrap().len(), 3);

    // The e-mail fell back: no run, no event for it.
    assert!(
        ReactionRun::of_message(t.db(), email_id.0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.gave_up.all().iter().all(|e| e.message != email_id));
    // And the rule for the fallen-back message does nothing either.
    let outcome = run_reaction(
        t.app.state(),
        email_id.0,
        Attempt {
            job_id: 0,
            attempt: 1,
        },
    )
    .await
    .unwrap();
    assert!(matches!(outcome, RunOutcome::Skipped(_)));
    assert_eq!(t.calls().len(), 3);
}

/// AC7: a handler that fails on every attempt: the run is `failed` after 5
/// attempts and one `AutomationFailed` (not compliance-critical) is
/// emitted; the message stays `abandoned`.
#[renox::test]
async fn a_reaction_that_keeps_failing_ends_the_run_failed() {
    let t = boot().await;
    t.handlers_fail(true);
    let (_, sms_id) = give_up_on_blue_dunes(&t).await;
    // The first attempt ran with the give-up; the others after their backoff.
    t.app.run_all_jobs().await;

    let runs = ReactionRun::of_message(t.db(), sms_id.0).await.unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(run.attempts, 5);
    assert!(
        run.error
            .as_deref()
            .is_some_and(|e| e.contains("the task service is down")),
        "{:?}",
        run.error
    );
    assert!(run.steps(t.db()).await.unwrap().is_empty());
    // The first reaction was tried on each attempt.
    assert_eq!(t.calls(), vec![(MarkContactInvalid, sms_id); 5]);

    let failed = t.automation_failed.all();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].rule, "NTF.RUL.01");
    assert!(!failed[0].compliance_critical);
    assert_eq!(failed[0].subject.subject_id, sms_id.0);
    assert_eq!(t.message(sms_id).await.status, MessageStatus::Abandoned);
}

/// AC8: a requester module listening to `MessageGaveUp` gets the event, and
/// the default reaction still runs, once.
#[renox::test]
async fn a_listening_requester_does_not_replace_the_default_reaction() {
    let t = boot().await;
    let (_, sms_id) = give_up_on_blue_dunes(&t).await;
    // The test app listens to `MessageGaveUp` like a requester module.
    let heard: Vec<_> = t.gave_up.all().iter().map(|e| e.message).collect();
    assert_eq!(heard, [sms_id]);
    let runs = ReactionRun::of_message(t.db(), sms_id.0).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, RunStatus::Succeeded);
    assert_eq!(t.calls().len(), 3);
}

/// AC10 (module): the seeder installs the registered types; installing
/// again keeps the reaction the client changed.
#[renox::test]
async fn the_install_keeps_the_clients_reaction() {
    let t = boot().await;
    let mut reminder = NotificationType::by_key(t.db(), REMINDER)
        .await
        .unwrap()
        .expect("the seeder installed the type");
    assert_eq!(
        reminder.give_up.0,
        [AlternativeRecipient, MarkContactInvalid, StaffTask]
    );
    assert_eq!(reminder.staff_role.as_deref(), Some("collector"));
    let count = NotificationType::query().count(t.db()).await.unwrap();

    reminder.give_up = Json(vec![StaffTask]);
    reminder.save(t.db()).await.unwrap();
    t.app
        .kernel()
        .call("notify:install", Vec::<String>::new())
        .await
        .expect("notify:install runs");
    t.app.kernel().seed().await.unwrap();
    let again = NotificationType::by_key(t.db(), REMINDER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.give_up.0, [StaffTask]);
    assert_eq!(
        NotificationType::query().count(t.db()).await.unwrap(),
        count
    );

    // The client's reaction is the one that runs.
    let (_, sms_id) = give_up_on_blue_dunes(&t).await;
    assert_eq!(t.calls(), [(StaffTask, sms_id)]);
}
