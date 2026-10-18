#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod allocate;
pub mod allocation;
pub mod config;
pub mod credit;
mod events;
mod machine;
pub mod payment;
pub mod record;
pub mod recorder;
mod texts;

use std::sync::Arc;

use renox::db::Migration;
use renox::prelude::*;

pub use allocation::Allocation;
pub use config::PaymentsConfig;
pub use payment::Payment;

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

    fn register(&self, app: &mut Registry) {
        app.model::<Payment>();
        app.model::<Allocation>();
        // `rnx state:diagram Payment` and the admin panel find the machine.
        renox::state_machine::register::<Payment>();
        // The settings the services read (`config_of`).
        app.provide(self.config.clone());
        // How other modules record, allocate and read payments.
        wl_kernel::payments::provide_payment_recorder(app, recorder::Payments);
        // `PaymentReceived` after a commit.
        events::register(app);
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
}
