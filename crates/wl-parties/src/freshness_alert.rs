//! The compliance alert of a stale sanctions list (wl-parties §3,
//! `PTY.RUL.02`, issue #40 A7, A8).
//!
//! [`SanctionsListStale`] is emitted once per stale period, after the list is
//! saved, so the alert is sent once per period too. It goes to every holder
//! of `parties.block_reason.view` (global scope), each user once: a bell
//! entry when the `notify` module is off, and the e-mail type
//! [`STALE_TYPE`] when it is on (its notifier then adds the bell entry of
//! the e-mail). Staff only; nothing goes to a party.

use std::collections::BTreeSet;

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::Subject;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::events::SanctionsListStale;
use crate::screening_task::dubai_label;

/// The notification type key of the stale list e-mail to compliance.
pub const STALE_TYPE: &str = "party.sanctions_list.stale";

/// The kind of the in-app notice of a stale list.
pub const STALE_KIND: &str = "sanctions-list-stale";

/// The subject type of a list in a notification.
pub const SUBJECT_TYPE: &str = "wl_parties.sanctions_list";

/// The stale list type: a statutory e-mail to staff, English and Arabic. It
/// names the list, its last successful update and the error.
pub fn stale_type() -> NotificationTypeDef {
    NotificationTypeDef::new(STALE_TYPE, "wl-parties")
        .group("party")
        .statutory(true)
        .channels(&[NotifyChannel::Email])
        .template(
            NotifyChannel::Email,
            "en",
            Some("A sanctions list is out of date"),
            "The sanctions list {{ list }} is out of date. Last successful update: {{ updated }} (Asia/Dubai). Reason: {{ reason }}.",
        )
        .template(
            NotifyChannel::Email,
            "ar",
            Some("قائمة العقوبات غير محدّثة"),
            "قائمة العقوبات {{ list }} غير محدّثة. آخر تحديث ناجح: {{ updated }} (توقيت دبي). السبب: {{ reason }}.",
        )
}

/// What the alert says, in English for the e-mail data.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Facts {
    list: String,
    updated: String,
    reason: String,
}

impl Facts {
    fn of(event: &SanctionsListStale) -> Self {
        Self {
            list: event.name.clone(),
            updated: event.last_updated_at.map_or_else(
                || crate::texts::english("wl_parties.freshness.never"),
                dubai_label,
            ),
            reason: event
                .last_error
                .clone()
                .unwrap_or_else(|| crate::texts::english("wl_parties.freshness.reason_old")),
        }
    }
}

/// The in-app bell entry of a stale list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StaleNotice {
    list: i64,
    facts: Facts,
}

impl Notification for StaleNotice {
    fn kind(&self) -> &'static str {
        STALE_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, state: &AppState) -> Result<Value> {
        let lang = state.current_lang();
        let params: [(&str, &dyn std::fmt::Display); 3] = [
            ("list", &self.facts.list),
            ("updated", &self.facts.updated),
            ("reason", &self.facts.reason),
        ];
        Ok(DatabaseMessage::info(crate::texts::t_with(
            &lang,
            "wl_parties.freshness.notice_title",
            &params,
        ))
        .body(crate::texts::t_with(
            &lang,
            "wl_parties.freshness.notice_body",
            &params,
        ))
        .with("sanctions_list", self.list)
        .into())
    }
}

/// Tells every holder of `parties.block_reason.view` (global scope) that the
/// list of `event` is stale. Returns the holders told, ordered by id.
pub async fn alert_stale(state: &AppState, event: &SanctionsListStale) -> Result<Vec<i64>> {
    let holders = users_with_permission_in(
        &state.db,
        crate::permissions::BLOCK_REASON_VIEW,
        &Scope::global(),
    )
    .await?;
    let mut seen = BTreeSet::new();
    let holders: Vec<User> = holders.into_iter().filter(|u| seen.insert(u.id)).collect();
    let facts = Facts::of(event);
    let notice = StaleNotice {
        list: event.list,
        facts: facts.clone(),
    };
    let mail = crate::notify::available(state);
    for user in &holders {
        if mail {
            let request = NotifyRequest::new(STALE_TYPE, Subject::new(SUBJECT_TYPE, event.list))
                .to_user(user.id)
                .subject_label(&facts.list)
                .data(json!({
                    "list": facts.list,
                    "updated": facts.updated,
                    "reason": facts.reason,
                }));
            notify::notifier(state).notify(state, request).await?;
        } else {
            state.notify(user, &notice).await?;
        }
    }
    let mut ids: Vec<i64> = holders.iter().map(|u| u.id).collect();
    ids.sort_unstable();
    Ok(ids)
}

/// Registers the type with the kernel.
pub(crate) fn register_type() {
    notify::register_type(stale_type());
}

