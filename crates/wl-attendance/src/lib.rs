#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod access;
pub mod adjustment;
pub mod after_last_day;
pub mod approval;
pub mod checks;
pub mod closing;
pub mod config;
pub mod crew;
pub mod crews_affected;
pub mod deem;
pub mod dispute;
pub mod entry;
pub mod escalation;
pub mod flags;
pub mod history;
pub mod location;
pub mod lock;
pub mod midday;
mod notices;
pub mod overlap;
pub mod overtime;
pub mod pages;
pub mod payroll;
pub mod period_end;
pub mod permit;
pub mod proposal;
pub mod reconcile;
pub mod record;
pub mod review;
pub mod service;
pub mod submit;
mod texts;
pub mod timesheet;
pub mod timesheet_pages;
mod timesheet_texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use config::AttendanceConfig;
pub use entry::TimesheetEntry;
pub use flags::{AttendanceFlag, FlagType};
pub use location::WorkLocation;
pub use midday::MiddayWindow;
pub use record::{
    AttendanceMethod, AttendanceRecord, AttendanceStatus, BreakSpan, FLAG as FLAG_TRANSITION,
};
pub use service::NewRecord;
pub use timesheet::{Timesheet, TimesheetStatus};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-attendance/crew.html",
        include_str!("../views/wl-attendance/crew.html"),
    ),
    (
        "wl-attendance/timesheets.html",
        include_str!("../views/wl-attendance/timesheets.html"),
    ),
    (
        "wl-attendance/timesheet.html",
        include_str!("../views/wl-attendance/timesheet.html"),
    ),
    (
        "wl-attendance/crew_form.html",
        include_str!("../views/wl-attendance/crew_form.html"),
    ),
];

/// The module's permissions (wl-attendance §6).
pub mod permissions {
    /// See attendance in scope.
    pub const VIEW: &str = "attendance.view";
    /// Mark attendance for crews and employees in scope (supervisor).
    pub const MARK: &str = "attendance.mark";
    /// Check in and out for oneself.
    pub const SELF_CHECK_IN: &str = "attendance.self";
    /// Accept or reject flagged records.
    pub const REVIEW: &str = "attendance.review";
    /// Correct times within the correction window.
    pub const CORRECT: &str = "attendance.correct";
    /// Create locations and geofences.
    pub const WORK_LOCATIONS_MANAGE: &str = "work_locations.manage";
    /// Submit a timesheet for approval.
    pub const TIMESHEETS_SUBMIT: &str = "timesheets.submit";
    /// Open an adjustment timesheet on a locked one (`HRM.ACL.08`).
    pub const TIMESHEETS_ADJUST: &str = "timesheets.adjust";
    /// Approve (or reject) a timesheet whose approver kind is internal.
    pub const TIMESHEETS_APPROVE: &str = "timesheets.approve";
    /// Approve for the external approver, with the signed sheet attached.
    pub const TIMESHEETS_APPROVE_ON_BEHALF: &str = "timesheets.approve_on_behalf";
    /// Reopen an approved timesheet that nothing has used.
    pub const TIMESHEETS_REOPEN: &str = "timesheets.reopen";
}

/// The attendance module. The host adds it with
/// `.module(wl_attendance::WlAttendance::new())` when `client.toml` enables
/// `attendance`.
#[derive(Default)]
pub struct WlAttendance {
    config: AttendanceConfig,
}

impl WlAttendance {
    /// The module with the §7 defaults ([`AttendanceConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Days back a supervisor may mark or correct (default 1, `HRM.ACL.01`).
    pub fn late_entry_days(mut self, days: i64) -> Self {
        self.config.late_entry_days = days;
        self
    }

    /// The role whose holders are told of a missing permit or visa.
    pub fn hr_role(mut self, role: &str) -> Self {
        self.config.hr_role = role.to_owned();
        self
    }

    /// The role whose holders are told of a midday break flag.
    pub fn hse_role(mut self, role: &str) -> Self {
        self.config.hse_role = role.to_owned();
        self
    }

