//! The one-time-code failure notice (wl-notify §5.4, #60 A6).
//!
//! An OTP type's default give-up reaction is `[none]`: no task, no alert.
//! The person is waiting on the login screen, so the screen tells them
//! at once. wl-portal calls [`failure_notice`] when sending the code on
//! `failed_channel` failed.

use renox::prelude::*;
use wl_kernel::notify::NotifyChannel;

use crate::channels::open_channels;
use crate::texts::{english, english_with};

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
