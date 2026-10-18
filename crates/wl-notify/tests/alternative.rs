//! The `alternative_recipient` give-up reaction (wl-notify §5.4): the
//! acceptance checks AC1 to AC6 of #57. The test directory knows the
//! relationships of Blue Dunes LLC, two students and a few other companies
//! (`support`); the providers are scripted; the staff task is the module's
//! own, with a recording task creator behind it.

mod support;

use wl_kernel::notify::{
    self, ContactPurpose, GiveUpReaction, MessageId, NotifyChannel, NotifyRequest,
};
use wl_kernel::{PartyId, Subject};
use wl_notify::alternative::{ALTERNATIVE_ALREADY_TRIED, NO_RULE, NONE_LEFT, QUEUED};
use wl_notify::give_up::{Attempt, RunOutcome, run_reaction};
use wl_notify::staff_task::TASK_CREATED;
use wl_notify::{Message, ReactionRun, ReactionStep, SendError, SuppressionReason};

use support::{
    ABSENCE, ALI, ALI_EMAIL, ALI_MOBILE, BLUE_DUNES, Booted, FIRST_GUARDIAN, LONE_STAR, MIXED_BAG,
    MONA, MONA_MOBILE, NADIA, OMAR, PAIR_STUDENT, RASHID, REMINDER, SAMI, SAMI_EMAIL,
    SECOND_GUARDIAN, SOLO_GUARDIAN, SOLO_STUDENT, TALA, ZAID, boot_alternative,
};

/// Queues a `type_key` message about invoice 7 to the rule `purpose` of `of`.
async fn send_rule(t: &Booted, type_key: &str, of: PartyId, purpose: ContactPurpose) -> MessageId {
    let request = NotifyRequest::new(type_key, Subject::new("invoicing.invoice", 7))
        .to_contact(of, purpose)
        .data(renox::serde_json::json!({ "number": "INV-PGP-2026-0007" }));
    let state = t.app.state();
    let ids = t
        .app
        .at_travelled_time(notify::notifier(state).notify(state, request))
        .await
        .expect("the notifier queues the message");
    assert_eq!(ids.len(), 1);
    ids[0]
}

fn finance() -> ContactPurpose {
    ContactPurpose::contact_of("finance")
}

/// Scripts `emails` hard failures of the e-mail provider and `smss` of the
/// SMS provider, in turn.
fn fail(t: &Booted, emails: usize, smss: usize) {
    t.email
        .then((0..emails).map(|_| Err(SendError::permanent("550", "mailbox unknown"))));
    t.sms
        .then((0..smss).map(|_| Err(SendError::permanent("504", "timeout"))));
}

/// The steps of the one run of message `id`.
async fn steps(t: &Booted, id: MessageId) -> Vec<ReactionStep> {
    let runs = ReactionRun::of_message(t.db(), id.0).await.unwrap();
    assert_eq!(runs.len(), 1, "one run for message {id}");
    runs[0].steps(t.db()).await.unwrap()
}

fn step_of(steps: &[ReactionStep], reaction: GiveUpReaction) -> &ReactionStep {
    steps
        .iter()
        .find(|s| s.reaction == reaction)
        .unwrap_or_else(|| panic!("no step for {reaction:?}"))
}

/// The messages that carry an `alternative_of`.
fn alternatives(all: &[Message]) -> Vec<&Message> {
    all.iter().filter(|m| m.alternative_of.is_some()).collect()
}

/// AC1 and AC5: Ali's e-mail and SMS give up; exactly one message is queued
/// to Mona, the same message in her language on her only channel; the
/// staff task is deferred and no task is opened.
#[renox::test]
async fn the_alternative_goes_to_the_next_finance_contact() {
    let t = boot_alternative().await;
    fail(&t, 1, 1);
    let first = send_rule(&t, REMINDER, BLUE_DUNES, finance()).await;
    t.app.run_all_jobs().await;

    let messages = t.messages().await;
    assert_eq!(messages.len(), 3, "Ali's e-mail and SMS, Mona's SMS");
    assert_eq!(messages[0].message_id(), first);
    assert_eq!(messages[0].party_id, Some(ALI.0));
    assert_eq!(messages[0].address.as_deref(), Some(ALI_EMAIL));
    assert_eq!(messages[1].party_id, Some(ALI.0));
    assert_eq!(messages[1].address.as_deref(), Some(ALI_MOBILE));
    let alternative = alternatives(&messages);
    assert_eq!(alternative.len(), 1, "exactly one alternative");
    let mona = alternative[0];
    assert_eq!(mona.party_id, Some(MONA.0));
    assert_eq!(mona.alternative_of, Some(first.0));
    assert_eq!(mona.type_key, REMINDER);
    assert_eq!(mona.subject(), Subject::new("invoicing.invoice", 7));
    assert_eq!(mona.data.0["number"], "INV-PGP-2026-0007");
    assert_eq!(mona.contact_of_party_id, Some(BLUE_DUNES.0));
    assert_eq!(mona.contact_position.as_deref(), Some("finance"));
    // AC5: Arabic, by SMS (she has no e-mail).
    assert_eq!(mona.channel, NotifyChannel::Sms);
    assert_eq!(mona.address.as_deref(), Some(MONA_MOBILE));
    assert_eq!(mona.rendered_body, "الفاتورة INV-PGP-2026-0007 متأخرة.");
    assert_eq!(t.email.sent().len(), 1, "no e-mail went to Mona");
    assert_eq!(t.sms.sent().len(), 2);
    assert_eq!(t.sms.sent()[1].address, MONA_MOBILE);

    let steps = steps(&t, messages[1].message_id()).await;
    let alt = step_of(&steps, GiveUpReaction::AlternativeRecipient);
    assert_eq!(alt.status, QUEUED);
    assert_eq!(alt.detail.0["message_id"], mona.id);
    assert_eq!(alt.detail.0["party_id"], MONA.0);
    assert_eq!(
        step_of(&steps, GiveUpReaction::StaffTask).status,
        "deferred"
    );
    assert!(t.tasks.opened().is_empty(), "no task is opened");
}

