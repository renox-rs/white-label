//! Channel providers (wl-notify §8 `ChannelProvider`): what sends a message
//! on one channel, and how its errors are told apart (#55 A6).
//!
//! The module has two built-in providers:
//!
//! - [`EmailProvider`] sends a Renox notification on `Channel::Mail` to the
//!   address, in the message's language;
//! - [`SmsProvider`] sends a Renox notification on the custom channel
//!   `sms`, which [`register`] adds with `Registry::channel`. That channel
//!   hands the text to the app's [`SmsGateway`]; with none provided
//!   ([`provide_sms_gateway`]) the provider is not available, so a client
//!   without an SMS gateway stays e-mail only.
//!
//! `WlNotify::new().provider(p)` replaces the provider of `p`'s channel.
//! [`open_channels`] lists the channels whose provider can send now.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::auth::{Channel, Notification, Recipient};
use renox::mail::Mail;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::notify::{MessageId, NotifyChannel};

use crate::message::Message;
use crate::suppressions::SuppressionReason;

/// A boxed future, so the provider traits stay object safe.
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The name of the Renox custom channel the SMS provider sends on.
pub const SMS_CHANNEL: &str = "sms";

/// What a provider gets to send: one message, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    /// The message.
    pub message: MessageId,
    /// The channel.
    pub channel: NotifyChannel,
    /// The e-mail address or number.
    pub address: String,
    /// The language it is written in.
    pub language: String,
    /// The subject (e-mail) or title: the rendered one, else the type key.
    pub title: String,
    /// The rendered body.
    pub body: String,
    /// The sender ID an SMS goes out with (`AD-...` for a broadcast), if any.
    pub sender_id: Option<String>,
    /// The files an e-mail carries (read from the stored documents at send
    /// time); always empty on other channels.
    pub attachments: Vec<OutgoingFile>,
}

/// A file attached to an outgoing e-mail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingFile {
    /// The name the recipient sees.
    pub file_name: String,
    /// The MIME type.
    pub content_type: String,
    /// The content.
    pub bytes: Vec<u8>,
}

impl Outgoing {
    /// `message`, to be sent to `address`.
    pub fn of(message: &Message, address: &str) -> Self {
        Outgoing {
            message: message.message_id(),
            channel: message.channel,
            address: address.to_owned(),
            language: message.language.clone(),
            title: message
                .rendered_subject
                .clone()
                .unwrap_or_else(|| message.type_key.clone()),
            body: message.rendered_body.clone(),
            sender_id: None,
            attachments: Vec::new(),
        }
    }

    /// The same, carrying `files` when it is an e-mail (other channels
    /// carry no file).
    pub fn with_attachments(mut self, files: Vec<OutgoingFile>) -> Self {
        if self.channel == NotifyChannel::Email {
            self.attachments = files;
        }
        self
    }

    /// The same, sent with `sender_id`.
    pub fn with_sender_id(mut self, sender_id: Option<&str>) -> Self {
        self.sender_id = sender_id.map(str::to_owned);
        self
    }
}

/// An error that says the address itself is wrong; the address goes on the
/// suppression list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HardError {
    /// The mailbox does not exist.
    HardBounce,
    /// The number is not a valid mobile number.
    InvalidNumber,
    /// The account (e.g. WhatsApp) does not exist.
    UnknownAccount,
}

/// What kind of error a send ran into, which decides what follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SendErrorKind {
    /// May pass on another attempt (a timeout, a busy gateway): retried.
    Transient,
    /// Another attempt on this channel will not help: no retry.
    Permanent,
    /// The address is wrong: no retry, and the address is suppressed.
    Hard(HardError),
}

impl SendErrorKind {
    /// The kind as the message stores it in `error_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            SendErrorKind::Transient => "transient",
            SendErrorKind::Permanent => "permanent",
            SendErrorKind::Hard(HardError::HardBounce) => "hard_bounce",
            SendErrorKind::Hard(HardError::InvalidNumber) => "invalid_number",
            SendErrorKind::Hard(HardError::UnknownAccount) => "unknown_account",
        }
    }

    /// Why the address goes on the suppression list, for a hard error.
    pub fn suppression(self) -> Option<SuppressionReason> {
        match self {
            SendErrorKind::Hard(HardError::HardBounce) => Some(SuppressionReason::HardBounce),
            SendErrorKind::Hard(HardError::InvalidNumber | HardError::UnknownAccount) => {
                Some(SuppressionReason::Invalid)
            }
            SendErrorKind::Transient | SendErrorKind::Permanent => None,
        }
    }
}

