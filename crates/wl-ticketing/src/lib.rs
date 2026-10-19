#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod assign;
pub mod auto_close;
pub mod categories;
pub mod category_pages;
pub mod charge;
pub mod comments;
pub mod config;
pub mod contract;
pub mod contract_end;
mod contract_machine;
pub mod contract_pages;
pub mod flag_pages;
pub mod flags;
pub mod job_cancel;
mod machine;
mod notices;
pub mod no_show;
pub mod open;
pub mod pages;
pub mod plans;
pub mod priority;
pub mod reasons;
pub mod seed;
pub mod sla;
mod texts;
pub mod ticket;
pub mod vendor_block;
pub mod vendor_flags;
pub mod vendor_guard;
pub mod visibility;
pub mod visit_check;
pub mod work;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use assign::{AssignKind, AssignRequest, Assignment, assign, assign_as_system};
pub use charge::{ChargeInput, ChargeLine};
pub use comments::{Comment, CommentVisibility};
pub use flags::ComplianceFlag;
pub use config::{Category, CustomField, FieldKind, HoldReason, InvoiceOn, SlaPolicy, SlaTarget};
pub use contract::{
    ContractStatus, Coverage, NewContract, ServiceContract, ServiceContracts,
};
pub use no_show::NoShow;
pub use open::Tickets;
pub use plans::{MaintenancePlan, MaintenancePlans, PlanStatus};
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
        "wl-ticketing/compliance_flags.html",
        include_str!("../views/wl-ticketing/compliance_flags.html"),
    ),
    (
        "wl-ticketing/contract.html",
        include_str!("../views/wl-ticketing/contract.html"),
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
    /// See service contracts and their coverage.
    pub const SERVICE_CONTRACTS_VIEW: &str = "service_contracts.view";
    /// Create, activate, renew and terminate service contracts.
    pub const SERVICE_CONTRACTS_MANAGE: &str = "service_contracts.manage";
    /// See and manage maintenance plans.
    pub const MAINTENANCE_PLANS_MANAGE: &str = "maintenance_plans.manage";
}

/// The ticketing module. The host adds it with
/// `.module(wl_ticketing::WlTicketing::new())` when `client.toml` enables
/// `ticketing`.
#[derive(Default)]
#[non_exhaustive]
pub struct WlTicketing {
    reasons: ReassignReasons,
    auto_close_days: Option<i64>,
    no_show_hours: Option<i64>,
    visit_check: visit_check::Settings,
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

    /// Sets the clock hours after a visit's scheduled time before a vendor
    /// that has not started the ticket is recorded as a no-show
    /// (`WRK.RUL.19`, §6: 2 unless set). At least 1.
    ///
    /// # Panics
    ///
    /// For fewer than one hour: a programming error.
    pub fn no_show_hours(mut self, hours: i64) -> Self {
        assert!(hours >= 1, "a no-show needs at least one hour");
        self.no_show_hours = Some(hours);
        self
    }

    /// Sets the days after a planned visit's planned due date before
    /// `WRK.RUL.14` reports it (§6: 3 unless set). At least 1.
    ///
    /// # Panics
    ///
    /// For fewer than one day: a programming error.
    pub fn visit_missed_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "a missed visit needs at least one day");
        self.visit_check.days = days;
        self
    }

    /// Sets the role told of a missed planned visit (`coordinator` unless
    /// set).
    pub fn coordinator_role(mut self, role: &str) -> Self {
        self.visit_check.coordinator_role = role.to_owned();
        self
    }

    /// Sets the role of the manager in scope, told of a missed visit of a
    /// compliance-critical category (the company administrators unless set;
    /// 01: the property manager).
    pub fn compliance_manager_role(mut self, role: &str) -> Self {
        self.visit_check.manager_role = role.to_owned();
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
        pages::routes()
            .merge(category_pages::routes())
            .merge(flag_pages::routes())
            .merge(contract_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<category_pages::CategoriesPage>()
            .view_data::<flag_pages::FlagsPage>()
            .view_data::<contract_pages::ContractPage>();
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
        app.nav(
            "app",
            NavLink::route("wl_ticketing.compliance_flags", "nav.compliance_flags")
                .icon("shield-alert")
                .group("nav.app")
                .order(56)
                .permission(permissions::VIEW),
        );
        app.model::<SlaPolicy>()
            .model::<SlaTarget>()
            .model::<HoldReason>()
            .model::<Category>()
            .model::<Ticket>()
            .model::<Assignment>()
            .model::<Comment>()
            .model::<ChargeLine>()
            .model::<ComplianceFlag>()
            .model::<ServiceContract>()
            .model::<Coverage>()
            .model::<MaintenancePlan>()
            .model::<NoShow>();
        // `rnx state:diagram Ticket` and the admin panel find the machine.
        renox::state_machine::register::<Ticket>();
        renox::state_machine::register::<ServiceContract>();
        renox::state_machine::register::<MaintenancePlan>();
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
        contract::register_scope();
        // Other modules open tickets through the kernel's `TaskCreator`.
        wl_kernel::tickets::provide_task_creator(app, Tickets);
        // The invoicing match asks whether a contract covers a day of work.
        wl_kernel::tickets::provide_service_contract_coverage(app, contract::ContractCoverage);
        // The reassignment reasons: the defaults and the vertical's own.
        app.provide(self.reasons.clone());
        // `WRK.RUL.02`: the system confirms a resolved ticket whose
        // requester did nothing for N days.
        app.provide(auto_close::AutoCloseDays(
            self.auto_close_days.unwrap_or(auto_close::DEFAULT_DAYS),
        ));
        auto_close::schedule(app);
        // `WRK.RUL.14`: planned visits not resolved N days after their
        // planned due date; compliance-critical (`PLT.RUL.04`).
        app.provide(self.visit_check.clone());
        visit_check::register(app);
        // `WRK.RUL.18`: what happens when a service contract ends;
        // compliance-critical (`PLT.RUL.04`).
        contract_end::register(app);
        // `WRK.ACL.11`: the vendor document guards of `resolve` and
        // contract `activate`.
        vendor_guard::register();
        // `DocumentExpired` and `DocumentAccepted` of a guarding vendor
        // document flag and unflag the contracts, visits and subjects.
        vendor_flags::register(app);
        // A blocked vendor's service contracts are reviewed, not ended
        // (`PTY.ACL.03`).
        vendor_block::register();
        // `WRK.RUL.19`: a vendor that did not start a ticket in time.
        app.provide(no_show::NoShowHours(
            self.no_show_hours.unwrap_or(no_show::DEFAULT_HOURS),
        ));
        no_show::register(app);
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