/// AC2: Mona's message gives up too: the run records
/// `alternative_already_tried`, the collector gets one task that lists both
/// recipients' channels with masked addresses, nothing else is queued, and
/// dispatching either run again changes nothing.
#[renox::test]
async fn the_alternative_gives_up_and_one_task_covers_the_chain() {
    let t = boot_alternative().await;
    fail(&t, 1, 2);
    send_rule(&t, REMINDER, BLUE_DUNES, finance()).await;
    t.app.run_all_jobs().await;

    let messages = t.messages().await;
    assert_eq!(messages.len(), 3, "no third message");
    let parties: Vec<_> = messages.iter().map(|m| m.party_id).collect();
    assert_eq!(parties, [Some(ALI.0), Some(ALI.0), Some(MONA.0)]);
    for never in [OMAR, ZAID] {
        assert!(!parties.contains(&Some(never.0)));
    }

    let ali_sms = messages[1].message_id();
    let mona_sms = messages[2].message_id();
    let ali_steps = steps(&t, ali_sms).await;
    assert_eq!(
        step_of(&ali_steps, GiveUpReaction::AlternativeRecipient).status,
        QUEUED
    );
    let mona_steps = steps(&t, mona_sms).await;
    assert_eq!(
        step_of(&mona_steps, GiveUpReaction::AlternativeRecipient).status,
        ALTERNATIVE_ALREADY_TRIED
    );
    assert_eq!(
        step_of(&mona_steps, GiveUpReaction::StaffTask).status,
        TASK_CREATED
    );

    let opened = t.tasks.opened();
    assert_eq!(opened.len(), 1, "one task for the chain");
    let text = format!("{}\n{}", opened[0].title, opened[0].description);
    assert!(text.contains("Mona"), "{text}");
    assert!(text.contains("Ali"), "{text}");
    assert!(text.contains("550 mailbox unknown"), "{text}");
    assert!(text.contains("504 timeout"), "{text}");
    for secret in [
        ALI_EMAIL,
        ALI_MOBILE,
        MONA_MOBILE,
        "Please pay",
        "is overdue",
    ] {
        assert!(!text.contains(secret), "`{secret}` leaked into: {text}");
    }

    // Dispatching either run again queues and opens nothing.
    for id in [ali_sms, mona_sms] {
        let attempt = Attempt {
            job_id: 99,
            attempt: 1,
        };
        let again = run_reaction(t.app.state(), id.0, attempt).await.unwrap();
        assert!(matches!(again, RunOutcome::Skipped(_)));
    }
    t.app.run_all_jobs().await;
    assert_eq!(t.messages().await.len(), 3);
    assert_eq!(t.tasks.opened().len(), 1);
    assert!(t.tasks.comments().is_empty());
}

/// AC3: a student with one guardian has nobody left: the task opens at once
/// (the type has no `staff_task` step of its own) and nothing is queued;
/// with two guardians the second, not the primary, gets the alternative.
#[renox::test]
async fn guardians_have_an_alternative_only_when_there_is_another() {
    let t = boot_alternative().await;
    fail(&t, 1, 1);
    send_rule(&t, ABSENCE, SOLO_STUDENT, ContactPurpose::guardian_of()).await;
    t.app.run_all_jobs().await;
    let messages = t.messages().await;
    assert_eq!(messages.len(), 2, "e-mail and SMS only");
    assert!(alternatives(&messages).is_empty());
    let solo = steps(&t, messages[1].message_id()).await;
    assert_eq!(solo.len(), 1);
    assert_eq!(solo[0].status, NONE_LEFT);
    assert_eq!(
        solo[0].detail.0["tried"],
        renox::serde_json::json!([SOLO_GUARDIAN.0])
    );
    assert_eq!(solo[0].detail.0["staff_task"]["status"], TASK_CREATED);
    assert_eq!(t.tasks.opened().len(), 1);

    fail(&t, 2, 2);
    send_rule(&t, ABSENCE, PAIR_STUDENT, ContactPurpose::guardian_of()).await;
    t.app.run_all_jobs().await;
    let messages = t.messages().await;
    let alternative = alternatives(&messages);
    // The alternative's e-mail, then its SMS fall-back; all to the second.
    assert_eq!(alternative.len(), 2);
    assert!(
        alternative
            .iter()
            .all(|m| m.party_id == Some(SECOND_GUARDIAN.0))
    );
    assert_eq!(alternative[1].fallback_of, Some(alternative[0].id));
    assert!(
        messages
            .iter()
            .any(|m| m.party_id == Some(FIRST_GUARDIAN.0) && m.alternative_of.is_none())
    );
    // The alternative gave up too: one more task, for the second chain.
    assert_eq!(t.tasks.opened().len(), 2);
}

