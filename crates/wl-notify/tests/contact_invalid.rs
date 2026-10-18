//! Marking a contact point invalid after a hard failure (#58 AC1 to AC3,
//! AC5 and AC7), with a party directory that records the invalidations and
//! scripted providers. The real status, editing and page are tested in
//! wl-parties.

mod support;

use renox::db::now;
use wl_kernel::notify::{ContactPointInvalidated, MessageId, NotifyChannel, NotifyRecipient};
use wl_kernel::parties::{NoParties, PartyDirectory};
use wl_notify::channels::HardError;
use wl_notify::{MessageStatus, ReactionRun, ReactionStep, RunStatus, SendError};

use support::{
    BLUE_DUNES, BLUE_DUNES_EMAIL, BLUE_DUNES_MOBILE, Booted, INVOICE_ISSUED, RECEIPT, REMINDER,
    SAMI, boot_invalid,
};

fn bounce() -> SendError {
    SendError::hard(HardError::HardBounce, "550", "mailbox does not exist")
}

/// The first step (`mark_contact_invalid` runs first) of the only run of the
/// message `id`, after checking the run succeeded.
async fn mark_step(t: &Booted, id: MessageId) -> ReactionStep {
    let runs = ReactionRun::of_message(t.db(), id.0).await.unwrap();
    assert_eq!(runs.len(), 1, "one run");
    assert_eq!(runs[0].status, RunStatus::Succeeded);
    let steps = runs[0].steps(t.db()).await.unwrap();
    assert_eq!(steps[0].position, 1);
    steps.into_iter().next().unwrap()
}

/// AC1: a hard bounce on the only e-mail of an e-mail-only type marks the
/// point invalid with the give-up time as since, emits the event once and
/// records the step `marked`.
#[renox::test]
async fn a_hard_bounce_marks_the_contact_point_invalid() {
    let t = boot_invalid().await;
    t.email.then([Err(bounce())]);
    let ids = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    t.app.run_all_jobs().await;

    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Abandoned);
    assert_eq!(message.error_kind.as_deref(), Some("hard_bounce"));
    assert_eq!(message.contact_point_id, Some(BLUE_DUNES_EMAIL.1));

    let invalidations = t.directory.invalidations();
    assert_eq!(invalidations.len(), 1);
    assert_eq!(invalidations[0].point, BLUE_DUNES_EMAIL.1);
    assert_eq!(invalidations[0].reason, "hard_bounce");
    assert_eq!(Some(invalidations[0].since), message.updated_at);

    assert_eq!(
        t.invalidated.all(),
        [ContactPointInvalidated {
            contact_point: BLUE_DUNES_EMAIL.1,
            party: BLUE_DUNES,
            channel: NotifyChannel::Email,
            reason: "hard_bounce".into(),
        }]
    );
    let step = mark_step(&t, ids[0]).await;
    assert_eq!(step.status, "marked");
    assert_eq!(
        step.detail.0["marked"][0]["contact_point"],
        BLUE_DUNES_EMAIL.1
    );
    // The staff task reaction still ran after it.
    assert_eq!(t.calls().len(), 1);
}

/// AC2: a timeout on every attempt, and a permanent non-hard error, leave
/// the point valid: `no_hard_failure`, no event.
#[renox::test]
async fn soft_failures_mark_nothing() {
    let t = boot_invalid().await;
    t.email
        .then((0..3).map(|_| Err(SendError::transient("504", "gateway timeout"))));
    let timeout = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    t.app.run_all_jobs().await;
    assert_eq!(t.message(timeout[0]).await.status, MessageStatus::Abandoned);
    assert_eq!(mark_step(&t, timeout[0]).await.status, "no_hard_failure");

    t.email
        .then([Err(SendError::permanent("550", "mailbox unknown"))]);
    let permanent = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    t.app.run_all_jobs().await;
    let message = t.message(permanent[0]).await;
    assert_eq!(message.status, MessageStatus::Abandoned);
    assert_eq!(message.error_kind.as_deref(), Some("permanent"));
    assert_eq!(mark_step(&t, permanent[0]).await.status, "no_hard_failure");

    assert!(t.directory.invalidations().is_empty());
    assert!(t.invalidated.all().is_empty());
}

/// AC3: in an e-mail then SMS chain where the e-mail hard-bounces and the
/// SMS gives up, only the e-mail point becomes invalid.
#[renox::test]
async fn only_the_point_that_failed_hard_is_marked() {
    let t = boot_invalid().await;
    t.email.then([Err(bounce())]);
    t.sms
        .then((0..3).map(|_| Err(SendError::transient("504", "gateway timeout"))));
    t.notify_party(REMINDER, BLUE_DUNES).await;
    t.app.run_all_jobs().await;

    let messages = t.messages().await;
    assert_eq!(messages.len(), 2);
    let sms = messages[1].message_id();
    assert_eq!(messages[1].status, MessageStatus::Abandoned);
    assert_eq!(messages[1].contact_point_id, Some(BLUE_DUNES_MOBILE.1));

    let points: Vec<_> = t
        .directory
        .invalidations()
        .iter()
        .map(|i| i.point)
        .collect();
    assert_eq!(points, [BLUE_DUNES_EMAIL.1]);
    let events = t.invalidated.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].channel, NotifyChannel::Email);
    assert_eq!(mark_step(&t, sms).await.status, "marked");
}

