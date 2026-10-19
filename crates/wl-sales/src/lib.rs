#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod config;
mod texts;

use renox::prelude::*;

pub use config::SalesConfig;

/// The module's permissions (wl-sales §6).
pub mod permissions {
    /// See campaigns and their cost entries.
    pub const CAMPAIGNS_VIEW: &str = "campaigns.view";
    /// Create, edit, activate, end, reopen and delete campaigns, and add cost entries.
    pub const CAMPAIGNS_MANAGE: &str = "campaigns.manage";
    /// Approve a cost over the budget (`SAL.ACL.07`), enter a negative cost and void an entry.
    pub const CAMPAIGNS_COSTS_MANAGE: &str = "campaigns.costs.manage";
    /// See the acquisition reports.
    pub const ACQUISITION_REPORTS_VIEW: &str = "acquisition.reports.view";
}

/// The sales module. The host adds it with
/// `.module(wl_sales::WlSales::new())` when `client.toml` enables `sales`.
#[derive(Default)]
pub struct WlSales {
    config: SalesConfig,
}

impl WlSales {
    /// The module with the §7 defaults ([`SalesConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a cost entry over the budget needs a manager (`SAL.ACL.07`).
    /// Off, such an entry is approved on save.
    pub fn over_budget_approval(mut self, on: bool) -> Self {
        self.config.over_budget_approval = on;
        self
    }

    /// The budget alert thresholds in percent (`SAL.RUL.10`), e.g. `&[80, 100]`.
    /// They are stored as basis points, sorted, and repeated values count once.
    pub fn budget_alerts(mut self, percents: &[u32]) -> Self {
        let mut list: Vec<u32> = percents.iter().map(|p| p.saturating_mul(100)).collect();
        list.sort_unstable();
        list.dedup();
        self.config.budget_alerts = list;
        self
    }

    /// The days after which a lead's response counts as late.
    pub fn late_response_days(mut self, days: u32) -> Self {
        self.config.late_response_days = days;
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &SalesConfig {
        &self.config
    }
}

impl Module for WlSales {
    fn name(&self) -> &'static str {
        "wl-sales"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_spec_values() {
        let module = WlSales::new();
        let config = module.config();
        assert!(config.over_budget_approval);
        assert_eq!(config.budget_alerts, [8000, 10000]);
        assert_eq!(config.late_response_days, 30);
    }

    #[test]
    fn the_builder_changes_the_settings() {
        let module = WlSales::new()
            .over_budget_approval(false)
            .budget_alerts(&[100, 50, 100])
            .late_response_days(14);
        let config = module.config();
        assert!(!config.over_budget_approval);
        assert_eq!(config.budget_alerts, [5000, 10000]);
        assert_eq!(config.late_response_days, 14);
    }
}
