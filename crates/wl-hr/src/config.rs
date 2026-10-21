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

/// The settings of the HR module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HrConfig {
    /// The role of the managers of a scope.
    pub manager_role: String,
    /// The owner role.
    pub owner_role: String,
    /// Absence days before `report_absconding` is allowed.
    pub absence_days: i64,
    /// The role of the MD (`HRM.RUL.11`).
    pub md_role: String,
    /// The role of the accountant (`HRM.RUL.11`).
    pub accountant_role: String,
    /// N of `HRM.RUL.11`: days before the WPS due date.
    pub wps_escalation_days: i64,
    /// The payroll cut-off day of the month (D of `HRM.RUL.02`).
    pub cut_off_day: u32,
}

impl Default for HrConfig {
    fn default() -> Self {
        Self {
            manager_role: DEFAULT_MANAGER_ROLE.to_owned(),
            owner_role: DEFAULT_OWNER_ROLE.to_owned(),
            absence_days: DEFAULT_ABSENCE_DAYS,
            md_role: DEFAULT_MD_ROLE.to_owned(),
            accountant_role: DEFAULT_ACCOUNTANT_ROLE.to_owned(),
            wps_escalation_days: DEFAULT_WPS_ESCALATION_DAYS,
            cut_off_day: DEFAULT_CUT_OFF_DAY,
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
