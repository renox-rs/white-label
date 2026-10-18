//! The module's [`Notifier`] (wl-notify §6 `notify`): how other modules
//! ask for messages through the kernel. [`crate::WlNotify`] provides it
//! with [`wl_kernel::notify::provide_notifier`].
//!
//! `notify(type, subject, recipients, data)` stores one `queued` message
//! per recipient, routed and rendered ([`crate::recipients`],
//! [`crate::render`]), in one transaction. A recipient nothing can reach
//! gets a message moved straight to `suppressed`, and `MessageSuppressed`
//! follows the commit. A staff user also gets a bell entry (a Renox
//! database notification) with the type's text. Each queued message gets
//! its [`SendMessage`] job in the same transaction ([`crate::send`]).

use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::notify::{
    self, FALLBACK_KIND, MessageId, NotificationTypeDef, Notifier, NotifyChannel, NotifyFuture,
    NotifyRecipient, NotifyRequest,
};

use crate::machine::SUPPRESS;
use crate::message::{Message, MessageStatus};
use crate::recipients::{self, Resolved};
use crate::render::{Rendered, render};
use crate::send::SendMessage;
use crate::types::NotificationType;

/// Queues messages in `wl_notify_messages` for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct Messages;

impl Notifier for Messages {
    fn notify<'a>(
        &'a self,
        state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(queue(state, request))
    }
}

/// The channels a provider can send on now ([`crate::channels`]): e-mail
/// through Renox mail, SMS once an SMS gateway is provided.
pub fn open_channels(state: &AppState) -> Vec<NotifyChannel> {
    crate::channels::open_channels(state)
}

/// The type `key` as the client has it: the stored row, else the
/// registered definition (not installed yet). Fails (permanent) when the
/// type is neither stored nor registered.
pub async fn type_def(db: &Db, key: &str) -> Result<NotificationTypeDef> {
    if let Some(row) = NotificationType::by_key(db, key).await? {
        return Ok(row.to_def());
    }
    notify::notification_type(key)
        .map(|def| (*def).clone())
        .ok_or_else(|| {
            renox::Error::permanent_message(format!("notification type `{key}` is not registered"))
        })
}

/// `recipients` without repeats, in order.
fn distinct(recipients: &[NotifyRecipient]) -> Vec<&NotifyRecipient> {
    let mut seen = Vec::new();
    for recipient in recipients {
        if !seen.contains(&recipient) {
            seen.push(recipient);
        }
    }
    seen
}

/// The `queued` message of `def` about `subject` for `recipient`, routed
/// by `resolved` and rendered with `data`.
pub fn compose(
    def: &NotificationTypeDef,
    subject: &Subject,
    recipient: &NotifyRecipient,
    resolved: &Resolved,
    data: &Value,
) -> Message {
    let route = &resolved.route;
    let Rendered {
        subject: title,
        body,
    } = render(def, route.channel, &resolved.language, data);
    let mut message = Message {
        type_key: def.key.clone(),
        language: resolved.language.clone(),
        rendered_subject: title,
        rendered_body: body,
        data: Json(data.clone()),
        status: MessageStatus::Queued,
        ..Default::default()
    };
    message.set_subject(subject);
    message.set_recipient(recipient);
    message.channel = route.channel;
    message.address = route.address.clone();
    message.contact_point_id = route.contact_point;
    message
}

/// Stores the messages of `request` and returns their ids, in the order of
/// its (distinct) recipients.
pub async fn queue(state: &AppState, request: NotifyRequest) -> Result<Vec<MessageId>> {
    let def = type_def(&state.db, &request.type_key).await?;
    let open = open_channels(state);
    let recipients = distinct(&request.recipients);
    let mut planned = Vec::with_capacity(recipients.len());
    for recipient in recipients {
        let resolved = recipients::resolve(state, &def, recipient, &open, &[]).await?;
        planned.push((recipient, resolved));
    }

    let mut tx = state.db.begin().await?;
    let mut ids = Vec::with_capacity(planned.len());
    let mut outcomes = Vec::new();
    let mut bells = Vec::new();
    for (recipient, resolved) in &planned {
        let mut message = compose(&def, &request.subject, recipient, resolved, &request.data);
        message.insert(&mut tx).await?;
        // The first message of its chain; a fallback keeps this id.
        message.chain_id = Some(message.id);
        message.save(&mut tx).await?;
        if let Some(reason) = resolved.route.suppress {
            let outcome = message
                .transition(SUPPRESS)
                .by_system()
                .reason(reason)
                .run_in(&mut tx)
                .await
                .map_err(TransitionError::into_error)?;
            outcomes.push(outcome);
        } else {
            state
                .queue
                .dispatch_in(
                    &mut tx,
                    SendMessage {
                        message_id: message.id,
                    },
                )
                .await?;
        }
        if let NotifyRecipient::User { id } = recipient {
            bells.push((*id, BellNotice::of(&def, &message)));
        }
        ids.push(message.message_id());
    }
    tx.commit().await?;

    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    for (id, notice) in bells {
        if let Some(user) = User::find(&state.db, id).await? {
            state.notify(&user, &notice).await?;
        }
    }
    Ok(ids)
}

