#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod acquisition;
pub mod acquisition_change;
pub mod acquisition_daily;
pub mod adjustment;
pub mod adjustment_machine;
pub mod adjustments;
pub mod attribution;
pub mod back_orders;
pub mod bill_links;
pub mod block;
pub mod budget;
pub mod campaign;
pub mod closure;
pub mod config;
pub mod cost_entry;
pub mod cost_machine;
pub mod costs;
pub mod counter_sale;
pub mod counter_sale_machine;
pub mod counter_sales;
pub mod daily;
mod events;
pub mod expiry;
pub mod lead;
mod lead_events;
pub mod lead_machine;
pub mod lead_source;
pub mod liability;
pub mod loyalty_account;
pub mod loyalty_attach;
pub mod loyalty_points;
pub mod loyalty_programme;
pub mod loyalty_rules;
pub mod machine;
pub mod merge;
pub mod notices;
pub mod package;
pub mod package_machine;
pub mod package_reopen;
pub mod packages;
pub mod pages;
pub mod policy;
pub mod promo_code;
pub mod redeem;
pub mod redemption;
pub mod redemption_machine;
pub mod report;
pub mod retention;
pub mod sale_customers;
pub mod sale_line;
pub mod sales_order;
pub mod sales_order_machine;
pub mod sales_orders;
pub mod sales_return;
pub mod sales_return_machine;
pub mod sales_returns;
pub mod seed;
pub mod segment;
pub mod segments;
pub mod terms;
mod texts;
pub mod voids;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use acquisition::{AcquisitionRefused, AcquisitionSale, PartyAcquisition};
pub use acquisition_change::{
    AcquisitionChange, ChangeRefused, ChangeRequest, ChangeStatus, ChangeTarget,
};
pub use adjustment::{AdjustmentStatus, PointsAdjustment};
pub use budget::BudgetAlert;
pub use campaign::{
    Campaign, CampaignCode, CampaignRefused, CampaignStatus, MarketingChannel, Purpose,
};
pub use config::SalesConfig;
pub use cost_entry::{CostEntry, CostRefused, CostStatus};
pub use counter_sale::{CounterSale, CounterSalePayment, CounterSaleStatus};
pub use lead::{Captured, Lead, LeadNote, LeadRefused, LeadStatus, NewLead};
pub use lead_source::{LeadSource, LeadSourceRefused};
pub use loyalty_account::{AccountStatus, Enrolment, LoyaltyAccount};
pub use loyalty_attach::MemberRef;
pub use loyalty_points::{LoyaltyEarning, LoyaltyReview, PointsKind, PointsMovement};
pub use loyalty_programme::{EarnKind, LoyaltyProgramme, NewProgramme};
pub use package::{MovementKind, Package, PackageMovement, PackageStatus};
pub use package_reopen::{PackageRefusal, PackageReopening};
pub use promo_code::PromoCode;
pub use redemption::{Redemption, RedemptionStatus};
pub use retention::{BroadcastState, CampaignBroadcast, CampaignGuard, CampaignGuards};
pub use sale_line::SaleLine;
pub use sales_order::{FulfilmentRecord, SalesOrder, SalesOrderStatus};
pub use sales_return::{ReturnKind, ReturnResolution, ReturnStatus, SalesReturn, SalesReturnLine};
pub use segment::{MatchMode, Rule, Segment, SegmentRefused};
pub use segments::{SegmentRule, SegmentRules};
pub use terms::{LoyaltyTermsChange, TermsChange};

