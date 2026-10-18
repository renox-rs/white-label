#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod texts;

use renox::prelude::*;

/// The module's permissions (wl-notify §6).
pub mod permissions {
    /// See the delivery log for records in scope.
    pub const LOG_VIEW: &str = "notifications.log.view";
    /// Resend a message (same or another channel).
    pub const RESEND: &str = "notifications.resend";
    /// Edit templates and submit WhatsApp templates.
    pub const TEMPLATES_MANAGE: &str = "notifications.templates.manage";
    /// Configure providers, sender identity, quiet hours.
    pub const CHANNELS_MANAGE: &str = "notifications.channels.manage";
    /// Change a party's preferences on their request.
    pub const PREFERENCES_MANAGE: &str = "notifications.preferences.manage";
    /// Send a manual or bulk message (Later).
    pub const BROADCAST_SEND: &str = "notifications.broadcast.send";
    /// Approve, cancel and stop marketing broadcasts.
    pub const MARKETING_SEND: &str = "notifications.marketing.send";
}

/// The notifications module. The host adds it with
/// `.module(wl_notify::WlNotify::new())` when `client.toml` enables `notify`.
#[derive(Default)]
#[non_exhaustive]
pub struct WlNotify {}

impl WlNotify {
    /// The module with its built-in channel providers and give-up reactions.
    pub fn new() -> Self {
        Self::default()
    }
}

impl Module for WlNotify {
    fn name(&self) -> &'static str {
        "wl-notify"
    }
}
