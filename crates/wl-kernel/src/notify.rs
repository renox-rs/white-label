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
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use crate::broadcast::{BroadcastId, BroadcastRequest, broadcasts_off};
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

/// How staff delivered a statutory notice another way (wl-notify §5.4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProofMethod {
    /// By courier.
    #[default]
    Courier,
    /// By registered mail.
    RegisteredMail,
    /// Handed over in person.
    HandDelivery,
    /// By a phone call.
    Call,
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

/// A give-up definition the rules refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveUpConfigError {
    /// `mark_contact_invalid` is the only reaction of a non-marketing type:
    /// staff would never hear that the contact needs fixing.
    MarkContactInvalidAlone,
}

impl std::fmt::Display for GiveUpConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GiveUpConfigError::MarkContactInvalidAlone => f.write_str(
                "Mark contact invalid must be combined with another reaction for this type; add Staff task.",
            ),
        }
    }
}

impl std::error::Error for GiveUpConfigError {}

/// Checks the give-up `reactions` of a type of `category`: without `none`,
/// `mark_contact_invalid` alone is refused unless the category is
/// `marketing` (wl-notify §5.4). Any other combination passes.
pub fn check_give_up(
    category: Category,
    reactions: &[GiveUpReaction],
) -> Result<(), GiveUpConfigError> {
    let mut real = reactions.iter().filter(|r| **r != GiveUpReaction::None);
    let only_invalid =
        real.clone().next().is_some() && real.all(|r| *r == GiveUpReaction::MarkContactInvalid);
    if only_invalid && category != Category::Marketing {
        return Err(GiveUpConfigError::MarkContactInvalidAlone);
    }
    Ok(())
}

/// Group key: invoices.
pub const GROUP_INVOICES: &str = "invoices";
/// Group key: statements.
pub const GROUP_STATEMENTS: &str = "statements";
/// Group key: payments.
pub const GROUP_PAYMENTS: &str = "payments";
/// Group key: reminders.
pub const GROUP_REMINDERS: &str = "reminders";
/// Group key: bookings.
pub const GROUP_BOOKINGS: &str = "bookings";
/// Group key: portal.
pub const GROUP_PORTAL: &str = "portal";
/// Group key: documents.
pub const GROUP_DOCUMENTS: &str = "documents";
/// Group key: signatures.
pub const GROUP_SIGNATURES: &str = "signatures";
/// Group key: guardian.
pub const GROUP_GUARDIAN: &str = "guardian";
/// Group key: employee security.
pub const GROUP_EMPLOYEE_SECURITY: &str = "employee_security";
/// Group key: one-time passwords.
pub const GROUP_OTP: &str = "otp";
/// Group key: marketing.
pub const GROUP_MARKETING: &str = "marketing";
/// Group key: tracking.
pub const GROUP_TRACKING: &str = "tracking";

/// The role told on give-up for the receivables groups.
const ROLE_RECEIVABLES: &str = "receivables";
/// The role told on give-up for `employee_security`.
const ROLE_HR: &str = "hr";

/// The default give-up reaction of a type (wl-notify §5.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GiveUpDefault {
    /// The reactions, in order.
    pub reactions: Vec<GiveUpReaction>,
    /// The staff role told, when the group names one.
    pub staff_role: Option<String>,
}

