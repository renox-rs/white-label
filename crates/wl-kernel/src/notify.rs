//! The notice contract (ARCHITECTURE §5, wl-notify §6): the one way every
//! module sends a notification, without depending on the `wl-notify` crate.
//!
//! A module declares its notification types once with [`register_type`] in
//! `register` (a process-wide registry by type key, like the approvals
//! machine registry), then asks for a message with
//! `notifier(&state).notify(&state, request)`:
//!
//! ```ignore
//! use wl_kernel::notify::{self, NotifyRequest};
//!
//! let request = NotifyRequest::new("invoice.reminder", invoice.subject())
//!     .to_party(invoice.party())
//!     .to_user(collector.id)
//!     .data(json!({ "number": invoice.number }));
//! let ids = notify::notifier(&state).notify(&state, request).await?;
//! ```
//!
//! `wl-notify` provides the real [`Notifier`] with [`provide_notifier`] and
//! emits [`MessageFailed`], [`MessageSuppressed`] and [`MessageGaveUp`].
//! Without it, [`notifier`] gives [`NoNotifier`]: staff users still get a
//! Renox database notification (the bell), party and address recipients get
//! nothing, and no message id is returned.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::{Channel, DatabaseMessage, Notification, Recipient};
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::subject::Subject;

/// The kind of the bell entry [`NoNotifier`] gives a staff user.
pub const FALLBACK_KIND: &str = "wl-notification";

/// The channel a message goes out on.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum NotifyChannel {
    /// E-mail.
    #[default]
    Email,
    /// Text message.
    Sms,
    /// WhatsApp.
    Whatsapp,
}

impl NotifyChannel {
    /// The kind of contact point that holds this channel's address.
    pub fn contact_kind(self) -> ContactKind {
        match self {
            NotifyChannel::Email => ContactKind::Email,
            NotifyChannel::Sms | NotifyChannel::Whatsapp => ContactKind::Mobile,
        }
    }
}

/// What a notification type is for (wl-notify §3); decides the consent and
/// opt-out rules.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Category {
    /// Part of a business transaction: invoice issued, receipt.
    #[default]
    Transactional,
    /// A reminder: invoice overdue, renewal due.
    Reminder,
    /// Account security: login codes, password changes.
    Security,
    /// Marketing; needs consent.
    Marketing,
}

/// How urgent a notification type is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Urgency {
    /// Respects quiet hours.
    #[default]
    Normal,
    /// Goes out at once.
    Urgent,
}

/// What happens when a message gives up (wl-notify §5.4, `NTF.RUL.01`).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GiveUpReaction {
    /// A task for the type's staff role.
    StaffTask,
    /// Try another contact of the party.
    AlternativeRecipient,
    /// Mark the contact point that failed `invalid`.
    MarkContactInvalid,
    /// The statutory-notice task (deliver it another way, record proof).
    StatutoryNotice,
    /// Nothing.
    #[default]
    None,
}

/// The order `reactions` run in: `mark_contact_invalid` first, so an
/// alternative recipient never reuses the invalid contact, then the others
/// in the given order; each once, `none` left out.
pub fn reaction_order(reactions: &[GiveUpReaction]) -> Vec<GiveUpReaction> {
    let mut ordered = Vec::new();
    if reactions.contains(&GiveUpReaction::MarkContactInvalid) {
        ordered.push(GiveUpReaction::MarkContactInvalid);
    }
    for &reaction in reactions {
        if reaction != GiveUpReaction::None && !ordered.contains(&reaction) {
            ordered.push(reaction);
        }
    }
    ordered
}

/// The default text of a notification type for one channel and language.
/// `{{ name }}` in the subject or body is replaced with the request data's
/// field `name` ([`fill`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateDef {
    /// The channel.
    pub channel: NotifyChannel,
    /// The language, `en` or `ar`.
    pub language: String,
    /// The subject (e-mail) or title.
    pub subject: Option<String>,
    /// The body.
    pub body: String,
}

