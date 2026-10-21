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
}

impl Default for HrConfig {
    fn default() -> Self {
        Self {
            manager_role: DEFAULT_MANAGER_ROLE.to_owned(),
            owner_role: DEFAULT_OWNER_ROLE.to_owned(),
            absence_days: DEFAULT_ABSENCE_DAYS,
            sick_note_days: DEFAULT_SICK_NOTE_DAYS,
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