/// Alerts compliance when `SanctionsListStale` arrives.
pub(crate) fn register(app: &mut Registry) {
    register_type();
    app.listen(|event: SanctionsListStale, state: AppState| async move {
        alert_stale(&state, &event).await.map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::block_effects::tests::{boot, boot_with};
    use renox::testing::TestApp;
    use wl_kernel::notify::Notifier;

    /// A notifier that records its requests and answers one id each.
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

    struct WithNotifier(Recording);

    impl Module for WithNotifier {
        fn name(&self) -> &'static str {
            "test-notifier"
        }

        fn register(&self, app: &mut Registry) {
            notify::provide_notifier(app, self.0.clone());
        }
    }

    fn stale_event(error: Option<&str>) -> SanctionsListStale {
        SanctionsListStale {
            list: 7,
            name: "UN Consolidated List".into(),
            last_updated_at: None,
            last_error: error.map(str::to_owned),
        }
    }

    async fn user(app: &TestApp, name: &str, permission: &str, role: &str) -> User {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(app.db(), name, &email, "password123")
            .await
            .unwrap();
        renox::auth::permissions::define_role(app.db(), role, &[permission])
            .await
            .unwrap();
        user.assign_role_in(app.db(), role, &Scope::global())
            .await
            .unwrap();
        user
    }

    #[test]
    fn the_type_has_english_and_arabic_and_is_statutory() {
        let def = stale_type();
        for language in ["en", "ar"] {
            assert!(
                def.templates
                    .iter()
                    .any(|t| t.channel == NotifyChannel::Email && t.language == language)
            );
        }
        assert!(def.statutory);
    }

    #[test]
    fn the_facts_say_never_and_the_error_or_age() {
        let never = Facts::of(&stale_event(Some("timeout")));
        assert_eq!(never.updated, "never");
        assert_eq!(never.reason, "timeout");
        assert_eq!(
            Facts::of(&stale_event(None)).reason,
            "The update is too old"
        );
    }

    #[renox::test]
    async fn without_notify_each_holder_gets_one_bell_entry_and_others_none() {
        let (app, _party, _views) = boot().await;
        let view = crate::permissions::BLOCK_REASON_VIEW;
        let mlro = user(&app, "Mlro", view, "mlro").await;
        let deputy = user(&app, "Deputy", view, "deputy").await;
        let clerk = user(&app, "Clerk", "parties.view", "clerk").await;
        let told = alert_stale(app.state(), &stale_event(Some("timeout")))
            .await
            .unwrap();
        assert_eq!(told, vec![mlro.id, deputy.id]);
        for holder in [&mlro, &deputy] {
            let bells = holder.unread_notifications(app.db()).await.unwrap();
            assert_eq!(bells.len(), 1);
            assert!(bells[0].data.to_string().contains("UN Consolidated List"));
        }
        assert!(
            clerk
                .unread_notifications(app.db())
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[renox::test]
    async fn a_user_with_two_roles_is_told_once() {
        let (app, _party, _views) = boot().await;
        let view = crate::permissions::BLOCK_REASON_VIEW;
        let both = user(&app, "Both", view, "mlro").await;
        renox::auth::permissions::define_role(app.db(), "deputy", &[view])
            .await
            .unwrap();
        both.assign_role_in(app.db(), "deputy", &Scope::global())
            .await
            .unwrap();
        let told = alert_stale(app.state(), &stale_event(None)).await.unwrap();
        assert_eq!(told, vec![both.id]);
        assert_eq!(both.unread_notifications(app.db()).await.unwrap().len(), 1);
    }

    #[renox::test]
    async fn with_notify_each_holder_gets_one_email_request_and_no_own_bell() {
        let recording = Recording::default();
        let seen = recording.clone();
        let (app, _party, _views) = boot_with(|app| app.module(WithNotifier(recording))).await;
        register_type();
        let view = crate::permissions::BLOCK_REASON_VIEW;
        let mlro = user(&app, "Mlro", view, "mlro").await;
        let deputy = user(&app, "Deputy", view, "deputy").await;
        let clerk = user(&app, "Clerk", "parties.view", "clerk").await;
        alert_stale(app.state(), &stale_event(Some("timeout")))
            .await
            .unwrap();
        let requests = seen.0.lock().unwrap().clone();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|r| r.type_key == STALE_TYPE));
        assert_eq!(requests[0].data["list"], "UN Consolidated List");
        assert_eq!(requests[0].data["reason"], "timeout");
        for u in [&mlro, &deputy, &clerk] {
            assert!(u.unread_notifications(app.db()).await.unwrap().is_empty());
        }
    }

    #[renox::test]
    async fn the_event_alerts_through_the_listener() {
        let (app, _party, _views) = boot().await;
        let mlro = user(&app, "Mlro", crate::permissions::BLOCK_REASON_VIEW, "mlro").await;
        app.state().emit(stale_event(None)).await.unwrap();
        assert_eq!(mlro.unread_notifications(app.db()).await.unwrap().len(), 1);
    }
}