/// A kind of message a module can send (wl-notify §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationTypeDef {
    /// The key, e.g. `invoice.reminder`.
    pub key: String,
    /// The module that sends it, e.g. `invoicing`.
    pub module: String,
    /// The row of the wl-notify §5.4 default table, e.g. `invoices`.
    pub group: String,
    /// What it is for.
    pub category: Category,
    /// How urgent it is.
    pub urgency: Urgency,
    /// Whether it is a notice with legal or contractual effect.
    pub statutory: bool,
    /// What happens on give-up, in order (see [`reaction_order`]).
    pub give_up: Vec<GiveUpReaction>,
    /// The staff role told on give-up, e.g. `collector`.
    pub staff_role: Option<String>,
    /// The channels to try, in order.
    pub channels: Vec<NotifyChannel>,
    /// The default texts per channel and language.
    pub templates: Vec<TemplateDef>,
}

impl NotificationTypeDef {
    /// A transactional, normal, non-statutory type `key` of `module`, by
    /// e-mail, with no give-up reaction; the group is the key's first part.
    pub fn new(key: &str, module: &str) -> Self {
        NotificationTypeDef {
            key: key.to_owned(),
            module: module.to_owned(),
            group: key.split('.').next().unwrap_or_default().to_owned(),
            category: Category::Transactional,
            urgency: Urgency::Normal,
            statutory: false,
            give_up: Vec::new(),
            staff_role: None,
            channels: vec![NotifyChannel::Email],
            templates: Vec::new(),
        }
    }

    /// With this group.
    pub fn group(mut self, group: &str) -> Self {
        self.group = group.to_owned();
        self
    }

    /// With this category.
    pub fn category(mut self, category: Category) -> Self {
        self.category = category;
        self
    }

    /// With this urgency.
    pub fn urgency(mut self, urgency: Urgency) -> Self {
        self.urgency = urgency;
        self
    }

    /// Marked statutory or not.
    pub fn statutory(mut self, statutory: bool) -> Self {
        self.statutory = statutory;
        self
    }

    /// With these give-up reactions.
    pub fn give_up(mut self, reactions: &[GiveUpReaction]) -> Self {
        self.give_up = reactions.to_vec();
        self
    }

    /// Telling this staff role on give-up.
    pub fn staff_role(mut self, role: &str) -> Self {
        self.staff_role = Some(role.to_owned());
        self
    }

    /// Trying these channels, in order.
    pub fn channels(mut self, channels: &[NotifyChannel]) -> Self {
        self.channels = channels.to_vec();
        self
    }

    /// With a default text for `channel` in `language`.
    pub fn template(
        mut self,
        channel: NotifyChannel,
        language: &str,
        subject: Option<&str>,
        body: &str,
    ) -> Self {
        self.templates.push(TemplateDef {
            channel,
            language: language.to_owned(),
            subject: subject.map(str::to_owned),
            body: body.to_owned(),
        });
        self
    }

    /// The text for `channel` in `language`, falling back to English, then
    /// to any text in `language`, then in English.
    pub fn template_for(&self, channel: NotifyChannel, language: &str) -> Option<&TemplateDef> {
        let find = |channel: Option<NotifyChannel>, language: &str| {
            self.templates.iter().find(|t| {
                t.language == language && channel.is_none_or(|channel| t.channel == channel)
            })
        };
        find(Some(channel), language)
            .or_else(|| find(Some(channel), "en"))
            .or_else(|| find(None, language))
            .or_else(|| find(None, "en"))
    }
}

/// `template` with each `{{ name }}` replaced by the string, number or
/// boolean `data[name]`; unknown names become empty.
pub fn fill(template: &str, data: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let Some(end) = rest[start..].find("}}") else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = rest[start + 2..start + end].trim();
        match data.get(name) {
            Some(Value::String(text)) => out.push_str(text),
            Some(Value::Null) | None => {}
            Some(other) => out.push_str(&other.to_string()),
        }
        rest = &rest[start + end + 2..];
    }
    out.push_str(rest);
    out
}

static TYPES: RwLock<BTreeMap<String, Arc<NotificationTypeDef>>> = RwLock::new(BTreeMap::new());

