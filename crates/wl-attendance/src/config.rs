//! The module's settings (wl-attendance §7). They are client defaults kept in
//! code, set on [`WlAttendance`](crate::WlAttendance) with builder calls; a
//! settings page comes later.

use renox::chrono::NaiveTime;
use renox::prelude::AppState;

/// How far back a supervisor may mark attendance (`HRM.ACL.01`): 1 day.
pub const DEFAULT_LATE_ENTRY_DAYS: i64 = 1;

/// The role whose holders are told of a permit, visa or qualification that is
/// not valid (`HRM.RUL.14`).
pub const DEFAULT_HR_ROLE: &str = "hr";

/// The role whose holders are told of a midday break flag (`HRM.RUL.01`,
/// `HRM.RUL.13`).
pub const DEFAULT_HSE_ROLE: &str = "hse";

/// Overtime above the normal hours that `HRM.RUL.13` allows in a day: 2 h.
pub const DEFAULT_OVERTIME_DAY_MINUTES: i64 = 2 * 60;

/// Overtime that `HRM.RUL.13` allows in the rolling window: 144 h.
pub const DEFAULT_OVERTIME_WINDOW_MINUTES: i64 = 144 * 60;

/// The rolling window of the overtime cap: three weeks.
pub const DEFAULT_OVERTIME_WINDOW_DAYS: i64 = 21;

/// The days after `submit` at which `HRM.RUL.15` escalates an unanswered
/// external timesheet (E): 6.
pub const DEFAULT_ESCALATION_DAYS: i64 = 6;

/// The document kinds `HRM.RUL.14` checks by default (wl-attendance §7).
pub const DEFAULT_PERMIT_KINDS: [&str; 2] = ["work_permit", "residence_visa"];

/// The midday break of the year when no scope has its own (§7): from
/// 15 June to 15 September, 12:30 to 15:00. MoHRE announces it every year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiddayDefault {
    /// The first day of the season, `(month, day)`.
    pub from: (u32, u32),
    /// The last day of the season, `(month, day)`.
    pub to: (u32, u32),
    /// When the break starts, Asia/Dubai.
    pub starts: NaiveTime,
    /// When the break ends, Asia/Dubai.
    pub ends: NaiveTime,
}

impl Default for MiddayDefault {
    fn default() -> Self {
        MiddayDefault {
            from: (6, 15),
            to: (9, 15),
            starts: NaiveTime::from_hms_opt(12, 30, 0).expect("12:30"),
            ends: NaiveTime::from_hms_opt(15, 0, 0).expect("15:00"),
        }
    }
}

/// The settings of the attendance module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttendanceConfig {
    /// Days back a supervisor may mark or correct (`HRM.ACL.01`).
    pub late_entry_days: i64,
    /// The HR role.
    pub hr_role: String,
    /// The HSE officer's role.
    pub hse_role: String,
    /// Overtime minutes allowed above the normal hours in a day.
    pub overtime_day_minutes: i64,
    /// Overtime minutes allowed in the rolling window.
    pub overtime_window_minutes: i64,
    /// Days of the rolling window, the attendance date included.
    pub overtime_window_days: i64,
    /// Whether `HRM.RUL.14` runs.
    pub permit_check: bool,
    /// The document kinds `HRM.RUL.14` checks for every employee.
    pub permit_kinds: Vec<String>,
    /// The midday break when a scope has none of its own.
    pub midday: MiddayDefault,
    /// Days after `submit` at which an unanswered external timesheet is
    /// escalated (`HRM.RUL.15`, E).
    pub escalation_days: i64,
}

impl Default for AttendanceConfig {
    fn default() -> Self {
        Self {
            late_entry_days: DEFAULT_LATE_ENTRY_DAYS,
            hr_role: DEFAULT_HR_ROLE.to_owned(),
            hse_role: DEFAULT_HSE_ROLE.to_owned(),
            overtime_day_minutes: DEFAULT_OVERTIME_DAY_MINUTES,
            overtime_window_minutes: DEFAULT_OVERTIME_WINDOW_MINUTES,
            overtime_window_days: DEFAULT_OVERTIME_WINDOW_DAYS,
            permit_check: true,
            permit_kinds: DEFAULT_PERMIT_KINDS
                .iter()
                .map(|k| (*k).to_owned())
                .collect(),
            midday: MiddayDefault::default(),
            escalation_days: DEFAULT_ESCALATION_DAYS,
        }
    }
}

/// The settings the module provided, else the defaults.
pub fn config_of(state: &AppState) -> AttendanceConfig {
    state
        .provided::<AttendanceConfig>()
        .map(|config| (*config).clone())
        .unwrap_or_default()
}