/// The default give-up reaction of `def`, by the precedence statutory, then
/// OTP, then group, then urgency. Pure: it ignores the type's own
/// `give_up` and `staff_role`.
pub fn default_give_up(def: &NotificationTypeDef) -> GiveUpDefault {
    use GiveUpReaction::{AlternativeRecipient, MarkContactInvalid, StaffTask, StatutoryNotice};
    let (mut reactions, role): (Vec<GiveUpReaction>, Option<&str>) = match def.group.as_str() {
        GROUP_INVOICES | GROUP_STATEMENTS | GROUP_PAYMENTS | GROUP_REMINDERS => (
            vec![MarkContactInvalid, AlternativeRecipient, StaffTask],
            Some(ROLE_RECEIVABLES),
        ),
        GROUP_BOOKINGS | GROUP_MARKETING => (vec![MarkContactInvalid], None),
        GROUP_PORTAL | GROUP_DOCUMENTS | GROUP_SIGNATURES => {
            (vec![MarkContactInvalid, StaffTask], None)
        }
        GROUP_GUARDIAN => (vec![AlternativeRecipient, StaffTask], None),
        GROUP_EMPLOYEE_SECURITY => (vec![MarkContactInvalid, StaffTask], Some(ROLE_HR)),
        GROUP_OTP | GROUP_TRACKING => (vec![GiveUpReaction::None], None),
        _ => (vec![StaffTask], None),
    };
    if def.statutory {
        reactions = vec![MarkContactInvalid, StatutoryNotice];
    } else if def.urgency == Urgency::Urgent
        && reactions.last() != Some(&StaffTask)
        && reactions != [GiveUpReaction::None]
    {
        reactions.push(StaffTask);
    }
    GiveUpDefault {
        reactions,
        staff_role: role.map(str::to_owned),
    }
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
    /// How staff may prove a statutory notice of this type was delivered
    /// another way (all four by default).
    pub proof_methods: Vec<ProofMethod>,
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
            proof_methods: ProofMethod::ALL.to_vec(),
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

    /// Offering these proof methods for a statutory notice of this type.
    pub fn proof_methods(mut self, methods: &[ProofMethod]) -> Self {
        self.proof_methods = methods.to_vec();
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
/// call it in `register`. Registering the same key again replaces it. A
/// definition with an empty `give_up` gets [`default_give_up`] (its role only
/// when `staff_role` is `None`); its own list, even `[none]`, is kept.
///
/// # Panics
///
/// When [`check_give_up`] refuses the definition, so a module's mistake
/// fails at boot.
pub fn register_type(mut def: NotificationTypeDef) {
    // The combination rule checks the list a module gives, not the seeded
    // default (the booking row is `mark_contact_invalid` alone).
    if let Err(error) = check_give_up(def.category, &def.give_up) {
        panic!("notification type `{}`: {error}", def.key);
    }
    if def.give_up.is_empty() {
        let default = default_give_up(&def);
        def.give_up = default.reactions;
        if def.staff_role.is_none() {
            def.staff_role = default.staff_role;
        }
    }
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

/// The kind of relationship that makes a party a contact of another one
/// (wl-parties §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactRelation {
    /// A person who is a contact of a company (or of another party), in a
    /// position such as `finance`.
    ContactOf,
    /// A person who is a guardian of another (a student, a minor).
    GuardianOf,
}

impl ContactRelation {
    /// The stored name: `contact_of` or `guardian_of`.
    pub fn as_str(self) -> &'static str {
        match self {
            ContactRelation::ContactOf => "contact_of",
            ContactRelation::GuardianOf => "guardian_of",
        }
    }
}

/// What makes a party the right contact: the relationship and, optionally,
/// its position. With no position, any position matches.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContactPurpose {
    /// The relationship.
    pub relation: ContactRelation,
    /// The position, e.g. `finance`; `None` for any.
    pub position: Option<String>,
}

impl ContactPurpose {
    /// A contact of the party in `position`, e.g. `finance`.
    pub fn contact_of(position: &str) -> Self {
        ContactPurpose {
            relation: ContactRelation::ContactOf,
            position: Some(position.to_owned()),
        }
    }

