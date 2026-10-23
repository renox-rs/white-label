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

/// The earning sales per member per day before the account is listed for
/// the manager's review (§7, `SAL.ACL.08`).
pub const DEFAULT_LOYALTY_DAILY_SALES_LIMIT: i64 = 5;

/// The minutes points are held for an open sale once the member confirmed
/// (§7: 30).
pub const DEFAULT_REDEMPTION_HOLD_MINUTES: i64 = 30;

/// The days points given back after their expiry are valid (§7: 30).
pub const DEFAULT_GIVE_BACK_DAYS: i64 = 30;

/// The days before an earn lot expires that its member is reminded
/// (`SAL.RUL.08`, §7: 30).
pub const DEFAULT_EXPIRY_REMINDER_DAYS: i64 = 30;

/// The points above which a manual adjustment needs a manager (`SAL.ACL.07`,
/// §7: 500).
pub const DEFAULT_ADJUSTMENT_LIMIT: i64 = 500;

/// The days after a retention campaign's send within which a purchase makes
/// the member count as returned (§7: 30).
pub const DEFAULT_RETENTION_RESPONSE_DAYS: i64 = 30;

/// The days before a change of the loyalty terms takes effect that members
/// must be told (§7: 30).
pub const DEFAULT_PROGRAMME_NOTICE_DAYS: i64 = 30;

/// The days members can redeem after the programme is turned off (§7: 90).
pub const DEFAULT_PROGRAMME_CLOSING_DAYS: i64 = 90;

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
    /// The earning sales per member per day before the account is listed for
    /// review (`SAL.ACL.08`).
    pub loyalty_daily_sales_limit: i64,
    /// Whether a redemption is confirmed by a one-time code to the member's
    /// mobile (§7: on). Off, a cashier's redemption is held at once; a
    /// member logged in to the portal never needs the code.
    pub confirm_redemption_by_code: bool,
    /// The minutes points are held for an open sale once confirmed.
    pub redemption_hold_minutes: i64,
    /// The days points given back after their expiry are valid.
    pub give_back_days: i64,
    /// The days before an earn lot expires that its member is reminded.
    pub expiry_reminder_days: i64,
    /// The points above which a manual adjustment needs a manager.
    pub adjustment_limit: i64,
    /// The days after a retention campaign's send within which a purchase
    /// counts as the member returning.
    pub retention_response_days: i64,
    /// The item codes of clinical services (05). A segment rule on one is
    /// refused, and the segment editor does not offer them, unless
    /// [`segment_clinical_rules`](Self::segment_clinical_rules) is on.
    pub clinical_items: Vec<String>,
    /// Whether segment rules may use clinical items (§7: off).
    pub segment_clinical_rules: bool,
    /// The least days ahead a change of the loyalty terms may take effect.
    pub programme_notice_days: i64,
    /// The days after the programme is turned off that points can still be
    /// redeemed.
    pub programme_closing_days: i64,
    /// The order channels whose lines without stock are back-ordered
    /// (§7 *Back-orders allowed per channel*). Empty: none.
    pub back_order_channels: Vec<String>,
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
            loyalty_daily_sales_limit: DEFAULT_LOYALTY_DAILY_SALES_LIMIT,
            confirm_redemption_by_code: true,
            redemption_hold_minutes: DEFAULT_REDEMPTION_HOLD_MINUTES,
            give_back_days: DEFAULT_GIVE_BACK_DAYS,
            expiry_reminder_days: DEFAULT_EXPIRY_REMINDER_DAYS,
            adjustment_limit: DEFAULT_ADJUSTMENT_LIMIT,
            retention_response_days: DEFAULT_RETENTION_RESPONSE_DAYS,
            clinical_items: Vec::new(),
            segment_clinical_rules: false,
            programme_notice_days: DEFAULT_PROGRAMME_NOTICE_DAYS,
            programme_closing_days: DEFAULT_PROGRAMME_CLOSING_DAYS,
            back_order_channels: Vec::new(),
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

    /// Whether the item `code` is a clinical service a segment may not use.
    pub fn is_blocked_clinical(&self, code: &str) -> bool {
        !self.segment_clinical_rules
            && self
                .clinical_items
                .iter()
                .any(|c| c.eq_ignore_ascii_case(code.trim()))
    }

    /// Whether a line without stock is back-ordered on `channel`.
    pub fn back_orders_allowed(&self, channel: &str) -> bool {
        self.back_order_channels
            .iter()
            .any(|c| c.eq_ignore_ascii_case(channel.trim()))
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

    #[test]
    fn clinical_items_are_left_out_of_segments_unless_the_setting_is_on() {
        let mut config = SalesConfig {
            clinical_items: vec!["XRAY".into()],
            ..SalesConfig::default()
        };
        assert!(config.is_blocked_clinical("xray"));
        assert!(!config.is_blocked_clinical("shampoo"));
        config.segment_clinical_rules = true;
        assert!(!config.is_blocked_clinical("xray"));
        assert_eq!(config.retention_response_days, 30);
        assert_eq!(
            (config.programme_notice_days, config.programme_closing_days),
            (30, 90)
        );
    }
}