/// The bell entry a staff user gets for a message: its rendered title
/// (else the type key) and body. The kind is the kernel's, so the bell
/// shows it like the no-module fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BellNotice {
    /// The message.
    pub message: MessageId,
    /// The type key.
    pub type_key: String,
    /// The record it is about.
    pub subject: Subject,
    /// The title.
    pub title: String,
    /// The body.
    pub body: String,
}

impl BellNotice {
    /// The bell entry for `message` of `def`.
    pub fn of(def: &NotificationTypeDef, message: &Message) -> Self {
        BellNotice {
            message: message.message_id(),
            type_key: def.key.clone(),
            subject: message.subject(),
            title: message
                .rendered_subject
                .clone()
                .unwrap_or_else(|| def.key.clone()),
            body: message.rendered_body.clone(),
        }
    }
}

impl Notification for BellNotice {
    fn kind(&self) -> &'static str {
        FALLBACK_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, _state: &AppState) -> Result<Value> {
        let mut entry = DatabaseMessage::info(self.title.clone());
        if !self.body.is_empty() {
            entry = entry.body(self.body.clone());
        }
        Ok(entry
            .with("notification_type", &self.type_key)
            .with("message_id", self.message.0)
            .with("subject_type", &self.subject.subject_type)
            .with("subject_id", self.subject.subject_id)
            .into())
    }
}

#[cfg(test)]
mod tests {
    use renox::serde_json::json;
    use wl_kernel::PartyId;

    use super::*;
    use crate::message::{NO_CONTACT_POINT, RecipientKind};
    use crate::recipients::Route;

    fn reminder() -> NotificationTypeDef {
        NotificationTypeDef::new("invoice.reminder", "invoicing")
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }} is overdue"),
                "Please pay invoice {{ number }}.",
            )
    }

    #[test]
    fn a_message_is_composed_from_the_route_and_the_template() {
        let resolved = Resolved {
            language: "ar".into(),
            route: Route {
                channel: NotifyChannel::Email,
                address: Some("billing@bluedunes.ae".into()),
                contact_point: Some(11),
                suppress: None,
            },
        };
        let subject = Subject::new("invoicing.invoice", 9);
        let data = json!({ "number": "INV-0001" });
        let party = NotifyRecipient::Party { id: PartyId(3) };
        let message = compose(&reminder(), &subject, &party, &resolved, &data);
        assert_eq!(message.status, MessageStatus::Queued);
        assert_eq!(message.recipient_kind, RecipientKind::Party);
        assert_eq!(message.party_id, Some(3));
        assert_eq!(message.contact_point_id, Some(11));
        assert_eq!(message.address.as_deref(), Some("billing@bluedunes.ae"));
        assert_eq!(message.language, "ar");
        assert_eq!(
            message.rendered_subject.as_deref(),
            Some("Invoice INV-0001 is overdue")
        );
        assert_eq!(message.rendered_body, "Please pay invoice INV-0001.");
        assert_eq!(message.subject(), subject);
        assert_eq!(message.data.0, data);
        assert_eq!(message.attempts, 0);
        assert_eq!((message.sent_at, message.delivered_at), (None, None));
    }

    #[test]
    fn a_message_with_nowhere_to_go_has_no_address() {
        let resolved = Resolved {
            language: "en".into(),
            route: Route {
                channel: NotifyChannel::Email,
                address: None,
                contact_point: None,
                suppress: Some(NO_CONTACT_POINT),
            },
        };
        let user = NotifyRecipient::User { id: 5 };
        let message = compose(
            &reminder(),
            &Subject::new("invoicing.invoice", 9),
            &user,
            &resolved,
            &json!({}),
        );
        assert_eq!(message.user_id, Some(5));
        assert_eq!(message.address, None);
    }

    #[test]
    fn repeated_recipients_get_one_message() {
        let recipients = [
            NotifyRecipient::User { id: 1 },
            NotifyRecipient::Party { id: PartyId(2) },
            NotifyRecipient::User { id: 1 },
        ];
        assert_eq!(distinct(&recipients).len(), 2);
    }

    #[test]
    fn the_bell_entry_uses_the_rendered_text() {
        let message = Message {
            id: 8,
            subject_type: "invoicing.invoice".into(),
            subject_id: 9,
            rendered_subject: Some("Invoice INV-0001 is overdue".into()),
            rendered_body: "Please pay.".into(),
            ..Default::default()
        };
        let notice = BellNotice::of(&reminder(), &message);
        assert_eq!(notice.title, "Invoice INV-0001 is overdue");
        assert_eq!(notice.message, MessageId(8));
        assert_eq!(notice.kind(), FALLBACK_KIND);
        let bare = Message {
            rendered_subject: None,
            ..message
        };
        assert_eq!(BellNotice::of(&reminder(), &bare).title, "invoice.reminder");
    }
}
