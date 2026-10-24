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
pub mod deadline;
pub mod deadline_machine;
pub mod deadline_pages;
pub mod deadlines;
pub mod flag_pages;
pub mod flags;
pub mod job_cancel;
pub mod late;
mod machine;
pub mod no_show;
mod notices;
pub mod open;
pub mod pages;
pub mod plans;
pub mod priority;
pub mod reasons;
pub mod role_queue;
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
pub use config::{Category, CustomField, FieldKind, HoldReason, InvoiceOn, SlaPolicy, SlaTarget};
pub use contract::{ContractStatus, Coverage, NewContract, ServiceContract, ServiceContracts};
pub use deadline::{DeadlineTask, TaskSource};
pub use deadlines::Deadlines;
pub use flags::ComplianceFlag;
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
    (
        "wl-ticketing/deadline_tasks.html",
        include_str!("../views/wl-ticketing/deadline_tasks.html"),
    ),
    (
        "wl-ticketing/deadline_task.html",
        include_str!("../views/wl-ticketing/deadline_task.html"),
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
    /// See deadline tasks (§4.4).
    pub const TASKS_VIEW: &str = "tasks.view";
    /// Work deadline tasks: start, wait, unblock, submit for review and
    /// complete one without a reviewer.
    pub const TASKS_WORK: &str = "tasks.work";
    /// Review deadline tasks: send back or complete one in review.
    pub const TASKS_REVIEW: &str = "tasks.review";
    /// Configure deadline tasks, waive them and close them as missed.
    pub const TASKS_MANAGE: &str = "tasks.manage";
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
    deadlines: deadlines::Settings,
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

    /// Sets whether a started deadline task (`in_progress` or `waiting`) may
    /// be waived (§7 *waive a started task*: on unless set; 04: off, a
    /// started task that no longer applies is listed for the manager).
    pub fn waive_started_tasks(mut self, allowed: bool) -> Self {
        self.deadlines.waive_started = allowed;
        self
    }

    /// Sets the role told of overdue deadline tasks and given the lists on
    /// release and on the end of an agreement (the company administrators
    /// unless set; 04: the manager).
    pub fn task_manager_role(mut self, role: &str) -> Self {
        self.deadlines.manager_role = role.to_owned();
        self
    }

    /// Sets the vertical's label of the `waiting` state of a deadline task
    /// (the module's own *Waiting* unless set; 04: *Waiting on client*).
    pub fn waiting_label(mut self, label: &str) -> Self {
        self.deadlines.waiting_label = Some(label.trim().to_owned());
        self
    }

    /// Sets the rule id the nightly `mark_overdue` of deadline tasks runs
    /// and is recorded under (`WRK.RUL.05` unless set; 04: `ACC.RUL.04`).
    /// The vertical marks that id compliance-critical, so a failed or missed
    /// run alerts its owner role.
    ///
    /// # Panics
    ///
    /// For a blank id: a programming error.
    pub fn overdue_rule_id(mut self, rule: &str) -> Self {
        assert!(!rule.trim().is_empty(), "the overdue rule needs an id");
        self.deadlines.overdue_rule = rule.trim().to_owned();
        self
    }

    /// Adds a role told when a deadline task goes overdue, besides its
    /// owners and the task manager role (holders in the source's scope; none
    /// unless set; 04 `ACC.RUL.04`: the partner).
    pub fn task_overdue_role(mut self, role: &str) -> Self {
        if !self.deadlines.overdue_roles.iter().any(|r| r == role) {
            self.deadlines.overdue_roles.push(role.to_owned());
        }
        self
    }

    /// Sets the days before a deadline task's due date its owner (and, for a
    /// task that needs client input, the client) is reminded (none unless
    /// set; 04 `ACC.RUL.01`: 30, 14, 7 and 3).
    ///
    /// # Panics
    ///
    /// For a negative day: a programming error.
    pub fn task_reminder_days(mut self, days: &[i64]) -> Self {
        assert!(
            days.iter().all(|d| *d >= 0),
            "reminder days are not negative"
        );
        self.deadlines.reminder_days = days.to_vec();
        self
    }

    /// Adds the reason `code` to the list `close_missed` takes from (§4.4;
    /// any reason text while the list is empty; 04 §7.1: its four reasons);
    /// `name` is its English name, stored as the task's reason. Adding a
    /// code that exists replaces it.
    pub fn missed_reason(mut self, code: &str, name: &str) -> Self {
        self.add_missed_reason(code, name, false);
        self
    }

    /// Like [`missed_reason`](Self::missed_reason), but closing with this
    /// reason needs a text, stored as "name: text" (*other*).
    pub fn missed_reason_with_text(mut self, code: &str, name: &str) -> Self {
        self.add_missed_reason(code, name, true);
        self
    }

    fn add_missed_reason(&mut self, code: &str, name: &str, needs_text: bool) {
        let reason = deadlines::MissedReason {
            code: code.trim().to_owned(),
            name: name.trim().to_owned(),
            needs_text,
        };
        let reasons = &mut self.deadlines.missed_reasons;
        match reasons.iter_mut().find(|r| r.code == reason.code) {
            Some(found) => *found = reason,
            None => reasons.push(reason),
        }
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
            .merge(deadline_pages::routes())
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
            .view_data::<contract_pages::ContractPage>()
            .view_data::<deadline_pages::TasksPage>()
            .view_data::<deadline_pages::TaskPage>();
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
                .icon("clipboard-list")
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
                .icon("triangle-alert")
                .group("nav.app")
                .order(56)
                .permission(permissions::VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_ticketing.deadline_tasks", "nav.deadline_tasks")
                .icon("calendar-check")
                .group("nav.app")
                .order(57)
                .permission(permissions::TASKS_VIEW),
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
            .model::<NoShow>()
            .model::<DeadlineTask>()
            .model::<TaskSource>();
        // `rnx state:diagram Ticket` and the admin panel find the machine.
        renox::state_machine::register::<Ticket>();
        renox::state_machine::register::<ServiceContract>();
        renox::state_machine::register::<MaintenancePlan>();
        renox::state_machine::register::<DeadlineTask>();
        // `TicketClosed` after a `confirm` or `close` commits.
        work::register_events(app);
        // The invoice draft of a chargeable ticket, after `resolve` or at
        // close, as its category says.
        charge::register_events(app);
        // A `visit_missed` compliance flag clears when its visit is resolved.
        flags::register_events(app);
        // `TicketAssigned` and the notices after an `assign` commits.
        notices::register(app);
        // BL-TKT-12: the notice that a ticket found nobody in its role.
        role_queue::register();
        // BL-TKT-13: the ticket's late-entry definition (`reported_at`).
        late::register();
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
        // Deadline tasks (§4.4): the next occurrence, events and notices after
        // each transition, `WRK.RUL.05` and `WRK.RUL.06`, and the kernel's
        // `DeadlineTasks` for the verticals.
        app.provide(self.deadlines.clone());
        deadlines::register(app);
        wl_kernel::deadlines::provide_deadline_tasks(app, Deadlines);
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
