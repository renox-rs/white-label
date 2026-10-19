#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod bill_links;
pub mod budget;
pub mod campaign;
pub mod config;
pub mod cost_entry;
pub mod cost_machine;
pub mod costs;
pub mod daily;
mod events;
pub mod lead_source;
pub mod machine;
pub mod notices;
pub mod pages;
pub mod policy;
pub mod promo_code;
pub mod seed;
mod texts;
pub mod voids;

use renox::db::Migration;
use renox::prelude::*;

pub use budget::BudgetAlert;
pub use campaign::{
    Campaign, CampaignCode, CampaignRefused, CampaignStatus, MarketingChannel, Purpose,
};
pub use config::SalesConfig;
pub use cost_entry::{CostEntry, CostRefused, CostStatus};
pub use lead_source::{LeadSource, LeadSourceRefused};
pub use promo_code::PromoCode;

/// The campaign services under the names of the plan: `campaigns::delete`
/// and `campaigns::run_daily`.
pub mod campaigns {
    pub use crate::campaign::{DELETE_CHECKS, DeleteCheck, delete, delete_with};
    pub use crate::daily::{Report, run_daily};
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[(
    "wl-sales/lead_sources.html",
    include_str!("../views/wl-sales/lead_sources.html"),
)];

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

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::lead_sources::IndexPage>();
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
        app.model::<Campaign>();
        app.model::<CampaignCode>();
        app.model::<PromoCode>();
        app.model::<CostEntry>();
        app.model::<BudgetAlert>();
        app.provide(self.config.clone());
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
        app.seeder(|state: AppState| async move { seed::install(&state.db).await.map(|_| ()) });
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