/// AC3: a hard bounce that falls back to an SMS that is sent gives up
/// nothing, so nothing is marked.
#[renox::test]
async fn a_hard_bounce_with_a_sent_fallback_marks_nothing() {
    let t = boot_invalid().await;
    t.email.then([Err(bounce())]);
    t.notify_party(REMINDER, BLUE_DUNES).await;
    t.app.run_all_jobs().await;

    let statuses: Vec<_> = t.messages().await.iter().map(|m| m.status).collect();
    assert_eq!(statuses, [MessageStatus::Abandoned, MessageStatus::Sent]);
    assert!(t.gave_up.all().is_empty());
    assert!(t.directory.invalidations().is_empty());
    assert!(t.invalidated.all().is_empty());
}

/// AC5: an invalid only e-mail is skipped: an e-mail then SMS type goes by
/// SMS, an e-mail-only type is `suppressed` with `no_contact_point`.
#[renox::test]
async fn an_invalid_point_is_never_chosen() {
    let t = boot_invalid().await;
    let marked = t
        .directory
        .invalidate_contact(t.db(), BLUE_DUNES_EMAIL.1, now(), "hard_bounce")
        .await
        .unwrap();
    assert!(marked.is_some());

    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    assert_eq!(ids.len(), 1);
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.channel, NotifyChannel::Sms);
    assert_eq!(message.status, MessageStatus::Sent);
    assert_eq!(message.address.as_deref(), Some(BLUE_DUNES_MOBILE.0));
    assert!(t.email.sent().is_empty());

    let ids = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Suppressed);
    assert_eq!(message.suppress_reason.as_deref(), Some("no_contact_point"));
    t.run_jobs().await;
    assert!(t.email.sent().is_empty());
}

/// AC5: a message queued before its point became invalid is `suppressed`
/// with `invalid_contact_point` when its send job runs; the provider is not
/// called.
#[renox::test]
async fn a_queued_message_to_a_point_that_became_invalid_is_not_sent() {
    let t = boot_invalid().await;
    let ids = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Queued);

    t.directory
        .invalidate_contact(t.db(), BLUE_DUNES_EMAIL.1, now(), "hard_bounce")
        .await
        .unwrap();
    t.run_jobs().await;

    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Suppressed);
    assert_eq!(
        message.suppress_reason.as_deref(),
        Some("invalid_contact_point")
    );
    assert!(t.email.sent().is_empty(), "the provider is not called");
    let suppressed = t.suppressed.all();
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0].reason, "invalid_contact_point");
    assert_eq!(
        suppressed[0].recipient,
        NotifyRecipient::Party { id: BLUE_DUNES }
    );
}

/// AC7: two messages to the same point both bounce; the first run marks it,
/// the second records `already_invalid`, and there is one mark, one event.
#[renox::test]
async fn a_point_that_is_invalid_already_is_not_marked_again() {
    let t = boot_invalid().await;
    t.email.then([Err(bounce()), Err(bounce())]);
    let first = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    let second = t.notify_party(INVOICE_ISSUED, BLUE_DUNES).await;
    t.app.run_all_jobs().await;

    let mut outcomes = vec![
        mark_step(&t, first[0]).await.status,
        mark_step(&t, second[0]).await.status,
    ];
    outcomes.sort();
    assert_eq!(outcomes, ["already_invalid", "marked"]);
    assert_eq!(t.directory.invalidations().len(), 1);
    assert_eq!(t.invalidated.all().len(), 1);
}

/// AC7: with parties off the directory marks nothing (`None` for every
/// point), and the run still succeeds with `already_invalid`.
#[renox::test]
async fn a_directory_that_marks_nothing_does_not_fail_the_run() {
    let t = boot_invalid().await;
    assert_eq!(
        NoParties
            .invalidate_contact(t.db(), BLUE_DUNES_EMAIL.1, now(), "hard_bounce")
            .await
            .unwrap(),
        None
    );

    t.email.then([Err(bounce())]);
    // Sami's point is not one the test directory can mark.
    let ids = t.notify_party(INVOICE_ISSUED, SAMI).await;
    t.app.run_all_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Abandoned);
    assert_eq!(mark_step(&t, ids[0]).await.status, "already_invalid");
    assert!(t.invalidated.all().is_empty());
}

/// A type without the reaction marks nothing, however it failed.
#[renox::test]
async fn a_type_without_the_reaction_marks_nothing() {
    let t = boot_invalid().await;
    t.email.then([Err(bounce())]);
    t.notify_party(RECEIPT, BLUE_DUNES).await;
    t.app.run_all_jobs().await;
    assert!(t.directory.invalidations().is_empty());
}