/// AC4: a contact whose only address is on the suppression list is skipped
/// and the next one left is chosen; with nobody left, `none_left` and a
/// task. Contacts of another company or of another position are never used.
#[renox::test]
async fn unreachable_contacts_are_skipped() {
    let t = boot_alternative().await;
    let mut tx = t.db().begin().await.unwrap();
    wl_notify::suppressions::add(
        &mut tx,
        NotifyChannel::Email,
        SAMI_EMAIL,
        SuppressionReason::HardBounce,
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Tala is primary; Sami (suppressed) is skipped; Nadia is chosen.
    fail(&t, 1, 1);
    send_rule(&t, REMINDER, MIXED_BAG, finance()).await;
    t.app.run_all_jobs().await;
    let messages = t.messages().await;
    let parties: Vec<_> = messages.iter().map(|m| m.party_id).collect();
    assert_eq!(parties[0], Some(TALA.0));
    let alternative = alternatives(&messages);
    assert_eq!(alternative.len(), 1);
    assert_eq!(alternative[0].party_id, Some(NADIA.0));
    assert!(!parties.contains(&Some(SAMI.0)));
    assert!(t.tasks.opened().is_empty());

    // Rashid is primary; Sami is the only other one and is skipped.
    let before = t.messages().await.len();
    fail(&t, 1, 1);
    send_rule(&t, REMINDER, LONE_STAR, finance()).await;
    t.app.run_all_jobs().await;
    let messages = t.messages().await;
    let ours = &messages[before..];
    assert_eq!(ours.len(), 2);
    assert!(ours.iter().all(|m| m.party_id == Some(RASHID.0)));
    let last = steps(&t, ours[1].message_id()).await;
    let alt = step_of(&last, GiveUpReaction::AlternativeRecipient);
    assert_eq!(alt.status, NONE_LEFT);
    assert_eq!(
        alt.detail.0["unreachable"],
        renox::serde_json::json!([SAMI.0])
    );
    assert_eq!(
        step_of(&last, GiveUpReaction::StaffTask).status,
        TASK_CREATED
    );
    assert_eq!(t.tasks.opened().len(), 1);
}

/// AC6: a give-up of a message to a plain `Party` recipient records
/// `no_rule`, and the type's own `staff_task` step opens the task.
#[renox::test]
async fn a_plain_party_has_no_rule_to_rerun() {
    let t = boot_alternative().await;
    fail(&t, 1, 1);
    let ids = t.notify_party(REMINDER, BLUE_DUNES).await;
    assert_eq!(ids.len(), 1);
    t.app.run_all_jobs().await;
    let messages = t.messages().await;
    assert_eq!(messages.len(), 2);
    let steps = steps(&t, messages[1].message_id()).await;
    let alt = step_of(&steps, GiveUpReaction::AlternativeRecipient);
    assert_eq!(alt.status, NO_RULE);
    assert_eq!(
        step_of(&steps, GiveUpReaction::StaffTask).status,
        TASK_CREATED
    );
    assert_eq!(t.tasks.opened().len(), 1);
}

/// AC7 (module side): a rule that finds nobody (a party nobody is related
/// to, as with parties off: the kernel's default directory is empty) gives a
/// `suppressed` message with reason `no_contact`, never a give-up.
#[renox::test]
async fn a_rule_that_finds_nobody_is_suppressed_as_no_contact() {
    let t = boot_alternative().await;
    let id = send_rule(&t, REMINDER, support::QUIET_SANDS, finance()).await;
    let message = t.message(id).await;
    assert_eq!(message.status, wl_notify::MessageStatus::Suppressed);
    assert_eq!(message.suppress_reason.as_deref(), Some("no_contact"));
    assert_eq!(message.party_id, None);
    let suppressed = t.suppressed.all();
    assert_eq!(suppressed.len(), 1);
    assert_eq!(suppressed[0].reason, "no_contact");
    t.app.run_all_jobs().await;
    assert!(t.gave_up.all().is_empty());
    assert!(t.tasks.opened().is_empty());
}