    /// A guardian of the party, in any position.
    pub fn guardian_of() -> Self {
        ContactPurpose {
            relation: ContactRelation::GuardianOf,
            position: None,
        }
    }
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
    /// A contact of a party, chosen by a rule ("the finance contacts of
    /// Blue Dunes LLC"); see [`ContactPurpose`]. The rule can be run again
    /// to find another contact when a message gave up.
    Contact {
        /// The party whose contacts (or whose guardians) are meant.
        of: PartyId,
        /// Which relationship and position make a contact.
        purpose: ContactPurpose,
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
    /// A short label for the subject in staff-facing text (e.g.
    /// `TKT-2026-0042`); `None` falls back to the subject's own text.
    pub subject_label: Option<String>,
}

impl NotifyRequest {
    /// A notification of `type_key` about `subject`, to nobody yet.
    pub fn new(type_key: &str, subject: Subject) -> Self {
        NotifyRequest {
            type_key: type_key.to_owned(),
            subject,
            recipients: Vec::new(),
            data: Value::Object(Default::default()),
            subject_label: None,
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

    /// Also to a contact of `of` chosen by `purpose` (e.g. its finance
    /// contacts).
    pub fn to_contact(self, of: PartyId, purpose: ContactPurpose) -> Self {
        self.to(NotifyRecipient::Contact { of, purpose })
    }

    /// With these template variables.
    pub fn data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    /// With this label for the subject in staff-facing text.
    pub fn subject_label(mut self, label: &str) -> Self {
        self.subject_label = Some(label.to_owned());
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

    /// Whether the statutory notice of `type_key` about `subject` reached
    /// its recipient. The default, and [`NoNotifier`], answer
    /// [`NoticeStatus::NotDelivered`], so a notice is never read as
    /// delivered without a provider receipt or a recorded proof.
    fn notice_status<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
        _type_key: &'a str,
    ) -> NotifyFuture<'a, NoticeStatus> {
        Box::pin(async { Ok(NoticeStatus::NotDelivered) })
    }

    /// The statutory notices about `subject` that gave up and have no proof
    /// yet. The default, and [`NoNotifier`], list none.
    fn undelivered_notices<'a>(
        &'a self,
        _state: &'a AppState,
        _subject: &'a Subject,
    ) -> NotifyFuture<'a, Vec<UndeliveredNotice>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    /// Records a marketing broadcast to a segment and returns its id
    /// (wl-notify §4.3). Fails (422) when the requester is not active, the
    /// type is not of category `marketing`, no user is named or the channel
    /// cannot be used. A request above the size limit waits for approval.
    /// The default, and [`NoNotifier`], refuse (422): broadcasts need the
    /// notify module.
    fn broadcast<'a>(
        &'a self,
        _state: &'a AppState,
        _request: BroadcastRequest,
    ) -> NotifyFuture<'a, BroadcastId> {
        Box::pin(async { Err(broadcasts_off()) })
    }

    /// Sends a one-time code to `request.to` for `request.purpose` about
    /// `request.subject` (wl-notify §6 `send_otp`). A new code for the same
    /// purpose and subject replaces the live one. The default, and
    /// [`NoNotifier`], refuse (422): one-time codes need the notify module.
    fn send_otp<'a>(
        &'a self,
        _state: &'a AppState,
        _request: OtpRequest,
    ) -> NotifyFuture<'a, OtpSent> {
        Box::pin(async { Err(otp_off()) })
    }

    /// Checks the code a person typed against the live code for `purpose` and
    /// `subject` (wl-notify §6 `verify_otp`). The default, and
    /// [`NoNotifier`], refuse (422).
    fn verify_otp<'a>(
        &'a self,
        _state: &'a AppState,
        _purpose: &'a str,
        _subject: &'a Subject,
        _code: &'a str,
    ) -> NotifyFuture<'a, OtpVerdict> {
        Box::pin(async { Err(otp_off()) })
    }
}

/// The refusal of [`Notifier::send_otp`] and [`Notifier::verify_otp`]
/// without the notify module.
fn otp_off() -> Error {
    Error::Status(
        StatusCode::UNPROCESSABLE_ENTITY,
        "One-time codes need the notifications module.".to_owned(),
    )
}

/// How long a one-time code is valid, in minutes (wl-notify §7).
pub const OTP_VALIDITY_MINUTES: i64 = 5;

/// How many wrong codes void a one-time code (wl-notify §7).
pub const OTP_WRONG_TRIES: i64 = 3;

/// A one-time code to send (wl-notify §6 `send_otp(contact_point, purpose)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OtpRequest {
    /// What the code confirms, e.g. `loyalty_redemption`.
    pub purpose: String,
    /// The record the code confirms: a code for one record never confirms
    /// another.
    pub subject: Subject,
    /// Where the code goes: a bare address on one channel, or a party.
    pub to: NotifyRecipient,
}

impl OtpRequest {
    /// A code for `purpose` about `subject`, to `to`.
    pub fn new(purpose: &str, subject: Subject, to: NotifyRecipient) -> Self {
        OtpRequest {
            purpose: purpose.to_owned(),
            subject,
            to,
        }
    }
}

/// What [`Notifier::send_otp`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OtpSent {
    /// The code was queued as a message.
    Sent {
        /// The message that carries it.
        message: MessageId,
        /// When the code stops being valid.
        expires_at: DateTime,
    },
    /// Nothing could be sent now (no open channel, no address, the address is
    /// suppressed): the person waiting sees `text` at once and may choose
    /// another way. No code is kept.
    NotSent {
        /// The text to show.
        text: String,
    },
}

/// What [`Notifier::verify_otp`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OtpVerdict {
    /// Right: the code is used up.
    Valid,
    /// Wrong, with this many wrong tries left. The code stays live.
    Wrong {
        /// Wrong tries left before the code is voided.
        tries_left: i64,
    },
    /// There is no live code: it expired, was used, or none was sent. A new
    /// one may be sent.
    Expired,
    /// The wrong tries are used up and the code is voided (the one that used
    /// the last try gets this answer too).
    Locked,
}

