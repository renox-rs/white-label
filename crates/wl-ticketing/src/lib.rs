#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod assign;
pub mod auto_close;
pub mod categories;
pub mod category_pages;
pub mod charge;
pub mod comments;
pub mod config;
pub mod flags;
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
pub mod visibility;
pub mod work;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use assign::{AssignKind, AssignRequest, Assignment, assign, assign_as_system};
pub use charge::{ChargeInput, ChargeLine};
pub use comments::{Comment, CommentVisibility};
pub use flags::ComplianceFlag;
pub use config::{Category, CustomField, FieldKind, HoldReason, InvoiceOn, SlaPolicy, SlaTarget};
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
        "wl-ticketing/categories.html",
        include_str!("../views/wl-ticketing/categories.html"),
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
        "wl-ticketing/_charges.html",
        include_str!("../views/wl-ticketing/_charges.html"),
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
    auto_close_days: Option<i64>,
}

impl WlTicketing {
    /// The module with the §7 defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the days after `resolve` before the system confirms a ticket
    /// whose requester did nothing (`WRK.RUL.02`, §7: 3 unless set). At
    /// least 1.
    ///
    /// # Panics
    ///
    /// For fewer than one day: a programming error.
    pub fn auto_close_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "auto-close needs at least one day");
        self.auto_close_days = Some(days);
        self
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

/// The modules the manifest enables whose platform categories are seeded
/// (all off when the kernel module is not part of the app).
fn enabled_modules(state: &AppState) -> seed::Enabled {
    let manifest = state.provided::<std::sync::Arc<wl_kernel::Manifest>>();
    let on = |name: &str| {
        manifest
            .as_ref()
            .is_some_and(|manifest| manifest.is_enabled(name))
    };
    seed::Enabled {
        invoicing: on("invoicing"),
        hr: on("hr"),
        delivery: on("delivery"),
    }
}

impl Module for WlTicketing {
    fn name(&self) -> &'static str {
        "wl-ticketing"
    }

    fn routes(&self) -> Routes {
        pages::routes().merge(category_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<category_pages::CategoriesPage>();
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
        app.nav(
            "app",
            NavLink::route("wl_ticketing.categories", "nav.ticket_categories")
                .icon("tag")
                .group("nav.admin")
                .order(55)
                .permission(permissions::MANAGE),
        );
        app.model::<SlaPolicy>()
            .model::<SlaTarget>()
            .model::<HoldReason>()
            .model::<Category>()
            .model::<Ticket>()
            .model::<Assignment>()
            .model::<Comment>()
            .model::<ChargeLine>()
            .model::<ComplianceFlag>();
        // `rnx state:diagram Ticket` and the admin panel find the machine.
        renox::state_machine::register::<Ticket>();
        // `TicketClosed` after a `confirm` or `close` commits.
        work::register_events(app);
        // The invoice draft of a chargeable ticket, after `resolve` or at
        // close, as its category says.
        charge::register_events(app);
        // A `visit_missed` compliance flag clears when its visit is resolved.
        flags::register_events(app);
        // `TicketAssigned` and the notices after an `assign` commits.
        notices::register(app);
        // A ticket's scope is its subject's (the approvals and wl-notify find
        // the people responsible through it).
        ticket::register_scope();
        // Other modules open tickets through the kernel's `TaskCreator`.
        wl_kernel::tickets::provide_task_creator(app, Tickets);
        // The reassignment reasons: the defaults and the vertical's own.
        app.provide(self.reasons.clone());
        // `WRK.RUL.02`: the system confirms a resolved ticket whose
        // requester did nothing for N days.
        app.provide(auto_close::AutoCloseDays(
            self.auto_close_days.unwrap_or(auto_close::DEFAULT_DAYS),
        ));
        auto_close::schedule(app);
        // The seeded defaults keep the client's changes.
        app.command(
            "ticketing:install",
            "Store the default SLA policy, hold reasons and categories, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = seed::install_for(&state.db, enabled_modules(&state)).await?;
                println!("{} ticketing defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move {
            seed::install_for(&state.db, enabled_modules(&state))
                .await
                .map(|_| ())
        });
    }
}
