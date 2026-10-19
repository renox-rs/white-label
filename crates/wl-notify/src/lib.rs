#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod alert;
pub mod alternative;
pub mod channels;
pub mod contact_invalid;
pub mod give_up;
mod machine;
pub mod mask;
pub mod message;
pub mod notices;
pub mod notifier;
pub mod pages;
pub mod reactions;
pub mod recipients;
pub mod render;
pub mod send;
pub mod staff_task;
pub mod suppressions;
mod texts;
pub mod types;

use std::sync::Arc;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use alternative::AlternativeRecipientHandler;
pub use channels::{
    ChannelProvider, EmailProvider, HardError, Outgoing, Providers, SendError, SendErrorKind,
    SmsGateway, SmsProvider,
};
pub use contact_invalid::MarkContactInvalidHandler;
pub use give_up::{ReactionRun, ReactionStep, RunGiveUpReaction, RunStatus};
pub use machine::{
    DEFER, DELIVER, FAIL, FALL_BACK, GIVE_UP, MARK_READ, RELEASE, RETRY, SEND, SUPPRESS,
};
pub use message::{Message, MessageStatus, RecipientKind};
pub use notices::{ProofFile, ProofInput, ProofRefusal, StatutoryNotice, validate_proof};
pub use notifier::Messages;
pub use reactions::{
    GiveUpContext, GiveUpHandler, ReactionOutcome, Reactions, RecordingHandler, StepOutcome,
};
pub use send::SendMessage;
pub use staff_task::{StaffTask, StaffTaskHandler, open_give_up_task};
pub use suppressions::{Suppression, SuppressionReason};
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

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-notify/types.html",
        include_str!("../views/wl-notify/types.html"),
    ),
    (
        "wl-notify/type_edit.html",
        include_str!("../views/wl-notify/type_edit.html"),
    ),
    (
        "wl-notify/log.html",
        include_str!("../views/wl-notify/log.html"),
    ),
];

/// The notifications module. The host adds it with
/// `.module(wl_notify::WlNotify::new())` when `client.toml` enables `notify`.
#[derive(Default)]
#[non_exhaustive]
pub struct WlNotify {
    providers: Providers,
    reactions: Reactions,
}

impl WlNotify {
    /// The module with its built-in channel providers and give-up reactions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sends `provider`'s channel through `provider`, in place of the
    /// built-in one (another SMS gateway, a scripted one in tests).
    pub fn provider(mut self, provider: impl ChannelProvider) -> Self {
        self.providers.replace(Arc::new(provider));
        self
    }

    /// Runs `handler`'s give-up reaction through `handler`, in place of the
    /// built-in one (`staff_task` opens a task, `alternative_recipient`
    /// queues the message to another contact, `mark_contact_invalid` marks
    /// the contact point invalid, the other only records
    /// their step) in the `NTF.RUL.01` run.
    pub fn reaction(mut self, handler: impl GiveUpHandler) -> Self {
        self.reactions.replace(Arc::new(handler));
        self
    }
}

impl Module for WlNotify {
    fn name(&self) -> &'static str {
        "wl-notify"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<NotificationType>()
            .model::<Message>()
            .model::<Suppression>()
            .model::<ReactionRun>()
            .model::<ReactionStep>()
            .model::<StatutoryNotice>()
            .view_data::<pages::TypesPage>()
            .view_data::<pages::TypeEditPage>()
            .view_data::<pages::LogPage>();
        // `rnx state:diagram Message` and the admin panel find the machine.
        renox::state_machine::register::<Message>();
        // `MessageSuppressed` after a `suppress` commits.
        machine::register(app);
        // Sending: the providers, the `sms` channel and the send job.
        app.provide(self.providers.clone());
        channels::register(app);
        app.job::<SendMessage>();
        // Fallback and give-up: `NTF.RUL.01` runs the type's reactions.
        app.provide(self.reactions.clone());
        app.job::<RunGiveUpReaction>();
        // Other modules send through the kernel (wl-notify §6).
        wl_kernel::notify::provide_notifier(app, Messages);
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
        // In the admin group of the staff sidebar; the labels are the app's
        // `nav.notification_types` and `nav.notification_log` texts.
        app.nav(
            "app",
            NavLink::route("wl_notify.types", "nav.notification_types")
                .icon("bell")
                .group("nav.admin")
                .order(40)
                .permission(permissions::TEMPLATES_MANAGE),
        );
        app.nav(
            "app",
            NavLink::route("wl_notify.log", "nav.notification_log")
                .icon("mail")
                .group("nav.admin")
                .order(41)
                .permission(permissions::LOG_VIEW),
        );
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
    }
}
