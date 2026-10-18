#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod allocate;
pub mod allocation;
pub mod config;
pub mod credit;
mod events;
pub mod followup;
pub mod gateway;
mod machine;
pub mod notices;
pub mod pages;
pub mod payment;
pub mod record;
pub mod recorder;
pub mod refund;
pub mod refund_attempt;
mod refund_fail;
mod refund_machine;
pub mod reverse;
pub mod seed;
mod texts;

use std::sync::Arc;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use allocation::Allocation;
pub use config::PaymentsConfig;
pub use followup::FollowupTask;
pub use gateway::{Gateway, NoGateway};
pub use payment::Payment;
pub use refund::Refund;
pub use refund_attempt::RefundAttempt;

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-payments/index.html",
        include_str!("../views/wl-payments/index.html"),
    ),
    (
        "wl-payments/new.html",
        include_str!("../views/wl-payments/new.html"),
    ),
    (
        "wl-payments/show.html",
        include_str!("../views/wl-payments/show.html"),
    ),
    (
        "wl-payments/_allocate_sheet.html",
        include_str!("../views/wl-payments/_allocate_sheet.html"),
    ),
    (
        "wl-payments/_reverse_sheet.html",
        include_str!("../views/wl-payments/_reverse_sheet.html"),
    ),
    (
        "wl-payments/tasks.html",
        include_str!("../views/wl-payments/tasks.html"),
    ),
];

/// The module's settings in `state`: those `register` provided, else the
/// §7 defaults.
pub(crate) fn config_of(state: &AppState) -> Arc<PaymentsConfig> {
    state
        .provided::<PaymentsConfig>()
        .unwrap_or_else(|| Arc::new(PaymentsConfig::default()))
}

/// The module's permissions (wl-payments §6).
pub mod permissions {
    /// See payments, receipts and allocations.
    pub const VIEW: &str = "payments.view";
    /// Record payments and allocate them.
    pub const RECORD: &str = "payments.record";
    /// Reverse a payment.
    pub const REVERSE: &str = "payments.reverse";
    /// Handle chargebacks.
    pub const CHARGEBACK: &str = "payments.chargeback";
    /// Request a refund.
    pub const REFUNDS_REQUEST: &str = "refunds.request";
    /// Approve or reject a refund (`FIN.ACL.04`).
    pub const REFUNDS_APPROVE: &str = "refunds.approve";
    /// Pay an approved refund.
    pub const REFUNDS_PAY: &str = "refunds.pay";
}

/// The payments module. The host adds it with
/// `.module(wl_payments::WlPayments::new())` when `client.toml` enables
/// `payments`.
#[derive(Default)]
pub struct WlPayments {
    config: PaymentsConfig,
    gateway: Option<Arc<dyn Gateway>>,
}

impl WlPayments {
    /// The module with the §7 defaults ([`PaymentsConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// The enabled payment methods, by code (`cash`, `transfer`, `cheque`,
    /// `card`, ...). Repeated codes count once.
    pub fn methods(mut self, methods: &[&str]) -> Self {
        let mut list: Vec<String> = Vec::new();
        for method in methods {
            if !list.iter().any(|m| m == method) {
                list.push((*method).to_owned());
            }
        }
        self.config.methods = list;
        self
    }

    /// Installs the payment gateway; without one, [`NoGateway`] knows no
    /// disputes.
    pub fn gateway(mut self, gateway: impl Gateway) -> Self {
        self.gateway = Some(Arc::new(gateway));
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &PaymentsConfig {
        &self.config
    }
}

impl Module for WlPayments {
    fn name(&self) -> &'static str {
        "wl-payments"
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Payment>();
        app.model::<Allocation>();
        app.model::<Refund>();
        app.model::<RefundAttempt>();
        app.model::<FollowupTask>();
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<pages::TasksPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        // `db:seed`: the demo receipts.
        app.seeder(|state: AppState| async move { seed::run(&state).await.map(|_| ()) });
        // In the app group of the staff sidebar; the label is the app's
        // `nav.payments` text.
        app.nav(
            "app",
            NavLink::route("wl_payments.index", "nav.payments")
                .icon("wallet")
                .group("nav.app")
                .order(42)
                .permission(permissions::VIEW),
        );
        // `rnx state:diagram Payment` and the admin panel find the machine.
        renox::state_machine::register::<Payment>();
        renox::state_machine::register::<Refund>();
        // The settings the services read (`config_of`).
        app.provide(self.config.clone());
        // The gateway the services ask about disputes (`gateway::gateway_of`).
        if let Some(gateway) = &self.gateway {
            app.provide(gateway::Installed(gateway.clone()));
        }
        // How other modules record, allocate and read payments.
        wl_kernel::payments::provide_payment_recorder(app, recorder::Payments);
        // The notification types of a bank return and a follow-up task.
        notices::register_types();
        // `PaymentReceived` and `PaymentReversed` after a commit.
        events::register(app);
        // `RefundPaid` after a commit.
        refund_machine::register(app);
        // `RefundFailed` and its notices after a commit.
        refund_fail::register(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_enable_cash_transfer_cheque_and_card() {
        let module = WlPayments::new();
        let config = module.config();
        for method in ["cash", "transfer", "cheque", "card"] {
            assert!(config.method_enabled(method), "{method}");
        }
        assert!(!config.method_enabled("set_off"));
    }

    #[test]
    fn the_builder_changes_the_methods() {
        let module = WlPayments::new().methods(&["cash", "set_off", "cash"]);
        assert_eq!(module.config().methods, ["cash", "set_off"]);
        assert!(!module.config().method_enabled("card"));
    }

    #[test]
    fn a_gateway_is_installed_with_the_builder() {
        assert!(WlPayments::new().gateway.is_none());
        let module = WlPayments::new().gateway(gateway::FakeGateway::new());
        assert_eq!(module.gateway.as_ref().map(|g| g.provider()), Some("fake"));
    }
}
