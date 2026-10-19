#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod assign;
pub mod comments;
pub mod config;
mod machine;
mod notices;
pub mod open;
pub mod pages;
pub mod priority;
pub mod reasons;
pub mod seed;
pub mod sla;
mod texts;
pub mod ticket;
pub mod work;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use assign::{AssignKind, AssignRequest, Assignment, assign, assign_as_system};
pub use comments::{Comment, CommentVisibility};
pub use config::{Category, HoldReason, SlaPolicy, SlaTarget};
pub use open::Tickets;
pub use priority::Priority;
pub use reasons::{ReasonRefused, ReassignReason, ReassignReasons};
pub use sla::{SlaCalendar, provide_sla_calendar, sla_calendar};
pub use ticket::{Ticket, TicketStatus};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-ticketing/index.html",
        include_str!("../views/wl-ticketing/index.html"),
    ),
    (
        "wl-ticketing/new.html",
        include_str!("../views/wl-ticketing/new.html"),
    ),
    (
        "wl-ticketing/show.html",
        include_str!("../views/wl-ticketing/show.html"),
    ),
    (
        "wl-ticketing/_assign_sheet.html",
        include_str!("../views/wl-ticketing/_assign_sheet.html"),
    ),
    (
        "wl-ticketing/_assignments.html",
        include_str!("../views/wl-ticketing/_assignments.html"),
    ),
    (
        "wl-ticketing/_comments.html",
        include_str!("../views/wl-ticketing/_comments.html"),
    ),
];

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

/// Whether the manifest enables `invoicing` (false when the kernel module is
/// not part of the app).
fn invoicing_on(state: &AppState) -> bool {
    state
        .provided::<std::sync::Arc<wl_kernel::Manifest>>()
        .is_some_and(|manifest| manifest.is_enabled("invoicing"))
}

impl Module for WlTicketing {
    fn name(&self) -> &'static str {
        "wl-ticketing"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // In the app group of the staff sidebar; the label is the app's
        // `nav.tickets` text.
        app.nav(
            "app",
            NavLink::route("wl_ticketing.index", "nav.tickets")
                .icon("ticket")
                .group("nav.app")
                .order(50)
                .permission(permissions::VIEW),
        );
        app.model::<SlaPolicy>()
            .model::<SlaTarget>()
            .model::<HoldReason>()
            .model::<Category>()
            .model::<Ticket>()
            .model::<Assignment>()
            .model::<Comment>();
        // `rnx state:diagram Ticket` and the admin panel find the machine.
        renox::state_machine::register::<Ticket>();
        // `TicketClosed` after a `confirm` or `close` commits.
        work::register_events(app);
        // `TicketAssigned` and the notices after an `assign` commits.
        notices::register(app);
        // A ticket's scope is its subject's (the approvals and wl-notify find
        // the people responsible through it).
        ticket::register_scope();
        // Other modules open tickets through the kernel's `TaskCreator`.
        wl_kernel::tickets::provide_task_creator(app, Tickets);
        // The reassignment reasons: the defaults and the vertical's own.
        app.provide(self.reasons.clone());
        // The seeded defaults keep the client's changes.
        app.command(
            "ticketing:install",
            "Store the default SLA policy, hold reasons and categories, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = seed::install_with(&state.db, invoicing_on(&state)).await?;
                println!("{} ticketing defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move {
            seed::install_with(&state.db, invoicing_on(&state))
                .await
                .map(|_| ())
        });
    }
}