/// Adds `def` to the process-wide registry of notification types; modules
/// call it in `register`. Registering the same key again replaces it.
pub fn register_type(def: NotificationTypeDef) {
    TYPES
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(def.key.clone(), Arc::new(def));
}

/// The registered type `key`, if any.
pub fn notification_type(key: &str) -> Option<Arc<NotificationTypeDef>> {
    TYPES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .cloned()
}

/// The registered types, by key.
pub fn notification_types() -> Vec<Arc<NotificationTypeDef>> {
    TYPES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .values()
        .cloned()
        .collect()
}

/// Who a notification is for.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifyRecipient {
    /// A staff (or portal) user, by user id.
    User {
        /// The user's id.
        id: i64,
    },
    /// A party, reached through its contact points ([`PartyContacts`]).
    Party {
        /// The party.
        id: PartyId,
    },
    /// A bare address on one channel.
    Address {
        /// The channel.
        channel: NotifyChannel,
        /// The e-mail address or E.164 number.
        address: String,
    },
}

/// One notification of a registered type about a record, to its recipients:
/// `notify(type, subject, recipients, data)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotifyRequest {
    /// The type key, e.g. `invoice.reminder`.
    pub type_key: String,
    /// The record it is about.
    pub subject: Subject,
    /// Who gets it; one message per recipient.
    pub recipients: Vec<NotifyRecipient>,
    /// The template variables.
    pub data: Value,
}

impl NotifyRequest {
    /// A notification of `type_key` about `subject`, to nobody yet.
    pub fn new(type_key: &str, subject: Subject) -> Self {
        NotifyRequest {
            type_key: type_key.to_owned(),
            subject,
            recipients: Vec::new(),
            data: Value::Object(Default::default()),
        }
    }

    /// Also to `recipient`.
    pub fn to(mut self, recipient: NotifyRecipient) -> Self {
        self.recipients.push(recipient);
        self
    }

    /// Also to the user `id`.
    pub fn to_user(self, id: i64) -> Self {
        self.to(NotifyRecipient::User { id })
    }

    /// Also to `party`.
    pub fn to_party(self, party: PartyId) -> Self {
        self.to(NotifyRecipient::Party { id: party })
    }

    /// With these template variables.
    pub fn data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }
}

/// The id of a message `wl-notify` queued.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MessageId(pub i64);

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The future a [`Notifier`] method returns.
pub type NotifyFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Sends notifications for other modules. `wl-notify` provides it; without
/// that module, [`notifier`] gives [`NoNotifier`].
pub trait Notifier: Send + Sync + 'static {
    /// Queues one message per recipient of `request` and returns their ids.
    /// Fails when the type is not registered.
    fn notify<'a>(
        &'a self,
        state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>>;
}

/// The notifier when the `notify` module is off. Each user recipient gets a
/// Renox database notification (the bell) with the type's text; party and
/// address recipients get nothing. No message is stored, so no id is
/// returned and no message event is emitted.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoNotifier;

impl Notifier for NoNotifier {
    fn notify<'a>(
        &'a self,
        state: &'a AppState,
        request: NotifyRequest,
    ) -> NotifyFuture<'a, Vec<MessageId>> {
        Box::pin(async move {
            let def = registered(&request.type_key)?;
            let lang = state.current_lang();
            for id in bell_users(&request.recipients) {
                let Some(user) = User::find(&state.db, id).await? else {
                    continue;
                };
                let notice = FallbackNotice::of(&def, &request, &lang.locale);
                state.notify(&user, &notice).await?;
            }
            Ok(Vec::new())
        })
    }
}

/// The registered type `key`, or an error naming it.
fn registered(key: &str) -> Result<Arc<NotificationTypeDef>> {
    notification_type(key).ok_or_else(|| {
        renox::Error::permanent_message(format!("notification type `{key}` is not registered"))
    })
}

/// The user ids among `recipients`, each once, in order: who [`NoNotifier`]
/// reaches.
fn bell_users(recipients: &[NotifyRecipient]) -> Vec<i64> {
    let mut ids = Vec::new();
    for recipient in recipients {
        if let NotifyRecipient::User { id } = recipient
            && !ids.contains(id)
        {
            ids.push(*id);
        }
    }
    ids
}