/// Why a provider did not send a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendError {
    /// Transient, permanent or hard.
    pub kind: SendErrorKind,
    /// The provider's error code, e.g. `550`.
    pub code: Option<String>,
    /// The provider's error text.
    pub message: String,
}

impl SendError {
    /// An error of `kind`.
    pub fn new(kind: SendErrorKind, code: Option<&str>, message: impl Into<String>) -> Self {
        SendError {
            kind,
            code: code.map(str::to_owned),
            message: message.into(),
        }
    }

    /// A transient error with `code`.
    pub fn transient(code: &str, message: impl Into<String>) -> Self {
        Self::new(SendErrorKind::Transient, Some(code), message)
    }

    /// A permanent error with `code`.
    pub fn permanent(code: &str, message: impl Into<String>) -> Self {
        Self::new(SendErrorKind::Permanent, Some(code), message)
    }

    /// A hard error with `code`.
    pub fn hard(hard: HardError, code: &str, message: impl Into<String>) -> Self {
        Self::new(SendErrorKind::Hard(hard), Some(code), message)
    }

    /// A Renox error: permanent when it was made with `Error::permanent`,
    /// else transient. Renox gives no code.
    pub fn from_renox(error: &renox::Error) -> Self {
        let kind = if error.is_permanent() {
            SendErrorKind::Permanent
        } else {
            SendErrorKind::Transient
        };
        Self::new(kind, None, error.to_string())
    }
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(code) => write!(f, "{code}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Sends messages on one channel (wl-notify §8).
pub trait ChannelProvider: Send + Sync + 'static {
    /// The provider's name, stored on the message, e.g. `renox-mail`.
    fn name(&self) -> &'static str;

    /// The channel it sends on.
    fn channel(&self) -> NotifyChannel;

    /// Whether it can send now (configured).
    fn available(&self, state: &AppState) -> bool;

    /// Sends `outgoing`; the provider's id for it, or why not.
    fn send<'a>(
        &'a self,
        state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>>;
}

/// The module's providers, one per channel, as the app holds them.
#[derive(Clone)]
pub struct Providers {
    list: Vec<Arc<dyn ChannelProvider>>,
}

impl Default for Providers {
    /// The built-in providers: e-mail and SMS.
    fn default() -> Self {
        Providers {
            list: vec![Arc::new(EmailProvider), Arc::new(SmsProvider)],
        }
    }
}

impl Providers {
    /// Uses `provider` for its channel, in place of the one there.
    pub fn replace(&mut self, provider: Arc<dyn ChannelProvider>) {
        let channel = provider.channel();
        match self.list.iter_mut().find(|p| p.channel() == channel) {
            Some(slot) => *slot = provider,
            None => self.list.push(provider),
        }
    }

    /// The provider of `channel`.
    pub fn for_channel(&self, channel: NotifyChannel) -> Option<Arc<dyn ChannelProvider>> {
        self.list.iter().find(|p| p.channel() == channel).cloned()
    }

    /// The channels whose provider can send now, in the providers' order.
    pub fn open(&self, state: &AppState) -> Vec<NotifyChannel> {
        self.list
            .iter()
            .filter(|p| p.available(state))
            .map(|p| p.channel())
            .collect()
    }
}

/// The app's providers; the built-in ones when the module did not provide
/// any (e.g. a test state without the module).
pub fn providers(state: &AppState) -> Arc<Providers> {
    state.provided::<Providers>().unwrap_or_default()
}

/// The channels a provider can send on now.
pub fn open_channels(state: &AppState) -> Vec<NotifyChannel> {
    providers(state).open(state)
}

/// The built-in e-mail provider: Renox mail, through a notification on
/// `Channel::Mail`.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmailProvider;

impl ChannelProvider for EmailProvider {
    fn name(&self) -> &'static str {
        "renox-mail"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Email
    }

    fn available(&self, _state: &AppState) -> bool {
        true
    }

    fn send<'a>(
        &'a self,
        state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        Box::pin(async move {
            let to = Recipient::to("mail", outgoing.address.clone())
                .in_locale(outgoing.language.clone());
            state
                .notify(to, &Outgoing::notice(outgoing))
                .await
                .map_err(|e| SendError::from_renox(&e))?;
            // Renox mail gives no id of its own.
            Ok(format!("mail:{}", outgoing.message.0))
        })
    }
}

