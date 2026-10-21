//! One-time codes (wl-notify §6 `send_otp` / `verify_otp`, §7) and the
//! failure notice (§5.4, #60 A6).
//!
//! **Codes.** [`send`] issues a six-digit code with Renox's `OneTimeCode`
//! (only a keyed hash is stored there), queues it as a message of the type
//! [`OTP_TYPE`] to the address named, and keeps an [`OtpCode`] row beside it.
//! The row is what lets [`verify`] answer `expired` and `locked` apart, where
//! Renox only knows "no live code". A code is valid for the OTP validity and
//! voided after the wrong tries ([`OtpSettings`], 5 minutes and 3).
//!
//! Decisions:
//! - A code belongs to a purpose and a subject (`loyalty_redemption`,
//!   `sales.counter_sale:7`): it never confirms another record. A new code for
//!   the same pair replaces the live one.
//! - The code travels in the message data (`{{ code }}`), so it is in the
//!   delivery log like any message text; it is short-lived and single-use.
//! - The wrong try that uses the last try answers `locked` at once (the code
//!   is voided then), not `wrong` with no tries left.
//! - When nothing can be sent now (no open channel, no address, the address on
//!   the suppression list) the answer is `NotSent` with the failure notice, no
//!   code is kept, and the person waiting may choose another way. A provider
//!   that fails later gives up as any message; the type's give-up reaction is
//!   `none` (§5.4) and `MessageGaveUp` tells the requester.
//!
//! **Failure notice.** An OTP type's default give-up reaction is `[none]`: no
//! task, no alert. The person is waiting on the login screen, so the screen
//! tells them at once. wl-portal calls [`failure_notice`] when sending the
//! code on `failed_channel` failed.

use std::time::Duration;

use renox::auth::{CodeCheck, OneTimeCode};
use renox::chrono::TimeDelta;
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::notify::{
    self, Category, GiveUpReaction, MessageId, NotificationTypeDef, NotifyChannel,
    NotifyRecipient, NotifyRequest, OTP_VALIDITY_MINUTES, OTP_WRONG_TRIES, OtpRequest, OtpSent,
    OtpVerdict, Urgency,
};

use crate::channels::open_channels;
use crate::message::{Message, MessageStatus};
use crate::texts::{english, english_with};

/// The notification type of the message that carries a code.
pub const OTP_TYPE: &str = "notify.otp_code";

/// The OTP settings (§7), set on [`WlNotify`](crate::WlNotify) with
/// `otp_validity_minutes` and `otp_wrong_tries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OtpSettings {
    /// Minutes a code is valid.
    pub validity_minutes: i64,
    /// Wrong tries before the code is voided.
    pub wrong_tries: i64,
}

impl Default for OtpSettings {
    fn default() -> Self {
        OtpSettings {
            validity_minutes: OTP_VALIDITY_MINUTES,
            wrong_tries: OTP_WRONG_TRIES,
        }
    }
}

/// The app's settings, or the defaults.
pub fn settings(state: &AppState) -> OtpSettings {
    state
        .provided::<OtpSettings>()
        .map(|s| *s)
        .unwrap_or_default()
}

/// Where a code is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OtpStatus {
    /// Sent and not used, voided or replaced.
    #[default]
    Live,
    /// The right code was entered.
    Used,
    /// The wrong tries are used up.
    Locked,
    /// A newer code for the same purpose and subject replaced it.
    Replaced,
}