/// The bell entry [`NoNotifier`] gives a user: the type's e-mail text (else
/// any) in the current language, filled with the request data.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FallbackNotice {
    type_key: String,
    subject: Subject,
    title: String,
    body: String,
}

impl FallbackNotice {
    fn of(def: &NotificationTypeDef, request: &NotifyRequest, language: &str) -> Self {
        let template = def.template_for(NotifyChannel::Email, language);
        let title = template
            .and_then(|t| t.subject.as_deref())
            .map(|subject| fill(subject, &request.data))
            .unwrap_or_else(|| def.key.clone());
        let body = template
            .map(|t| fill(&t.body, &request.data))
            .unwrap_or_default();
        FallbackNotice {
            type_key: def.key.clone(),
            subject: request.subject.clone(),
            title,
            body,
        }
    }
}

impl Notification for FallbackNotice {
    fn kind(&self) -> &'static str {
        FALLBACK_KIND
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        vec![Channel::Database]
    }

    fn to_database(&self, _to: &Recipient, _state: &AppState) -> Result<Value> {
        let mut message = DatabaseMessage::info(self.title.clone());
        if !self.body.is_empty() {
            message = message.body(self.body.clone());
        }
        Ok(message
            .with("notification_type", &self.type_key)
            .with("subject_type", &self.subject.subject_type)
            .with("subject_id", self.subject.subject_id)
            .into())
    }
}

/// Makes `notifier` the app's [`Notifier`]; `wl-notify` calls it in
/// `register`.
pub fn provide_notifier(app: &mut Registry, notifier: impl Notifier) {
    let notifier: Arc<dyn Notifier> = Arc::new(notifier);
    app.provide(notifier);
}

/// The app's [`Notifier`], or [`NoNotifier`] when none is provided.
pub fn notifier(state: &AppState) -> Arc<dyn Notifier> {
    match state.provided::<Arc<dyn Notifier>>() {
        Some(notifier) => Arc::clone(&notifier),
        None => Arc::new(NoNotifier),
    }
}

/// The kind of a party's contact point that a channel can use.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContactKind {
    /// An e-mail address.
    #[default]
    Email,
    /// A mobile number (SMS, WhatsApp).
    Mobile,
}

/// One contact point of a party a message can go to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyContactPoint {
    /// The contact point's id in `wl-parties`.
    pub id: i64,
    /// E-mail or mobile.
    pub kind: ContactKind,
    /// The e-mail address or number.
    pub address: String,
    /// Whether it is the party's primary point of its kind.
    pub primary: bool,
    /// Whether it was verified.
    pub verified: bool,
}

/// How to reach a party: [`crate::parties::PartyDirectory::contacts`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyContacts {
    /// The party.
    pub party: PartyId,
    /// The language to write in, `en` or `ar`.
    pub language: String,
    /// The channel the party prefers, if set.
    pub preferred_channel: Option<NotifyChannel>,
    /// Its e-mail and mobile contact points.
    pub points: Vec<PartyContactPoint>,
}

impl PartyContacts {
    /// The point `channel` goes to: the primary one of its kind, else the
    /// first.
    pub fn point_for(&self, channel: NotifyChannel) -> Option<&PartyContactPoint> {
        let kind = channel.contact_kind();
        let mut of_kind = self.points.iter().filter(|p| p.kind == kind);
        let first = of_kind.clone().next();
        of_kind.find(|p| p.primary).or(first)
    }
}

/// The last error of one channel a message chain tried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelTried {
    /// The message on that channel.
    pub message: MessageId,
    /// The channel.
    pub channel: NotifyChannel,
    /// The provider's error code, e.g. `550`.
    pub last_error_code: Option<String>,
    /// The error text.
    pub last_error: Option<String>,
}

/// A send or delivery failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageFailed {
    /// The message.
    pub message: MessageId,
    /// The type key.
    pub type_key: String,
    /// The record it is about.
    pub subject: Subject,
    /// The channel.
    pub channel: NotifyChannel,
    /// The provider's error code.
    pub error_code: Option<String>,
    /// Why it failed.
    pub reason: String,
    /// Whether no retry follows on this channel.
    pub is_final: bool,
}