/// The campaign services under the names of the plan: `campaigns::delete`
/// and `campaigns::run_daily`.
pub mod campaigns {
    pub use crate::campaign::{DELETE_CHECKS, DeleteCheck, delete, delete_with};
    pub use crate::daily::{Report, run_daily};
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-sales/lead_sources.html",
        include_str!("../views/wl-sales/lead_sources.html"),
    ),
    (
        "wl-sales/index.html",
        include_str!("../views/wl-sales/index.html"),
    ),
    (
        "wl-sales/form.html",
        include_str!("../views/wl-sales/form.html"),
    ),
    (
        "wl-sales/show.html",
        include_str!("../views/wl-sales/show.html"),
    ),
    (
        "wl-sales/acquisition_report.html",
        include_str!("../views/wl-sales/acquisition_report.html"),
    ),
    (
        "wl-sales/acquisition_exceptions.html",
        include_str!("../views/wl-sales/acquisition_exceptions.html"),
    ),
    (
        "wl-sales/_cost_sheet.html",
        include_str!("../views/wl-sales/_cost_sheet.html"),
    ),
    (
        "wl-sales/counter_sales.html",
        include_str!("../views/wl-sales/counter_sales.html"),
    ),
    (
        "wl-sales/counter_sale.html",
        include_str!("../views/wl-sales/counter_sale.html"),
    ),
    (
        "wl-sales/sales_orders.html",
        include_str!("../views/wl-sales/sales_orders.html"),
    ),
    (
        "wl-sales/sales_order.html",
        include_str!("../views/wl-sales/sales_order.html"),
    ),
    (
        "wl-sales/sales_returns.html",
        include_str!("../views/wl-sales/sales_returns.html"),
    ),
    (
        "wl-sales/sales_return.html",
        include_str!("../views/wl-sales/sales_return.html"),
    ),
    (
        "wl-sales/loyalty.html",
        include_str!("../views/wl-sales/loyalty.html"),
    ),
    (
        "wl-sales/loyalty_review.html",
        include_str!("../views/wl-sales/loyalty_review.html"),
    ),
    (
        "wl-sales/adjustments.html",
        include_str!("../views/wl-sales/adjustments.html"),
    ),
    (
        "wl-sales/liability.html",
        include_str!("../views/wl-sales/liability.html"),
    ),
    (
        "wl-sales/portal_loyalty.html",
        include_str!("../views/wl-sales/portal_loyalty.html"),
    ),
    (
        "wl-sales/segments.html",
        include_str!("../views/wl-sales/segments.html"),
    ),
    (
        "wl-sales/segment.html",
        include_str!("../views/wl-sales/segment.html"),
    ),
];

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
    /// See packages and their movements.
    pub const PACKAGES_VIEW: &str = "packages.view";
    /// Sell a package and cancel one nothing was paid for.
    pub const PACKAGES_SELL: &str = "packages.sell";
    /// Extend a package's end date.
    pub const PACKAGES_EXTEND: &str = "packages.extend";
    /// Give credits of a package back by hand.
    pub const PACKAGES_ADJUST: &str = "packages.adjust";
    /// See leads.
    pub const LEADS_VIEW: &str = "leads.view";
    /// Contact, qualify, lose and reopen leads.
    pub const LEADS_MANAGE: &str = "leads.manage";
    /// Ring up and complete counter sales.
    pub const POS_SELL: &str = "pos.sell";
    /// See sales orders.
    pub const SALES_ORDERS_VIEW: &str = "sales_orders.view";
    /// Enter, edit and submit sales orders.
    pub const SALES_ORDERS_CREATE: &str = "sales_orders.create";
    /// Accept or reject an order that waits for the seller.
    pub const SALES_ORDERS_ACCEPT: &str = "sales_orders.accept";
    /// Cancel a sales order.
    pub const SALES_ORDERS_CANCEL: &str = "sales_orders.cancel";
    /// Request a sales return.
    pub const RETURNS_REQUEST: &str = "returns.request";
    /// Receive, resolve, reject and reverse sales returns.
    pub const RETURNS_HANDLE: &str = "returns.handle";
    /// See the loyalty programme, its accounts and the review list.
    pub const LOYALTY_VIEW: &str = "loyalty.view";
    /// Enrol a member and attach a member to a sale.
    pub const LOYALTY_ENROL: &str = "loyalty.enrol";
    /// Redeem a member's points on a sale (the cashier asks for the code).
    pub const LOYALTY_REDEEM: &str = "loyalty.redeem";
    /// Correct a member's points by hand, with a reason (`SAL.ACL.07`).
    pub const LOYALTY_ADJUST: &str = "loyalty.adjust";
    /// Turn the loyalty programme on, review accounts and approve a large
    /// adjustment.
    pub const LOYALTY_MANAGE: &str = "loyalty.manage";
    /// Define customer segments and preview their size.
    pub const SEGMENTS_MANAGE: &str = "segments.manage";
    /// Export the members of a segment: a sensitive export, audited
    /// (`NTF.ACL.01`).
    pub const SEGMENTS_EXPORT: &str = "segments.export";
}

