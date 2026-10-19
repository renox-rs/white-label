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

/// The settings of the HR module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HrConfig {
    /// The role of the managers of a scope.
    pub manager_role: String,
    /// The owner role.
    pub owner_role: String,
}

impl Default for HrConfig {
    fn default() -> Self {
        Self {
            manager_role: DEFAULT_MANAGER_ROLE.to_owned(),
            owner_role: DEFAULT_OWNER_ROLE.to_owned(),
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
