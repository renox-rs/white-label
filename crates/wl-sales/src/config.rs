//! The module's settings (wl-sales §7). They are client defaults kept in
//! code, set on [`WlSales`](crate::WlSales) with builder calls; a settings
//! page comes later.

/// The budget alert thresholds (`SAL.RUL.10`) in basis points: 80% and 100%.
pub const DEFAULT_BUDGET_ALERTS: &[u32] = &[8000, 10000];

/// The days after which a lead's response counts as late.
pub const DEFAULT_LATE_RESPONSE_DAYS: u32 = 30;

/// The settings of the sales module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalesConfig {
    /// Whether a cost entry over the budget needs a manager (`SAL.ACL.07`).
    pub over_budget_approval: bool,
    /// The budget alert thresholds in basis points (`SAL.RUL.10`), ascending.
    pub budget_alerts: Vec<u32>,
    /// The days after which a lead's response counts as late.
    pub late_response_days: u32,
}

impl Default for SalesConfig {
    fn default() -> Self {
        Self {
            over_budget_approval: true,
            budget_alerts: DEFAULT_BUDGET_ALERTS.to_vec(),
            late_response_days: DEFAULT_LATE_RESPONSE_DAYS,
        }
    }
}