    /// The overtime `HRM.RUL.13` allows above the normal hours: `per_day`
    /// hours a day (default 2) and `per_window` hours in any three weeks
    /// (default 144).
    pub fn overtime_cap(mut self, per_day: i64, per_window: i64) -> Self {
        self.config.overtime_day_minutes = per_day * 60;
        self.config.overtime_window_minutes = per_window * 60;
        self
    }

    /// Whether `HRM.RUL.14` checks the work permit, the residence visa and the
    /// qualifications the context requires (default on).
    pub fn permit_check(mut self, on: bool) -> Self {
        self.config.permit_check = on;
        self
    }

    /// The document kinds `HRM.RUL.14` checks for every employee (default
    /// `work_permit` and `residence_visa`).
    pub fn permit_kinds(mut self, kinds: &[&str]) -> Self {
        self.config.permit_kinds = kinds.iter().map(|k| (*k).to_owned()).collect();
        self
    }

    /// Days after `submit` at which `HRM.RUL.15` escalates an unanswered
    /// external timesheet (default 6, calendar days).
    pub fn escalation_days(mut self, days: i64) -> Self {
        self.config.escalation_days = days;
        self
    }

    /// What payroll pays at the cut-off for a timesheet that an approver of
    /// `kind` has not approved: `Attendance` (default, the accepted
    /// attendance hours, provisional) or `Wait` (only for an internal
    /// approver).
    ///
    /// # Errors
    ///
    /// [`config::SettingRefused`] for `Wait` with an external approver.
    pub fn payroll_basis(
        mut self,
        kind: timesheet::ApproverKind,
        basis: config::PayrollBasis,
    ) -> Result<Self, config::SettingRefused> {
        self.config.set_payroll_basis(kind, basis)?;
        Ok(self)
    }

    /// The module's settings.
    pub fn config(&self) -> &AttendanceConfig {
        &self.config
    }
}

impl Module for WlAttendance {
    fn name(&self) -> &'static str {
        "wl-attendance"
    }

    fn routes(&self) -> Routes {
        pages::routes()
            .merge(crew::routes())
            .merge(timesheet_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<crew::CrewPage>()
            .view_data::<crew::CrewForm>()
            .view_data::<timesheet_pages::ListPage>()
            .view_data::<timesheet_pages::SheetPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // The crew list in the app group of the sidebar; the label is the
        // app's `nav.crew` text.
        app.nav(
            "app",
            NavLink::route("wl_attendance.crew", "nav.crew")
                .icon("users")
                .group("nav.app")
                .order(50)
                .permission(permissions::MARK),
        );
        app.model::<WorkLocation>();
        app.model::<AttendanceRecord>();
        app.model::<AttendanceFlag>();
        app.model::<MiddayWindow>();
        app.model::<Timesheet>();
        app.model::<TimesheetEntry>();
        app.model::<approval::ApprovalEvidence>();
        app.model::<dispute::AdjustmentEvidence>();
        app.model::<payroll::ProvisionalHours>();
        app.provide(self.config.clone());
        overtime::provide_entry_hours(app, entry::TimesheetEntryHours);
        wl_kernel::timesheets::provide_timesheets(app, payroll::AttendanceTimesheets);
        wl_kernel::attendance::provide_attendance_review(app, after_last_day::Reviewer);
        wl_kernel::attendance::provide_attendance_history(app, history::Attended);
        wl_kernel::timesheets::provide_timesheet_closing(app, closing::Closer);
        renox::state_machine::register::<AttendanceRecord>();
        renox::state_machine::register::<Timesheet>();
        notices::register_types();
        dispute::register(app);
        reconcile::register(app);
        escalation::schedule(app);
        period_end::schedule(app);
        wl_kernel::documents::register_future_work(
            crews_affected::GUARDED,
            crews_affected::CrewsAffected,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_values() {
        let module = WlAttendance::new();
        assert_eq!(module.config().late_entry_days, 1);
        assert_eq!(module.config().hr_role, "hr");
        assert_eq!(module.config().hse_role, "hse");
    }

    #[test]
    fn the_builder_changes_the_settings() {
        let module = WlAttendance::new().late_entry_days(2).hr_role("people");
        assert_eq!(module.config().late_entry_days, 2);
        assert_eq!(module.config().hr_role, "people");
    }
}