impl Event for MessageFailed {}

/// A message was not sent by rule (no contact point, suppressed address).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSuppressed {
    /// The message.
    pub message: MessageId,
    /// The type key.
    pub type_key: String,
    /// The record it is about.
    pub subject: Subject,
    /// Who it was for.
    pub recipient: NotifyRecipient,
    /// Why, e.g. `no_contact_point`, `suppressed`.
    pub reason: String,
}

impl Event for MessageSuppressed {}

/// A message ended `abandoned` after all retries and fallbacks (`give_up`).
/// The type's give-up reaction runs whether or not anyone listens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageGaveUp {
    /// The last message of the chain.
    pub message: MessageId,
    /// The type key.
    pub type_key: String,
    /// The record it is about.
    pub subject: Subject,
    /// Who it was for.
    pub recipient: NotifyRecipient,
    /// The party contact point of the last message, if any.
    pub contact_point: Option<i64>,
    /// Every channel of the chain, in order, with its last error.
    pub channels_tried: Vec<ChannelTried>,
    /// Whether the type is statutory.
    pub statutory: bool,
    /// The reactions applied, in run order.
    pub reactions: Vec<GiveUpReaction>,
}

impl Event for MessageGaveUp {}

#[cfg(test)]
mod tests {
    use renox::serde_json::json;

    use super::*;
    use GiveUpReaction::{AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice};

    #[test]
    fn mark_contact_invalid_runs_first_then_the_type_order() {
        assert_eq!(
            reaction_order(&[AlternativeRecipient, MarkContactInvalid, StaffTask]),
            [MarkContactInvalid, AlternativeRecipient, StaffTask]
        );
        assert_eq!(
            reaction_order(&[StatutoryNotice, MarkContactInvalid]),
            [MarkContactInvalid, StatutoryNotice]
        );
        assert_eq!(reaction_order(&[StaffTask]), [StaffTask]);
    }

    #[test]
    fn none_and_repeats_run_no_step() {
        assert!(reaction_order(&[GiveUpReaction::None]).is_empty());
        assert!(reaction_order(&[]).is_empty());
        assert_eq!(
            reaction_order(&[
                StaffTask,
                GiveUpReaction::None,
                StaffTask,
                MarkContactInvalid
            ]),
            [MarkContactInvalid, StaffTask]
        );
    }

    #[test]
    fn reactions_and_channels_are_stored_in_snake_case() {
        let names: Vec<_> = GiveUpReaction::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(
            names,
            [
                "staff_task",
                "alternative_recipient",
                "mark_contact_invalid",
                "statutory_notice",
                "none"
            ]
        );
        assert_eq!(NotifyChannel::Sms.as_str(), "sms");
        assert_eq!(NotifyChannel::Sms.contact_kind(), ContactKind::Mobile);
        assert_eq!(NotifyChannel::Email.contact_kind(), ContactKind::Email);
    }

    #[test]
    fn placeholders_are_filled_from_the_data() {
        let data = json!({ "number": "INV-0001", "days": 5, "none": null });
        assert_eq!(
            fill(
                "Invoice {{ number }} is {{days}} days late{{ none }}{{ x }}.",
                &data
            ),
            "Invoice INV-0001 is 5 days late."
        );
        assert_eq!(fill("No end {{ number", &data), "No end {{ number");
    }

    fn reminder() -> NotificationTypeDef {
        NotificationTypeDef::new("invoice.reminder", "invoicing")
            .category(Category::Reminder)
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .give_up(&[AlternativeRecipient, MarkContactInvalid, StaffTask])
            .staff_role("collector")
            .template(
                NotifyChannel::Email,
                "en",
                Some("Invoice {{ number }} is overdue"),
                "Please pay invoice {{ number }}.",
            )
            .template(
                NotifyChannel::Sms,
                "en",
                None,
                "Invoice {{ number }} overdue",
            )
    }

