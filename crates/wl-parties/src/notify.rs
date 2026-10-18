//! The neutral on-hold message (wl-parties §5.6, #36 A10).
//!
//! `party.on_hold.neutral` says the account is on hold and asks the party to
//! contact the business. It never gives a reason. Nothing sends it
//! automatically: the decider of a review task chooses to, with the "Send
//! neutral message" action, and the task gets a note. The type is never
//! marked `announces_pause`, so it still reaches a blocked party.

use std::collections::BTreeSet;
use std::sync::Arc;

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::notify::{self, NotificationTypeDef, Notifier, NotifyChannel, NotifyRequest};
use wl_kernel::{PartyId, Subject};

use crate::party::Party;
use crate::review_tasks::ReviewTask;
use crate::screening_task::{ScreeningTask, dubai_label};

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

/// The notification type key of the possible-match e-mail to compliance.
pub const POSSIBLE_MATCH_TYPE: &str = "party.screening.possible_match";

/// The kind of the in-app notice of a possible match.
pub const POSSIBLE_MATCH_KIND: &str = "party-possible-match";

/// The possible-match type (#38 A7): a statutory e-mail to staff, English and
/// Arabic. It names the party code and the due time, never a candidate or a
/// list, and it is never sent to the party.
pub fn possible_match_type() -> NotificationTypeDef {
    NotificationTypeDef::new(POSSIBLE_MATCH_TYPE, "wl-parties")
        .group("party")
        .statutory(true)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("A possible sanctions match needs review"),
            "Party {{ party_code }} needs a compliance review by {{ due }} (Asia/Dubai).",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("تطابق محتمل في فحص العقوبات يحتاج إلى مراجعة"),
            "الطرف {{ party_code }} يحتاج إلى مراجعة من الامتثال قبل {{ due }} (توقيت دبي).",
        )
}

/// Registers the types with the kernel (`wl_kernel::notify::register_type`).
pub(crate) fn register() {
    notify::register_type(neutral_type());
    notify::register_type(possible_match_type());
}

/// The in-app bell entry of a possible match: the party code and the due
/// time, no candidate and no list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PossibleMatchNotice {
    task: i64,
    code: String,
    due: String,
}

impl Notification for PossibleMatchNotice {
    fn kind(&self) -> &'static str {
        POSSIBLE_MATCH_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, state: &AppState) -> Result<Value> {
        let lang = state.current_lang();
        let body = crate::texts::t_with(
            &lang,
            "wl_parties.screening.notice_body",
            &[("code", &self.code), ("due", &self.due)],
        );
        Ok(
            DatabaseMessage::info(crate::texts::t(&lang, "wl_parties.screening.notice_title"))
                .body(body)
                .with("screening_task", self.task)
                .into(),
        )
    }
}

/// Tells every holder of `parties.block_reason.view` (global scope) that
/// `task` of `party` needs a review: one bell entry each, and one e-mail each
/// when the `notify` module is on. Staff only; nothing goes to the party.
/// Returns the holders told, ordered by id.
pub async fn alert_possible_match(
    state: &AppState,
    party: &Party,
    task: &ScreeningTask,
) -> Result<Vec<i64>> {
    let holders = users_with_permission_in(
        &state.db,
        crate::permissions::BLOCK_REASON_VIEW,
        &Scope::global(),
    )
    .await?;
    let mut seen = BTreeSet::new();
    let holders: Vec<User> = holders.into_iter().filter(|u| seen.insert(u.id)).collect();
    let due = dubai_label(task.due_at);
    let notice = PossibleMatchNotice {
        task: task.id,
        code: party.code.clone(),
        due: due.clone(),
    };
    let mail = available(state);
    for user in &holders {
        state.notify(user, &notice).await?;
        if mail {
            let request = NotifyRequest::new(
                POSSIBLE_MATCH_TYPE,
                Subject::new("wl_parties.party", party.id),
            )
            .to_user(user.id)
            .subject_label(&party.code)
            .data(json!({ "party_code": party.code, "due": due }));
            notify::notifier(state).notify(state, request).await?;
        }
    }
    let mut ids: Vec<i64> = holders.iter().map(|u| u.id).collect();
    ids.sort_unstable();
    Ok(ids)
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