/// Sends an SMS for the built-in SMS provider (an HTTP gateway client).
/// Provide one with [`provide_sms_gateway`].
pub trait SmsGateway: Send + Sync + 'static {
    /// Sends `text` to the number `to`.
    fn send<'a>(
        &'a self,
        state: &'a AppState,
        to: &'a str,
        text: &'a str,
    ) -> ProviderFuture<'a, Result>;

    /// As [`SmsGateway::send`], from the sender ID `sender` (the `AD-...` ID
    /// of a marketing broadcast). A gateway that has no sender IDs keeps this
    /// default and sends from its own.
    fn send_from<'a>(
        &'a self,
        state: &'a AppState,
        to: &'a str,
        text: &'a str,
        sender: &'a str,
    ) -> ProviderFuture<'a, Result> {
        let _ = sender;
        self.send(state, to, text)
    }
}

/// The app's SMS gateway, as provided.
#[derive(Clone)]
pub struct SmsGatewayHandle(pub Arc<dyn SmsGateway>);

/// Makes `gateway` the app's SMS gateway; the SMS provider is available
/// from then on.
pub fn provide_sms_gateway(app: &mut Registry, gateway: impl SmsGateway) {
    app.provide(SmsGatewayHandle(Arc::new(gateway)));
}

/// The built-in SMS provider: a notification on the Renox custom channel
/// [`SMS_CHANNEL`]. Available when an [`SmsGateway`] is provided.
#[derive(Debug, Clone, Copy, Default)]
pub struct SmsProvider;

impl ChannelProvider for SmsProvider {
    fn name(&self) -> &'static str {
        "renox-sms"
    }

    fn channel(&self) -> NotifyChannel {
        NotifyChannel::Sms
    }

    fn available(&self, state: &AppState) -> bool {
        state.provided::<SmsGatewayHandle>().is_some()
    }

    fn send<'a>(
        &'a self,
        state: &'a AppState,
        outgoing: &'a Outgoing,
    ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
        Box::pin(async move {
            let to = Recipient::to(SMS_CHANNEL, outgoing.address.clone())
                .in_locale(outgoing.language.clone());
            state
                .notify(to, &Outgoing::notice(outgoing))
                .await
                .map_err(|e| SendError::from_renox(&e))?;
            Ok(format!("sms:{}", outgoing.message.0))
        })
    }
}

/// Adds the Renox custom channel [`SMS_CHANNEL`]: it hands the text to the
/// app's [`SmsGateway`], and fails (permanent) without one.
pub fn register(app: &mut Registry) {
    app.channel(
        SMS_CHANNEL,
        |to: Recipient, message: Value, state: AppState| async move {
            let gateway = state
                .provided::<SmsGatewayHandle>()
                .ok_or_else(|| renox::Error::permanent_message("no SMS gateway is configured"))?;
            let number = to
                .address(SMS_CHANNEL)
                .ok_or_else(|| renox::Error::permanent_message("the SMS has no number"))?;
            let text = message["text"].as_str().unwrap_or_default();
            match message["from"].as_str() {
                Some(sender) => gateway.0.send_from(&state, &number, text, sender).await,
                None => gateway.0.send(&state, &number, text).await,
            }
        },
    );
}

/// The Renox notification a built-in provider sends for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderNotice {
    /// The channel.
    pub channel: NotifyChannel,
    /// The address.
    pub address: String,
    /// The subject or title.
    pub title: String,
    /// The body.
    pub body: String,
    /// The sender ID of an SMS.
    pub sender_id: Option<String>,
    /// The files of an e-mail.
    pub attachments: Vec<OutgoingFile>,
}

impl Outgoing {
    /// The notification that carries this message.
    pub fn notice(&self) -> ProviderNotice {
        ProviderNotice {
            channel: self.channel,
            address: self.address.clone(),
            title: self.title.clone(),
            body: self.body.clone(),
            sender_id: self.sender_id.clone(),
            attachments: self.attachments.clone(),
        }
    }
}

