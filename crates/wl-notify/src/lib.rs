#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod texts;
pub mod types;

use renox::db::Migration;
use renox::prelude::*;

pub use types::NotificationType;

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

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<NotificationType>();
        // The kernel's `wl:install` knows only kernel data, so the types the
        // modules register are installed by this module's own command and
        // seeder; both keep the client's changes.
        app.command(
            "notify:install",
            "Store the notification types the modules register, where the database lacks them",
            |_args, state: AppState| async move {
                let created = types::install(&state.db).await?;
                println!("{created} notification types created.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { types::install(&state.db).await.map(|_| ()) });
    }
}
