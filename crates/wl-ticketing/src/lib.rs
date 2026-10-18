#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod config;
pub mod priority;
pub mod reasons;
pub mod seed;
pub mod sla;
mod texts;

use renox::db::Migration;
use renox::prelude::*;

pub use config::{Category, HoldReason, SlaPolicy, SlaTarget};
pub use priority::Priority;
pub use reasons::{ReasonRefused, ReassignReason, ReassignReasons};
pub use sla::{SlaCalendar, provide_sla_calendar, sla_calendar};

/// The module's permissions (wl-ticketing §6).
pub mod permissions {
    /// See tickets in scope.
    pub const VIEW: &str = "tickets.view";
    /// Raise tickets (portal roles get it for their own subjects).
    pub const CREATE: &str = "tickets.create";
    /// Triage, assign, close and cancel tickets.
    pub const ASSIGN: &str = "tickets.assign";
    /// Work assigned tickets: start, hold, resume, resolve, comment.
    pub const WORK: &str = "tickets.work";
    /// See confidential categories (complaints, grievances); each view is
    /// audited (`WRK.ACL.03`).
    pub const CONFIDENTIAL_VIEW: &str = "tickets.confidential.view";
    /// Configure categories, SLA policies and hold reasons.
    pub const MANAGE: &str = "tickets.manage";
}

/// The ticketing module. The host adds it with
/// `.module(wl_ticketing::WlTicketing::new())` when `client.toml` enables
/// `ticketing`.
#[derive(Default)]
#[non_exhaustive]
pub struct WlTicketing {
    reasons: ReassignReasons,
}

impl WlTicketing {
    /// The module with the §7 defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the reassignment reason `code` (a vertical's own, besides the
    /// five §7 defaults); `name` is its English name. Adding a code that
    /// exists replaces it.
    pub fn reassign_reason(mut self, code: &str, name: &str) -> Self {
        self.reasons.add(code, name, false);
        self
    }

    /// Like [`reassign_reason`](Self::reassign_reason), but a reassignment
    /// with this code must come with a text.
    pub fn reassign_reason_with_text(mut self, code: &str, name: &str) -> Self {
        self.reasons.add(code, name, true);
        self
    }
}

impl Module for WlTicketing {
    fn name(&self) -> &'static str {
        "wl-ticketing"
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<SlaPolicy>()
            .model::<SlaTarget>()
            .model::<HoldReason>()
            .model::<Category>();
        // The reassignment reasons: the defaults and the vertical's own.
        app.provide(self.reasons.clone());
        // The seeded defaults keep the client's changes.
        app.command(
            "ticketing:install",
            "Store the default SLA policy, hold reasons and category, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = seed::install(&state.db).await?;
                println!("{} ticketing defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { seed::install(&state.db).await.map(|_| ()) });
    }
}