/// A one-time code that was sent.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_notify_otp_codes", index(purpose, subject_type, subject_id))]
pub struct OtpCode {
    /// The primary key.
    pub id: i64,
    /// What the code confirms.
    pub purpose: String,
    /// The type of the record it confirms.
    pub subject_type: String,
    /// The id of the record it confirms.
    pub subject_id: i64,
    /// The channel it was sent on.
    pub channel: NotifyChannel,
    /// The message that carries it.
    pub message_id: Option<i64>,
    /// When it stops being valid.
    pub expires_at: Option<DateTime>,
    /// The wrong tries so far.
    pub tries: i64,
    /// The wrong tries that void it.
    pub max_tries: i64,
    /// Where it is.
    pub status: OtpStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl OtpCode {
    /// The newest code for `purpose` and `subject`.
    pub async fn latest(db: &Db, purpose: &str, subject: &Subject) -> Result<Option<OtpCode>> {
        OtpCode::query()
            .where_(OtpCode::PURPOSE.eq(purpose.to_owned()))
            .where_(OtpCode::SUBJECT_TYPE.eq(subject.subject_type.clone()))
            .where_(OtpCode::SUBJECT_ID.eq(subject.subject_id))
            .order_by_desc(OtpCode::ID)
            .first(db)
            .await
    }
}

/// The type of the message that carries a code: urgent (never held for quiet
/// hours), security, and no give-up reaction.
pub fn otp_type() -> NotificationTypeDef {
    NotificationTypeDef::new(OTP_TYPE, "notify")
        .group(notify::GROUP_OTP)
        .category(Category::Security)
        .urgency(Urgency::Urgent)
        .give_up(&[GiveUpReaction::None])
        .channels(&[NotifyChannel::Sms, NotifyChannel::Email])
        .template(
            NotifyChannel::Sms,
            "en",
            None,
            "{{ code }} is your confirmation code. It is valid for {{ minutes }} minutes.",
        )
        .template(
            NotifyChannel::Email,
            "en",
            Some("Your confirmation code"),
            "{{ code }} is your confirmation code. It is valid for {{ minutes }} minutes.",
        )
}

/// Registers the code message type with the kernel registry.
pub(crate) fn register_type() {
    notify::register_type(otp_type());
}

/// The key Renox stores the code under for `subject`.
fn key_of(subject: &Subject) -> String {
    subject.to_string()
}

/// Sends a code for `request` ([`Notifier::send_otp`](wl_kernel::notify::Notifier)).
pub async fn send(state: &AppState, request: OtpRequest) -> Result<OtpSent> {
    let config = settings(state);
    let minutes = config.validity_minutes.max(1);
    let tries = config.wrong_tries.max(1);
    let key = key_of(&request.subject);
    let code = OneTimeCode::issue(
        state,
        &request.purpose,
        &key,
        Duration::from_secs(u64::try_from(minutes).unwrap_or(5) * 60),
        u32::try_from(tries).unwrap_or(3),
    )
    .await?;
    let notice = NotifyRequest::new(OTP_TYPE, request.subject.clone())
        .to(request.to.clone())
        .data(json!({ "code": code, "minutes": minutes }));
    let ids = crate::notifier::queue(state, notice).await?;
    let message = match ids.first() {
        Some(MessageId(id)) => Message::find(&state.db, *id).await?,
        None => None,
    };
    let Some(message) = message.filter(|m| m.status != MessageStatus::Suppressed) else {
        OneTimeCode::discard(state, &request.purpose, &key).await?;
        let channel = match &request.to {
            NotifyRecipient::Address { channel, .. } => *channel,
            _ => NotifyChannel::Sms,
        };
        return Ok(OtpSent::NotSent {
            text: failure_notice(state, channel).text,
        });
    };
    // The older live codes of the pair no longer count.
    OtpCode::query()
        .where_(OtpCode::PURPOSE.eq(request.purpose.clone()))
        .where_(OtpCode::SUBJECT_TYPE.eq(request.subject.subject_type.clone()))
        .where_(OtpCode::SUBJECT_ID.eq(request.subject.subject_id))
        .where_(OtpCode::STATUS.eq(OtpStatus::Live))
        .update(&state.db, &[("status", &OtpStatus::Replaced)])
        .await?;
    let expires_at = now() + TimeDelta::minutes(minutes);
    OtpCode::create(
        &state.db,
        OtpCode {
            purpose: request.purpose.clone(),
            subject_type: request.subject.subject_type.clone(),
            subject_id: request.subject.subject_id,
            channel: message.channel,
            message_id: Some(message.id),
            expires_at: Some(expires_at),
            max_tries: tries,
            ..OtpCode::default()
        },
    )
    .await?;
    Ok(OtpSent::Sent {
        message: message.message_id(),
        expires_at,
    })
}

/// Checks `typed` against the live code for `purpose` and `subject`
/// ([`Notifier::verify_otp`](wl_kernel::notify::Notifier)).
pub async fn verify(
    state: &AppState,
    purpose: &str,
    subject: &Subject,
    typed: &str,
) -> Result<OtpVerdict> {
    let Some(row) = OtpCode::latest(&state.db, purpose, subject).await? else {
        return Ok(OtpVerdict::Expired);
    };
    match row.status {
        OtpStatus::Locked => return Ok(OtpVerdict::Locked),
        OtpStatus::Used | OtpStatus::Replaced => return Ok(OtpVerdict::Expired),
        OtpStatus::Live => {}
    }
    if row.expires_at.is_some_and(|at| at <= now()) {
        return Ok(OtpVerdict::Expired);
    }
    let checked = OneTimeCode::verify(state, purpose, &key_of(subject), typed).await?;
    let (status, tries, verdict) = match checked {
        CodeCheck::Valid => (OtpStatus::Used, row.tries, OtpVerdict::Valid),
        CodeCheck::Wrong { tries_left } if tries_left > 0 => (
            OtpStatus::Live,
            row.tries + 1,
            OtpVerdict::Wrong {
                tries_left: i64::from(tries_left),
            },
        ),
        CodeCheck::Wrong { .. } => (OtpStatus::Locked, row.tries + 1, OtpVerdict::Locked),
        CodeCheck::Missing => (row.status, row.tries, OtpVerdict::Expired),
    };
    OtpCode::query()
        .where_(OtpCode::ID.eq(row.id))
        .update(
            &state.db,
            &[("status", &status), ("tries", &tries), ("updated_at", &now())],
        )
        .await?;
    Ok(verdict)
}

/// What the login screen shows when a code could not be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtpFailure {
    /// The text to show, e.g. "Code could not be sent; try email".
    pub text: String,
    /// The channels still open, other than the one that failed, in the
    /// providers' order.
    pub other_channels: Vec<NotifyChannel>,
}

