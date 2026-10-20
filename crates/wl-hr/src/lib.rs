#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod absconding;
pub mod absence;
pub mod activation;
pub mod assignees;
pub mod config;
pub mod employee;
pub mod employee_machine;
mod events;
pub mod exits;
pub mod leave;
mod notices;
pub mod onboarding;
pub mod onboarding_machine;
pub mod pages;
pub mod payroll;
pub mod qualification;
pub mod qualification_expiry;
pub mod reminders;
pub mod seed;
pub mod settlement;
pub mod steps;
pub mod template;
mod texts;
pub mod visa;
pub mod workforce;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use absence::{AbsenceCase, AbsenceReport, ContactAttempt, ContactOutcome};
pub use config::HrConfig;
pub use employee::{
    Employee, EmployeeScope, EmployeeStatus, EmploymentType, NewEmployee, change_job_role,
    correct_hire_date, set_scope,
};
pub use leave::{LeaveRequest, LeaveStatus};
pub use onboarding::{
    AppliedTemplate, Onboarding, OnboardingStatus, OnboardingStep, StartRefused, StartRequest,
    StartTrigger, StepAssignee, StepStatus,
};
pub use payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
pub use qualification::{NewQualification, Qualification, Quals};
pub use qualification_expiry::QualificationExpiry;
pub use settlement::{FinalSettlement, SettlementStatus};
pub use steps::{DoneInput, InPerson, StepRefused};
pub use template::{
    OnboardingTemplate, StepInput, StepKind, TemplateInput, TemplateRefused, TemplateStep,
};
pub use visa::{VisaCase, VisaCaseStatus};

/// The employee services under the names of the plan: `employees::create`.
pub mod employees {
    pub use crate::employee::{change_job_role, correct_hire_date, create, set_scope};
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-hr/employees.html",
        include_str!("../views/wl-hr/employees.html"),
    ),
    (
        "wl-hr/employee.html",
        include_str!("../views/wl-hr/employee.html"),
    ),
    (
        "wl-hr/templates.html",
        include_str!("../views/wl-hr/templates.html"),
    ),
];

/// The module's permissions (wl-hr §6).
pub mod permissions {
    /// See employee records in scope.
    pub const EMPLOYEES_VIEW: &str = "employees.view";
    /// Create and edit employees; activate them.
    pub const EMPLOYEES_MANAGE: &str = "employees.manage";
    /// See onboardings in scope.
    pub const ONBOARDING_VIEW: &str = "onboarding.view";
    /// Manage templates, start a manual onboarding, waive and reopen steps;
    /// its holders are the `hr` owner of steps.
    pub const ONBOARDING_MANAGE: &str = "onboarding.manage";
    /// Prepare a final settlement, and cancel it when it is not needed.
    pub const SETTLEMENTS_PREPARE: &str = "settlements.prepare";
    /// Manage visa cases, and cancel one.
    pub const VISAS_MANAGE: &str = "visas.manage";
    /// Approve the non-standard exits `cancel_onboarding`, `withdraw_notice`,
    /// `report_absconding`, `reinstate` and `record_death` (`HRM.ACL.07`,
    /// default: the MD).
    pub const EMPLOYEES_EXIT_APPROVE: &str = "employees.exit.approve";
}

/// The HR module. The host adds it with `.module(wl_hr::WlHr::new())` when
/// `client.toml` enables `hr`.
#[derive(Default)]
pub struct WlHr {
    config: HrConfig,
}

impl WlHr {
    /// The module with the §7 defaults ([`HrConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// The role whose holders are the manager of an employee in a scope.
    pub fn manager_role(mut self, role: &str) -> Self {
        self.config.manager_role = role.to_owned();
        self
    }

    /// The role that holds `onboarding.manage` as the second person for
    /// `waive` and `reopen`, and gets `hr` steps only when nobody else holds
    /// it.
    pub fn owner_role(mut self, role: &str) -> Self {
        self.config.owner_role = role.to_owned();
        self
    }

    /// Absence days before `report_absconding` is allowed (default 7).
    pub fn absence_days(mut self, days: i64) -> Self {
        self.config.absence_days = days;
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &HrConfig {
        &self.config
    }
}

impl Module for WlHr {
    fn name(&self) -> &'static str {
        "wl-hr"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::employees::IndexPage>()
            .view_data::<pages::employees::ShowPage>()
            .view_data::<pages::templates::IndexPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Employee>();
        app.model::<EmployeeScope>();
        app.model::<OnboardingTemplate>();
        app.model::<TemplateStep>();
        app.model::<Onboarding>();
        app.model::<AppliedTemplate>();
        app.model::<OnboardingStep>();
        app.model::<StepAssignee>();
        app.model::<LeaveRequest>();
        app.model::<Qualification>();
        app.model::<QualificationExpiry>();
        app.model::<AbsenceCase>();
        app.model::<ContactAttempt>();
        app.model::<AbsenceReport>();
        app.model::<PayrollRun>();
        app.model::<PayrollLine>();
        app.model::<FinalSettlement>();
        app.model::<VisaCase>();
        app.provide(self.config.clone());
        employee_machine::register(app);
        renox::state_machine::register::<Onboarding>();
        renox::state_machine::register::<OnboardingStep>();
        renox::state_machine::register::<PayrollRun>();
        renox::state_machine::register::<FinalSettlement>();
        renox::state_machine::register::<VisaCase>();
        events::register(app);
        // `HRM.ACL.07`: the policies of the five non-standard exits and the
        // audit of their requests and decisions.
        exits::register(app);
        notices::register_types();
        // `HRM.RUL.17`: the daily reminders of overdue onboarding steps.
        reminders::schedule(app);
        wl_kernel::workforce::provide_workforce(app, workforce::Staff);
        wl_kernel::workforce::provide_leave_calendar(app, workforce::Staff);
        wl_kernel::workforce::provide_qualifications(app, Quals);
        // The all staff induction template; a re-run keeps the client's changes.
        app.command(
            "hr:install",
            "Store the default onboarding template, where the database lacks one",
            |_args, state: AppState| async move {
                let installed = seed::install(&state.db).await?;
                println!("{} HR defaults created.", installed.total());
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { seed::install(&state.db).await.map(|_| ()) });
        // In the app group of the staff sidebar; the label is the app's
        // `nav.employees` text.
        app.nav(
            "app",
            NavLink::route("wl_hr.employees", "nav.employees")
                .icon("users")
                .group("nav.app")
                .order(60)
                .permission(permissions::EMPLOYEES_VIEW),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_values() {
        let module = WlHr::new();
        assert_eq!(module.config().manager_role, "manager");
        assert_eq!(module.config().owner_role, "owner");
        assert_eq!(module.config().absence_days, 7);
    }

    #[test]
    fn the_builder_changes_the_settings() {
        let module = WlHr::new().manager_role("line_manager").owner_role("md");
        assert_eq!(module.config().manager_role, "line_manager");
        assert_eq!(module.config().owner_role, "md");
    }
}
