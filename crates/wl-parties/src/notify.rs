//! The neutral on-hold message (wl-parties §5.6, #36 A10).
//!
//! `party.on_hold.neutral` says the account is on hold and asks the party to
//! contact the business. It never gives a reason. Nothing sends it
//! automatically: the decider of a review task chooses to, with the "Send
//! neutral message" action, and the task gets a note. The type is never
//! marked `announces_pause`, so it still reaches a blocked party.

use std::sync::Arc;

use renox::prelude::*;
use wl_kernel::notify::{self, NotificationTypeDef, Notifier, NotifyChannel, NotifyRequest};
use wl_kernel::{PartyId, Subject};

use crate::review_tasks::ReviewTask;

/// The notification type key of the neutral message.
pub const NEUTRAL_TYPE: &str = "party.on_hold.neutral";

/// The type's definition: e-mail and SMS, English and Arabic, no reason.
pub fn neutral_type() -> NotificationTypeDef {
    NotificationTypeDef::new(NEUTRAL_TYPE, "wl-parties")
        .group("party")
        .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
        .template(
            NotifyChannel::Email,
            "en",
            Some("Your account is on hold"),
            "Your account is on hold for now. Please contact us to find out more.",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("حسابك معلّق"),
            "حسابك معلّق في الوقت الحالي. يُرجى التواصل معنا لمعرفة المزيد.",
        )
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            "Your account is on hold for now. Please contact us.",
        )
        .template(
            NotifyChannel::Sms,
            "ar",
            None,
            "حسابك معلّق حاليًا. يُرجى التواصل معنا.",
        )
}

/// Registers the type with the kernel (`wl_kernel::notify::register_type`).
pub(crate) fn register() {
    notify::register_type(neutral_type());
}

/// Whether the `notify` module is on, so the action can be offered.
pub fn available(state: &AppState) -> bool {
    state.provided::<Arc<dyn Notifier>>().is_some()
}

/// Queues the neutral message to the task's party for `user` and adds the
/// task note. Returns whether a message was queued; `false` (nothing done)
/// without the `notify` module.
pub async fn send_neutral(state: &AppState, user: i64, task: &ReviewTask) -> Result<bool> {
    if !available(state) {
        return Ok(false);
    }
    let request = NotifyRequest::new(
        NEUTRAL_TYPE,
        Subject::new("wl_parties.party", task.party_id),
    )
    .to_party(PartyId(task.party_id));
    let ids = notify::notifier(state).notify(state, request).await?;
    if ids.is_empty() {
        return Ok(false);
    }
    task.add_note(
        &state.db,
        Some(user),
        &crate::texts::english("wl_parties.review.neutral_note"),
    )
    .await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::block_effects::tests::{boot, boot_with};

    #[test]
    fn the_neutral_type_has_english_and_arabic_and_no_reason() {
        let def = neutral_type();
        for channel in [NotifyChannel::Email, NotifyChannel::Sms] {
            for language in ["en", "ar"] {
                let template = def
                    .templates
                    .iter()
                    .find(|t| t.channel == channel && t.language == language)
                    .expect("a template");
                let text = format!("{:?} {}", template.subject, template.body).to_lowercase();
                assert!(!text.contains("sanction") && !text.contains("{{"));
            }
        }
        assert!(!wl_kernel::block_effects::announces_pause(NEUTRAL_TYPE));
    }

    /// A notifier that records its requests and answers one message id each.
    #[derive(Clone, Default)]
    struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

    impl Notifier for Recording {
        fn notify<'a>(
            &'a self,
            _state: &'a AppState,
            request: NotifyRequest,
        ) -> notify::NotifyFuture<'a, Vec<notify::MessageId>> {
            let count = request.recipients.len() as i64;
            self.0.lock().unwrap().push(request);
            Box::pin(async move { Ok((1..=count).map(notify::MessageId).collect()) })
        }
    }

    /// A module that provides the recording notifier, as `wl-notify` does.
    struct WithNotifier(Recording);

    impl Module for WithNotifier {
        fn name(&self) -> &'static str {
            "test-notifier"
        }

        fn register(&self, app: &mut Registry) {
            notify::provide_notifier(app, self.0.clone());
        }
    }

    async fn task_for(app: &renox::testing::TestApp, party: i64) -> ReviewTask {
        let mut task = ReviewTask::new(party, "booking", None, Some("clinic_manager"), "on_hold");
        task.insert(app.db()).await.unwrap();
        task
    }

    #[renox::test]
    async fn the_neutral_message_is_queued_once_with_a_task_note() {
        let recording = Recording::default();
        let seen = recording.clone();
        let (app, party, _views) = boot_with(|app| app.module(WithNotifier(recording))).await;
        register();
        assert!(available(app.state()));
        let task = task_for(&app, party.id).await;
        assert!(send_neutral(app.state(), 5, &task).await.unwrap());

        let requests = seen.0.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].type_key, NEUTRAL_TYPE);
        assert_eq!(requests[0].recipients.len(), 1);
        // No reason code travels with the message.
        assert_eq!(requests[0].data, renox::serde_json::json!({}));
        let notes = task.notes(app.db()).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].by_user, Some(5));
    }

    #[renox::test]
    async fn without_notify_nothing_is_sent_and_no_note_is_added() {
        let (app, party, _views) = boot().await;
        assert!(!available(app.state()));
        let task = task_for(&app, party.id).await;
        assert!(!send_neutral(app.state(), 5, &task).await.unwrap());
        assert!(task.notes(app.db()).await.unwrap().is_empty());
    }

    #[test]
    fn registering_makes_the_type_known() {
        register();
        let def = notify::notification_type(NEUTRAL_TYPE).expect("registered");
        assert_eq!(def.key, NEUTRAL_TYPE);
    }
}