/// The sales module. The host adds it with
/// `.module(wl_sales::WlSales::new())` when `client.toml` enables `sales`.
#[derive(Default)]
pub struct WlSales {
    config: SalesConfig,
    segment_rules: Vec<std::sync::Arc<dyn SegmentRule>>,
    campaign_guards: Vec<std::sync::Arc<dyn CampaignGuard>>,
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

    /// The lost reasons a lead can be closed with (`duplicate` and
    /// `spam_or_invalid` are always there, §7).
    pub fn lost_reasons(mut self, reasons: &[&str]) -> Self {
        let mut list: Vec<String> = [config::LOST_DUPLICATE, config::LOST_SPAM]
            .iter()
            .map(|r| (*r).to_owned())
            .collect();
        for reason in reasons {
            let reason = reason.trim().to_lowercase();
            if !reason.is_empty() && !list.contains(&reason) {
                list.push(reason);
            }
        }
        self.config.lost_reasons = list;
        self
    }

    /// The lead sources (by code) that need a campaign on a lead or a first
    /// sale (`SAL.CRM.02`; 05: `paid_ads`, `social`).
    pub fn campaign_required_for(mut self, sources: &[&str]) -> Self {
        self.config.campaign_required_sources = sources.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// The months after the first sale whose sales make the converted value;
    /// `None` for the first sale only (§7).
    pub fn attribution_window_months(mut self, months: Option<u32>) -> Self {
        self.config.attribution_window_months = months;
        self
    }

    /// Whether changing the attribution of an acquired party or a won lead
    /// needs a second person (`SAL.ACL.09`).
    pub fn acquisition_change_approval(mut self, on: bool) -> Self {
        self.config.acquisition_change_approval = on;
        self
    }

    /// The months a package is valid after activation (§7, default 6).
    pub fn package_validity_months(mut self, months: i64) -> Self {
        assert!(months >= 1, "a package is valid for at least a month");
        self.config.package_validity_months = months;
        self
    }

    /// How many times a package may be extended (§7, default 1).
    pub fn package_extension_limit(mut self, limit: i64) -> Self {
        self.config.package_extension_limit = limit.max(0);
        self
    }

    /// Whether `consume` is refused while the reopened balance of the
    /// package's invoice is unpaid (§7 *Package: suspend consumption…*, off by
    /// default).
    pub fn suspend_package_consumption_while_unpaid(mut self, on: bool) -> Self {
        self.config.suspend_package_consumption_while_unpaid = on;
        self
    }

    /// The role that gets the "collect before next use" task of a package
    /// with no seller user (05: `front_desk`; 10: `counsellor`).
    pub fn package_seller_role(mut self, role: &str) -> Self {
        self.config.package_seller_role = role.to_owned();
        self
    }

    /// Whether a counter sale may be charged to the customer account (§7,
    /// off by default; 08: on).
    pub fn counter_sales_on_account(mut self, on: bool) -> Self {
        self.config.counter_sales_on_account = on;
        self
    }

    /// The order channels that wait for the seller to accept an order (§7,
    /// default `website` and `portal`).
    pub fn order_acceptance_channels(mut self, channels: &[&str]) -> Self {
        self.config.order_acceptance_channels = channels.iter().map(|c| (*c).to_owned()).collect();
        self
    }

    /// The earning sales per member per day before the account is listed
    /// for the manager's review (§7, default 5).
    pub fn loyalty_daily_sales_limit(mut self, limit: i64) -> Self {
        assert!(limit >= 1, "the daily limit is at least one sale");
        self.config.loyalty_daily_sales_limit = limit;
        self
    }

    /// Whether a redemption is confirmed by a one-time code to the member's
    /// mobile (§7, default on).
    pub fn confirm_redemption_by_code(mut self, on: bool) -> Self {
        self.config.confirm_redemption_by_code = on;
        self
    }

    /// The minutes points are held for an open sale (§7, default 30).
    pub fn redemption_hold_minutes(mut self, minutes: i64) -> Self {
        assert!(minutes >= 1, "a hold lasts at least a minute");
        self.config.redemption_hold_minutes = minutes;
        self
    }

    /// The days points given back after their expiry are valid (§7, default
    /// 30).
    pub fn give_back_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "given back points are valid for at least a day");
        self.config.give_back_days = days;
        self
    }

    /// The days before an earn lot expires that its member is reminded
    /// (`SAL.RUL.08`, default 30).
    pub fn expiry_reminder_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "the reminder goes at least a day ahead");
        self.config.expiry_reminder_days = days;
        self
    }

    /// The points above which a manual adjustment needs a manager
    /// (`SAL.ACL.07`, default 500).
    pub fn adjustment_limit(mut self, points: i64) -> Self {
        assert!(points >= 0, "the limit cannot be negative");
        self.config.adjustment_limit = points;
        self
    }

    /// The days after a sale within which goods can come back (§7, default
    /// 14).
    pub fn return_window_days(mut self, days: i64) -> Self {
        assert!(days >= 0, "the return window cannot be negative");
        self.config.return_window_days = days;
        self
    }

    /// The days after a retention campaign's send within which a purchase
    /// makes the member count as returned (§7, default 30).
    pub fn retention_response_days(mut self, days: i64) -> Self {
        assert!(days >= 1, "the response period is at least a day");
        self.config.retention_response_days = days;
        self
    }

    /// The item codes of clinical services (05). A segment rule on one is
    /// refused and the segment editor does not offer them, unless
    /// [`segment_clinical_rules`](Self::segment_clinical_rules) is on.
    pub fn clinical_items(mut self, codes: &[&str]) -> Self {
        self.config.clinical_items = codes.iter().map(|c| (*c).to_owned()).collect();
        self
    }

    /// Whether segment rules may use clinical items (§7, off by default).
    pub fn segment_clinical_rules(mut self, on: bool) -> Self {
        self.config.segment_clinical_rules = on;
        self
    }

    /// Adds a segment rule kind of the vertical (§8 `SegmentRule`).
    pub fn segment_rule(mut self, rule: impl SegmentRule) -> Self {
        self.segment_rules.push(std::sync::Arc::new(rule));
        self
    }

    /// Adds a guard of the vertical that stops a retention campaign sending
    /// (05 `CLN.ACL.13`).
    pub fn campaign_guard(mut self, guard: impl CampaignGuard) -> Self {
        self.campaign_guards.push(std::sync::Arc::new(guard));
        self
    }

    /// The order channels whose lines without stock are back-ordered
    /// (wl-sales §7 *Back-orders allowed per channel*). Default: none.
    pub fn back_orders_for(mut self, channels: &[&str]) -> Self {
        self.config.back_order_channels = channels
            .iter()
            .map(|c| c.trim().to_lowercase())
            .filter(|c| !c.is_empty())
            .collect();
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

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        // A page that holds only a campaign's id (a permit, a document) shows
        // `SMILE-OCT · Smile month`.
        wl_kernel::documents::register_subject_label(
            wl_kernel::sales::CAMPAIGN_SUBJECT_TYPE,
            |db, id| {
                Box::pin(async move {
                    Ok(Campaign::find(db, id)
                        .await?
                        .map(|campaign| format!("{} · {}", campaign.code, campaign.name)))
                })
            },
        );
        // A return names the sale it is about (`RCPT-2026-0004`).
        for subject_type in [
            wl_kernel::sales::COUNTER_SALE_SUBJECT_TYPE,
            wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE,
        ] {
            wl_kernel::documents::register_subject_label(subject_type, move |db, id| {
                Box::pin(async move {
                    Ok(
                        if subject_type == wl_kernel::sales::COUNTER_SALE_SUBJECT_TYPE {
                            counter_sale::CounterSale::find(db, id)
                                .await?
                                .map(|sale| sale.number)
                        } else {
                            sales_order::SalesOrder::find(db, id)
                                .await?
                                .map(|order| order.number)
                        },
                    )
                })
            });
        }
        app.view_data::<pages::lead_sources::IndexPage>()
            .view_data::<pages::acquisition::ExceptionsPage>()
            .view_data::<pages::report::ReportPage>()
            .view_data::<pages::campaigns::IndexPage>()
            .view_data::<pages::campaigns::FormPage>()
            .view_data::<pages::campaigns::ShowPage>()
            .view_data::<pages::counter_sales::IndexPage>()
            .view_data::<pages::counter_sales::ShowPage>()
            .view_data::<pages::sales_orders::IndexPage>()
            .view_data::<pages::sales_orders::ShowPage>()
            .view_data::<pages::sales_returns::IndexPage>()
            .view_data::<pages::sales_returns::ShowPage>()
            .view_data::<pages::loyalty::IndexPage>()
            .view_data::<pages::loyalty::ReviewPage>()
            .view_data::<pages::adjustments::IndexPage>()
            .view_data::<pages::liability::ShowPage>()
            .view_data::<pages::segments::IndexPage>()
            .view_data::<pages::segments::EditPage>()
            .view_data::<pages::membership::PortalPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<LeadSource>();
        app.model::<Lead>();
        app.model::<LeadNote>();
        app.model::<PartyAcquisition>();
        app.model::<AcquisitionSale>();
        app.model::<AcquisitionChange>();
        app.model::<Campaign>();
        app.model::<CampaignCode>();
        app.model::<PromoCode>();
        app.model::<Package>();
        app.model::<PackageMovement>();
        app.model::<PackageReopening>();
        app.model::<PackageRefusal>();
        app.model::<SaleLine>();
        app.model::<CounterSale>();
        app.model::<CounterSalePayment>();
        app.model::<SalesOrder>();
        app.model::<FulfilmentRecord>();
        app.model::<back_orders::BackOrderServe>();
        app.model::<SalesReturn>();
        app.model::<SalesReturnLine>();
        app.model::<LoyaltyProgramme>();
        app.model::<LoyaltyAccount>();
        app.model::<LoyaltyEarning>();
        app.model::<PointsMovement>();
        app.model::<LoyaltyReview>();
        app.model::<Redemption>();
        app.model::<PointsAdjustment>();
        app.model::<LoyaltyTermsChange>();
        app.model::<Segment>();
        app.model::<CampaignBroadcast>();
        app.model::<CostEntry>();
        app.model::<BudgetAlert>();
        app.provide(self.config.clone());
        app.provide(SegmentRules(self.segment_rules.clone()));
        app.provide(CampaignGuards(self.campaign_guards.clone()));
        renox::state_machine::register::<Lead>();
        lead_events::register(app);
        acquisition_change::register(app);
        acquisition_daily::schedule(app);
        // Prepaid packages: the ledger, the listeners and the daily expiry.
        packages::register(app);
        package_reopen::register(app);
        // The counter sale: its machine and the `CounterSaleCompleted` event.
        counter_sales::register(app);
        // The sales order: its machine, its events and the kernel's
        // `OrderFulfilment`.
        sales_orders::register(app);
        // Back-orders: reservations, the arrivals that serve them, the demand.
        back_orders::register(app);
        // The sales return: its machine and the `ReturnResolved` event.
        sales_returns::register(app);
        sale_customers::register(app);
        // Loyalty points: `SAL.RUL.07` earning and `SAL.RUL.09` reversals.
        loyalty_rules::register(app);
        // Redeeming points on a sale: the code, the hold, the spending and the
        // giving back; and the module's `DiscountEngine`.
        redeem::register(app);
        // Manual adjustments with approval (`SAL.ACL.07`) and the daily
        // expiry and reminder (`SAL.RUL.08`).
        adjustments::register(app);
        expiry::schedule(app);
        // Retention campaigns: the broadcast asked on `activate`, the
        // segments for `wl-notify` and the campaign as a requester.
        retention::register(app);
        // BL-SAL-26: a merge moves the points, a block pauses the account.
        merge::register(app);
        block::register(app);
        // Verticals report first sales and win leads through the kernel.
        wl_kernel::sales::provide_campaign_attribution(app, attribution::Attribution);
        wl_kernel::sales::provide_lead_conversion(app, attribution::Conversion);
        cost_machine::register(app);
        bill_links::register(app);
        voids::register(app);
        notices::register_types();
        events::register(app);
        daily::schedule(app);
        // The nine lead sources; a re-run keeps the client's changes.
        app.command(
            "sales:install",
            "Store the default lead sources, where the database lacks them",
            |_args, state: AppState| async move {
                let installed = seed::install(&state.db).await?;
                println!("{} sales defaults created.", installed.total());
                Ok(())
            },
        );
        // `db:seed`: the lead sources and two demo campaigns.
        app.seeder(|state: AppState| async move {
            seed::install(&state.db).await?;
            seed::demo_campaigns(&state.db).await.map(|_| ())
        });
        // In the app group of the staff sidebar; the label is the app's
        // `nav.campaigns` text.
        app.nav(
            "app",
            NavLink::route("wl_sales.campaigns", "nav.campaigns")
                .icon("sparkles")
                .group("nav.app")
                .order(50)
                .permission(permissions::CAMPAIGNS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_sales.counter_sales", "nav.counter_sales")
                .icon("shopping-cart")
                .group("nav.app")
                .order(51)
                .permission(permissions::POS_SELL),
        );
        app.nav(
            "app",
            NavLink::route("wl_sales.sales_orders", "nav.sales_orders")
                .icon("clipboard-list")
                .group("nav.app")
                .order(52)
                .permission(permissions::SALES_ORDERS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_sales.sales_returns", "nav.sales_returns")
                .icon("refresh-cw")
                .group("nav.app")
                .order(53)
                .permission(permissions::RETURNS_REQUEST),
        );
        app.nav(
            "app",
            NavLink::route("wl_sales.loyalty", "nav.loyalty")
                .icon("gift")
                .group("nav.app")
                .order(54)
                .permission(permissions::LOYALTY_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_sales.segments", "nav.segments")
                .icon("users")
                .group("nav.app")
                .order(55)
                .permission(permissions::SEGMENTS_MANAGE),
        );
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
        assert!(config.campaign_required_sources.is_empty());
        assert_eq!(config.attribution_window_months, None);
        assert!(config.acquisition_change_approval);
    }

    #[test]
    fn the_builder_changes_the_settings() {
        let module = WlSales::new()
            .over_budget_approval(false)
            .budget_alerts(&[100, 50, 100])
            .late_response_days(14)
            .campaign_required_for(&["paid_ads", "social"])
            .attribution_window_months(Some(12))
            .acquisition_change_approval(false)
            .lost_reasons(&["Too Far", "duplicate"]);
        let config = module.config();
        assert!(config.campaign_required_for("social"));
        assert_eq!(config.attribution_window_months, Some(12));
        assert!(!config.acquisition_change_approval);
        assert_eq!(
            config.lost_reasons,
            ["duplicate", "spam_or_invalid", "too far"]
        );
        assert!(!config.over_budget_approval);
        assert_eq!(config.budget_alerts, [5000, 10000]);
        assert_eq!(config.late_response_days, 14);
    }
}