    #[test]
    fn a_type_has_defaults_and_finds_its_template() {
        let def = reminder();
        assert_eq!(def.group, "invoice");
        assert_eq!(def.urgency, Urgency::Normal);
        assert!(!def.statutory);
        let sms = def.template_for(NotifyChannel::Sms, "ar").unwrap();
        assert_eq!(
            (sms.channel, sms.language.as_str()),
            (NotifyChannel::Sms, "en")
        );
        let whatsapp = def.template_for(NotifyChannel::Whatsapp, "en").unwrap();
        assert_eq!(whatsapp.channel, NotifyChannel::Email);
    }

    #[test]
    fn registering_a_key_again_replaces_it() {
        register_type(NotificationTypeDef::new(
            "kernel_test.replaced",
            "kernel_test",
        ));
        register_type(
            NotificationTypeDef::new("kernel_test.replaced", "kernel_test").statutory(true),
        );
        assert!(notification_type("kernel_test.replaced").unwrap().statutory);
        let entries = notification_types()
            .iter()
            .filter(|t| t.key == "kernel_test.replaced")
            .count();
        assert_eq!(entries, 1);
        assert!(notification_type("kernel_test.missing").is_none());
        assert!(registered("kernel_test.missing").is_err());
    }

    #[test]
    fn the_fallback_reaches_staff_users_only() {
        let request = NotifyRequest::new("invoice.reminder", Subject::new("invoicing.invoice", 9))
            .to_party(PartyId(3))
            .to_user(7)
            .to(NotifyRecipient::Address {
                channel: NotifyChannel::Email,
                address: "a@example.com".into(),
            })
            .to_user(8)
            .to_user(7);
        assert_eq!(bell_users(&request.recipients), [7, 8]);
        let request = NotifyRequest::new("invoice.reminder", Subject::new("invoicing.invoice", 9))
            .to_party(PartyId(3));
        assert!(bell_users(&request.recipients).is_empty());
    }

    #[test]
    fn the_fallback_notice_uses_the_types_text() {
        let request = NotifyRequest::new("invoice.reminder", Subject::new("invoicing.invoice", 9))
            .to_user(7)
            .data(json!({ "number": "INV-0001" }));
        let notice = FallbackNotice::of(&reminder(), &request, "ar");
        assert_eq!(notice.title, "Invoice INV-0001 is overdue");
        assert_eq!(notice.body, "Please pay invoice INV-0001.");
        assert_eq!(notice.kind(), FALLBACK_KIND);
        assert_eq!(
            notice.channels(&Recipient::default()),
            vec![Channel::Database]
        );
        let bare = NotificationTypeDef::new("lease.signed", "property");
        let notice = FallbackNotice::of(&bare, &request, "en");
        assert_eq!(
            (notice.title.as_str(), notice.body.as_str()),
            ("lease.signed", "")
        );
    }

    #[test]
    fn a_channel_goes_to_the_primary_point_of_its_kind() {
        let point = |id, kind, primary| PartyContactPoint {
            id,
            kind,
            address: format!("address-{id}"),
            primary,
            verified: false,
        };
        let contacts = PartyContacts {
            party: PartyId(1),
            language: "en".into(),
            preferred_channel: None,
            points: vec![
                point(1, ContactKind::Email, false),
                point(2, ContactKind::Email, true),
                point(3, ContactKind::Mobile, false),
            ],
        };
        assert_eq!(contacts.point_for(NotifyChannel::Email).unwrap().id, 2);
        assert_eq!(contacts.point_for(NotifyChannel::Sms).unwrap().id, 3);
        assert_eq!(contacts.point_for(NotifyChannel::Whatsapp).unwrap().id, 3);
        let none = PartyContacts {
            points: Vec::new(),
            ..contacts
        };
        assert!(none.point_for(NotifyChannel::Email).is_none());
    }

    #[test]
    fn recipients_are_tagged_in_json() {
        let json =
            renox::serde_json::to_string(&NotifyRecipient::Party { id: PartyId(4) }).unwrap();
        assert_eq!(json, r#"{"kind":"party","id":4}"#);
        let json = renox::serde_json::to_string(&NotifyRecipient::User { id: 2 }).unwrap();
        assert_eq!(json, r#"{"kind":"user","id":2}"#);
    }
}
