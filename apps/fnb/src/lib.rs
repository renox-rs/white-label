#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod cancel;
pub mod catalogue;
pub mod changes;
pub mod close;
pub mod cod;
mod events;
pub mod failed;
pub mod fulfilment;
pub mod kitchen;
pub mod money;
pub mod notices;
pub mod order;
pub mod order_machine;
pub mod pages;
pub mod payout;
pub mod payout_csv;
pub mod payout_machine;
pub mod payout_pages;
mod payout_texts;
pub mod payouts;
pub mod permissions;
pub mod pickup;
pub mod placing;
pub mod progress;
pub mod refunds;
pub mod reversal;
mod screen_texts;
pub mod seed;
pub mod settings;
mod texts;
pub mod ticket;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use catalogue::{Brand, MenuItem, Outlet, RecipeLine};
pub use changes::{ChangeNote, ExpectedRefund};
pub use cod::{CustomerFlag, PaymentOptions};
pub use order::{FnbOrder, FnbOrderStatus, SUBJECT_TYPE};
pub use order_machine::OrderInput;
pub use payout::{
    ImportPreset, LateOrder, PayoutLine, PayoutStatement, PayoutStatus, ReasonMap,
    STATEMENT_SERIES,
};
pub use payout_machine::PayoutInput;
pub use refunds::RefundAsk;
pub use settings::FnbSettings;
pub use ticket::{KitchenTicket, TicketStatus};

/// The F&B module. Add it to the app with `.module(fnb::Fnb::new())`.
#[derive(Default)]
pub struct Fnb {
    settings: FnbSettings,
}

impl Fnb {
    /// The module with the spec's defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// The settings as built, for a page that shows what is in force.
    pub fn settings(&self) -> &FnbSettings {
        &self.settings
    }

    /// Whether a prepaid order is refunded in full after a failure on the
    /// customer's side too (off: the spec's default until the lawyer answers
    /// open item L-33).
    pub fn customer_side_refund(mut self, on: bool) -> Self {
        self.settings.customer_side_refund = on;
        self
    }

    /// The days after a failed delivery was closed in which a cashier may ask
    /// for a manager-approved refund (`FNB.ACL.17`, default 7).
    pub fn refund_request_days(mut self, days: i64) -> Self {
        self.settings.refund_request_days = days;
        self
    }

    /// Cash on delivery goes off after `failures` customer-side failures of
    /// cash orders within `days` days (`FNB.RUL.14`, default 2 and 90).
    pub fn cod_failures(mut self, failures: i64, days: i64) -> Self {
        self.settings.cod_failures = failures;
        self.settings.cod_window_days = days;
        self
    }

    /// The minutes after an aggregator order is `ready` when the kitchen
    /// manager is told to chase the aggregator, and when the task to record
    /// what happened opens (`FNB.RUL.16`, default 15 and 45).
    pub fn aggregator_pickup(mut self, chase_minutes: i64, task_minutes: i64) -> Self {
        self.settings.pickup_chase_minutes = chase_minutes;
        self.settings.pickup_task_minutes = task_minutes;
        self
    }

    /// Turns `FNB.RUL.16` on or off (default on).
    pub fn aggregator_pickup_rule(mut self, on: bool) -> Self {
        self.settings.pickup_rule = on;
        self
    }

    /// The unmatched amount, in fils, above which an imported payout statement
    /// gives the accountant a reconciliation task (`FNB.RUL.10`, default AED
    /// 100), and whether the rule is on.
    pub fn payout_task(mut self, fils: i64, on: bool) -> Self {
        self.settings.payout_task_fils = fils;
        self.settings.payout_rule = on;
        self
    }

    /// Turns `FNB.RUL.14` on or off (default on).
    pub fn cod_rule(mut self, on: bool) -> Self {
        self.settings.cod_rule = on;
        self
    }
}

impl Module for Fnb {
    fn name(&self) -> &'static str {
        "fnb"
    }

    fn routes(&self) -> Routes {
        pages::routes().merge(payout_pages::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<pages::KitchenPage>()
            .view_data::<pages::CodPage>()
            .view_data::<pages::CheckoutPage>()
            .view_data::<pages::MyOrderPage>()
            .view_data::<payout_pages::PayoutsPage>()
            .view_data::<payout_pages::ShowPayoutPage>();
        app.templates(|env| {
            for (name, source) in pages::VIEWS.iter().chain(payout_pages::VIEWS) {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Brand>()
            .model::<Outlet>()
            .model::<MenuItem>()
            .model::<RecipeLine>()
            .model::<FnbOrder>()
            .model::<KitchenTicket>()
            .model::<ChangeNote>()
            .model::<ExpectedRefund>()
            .model::<RefundAsk>()
            .model::<CustomerFlag>()
            .model::<PayoutStatement>()
            .model::<PayoutLine>()
            .model::<ReasonMap>()
            .model::<ImportPreset>()
            .model::<LateOrder>();
        // The order and the ticket have machines of their own.
        renox::state_machine::register::<FnbOrder>();
        renox::state_machine::register::<KitchenTicket>();
        // The payout statement is a voidable subject with its own approval
        // action, and the order a statement names that the kitchen never
        // recorded is a late entry.
        payout_machine::register();
        payouts::register_late_entry();
        // The settings the rules read.
        app.provide(self.settings.clone());
        // The kitchen tells the sales and stock modules which lines it started
        // and what a line is made of (`SAL.ACL.05`, `FNB.ACL.16`).
        wl_kernel::sales::provide_fulfilment_progress(app, progress::KitchenProgress);
        wl_kernel::inventory::provide_sale_line_stock(app, progress::KitchenStock);
        notices::register_types();
        // The order follows the sales order and the delivery.
        events::register(app);
        pickup::register(app);
        // In the app group of the staff sidebar; the labels are the host's
        // `nav.orders`, `nav.kitchen` and `nav.cod_customers` texts.
        app.nav(
            "app",
            NavLink::route("fnb.orders", "nav.orders")
                .icon("utensils")
                .group("nav.app")
                .order(8)
                .permission(permissions::ORDERS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("fnb.kitchen", "nav.kitchen")
                .icon("chef-hat")
                .group("nav.app")
                .order(9)
                .permission(permissions::KDS_WORK),
        );
        app.nav(
            "app",
            NavLink::route("fnb.payouts", "nav.payouts")
                .icon("file-spreadsheet")
                .group("nav.app")
                .order(11)
                .permission(permissions::RECONCILIATION_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("fnb.cod_customers", "nav.cod_customers")
                .icon("banknote")
                .group("nav.app")
                .order(10)
                .permission(permissions::CUSTOMERS_COD),
        );
    }
}