/// Whether a statutory notice was delivered (wl-notify §5.4). A requester
/// starts a deadline only from the date this gives, never from the time a
/// proof was typed in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoticeStatus {
    /// The provider reported it delivered (or read) at this time.
    Delivered {
        /// The provider's delivery time.
        at: DateTime,
    },
    /// It gave up and staff recorded a proof of delivery by other means.
    ProofRecorded {
        /// The delivery date the proof states.
        delivered_on: NaiveDate,
    },
    /// Not delivered: not sent, still in flight, sent without a receipt, or
    /// given up without a proof.
    NotDelivered,
}

/// A statutory notice that gave up and waits for a proof of delivery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndeliveredNotice {
    /// The id of the statutory-notice record.
    pub notice: i64,
    /// The type key.
    pub type_key: String,
    /// The text to show, e.g. the type's label.
    pub label: String,
    /// When the notice gave up.
    pub gave_up_at: DateTime,
    /// The path of the page where staff record the proof.
    pub proof_path: String,
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
    /// Whether it can be used; `false` after a hard failure until the
    /// address is edited or verified.
    pub valid: bool,
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
    /// first; invalid points are never chosen.
    pub fn point_for(&self, channel: NotifyChannel) -> Option<&PartyContactPoint> {
        let kind = channel.contact_kind();
        let mut of_kind = self.points.iter().filter(|p| p.kind == kind && p.valid);
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
    /// The label of the subject the request carried, if any.
    pub subject_label: Option<String>,
    /// Who it was for.
    pub recipient: NotifyRecipient,
    /// When the message gave up; a retried reaction sees the same time.
    pub gave_up_at: DateTime,
    /// The party contact point of the last message, if any.
    pub contact_point: Option<i64>,
    /// Every channel of the chain, in order, with its last error.
    pub channels_tried: Vec<ChannelTried>,
    /// Whether the type is statutory.
    pub statutory: bool,
    /// The reactions applied, in run order.
    pub reactions: Vec<GiveUpReaction>,
    /// The first message of the original chain when this message is the
    /// alternative queued to another contact; `None` otherwise. Tells a
    /// chain's final give-up apart.
    #[serde(default)]
    pub alternative_of: Option<MessageId>,
    /// The party whose contact the recipient rule chose from, when the
    /// message was sent to a [`NotifyRecipient::Contact`].
    #[serde(default)]
    pub contact_of: Option<PartyId>,
}

impl Event for MessageGaveUp {}

/// A party's contact point was marked invalid after a hard failure
/// (`mark_contact_invalid`); emitted once per point that changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactPointInvalidated {
    /// The contact point's id in `wl-parties`.
    pub contact_point: i64,
    /// The party it belongs to.
    pub party: PartyId,
    /// The channel the failed message used.
    pub channel: NotifyChannel,
    /// The hard failure kind, e.g. `hard_bounce`.
    pub reason: String,
}

impl Event for ContactPointInvalidated {}

/// Staff recorded the proof of delivery of a statutory notice; emitted once
/// per notice, after commit. Requesters start their deadline from
/// `delivered_on`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatutoryNoticeProofRecorded {
    /// The id of the statutory-notice record.
    pub notice: i64,
    /// The record the notice is about.
    pub subject: Subject,
    /// The type key.
    pub type_key: String,
    /// How it was delivered.
    pub method: ProofMethod,
    /// The delivery date stated by the proof.
    pub delivered_on: NaiveDate,
}

