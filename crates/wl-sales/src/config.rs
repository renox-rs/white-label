//! The module's settings (wl-sales §7). They are client defaults kept in
//! code, set on [`WlSales`](crate::WlSales) with builder calls; a settings
//! page comes later.

/// The budget alert thresholds (`SAL.RUL.10`) in basis points: 80% and 100%.
pub const DEFAULT_BUDGET_ALERTS: &[u32] = &[8000, 10000];

/// The days after which a lead's response counts as late.
pub const DEFAULT_LATE_RESPONSE_DAYS: u32 = 30;

/// The lead lost reasons that are always there: leads closed with either are
/// left out of every lead count (wl-sales §5.8).
pub const LOST_DUPLICATE: &str = "duplicate";
/// See [`LOST_DUPLICATE`].
pub const LOST_SPAM: &str = "spam_or_invalid";

/// The months a package is valid after activation (§7: 6).
pub const DEFAULT_PACKAGE_VALIDITY_MONTHS: i64 = 6;

/// How many times a package may be extended (§7: one extension).
pub const DEFAULT_PACKAGE_EXTENSION_LIMIT: i64 = 1;

/// The days after a sale within which goods can come back (§7: 14).
pub const DEFAULT_RETURN_WINDOW_DAYS: i64 = 14;

/// The channels whose orders wait for the seller to accept them (§7: website
/// and portal).
pub const DEFAULT_ACCEPTANCE_CHANNELS: &[&str] = &["website", "portal"];

/// The lost reasons a lead can be closed with (§7: the starter list, always
/// with *duplicate* and *spam or invalid*).
pub const DEFAULT_LOST_REASONS: &[&str] = &[
    LOST_DUPLICATE,
    LOST_SPAM,
    "price",
    "no_response",
    "chose_competitor",
    "not_interested",
    "other",
];

/// The settings of the sales module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SalesConfig {
    /// Whether a cost entry over the budget needs a manager (`SAL.ACL.07`).
    pub over_budget_approval: bool,
    /// The budget alert thresholds in basis points (`SAL.RUL.10`), ascending.
    pub budget_alerts: Vec<u32>,
    /// The days after which a lead's response counts as late.
    pub late_response_days: u32,
    /// The lost reasons a lead can be closed with.
    pub lost_reasons: Vec<String>,
    /// The codes of the lead sources that need a campaign (`SAL.CRM.02`;
    /// 05: *paid_ads*, *social*). Empty: none.
    pub campaign_required_sources: Vec<String>,
    /// The months after the first sale whose sales make the converted value;
    /// `None`: the first sale only.
    pub attribution_window_months: Option<u32>,
    /// Whether changing the attribution of an acquired party or a won lead
    /// needs a second person (`SAL.ACL.09`).
    pub acquisition_change_approval: bool,
    /// The months a package is valid after activation.
    pub package_validity_months: i64,
    /// How many times a package may be extended.
    pub package_extension_limit: i64,
    /// Whether `consume` is refused while a reopened invoice balance of the
    /// package is unpaid (§7: off; 05, 10: a task to collect only).
    pub suspend_package_consumption_while_unpaid: bool,
    /// The role told to collect a reopened balance when the package has no
    /// seller user (05: front desk; 10: counsellor); the company admin when
    /// empty.
    pub package_seller_role: String,
    /// Whether a counter sale may be charged to the customer account (§7:
    /// off; 08: on).
    pub counter_sales_on_account: bool,
    /// The days after a sale within which goods can come back (§7: 14).
    pub return_window_days: i64,
    /// The order channels that need the seller to accept an order (§7).
    pub order_acceptance_channels: Vec<String>,
}

impl Default for SalesConfig {
    fn default() -> Self {
        Self {
            over_budget_approval: true,
            budget_alerts: DEFAULT_BUDGET_ALERTS.to_vec(),
            late_response_days: DEFAULT_LATE_RESPONSE_DAYS,
            lost_reasons: DEFAULT_LOST_REASONS
                .iter()
                .map(|r| (*r).to_owned())
                .collect(),
            campaign_required_sources: Vec::new(),
            attribution_window_months: None,
            acquisition_change_approval: true,
            package_validity_months: DEFAULT_PACKAGE_VALIDITY_MONTHS,
            package_extension_limit: DEFAULT_PACKAGE_EXTENSION_LIMIT,
            suspend_package_consumption_while_unpaid: false,
            package_seller_role: String::new(),
            counter_sales_on_account: false,
            return_window_days: DEFAULT_RETURN_WINDOW_DAYS,
            order_acceptance_channels: DEFAULT_ACCEPTANCE_CHANNELS
                .iter()
                .map(|c| (*c).to_owned())
                .collect(),
        }
    }
}

/// The settings the module provided, else the defaults.
pub fn config_of(state: &renox::prelude::AppState) -> SalesConfig {
    state
        .provided::<SalesConfig>()
        .map(|config| (*config).clone())
        .unwrap_or_default()
}

impl SalesConfig {
    /// Whether a lead or first sale with the source `code` needs a campaign.
    pub fn campaign_required_for(&self, code: &str) -> bool {
        self.campaign_required_sources.iter().any(|c| c == code)
    }

    /// Whether `reason` is one of the lost reasons.
    pub fn is_lost_reason(&self, reason: &str) -> bool {
        self.lost_reasons.iter().any(|r| r == reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_need_no_campaign_and_keep_the_two_always_there_reasons() {
        let config = SalesConfig::default();
        assert!(!config.campaign_required_for("paid_ads"));
        assert!(config.is_lost_reason(LOST_DUPLICATE) && config.is_lost_reason(LOST_SPAM));
        assert!(!config.is_lost_reason("because"));
        assert_eq!(config.attribution_window_months, None);
        assert!(config.acquisition_change_approval);
    }
}
