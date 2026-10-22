//! The module's settings (wl-hr §7). They are client defaults kept in code,
//! set on [`WlHr`](crate::WlHr) with builder calls; a settings page comes
//! later.

/// The role whose holders are the *manager* of an employee in a scope
/// (wl-hr §3 *The employee's manager*).
pub const DEFAULT_MANAGER_ROLE: &str = "manager";

/// The role that holds `onboarding.manage` by default as the second person
/// for `waive` and `reopen`, and gets `hr` steps only when nobody else holds
/// the permission (wl-hr §3 *Onboarding*).
pub const DEFAULT_OWNER_ROLE: &str = "owner";

/// Absence days before `report_absconding` is allowed (wl-hr §7): more than
/// this many consecutive calendar days (ER Art. 28).
pub const DEFAULT_ABSENCE_DAYS: i64 = 7;

/// Working days after the start of a same-day sick leave (`record_sick`) in
/// which the sick note may still follow (wl-hr §7 *Sick note due*): the HR
/// task is raised at 00:00 Asia/Dubai the day after the last of them.
pub const DEFAULT_SICK_NOTE_DAYS: i64 = 2;
/// The role whose holders are told of a WPS rejection that is still open
/// before the due date (`HRM.RUL.11`): the MD, which the host's `owner` role is.
pub const DEFAULT_MD_ROLE: &str = "owner";

/// The accountant role, told with the MD (`HRM.RUL.11`).
pub const DEFAULT_ACCOUNTANT_ROLE: &str = "accountant";

/// N of `HRM.RUL.11` (wl-hr §6, §7 *WPS rejection escalation*): days before the
/// WPS due date at which a rejection still open is escalated.
pub const DEFAULT_WPS_ESCALATION_DAYS: i64 = 1;

/// The payroll cut-off day of the month (wl-hr §7, `HRM.RUL.02` parameter D):
/// attendance dated after it is not counted as provisional hours.
pub const DEFAULT_CUT_OFF_DAY: u32 = 25;

/// The role whose holders get the tasks for HR: reaching a worker whose pay
/// account notice was not delivered (`HRM.ACL.05`, wl-hr §4 *Pay account*).
pub const DEFAULT_HR_ROLE: &str = "hr";

/// The cooling period of a new pay account in days (`HRM.ACL.05`, wl-hr §7).
pub const DEFAULT_COOLING_DAYS: i64 = 5;

/// N of `HRM.RUL.09` (wl-hr §6, §7 *Settlement alert*): days after the last
/// working day at which HR and the accountant are told.
pub const DEFAULT_SETTLEMENT_ALERT_DAYS: i64 = 10;

/// The legal deadline of the final settlement: days after the end of the
/// contract (Art. 53, `HRM.RUL.09`).
pub const DEFAULT_SETTLEMENT_DEADLINE_DAYS: i64 = 14;

/// Days after `pay` at which a settlement without a dispute closes (wl-hr §7).
pub const DEFAULT_SETTLEMENT_CLOSE_DAYS: i64 = 30;

/// Days before the current residence visa expires at which the follow-up task
/// of a refused renewal is due (wl-hr §7).
pub const DEFAULT_VISA_REJECTION_TASK_DAYS: i64 = 30;

/// Days the employee portal stays read-only after `exit`, then closes (wl-hr
/// §7 *Employee portal after exit*, `POR.ACL.01`).
pub const DEFAULT_PORTAL_DAYS_AFTER_EXIT: i64 = 60;

/// The settings of the HR module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HrConfig {
    /// The role of the managers of a scope.
    pub manager_role: String,
    /// The owner role.
    pub owner_role: String,
    /// Absence days before `report_absconding` is allowed.
    pub absence_days: i64,
    /// Working days the sick note of `record_sick` may follow.
    pub sick_note_days: i64,
    /// The role of the MD (`HRM.RUL.11`).
    pub md_role: String,
    /// The role of the accountant (`HRM.RUL.11`).
    pub accountant_role: String,
    /// N of `HRM.RUL.11`: days before the WPS due date.
    pub wps_escalation_days: i64,
    /// The payroll cut-off day of the month (D of `HRM.RUL.02`).
    pub cut_off_day: u32,
    /// The role whose holders get HR's tasks.
    pub hr_role: String,
    /// The cooling period of a new pay account, in days (`HRM.ACL.05`).
    pub cooling_days: i64,
    /// N of `HRM.RUL.09`: days after the last working day.
    pub settlement_alert_days: i64,
    /// The legal deadline of the final settlement, days after the last
    /// working day.
    pub settlement_deadline_days: i64,
    /// Days after `pay` at which a settlement without a dispute closes.
    pub settlement_close_days: i64,
    /// Days before the current residence visa expires at which the task after
    /// a refused renewal is due.
    pub visa_rejection_task_days: i64,
    /// Days the portal stays read-only after `exit`, then closes.
    pub portal_days_after_exit: i64,
    /// The routing codes of the banks and agents a pay account may name. Empty
    /// accepts any code of the right form: the list comes with the salary
    /// file formats.
    pub routing_codes: Vec<String>,
}

impl Default for HrConfig {
    fn default() -> Self {
        Self {
            manager_role: DEFAULT_MANAGER_ROLE.to_owned(),
            owner_role: DEFAULT_OWNER_ROLE.to_owned(),
            absence_days: DEFAULT_ABSENCE_DAYS,
            sick_note_days: DEFAULT_SICK_NOTE_DAYS,
            md_role: DEFAULT_MD_ROLE.to_owned(),
            accountant_role: DEFAULT_ACCOUNTANT_ROLE.to_owned(),
            wps_escalation_days: DEFAULT_WPS_ESCALATION_DAYS,
            cut_off_day: DEFAULT_CUT_OFF_DAY,
            hr_role: DEFAULT_HR_ROLE.to_owned(),
            cooling_days: DEFAULT_COOLING_DAYS,
            settlement_alert_days: DEFAULT_SETTLEMENT_ALERT_DAYS,
            settlement_deadline_days: DEFAULT_SETTLEMENT_DEADLINE_DAYS,
            settlement_close_days: DEFAULT_SETTLEMENT_CLOSE_DAYS,
            visa_rejection_task_days: DEFAULT_VISA_REJECTION_TASK_DAYS,
            portal_days_after_exit: DEFAULT_PORTAL_DAYS_AFTER_EXIT,
            routing_codes: Vec::new(),
        }
    }
}

/// The settings the module provided, else the defaults.
pub fn config_of(state: &renox::prelude::AppState) -> HrConfig {
    state
        .provided::<HrConfig>()
        .map(|config| (*config).clone())
        .unwrap_or_default()
}