impl Event for StatutoryNoticeProofRecorded {}

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

    fn row(group: &str) -> NotificationTypeDef {
        NotificationTypeDef::new("kernel_test.row", "kernel_test").group(group)
    }

    fn keyed(key: &str, group: &str) -> NotificationTypeDef {
        NotificationTypeDef::new(key, "kernel_test").group(group)
    }

    fn defaults(def: &NotificationTypeDef) -> (Vec<GiveUpReaction>, Option<String>) {
        let d = default_give_up(def);
        (d.reactions, d.staff_role)
    }

    #[test]
    fn the_default_table_has_a_row_per_group() {
        let receivables = (
            vec![MarkContactInvalid, AlternativeRecipient, StaffTask],
            Some("receivables".to_owned()),
        );
        for group in [
            GROUP_INVOICES,
            GROUP_STATEMENTS,
            GROUP_PAYMENTS,
            GROUP_REMINDERS,
        ] {
            assert_eq!(defaults(&row(group)), receivables, "{group}");
        }
        for group in [GROUP_BOOKINGS, GROUP_MARKETING] {
            assert_eq!(defaults(&row(group)), (vec![MarkContactInvalid], None));
        }
        for group in [GROUP_PORTAL, GROUP_DOCUMENTS, GROUP_SIGNATURES] {
            assert_eq!(
                defaults(&row(group)),
                (vec![MarkContactInvalid, StaffTask], None),
                "{group}"
            );
        }
        assert_eq!(
            defaults(&row(GROUP_GUARDIAN)),
            (vec![AlternativeRecipient, StaffTask], None)
        );
        assert_eq!(
            defaults(&row(GROUP_EMPLOYEE_SECURITY)),
            (vec![MarkContactInvalid, StaffTask], Some("hr".to_owned()))
        );
        for group in [GROUP_OTP, GROUP_TRACKING] {
            assert_eq!(defaults(&row(group)), (vec![GiveUpReaction::None], None));
        }
        assert_eq!(defaults(&row("unknown")), (vec![StaffTask], None));
        // The key-prefix group `new` fills in counts as no group.
        let fresh = NotificationTypeDef::new("lease.something", "property");
        assert_eq!(defaults(&fresh), (vec![StaffTask], None));
    }

    #[test]
    fn urgency_appends_a_staff_task_except_for_none() {
        let urgent = |group: &str| row(group).urgency(Urgency::Urgent);
        for group in [GROUP_BOOKINGS, GROUP_MARKETING] {
            assert_eq!(defaults(&urgent(group)).0, [MarkContactInvalid, StaffTask]);
        }
        assert_eq!(
            defaults(&urgent(GROUP_GUARDIAN)).0,
            [AlternativeRecipient, StaffTask]
        );
        assert_eq!(
            defaults(&urgent(GROUP_INVOICES)),
            defaults(&row(GROUP_INVOICES))
        );
        assert_eq!(defaults(&urgent("unknown")).0, [StaffTask]);
        assert_eq!(defaults(&urgent(GROUP_TRACKING)).0, [GiveUpReaction::None]);
        assert_eq!(defaults(&urgent(GROUP_OTP)).0, [GiveUpReaction::None]);
    }

    #[test]
    fn statutory_wins_over_group_otp_and_urgency() {
        for group in [GROUP_INVOICES, GROUP_OTP, GROUP_GUARDIAN, "unknown"] {
            for urgency in [Urgency::Normal, Urgency::Urgent] {
                let def = row(group).statutory(true).urgency(urgency);
                assert_eq!(
                    defaults(&def).0,
                    [MarkContactInvalid, GiveUpReaction::StatutoryNotice],
                    "{group}"
                );
            }
        }
    }

    #[test]
    fn register_type_seeds_an_empty_list_and_keeps_an_own_one() {
        register_type(keyed("kernel_test.seeded", GROUP_INVOICES));
        let seeded = notification_type("kernel_test.seeded").unwrap();
        assert_eq!(
            seeded.give_up,
            [MarkContactInvalid, AlternativeRecipient, StaffTask]
        );
        assert_eq!(seeded.staff_role.as_deref(), Some("receivables"));

        register_type(
            keyed("kernel_test.own", GROUP_INVOICES)
                .give_up(&[GiveUpReaction::None])
                .staff_role("boss"),
        );
        let own = notification_type("kernel_test.own").unwrap();
        assert_eq!(own.give_up, [GiveUpReaction::None]);
        assert_eq!(own.staff_role.as_deref(), Some("boss"));

        register_type(keyed("kernel_test.role", GROUP_INVOICES).staff_role("boss"));
        let role = notification_type("kernel_test.role").unwrap();
        assert_eq!(role.staff_role.as_deref(), Some("boss"));
        assert_eq!(role.give_up.len(), 3);
    }

    #[test]
    fn the_seeded_booking_default_passes_register_type() {
        register_type(keyed("kernel_test.booking", GROUP_BOOKINGS));
        assert_eq!(
            notification_type("kernel_test.booking").unwrap().give_up,
            [MarkContactInvalid]
        );
    }

    #[test]
    fn proof_methods_default_to_all_and_can_be_narrowed() {
        let def = reminder();
        assert_eq!(def.proof_methods, ProofMethod::ALL.to_vec());
        let names: Vec<_> = ProofMethod::ALL.iter().map(|m| m.as_str()).collect();
        assert_eq!(
            names,
            ["courier", "registered_mail", "hand_delivery", "call"]
        );
        let narrow = def.proof_methods(&[ProofMethod::Call]);
        assert_eq!(narrow.proof_methods, [ProofMethod::Call]);
    }

    #[test]
    fn notice_status_and_event_round_trip_as_json() {
        let on = NaiveDate::from_ymd_opt(2026, 9, 3).unwrap();
        let status = NoticeStatus::ProofRecorded { delivered_on: on };
        let back: NoticeStatus = renox::serde_json::from_value(json!(status)).unwrap();
        assert_eq!(back, status);
        let event = StatutoryNoticeProofRecorded {
            notice: 4,
            subject: Subject::new("property.lease", 7),
            type_key: "lease.changed_terms_notice".into(),
            method: ProofMethod::HandDelivery,
            delivered_on: on,
        };
        let back: StatutoryNoticeProofRecorded =
            renox::serde_json::from_value(json!(event)).unwrap();
        assert_eq!(back, event);
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
            valid: true,
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
    fn an_invalid_point_is_never_chosen() {
        let point = |id, primary, valid| PartyContactPoint {
            id,
            kind: ContactKind::Email,
            address: format!("address-{id}"),
            primary,
            verified: false,
            valid,
        };
        let mut contacts = PartyContacts {
            party: PartyId(1),
            language: "en".into(),
            preferred_channel: None,
            points: vec![point(1, false, true), point(2, true, false)],
        };
        assert_eq!(contacts.point_for(NotifyChannel::Email).unwrap().id, 1);
        assert!(contacts.point_for(NotifyChannel::Sms).is_none());
        contacts.points.remove(0);
        assert!(contacts.point_for(NotifyChannel::Email).is_none());
    }

    #[test]
    fn mark_contact_invalid_alone_needs_marketing() {
        let alone = [MarkContactInvalid];
        let refused = Err(GiveUpConfigError::MarkContactInvalidAlone);
        for category in [
            Category::Transactional,
            Category::Reminder,
            Category::Security,
        ] {
            assert_eq!(check_give_up(category, &alone), refused);
            assert_eq!(
                check_give_up(category, &[MarkContactInvalid, GiveUpReaction::None]),
                refused
            );
            assert_eq!(
                check_give_up(category, &[MarkContactInvalid, MarkContactInvalid]),
                refused
            );
            assert_eq!(
                check_give_up(category, &[MarkContactInvalid, StaffTask]),
                Ok(())
            );
            assert_eq!(check_give_up(category, &[StaffTask]), Ok(()));
            assert_eq!(check_give_up(category, &[]), Ok(()));
            assert_eq!(check_give_up(category, &[GiveUpReaction::None]), Ok(()));
        }
        assert_eq!(check_give_up(Category::Marketing, &alone), Ok(()));
        assert!(
            GiveUpConfigError::MarkContactInvalidAlone
                .to_string()
                .contains("add Staff task.")
        );
    }

    #[test]
    #[should_panic(expected = "Mark contact invalid must be combined")]
    fn register_type_panics_on_a_refused_definition() {
        register_type(
            NotificationTypeDef::new("refused.alone", "tests").give_up(&[MarkContactInvalid]),
        );
    }

    #[test]
    fn recipients_are_tagged_in_json() {
        let json =
            renox::serde_json::to_string(&NotifyRecipient::Party { id: PartyId(4) }).unwrap();
        assert_eq!(json, r#"{"kind":"party","id":4}"#);
        let json = renox::serde_json::to_string(&NotifyRecipient::User { id: 2 }).unwrap();
        assert_eq!(json, r#"{"kind":"user","id":2}"#);
        let json = renox::serde_json::to_string(&NotifyRecipient::Contact {
            of: PartyId(5),
            purpose: ContactPurpose::contact_of("finance"),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"kind":"contact","of":5,"purpose":{"relation":"contact_of","position":"finance"}}"#
        );
    }

    #[test]
    fn to_contact_adds_a_rule_recipient() {
        let request = NotifyRequest::new("invoice.reminder", Subject::new("invoicing.invoice", 1))
            .to_contact(PartyId(5), ContactPurpose::guardian_of());
        assert_eq!(
            request.recipients,
            [NotifyRecipient::Contact {
                of: PartyId(5),
                purpose: ContactPurpose {
                    relation: ContactRelation::GuardianOf,
                    position: None,
                },
            }]
        );
        assert_eq!(ContactRelation::ContactOf.as_str(), "contact_of");
        assert_eq!(ContactRelation::GuardianOf.as_str(), "guardian_of");
    }
}
