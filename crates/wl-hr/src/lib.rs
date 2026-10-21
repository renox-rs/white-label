#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod absconding;
pub mod absence;
pub mod activation;
pub mod adjustment;
pub mod assignees;
pub mod calculation;
pub mod caps;
pub mod config;
pub mod differences;
pub mod employee;
pub mod employee_machine;
pub mod establishment;
mod events;
pub mod exits;
pub mod hours;
pub mod leave;
mod notices;
pub mod onboarding;
pub mod onboarding_machine;
pub mod pages;
pub mod pay_account;
mod pay_account_events;
pub mod pay_account_use;
pub mod payroll;
mod payroll_events;
pub mod qualification;
pub mod qualification_expiry;
pub mod receivable;
pub mod recovery;
pub mod release;
pub mod reminders;
pub mod salary;
pub mod seed;
pub mod settlement;
pub mod settlement_alerts;
pub mod settlement_amounts;
pub mod settlement_close;
pub mod settlement_dispute;
mod settlement_events;
pub mod settlement_offset;
pub mod settlement_pay;
pub mod settlement_payment;
pub mod sif;
pub mod steps;
pub mod supplementary;
pub mod template;
mod texts;
pub mod visa;
pub mod workforce;
pub mod wps_alerts;
pub mod wps_partial;
pub mod wps_rejection;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use absence::{AbsenceCase, AbsenceReport, ContactAttempt, ContactOutcome};
pub use adjustment::{Adjustment, AdjustmentKind};
pub use calculation::PayrollHour;
pub use config::HrConfig;
pub use differences::HourDifference;
pub use employee::{
    Employee, EmployeeScope, EmployeeStatus, EmploymentType, NewEmployee, change_job_role,
    correct_hire_date, set_scope,
};
pub use establishment::Establishment;
pub use leave::{LeaveRequest, LeaveStatus};
pub use onboarding::{
    AppliedTemplate, Onboarding, OnboardingStatus, OnboardingStep, StartRefused, StartRequest,
    StartTrigger, StepAssignee, StepStatus,
};
pub use pay_account::{NewPayAccount, PayAccount, PayAccountStatus};
pub use payroll::{LineStatus, PayrollLine, PayrollRun, PayrollRunStatus};
pub use qualification::{NewQualification, Qualification, Quals};
pub use qualification_expiry::QualificationExpiry;
pub use receivable::Receivable;
pub use salary::SalaryStructure;
pub use settlement::{FinalSettlement, SettlementStatus};
pub use settlement_payment::SettlementPayment;
pub use sif::{SifFile, SifLine};
pub use steps::{DoneInput, InPerson, StepRefused};
pub use template::{
    OnboardingTemplate, StepInput, StepKind, TemplateInput, TemplateRefused, TemplateStep,
};
pub use visa::{VisaCase, VisaCaseStatus, VisaCaseStep};

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
        "wl-hr/payroll_run.html",
        include_str!("../views/wl-hr/payroll_run.html"),
    ),
    (
        "wl-hr/templates.html",
        include_str!("../views/wl-hr/templates.html"),
    ),
    (
        "wl-hr/pay_account.html",
        include_str!("../views/wl-hr/pay_account.html"),
    ),
    (
        "wl-hr/pay_account_stop.html",
        include_str!("../views/wl-hr/pay_account_stop.html"),
    ),
    (
        "wl-hr/settlement.html",
        include_str!("../views/wl-hr/settlement.html"),
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
    /// Prepare a final settlement (enter its amounts, also the revised ones
    /// of a dispute), dispute it, and cancel it when it is not needed.
    pub const SETTLEMENTS_PREPARE: &str = "settlements.prepare";
    /// Approve a final settlement, and resolve its dispute (`HRM.ACL.04`: not
    /// the user who entered the amounts).
    pub const SETTLEMENTS_APPROVE: &str = "settlements.approve";
    /// Pay a final settlement and record payments against it.
    pub const SETTLEMENTS_PAY: &str = "settlements.pay";
    /// Manage visa cases, and cancel one.
    pub const VISAS_MANAGE: &str = "visas.manage";
    /// See payroll runs, their lines and SIF files.
    pub const PAYROLL_VIEW: &str = "payroll.view";
    /// Calculate, submit and record the bank's results of a payroll run
    /// (`HRM.ACL.04`: not the same user as the approver).
    pub const PAYROLL_PREPARE: &str = "payroll.prepare";
    /// Approve or return a payroll run, comment on its lines and confirm
    /// payment (`HRM.ACL.04`).
    pub const PAYROLL_APPROVE: &str = "payroll.approve";
    /// Approve or reject a new pay account (`HRM.ACL.05`: not the requester).
    pub const PAY_ACCOUNTS_APPROVE: &str = "pay_accounts.approve";
    /// Add advances, recoveries and bonuses.
    pub const ADJUSTMENTS_MANAGE: &str = "adjustments.manage";
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

    /// The role told, with the accountant, of a WPS rejection still open before
    /// the due date (`HRM.RUL.11`; default `owner`, the MD).
    pub fn md_role(mut self, role: &str) -> Self {
        self.config.md_role = role.to_owned();
        self
    }

    /// The accountant role told of it (default `accountant`).
    pub fn accountant_role(mut self, role: &str) -> Self {
        self.config.accountant_role = role.to_owned();
        self
    }

    /// N of `HRM.RUL.11`: days before the WPS due date (default 1).
    pub fn wps_escalation_days(mut self, days: i64) -> Self {
        self.config.wps_escalation_days = days;
        self
    }

    /// The role whose holders get the tasks for HR (default `hr`).
    pub fn hr_role(mut self, role: &str) -> Self {
        self.config.hr_role = role.to_owned();
        self
    }

    /// The cooling period of a new pay account in days (default 5,
    /// `HRM.ACL.05`).
    pub fn cooling_days(mut self, days: i64) -> Self {
        self.config.cooling_days = days;
        self
    }

    /// N of `HRM.RUL.09`: days after the last working day at which HR and the
    /// accountant are told the final settlement is due (default 10).
    pub fn settlement_alert_days(mut self, days: i64) -> Self {
        self.config.settlement_alert_days = days;
        self
    }

    /// Days after `pay` at which a settlement without a dispute closes
    /// (default 30).
    pub fn settlement_close_days(mut self, days: i64) -> Self {
        self.config.settlement_close_days = days;
        self
    }

    /// Days before the current residence visa expires at which the task after
    /// a refused renewal is due (default 30).
    pub fn visa_rejection_task_days(mut self, days: i64) -> Self {
        self.config.visa_rejection_task_days = days;
        self
    }

    /// The routing codes a pay account may name; none listed accepts any code
    /// of the right form.
    pub fn routing_codes(mut self, codes: &[&str]) -> Self {
        self.config.routing_codes = codes.iter().map(|c| (*c).to_owned()).collect();
        self
    }

    /// The payroll cut-off day of the month (default 25).
    pub fn cut_off_day(mut self, day: u32) -> Self {
        self.config.cut_off_day = day;
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
            .view_data::<pages::payroll::ShowPage>()
            .view_data::<pages::pay_accounts::ShowPage>()
            .view_data::<pages::pay_accounts::StopPage>()
            .view_data::<pages::settlements::ShowPage>()
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
        app.model::<SifFile>();
        app.model::<SifLine>();
        app.model::<Establishment>();
        app.model::<Adjustment>();
        app.model::<SalaryStructure>();
        app.model::<PayrollHour>();
        app.model::<HourDifference>();
        app.model::<Receivable>();
        app.model::<FinalSettlement>();
        app.model::<SettlementPayment>();
        app.model::<VisaCase>();
        app.model::<VisaCaseStep>();
        app.model::<PayAccount>();
        app.provide(self.config.clone());
        employee_machine::register(app);
        renox::state_machine::register::<Onboarding>();
        renox::state_machine::register::<OnboardingStep>();
        renox::state_machine::register::<PayrollRun>();
        renox::state_machine::register::<FinalSettlement>();
        renox::state_machine::register::<VisaCase>();
        events::register(app);
        visa::register(app);
        settlement_events::register(app);
        // `HRM.ACL.05`: the pay account's machine, the MD's waiver policy, its
        // notices, the daily activation.
        pay_account::register(app, &self.config.md_role);
        // `HRM.ACL.07`: the policies of the five non-standard exits and the
        // audit of their requests and decisions.
        exits::register(app);
        notices::register_types();
        // `HRM.RUL.17`: the daily reminders of overdue onboarding steps.
        reminders::schedule(app);
        // `HRM.RUL.11`: the daily escalation of WPS rejections still open.
        wps_alerts::schedule(app);
        // `HRM.RUL.09`: the 14-day alerts of the final settlement.
        settlement_alerts::schedule(app);
        // A paid settlement closes after the period to dispute.
        settlement_close::schedule(app);
        // `HRM.RUL.12`: the daily proposal of off-cycle runs for released held lines.
        release::schedule(app);
        wl_kernel::workforce::provide_workforce(app, workforce::Staff);
        wl_kernel::workforce::provide_leave_calendar(app, workforce::Staff);
        wl_kernel::qualifications::provide_qualifications(app, Quals);
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
        assert_eq!(module.config().md_role, "owner");
        assert_eq!(module.config().accountant_role, "accountant");
        assert_eq!(module.config().wps_escalation_days, 1);
    }

    #[test]
    fn the_builder_changes_the_settings() {
        let module = WlHr::new().manager_role("line_manager").owner_role("md");
        assert_eq!(module.config().manager_role, "line_manager");
        assert_eq!(module.config().owner_role, "md");
    }
}