/// The notice for a code that could not be sent on `failed_channel`. The
/// other channels are the ones a provider can send on now; the text names
/// the first of them.
pub fn failure_notice(state: &AppState, failed_channel: NotifyChannel) -> OtpFailure {
    notice(failed_channel, open_channels(state))
}

/// The notice for `failed_channel` when `open` channels can send.
pub(crate) fn notice(failed_channel: NotifyChannel, open: Vec<NotifyChannel>) -> OtpFailure {
    let other_channels: Vec<NotifyChannel> = open
        .into_iter()
        .filter(|channel| *channel != failed_channel)
        .collect();
    let text = match other_channels.first() {
        Some(channel) => {
            let name = english(&format!("wl_notify.channel.{}", channel_key(*channel)));
            english_with(
                "wl_notify.otp.try_other",
                &[("channel", &name.to_lowercase())],
            )
        }
        None => english("wl_notify.otp.failed"),
    };
    OtpFailure {
        text,
        other_channels,
    }
}

fn channel_key(channel: NotifyChannel) -> &'static str {
    match channel {
        NotifyChannel::Email => "email",
        NotifyChannel::Sms => "sms",
        NotifyChannel::Whatsapp => "whatsapp",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use NotifyChannel::{Email, Sms, Whatsapp};

    #[test]
    fn a_failed_sms_points_to_email() {
        let failure = notice(Sms, vec![Email, Sms]);
        assert_eq!(failure.text, "Code could not be sent; try email");
        assert_eq!(failure.other_channels, [Email]);
    }

    #[test]
    fn the_first_other_open_channel_is_named() {
        let failure = notice(Email, vec![Email, Sms, Whatsapp]);
        assert_eq!(failure.text, "Code could not be sent; try sms");
        assert_eq!(failure.other_channels, [Sms, Whatsapp]);
    }

    #[test]
    fn with_no_other_channel_the_text_names_none() {
        let failure = notice(Email, vec![Email]);
        assert_eq!(failure.text, "Code could not be sent");
        assert!(failure.other_channels.is_empty());
    }
}