impl Notification for ProviderNotice {
    fn kind(&self) -> &'static str {
        "wl-notify-message"
    }

    fn channels(&self, _to: &Recipient) -> Vec<Channel> {
        match self.channel {
            NotifyChannel::Email => vec![Channel::Mail],
            NotifyChannel::Sms => vec![Channel::Custom(SMS_CHANNEL)],
            NotifyChannel::Whatsapp => vec![Channel::Custom("whatsapp")],
        }
    }

    fn to_mail(&self, _to: &Recipient, _state: &AppState) -> Result<Mail> {
        let mut mail = Mail::new(self.address.clone(), self.title.clone(), self.body.clone());
        for file in &self.attachments {
            mail = mail.attach(
                file.file_name.clone(),
                file.content_type.clone(),
                file.bytes.clone(),
            );
        }
        Ok(mail)
    }

    fn to_channel(&self, _channel: &str, _to: &Recipient, _state: &AppState) -> Result<Value> {
        let mut payload = json!({ "to": self.address, "text": self.body });
        if let Some(sender) = &self.sender_id {
            payload["from"] = json!(sender);
        }
        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_kinds_are_stored_by_name() {
        let names: Vec<_> = [
            SendErrorKind::Transient,
            SendErrorKind::Permanent,
            SendErrorKind::Hard(HardError::HardBounce),
            SendErrorKind::Hard(HardError::InvalidNumber),
            SendErrorKind::Hard(HardError::UnknownAccount),
        ]
        .iter()
        .map(|k| k.as_str())
        .collect();
        assert_eq!(
            names,
            [
                "transient",
                "permanent",
                "hard_bounce",
                "invalid_number",
                "unknown_account"
            ]
        );
    }

    #[test]
    fn only_hard_errors_suppress_the_address() {
        assert_eq!(SendErrorKind::Transient.suppression(), None);
        assert_eq!(SendErrorKind::Permanent.suppression(), None);
        assert_eq!(
            SendErrorKind::Hard(HardError::HardBounce).suppression(),
            Some(SuppressionReason::HardBounce)
        );
        assert_eq!(
            SendErrorKind::Hard(HardError::InvalidNumber).suppression(),
            Some(SuppressionReason::Invalid)
        );
    }

    #[test]
    fn a_renox_error_is_permanent_only_when_marked() {
        let permanent = SendError::from_renox(&renox::Error::permanent_message("bad address"));
        assert_eq!(permanent.kind, SendErrorKind::Permanent);
        assert_eq!(permanent.code, None);
        let other = renox::Error::Internal(renox::anyhow::anyhow!("connection refused"));
        assert_eq!(SendError::from_renox(&other).kind, SendErrorKind::Transient);
        assert_eq!(
            SendError::permanent("550", "mailbox unknown").to_string(),
            "550: mailbox unknown"
        );
    }

    struct Scripted;

    impl ChannelProvider for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }
        fn channel(&self) -> NotifyChannel {
            NotifyChannel::Sms
        }
        fn available(&self, _state: &AppState) -> bool {
            true
        }
        fn send<'a>(
            &'a self,
            _state: &'a AppState,
            _outgoing: &'a Outgoing,
        ) -> ProviderFuture<'a, std::result::Result<String, SendError>> {
            Box::pin(async { Ok("1".to_owned()) })
        }
    }

    #[test]
    fn a_provider_replaces_the_one_of_its_channel() {
        let mut providers = Providers::default();
        assert_eq!(
            providers.for_channel(NotifyChannel::Sms).unwrap().name(),
            "renox-sms"
        );
        providers.replace(Arc::new(Scripted));
        assert_eq!(providers.list.len(), 2);
        assert_eq!(
            providers.for_channel(NotifyChannel::Sms).unwrap().name(),
            "scripted"
        );
        assert_eq!(
            providers.for_channel(NotifyChannel::Email).unwrap().name(),
            "renox-mail"
        );
        assert!(providers.for_channel(NotifyChannel::Whatsapp).is_none());
    }

    #[test]
    fn the_outgoing_title_falls_back_to_the_type_key() {
        let message = Message {
            id: 3,
            type_key: "invoice.reminder".into(),
            rendered_body: "Please pay.".into(),
            language: "ar".into(),
            ..Default::default()
        };
        let outgoing = Outgoing::of(&message, "a@example.com");
        assert_eq!(outgoing.title, "invoice.reminder");
        assert_eq!(outgoing.language, "ar");
        let notice = outgoing.notice();
        assert_eq!(notice.channels(&Recipient::default()), [Channel::Mail]);
    }
}
