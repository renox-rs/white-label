#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod allocate;
pub mod allocation;
pub mod cheque;
pub mod cheque_bounce;
pub mod cheque_chase;
pub mod cheque_deposit;
mod cheque_events;
pub mod cheque_legal;
mod cheque_machine;
mod cheque_money;
pub mod cheque_pages;
pub mod cheque_represent;
pub mod cheque_settle;
pub mod cheque_write_off;
pub mod chequer;
pub mod config;
pub mod credit;
pub mod deposit;
pub mod deposit_adjust;
pub mod deposit_deductions;
mod deposit_events;
mod deposit_machine;
pub mod deposit_pages;
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
pub mod refund_recover;
pub mod refund_retry;
pub mod refund_pages;
pub mod reverse;
pub mod seed;
mod texts;

use std::sync::Arc;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use allocation::Allocation;
pub use cheque::{Cheque, ChequeLink, ChequeReturn};
pub use config::PaymentsConfig;
pub use deposit::Deposit;
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
    (
        "wl-payments/refunds.html",
        include_str!("../views/wl-payments/refunds.html"),
    ),
    (
        "wl-payments/refund.html",
        include_str!("../views/wl-payments/refund.html"),
    ),
    (
        "wl-payments/_fail_sheet.html",
        include_str!("../views/wl-payments/_fail_sheet.html"),
    ),
    (
        "wl-payments/deposits.html",
        include_str!("../views/wl-payments/deposits.html"),
    ),
    (
        "wl-payments/deposit.html",
        include_str!("../views/wl-payments/deposit.html"),
    ),
    (
        "wl-payments/cheques.html",
        include_str!("../views/wl-payments/cheques.html"),
    ),
    (
        "wl-payments/cheque.html",
        include_str!("../views/wl-payments/cheque.html"),
    ),
    (
        "wl-payments/_bounce_sheet.html",
        include_str!("../views/wl-payments/_bounce_sheet.html"),
    ),
    (
        "wl-payments/_represent_sheet.html",
        include_str!("../views/wl-payments/_represent_sheet.html"),
    ),
    (
        "wl-payments/_refer_sheet.html",
        include_str!("../views/wl-payments/_refer_sheet.html"),
    ),
    (
        "wl-payments/_settle_sheet.html",
        include_str!("../views/wl-payments/_settle_sheet.html"),
    ),
    (
        "wl-payments/_replacement_sheet.html",
        include_str!("../views/wl-payments/_replacement_sheet.html"),
    ),
    (
        "wl-payments/_publish_sheet.html",
        include_str!("../views/wl-payments/_publish_sheet.html"),
    ),
    (
        "wl-payments/_answer_sheet.html",
        include_str!("../views/wl-payments/_answer_sheet.html"),
    ),
    (
        "wl-payments/_forfeit_sheet.html",
        include_str!("../views/wl-payments/_forfeit_sheet.html"),
    ),
    (
        "wl-payments/_deposit_refund_sheet.html",
        include_str!("../views/wl-payments/_deposit_refund_sheet.html"),
    ),
    (
        "wl-payments/_adjust_sheet.html",
        include_str!("../views/wl-payments/_adjust_sheet.html"),
    ),
    (
        "wl-payments/_transfer_sheet.html",
        include_str!("../views/wl-payments/_transfer_sheet.html"),
    ),
    (
        "wl-payments/_write_off_sheet.html",
        include_str!("../views/wl-payments/_write_off_sheet.html"),
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
    /// See deposits.
    pub const DEPOSITS_VIEW: &str = "deposits.view";
    /// Hold a deposit and start its settlement.
    pub const DEPOSITS_MANAGE: &str = "deposits.manage";
    /// Refund a deposit.
    pub const DEPOSITS_REFUND: &str = "deposits.refund";
    /// See cheques.
    pub const CHEQUES_VIEW: &str = "cheques.view";
    /// Deposit, clear, bounce and re-present a cheque.
    pub const CHEQUES_DEPOSIT: &str = "cheques.deposit";
    /// Settle part of a bounced cheque, replace it and write it off.
    pub const CHEQUES_MANAGE: &str = "cheques.manage";
    /// Refer a bounced cheque to a legal case.
    pub const CHEQUES_LEGAL: &str = "cheques.legal";
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

    /// Sets when and how often a failed gateway refund is retried
    /// (`FIN.RUL.21`, §7: once, after 24 hours unless set). `hours` at least
    /// 1; `max_retries` counts the automatic and the manual retries of one
    /// refund together.
    ///
    /// # Panics
    ///
    /// For fewer than one hour: a programming error.
    pub fn refund_retry(mut self, hours: i64, max_retries: i64) -> Self {
        assert!(hours >= 1, "a retry waits at least one hour");
        self.config.refund_retry_hours = hours;
        self.config.refund_max_retries = max_retries.max(0);
        self
    }

    /// Sets the bounce fee added when a received cheque bounces (§7: none
    /// unless the vertical sets it): the `charge_type` of the client's
    /// charge catalogue and the `amount` before VAT, in fils.
    ///
    /// # Panics
    ///
    /// For an amount of 0 or less: a programming error.
    pub fn bounce_fee(mut self, charge_type: &str, amount: i64) -> Self {
        assert!(amount > 0, "a bounce fee is above 0");
        self.config.bounce_fee = Some(config::BounceFee {
            charge_type: charge_type.to_owned(),
            amount,
        });
        self
    }

    /// Sets the VAT treatment of the bounce fee (standard rated unless set,
    /// `⚠ open: accountant A-05`).
    pub fn bounce_fee_vat(mut self, vat: wl_kernel::invoicing::VatCategory) -> Self {
        self.config.bounce_fee_vat = vat;
        self
    }

    /// Sets when a bounced cheque is chased (`FIN.RUL.18`, §7: 3 and 14 days
    /// after the bounce): `remind_days` for the reminders to the party and the
    /// responsible staff, `decide_days` for the task to the accountant and the
    /// manager.
    ///
    /// # Panics
    ///
    /// For fewer than one day, or a task before the reminders: a programming
    /// error.
    pub fn cheque_chase(mut self, remind_days: i64, decide_days: i64) -> Self {
        assert!(remind_days >= 1, "a reminder waits at least one day");
        assert!(
            decide_days >= remind_days,
            "the decision task comes with or after the reminders"
        );
        self.config.cheque_reminder_days = remind_days;
        self.config.cheque_decision_days = decide_days;
        self
    }

    /// Lets the party dispute the published deposit deductions in the portal
    /// (§7, off unless set; 01 Property turns it on): `window_days` after
    /// each publication (14 by default) and `response_days` for staff to
    /// answer (7). While on, `refund` and `forfeit` of a deposit wait until
    /// the window of the latest published version has passed.
    ///
    /// # Panics
    ///
    /// For fewer than one day: a programming error.
    pub fn deductions_dispute(mut self, window_days: i64, response_days: i64) -> Self {
        assert!(window_days >= 1, "a dispute window is at least one day");
        assert!(response_days >= 1, "an answer takes at least one day");
        self.config.deductions_dispute = true;
        self.config.dispute_window_days = window_days;
        self.config.dispute_response_days = response_days;
        self
    }

    /// Sets how many times one cheque may be presented, the first deposit
    /// included (§7: 2, one re-presentation).
    pub fn cheque_presentations(mut self, max: i64) -> Self {
        self.config.cheque_max_presentations = max.max(1);
        self
    }

    /// Sets the bank return reasons that allow a criminal complaint (§7;
    /// compared without case).
    pub fn criminal_grounds(mut self, reasons: &[&str]) -> Self {
        self.config.criminal_grounds = reasons.iter().map(|r| r.trim().to_lowercase()).collect();
        self
    }

    /// Sets the open amount a legal case needs to be above, in fils (§7: 0).
    pub fn legal_minimum(mut self, amount: i64) -> Self {
        self.config.legal_minimum_amount = amount.max(0);
        self
    }

    /// Turns the cash-handover rule for cheques collected in the field on or
    /// off (`FIN.ACL.20`, on unless set).
    pub fn field_cheque_handover(mut self, on: bool) -> Self {
        self.config.field_cheque_handover = on;
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
            .merge(refund_pages::routes())
            .merge(deposit_pages::routes())
            .merge(cheque_pages::routes())
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Payment>();
        app.model::<Allocation>();
        app.model::<Refund>();
        app.model::<RefundAttempt>();
        app.model::<Deposit>();
        app.model::<deposit_adjust::DepositAdjustment>();
        app.model::<deposit_deductions::DepositDeduction>();
        app.model::<deposit_deductions::DeductionLine>();
        app.model::<deposit_deductions::DepositDispute>();
        app.model::<Cheque>();
        app.model::<ChequeLink>();
        app.model::<ChequeReturn>();
        app.model::<cheque_settle::ChequeReplacement>();
        app.model::<cheque_write_off::ChequeWriteOff>();
        app.model::<FollowupTask>();
        app.view_data::<pages::IndexPage>()
            .view_data::<pages::NewPage>()
            .view_data::<pages::ShowPage>()
            .view_data::<pages::TasksPage>()
            .view_data::<refund_pages::IndexPage>()
            .view_data::<refund_pages::ShowPage>()
            .view_data::<deposit_pages::IndexPage>()
            .view_data::<deposit_pages::ShowPage>()
            .view_data::<cheque_pages::IndexPage>()
            .view_data::<cheque_pages::ShowPage>();
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
        app.nav(
            "app",
            NavLink::route("wl_payments.refunds.index", "nav.refunds")
                .icon("undo-2")
                .group("nav.app")
                .order(43)
                .permission(permissions::VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_payments.deposits.index", "nav.deposits")
                .icon("piggy-bank")
                .group("nav.app")
                .order(44)
                .permission(permissions::DEPOSITS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_payments.cheques.index", "nav.cheques")
                .icon("landmark")
                .group("nav.app")
                .order(45)
                .permission(permissions::CHEQUES_VIEW),
        );
        // `rnx state:diagram Payment` and the admin panel find the machine.
        renox::state_machine::register::<Payment>();
        renox::state_machine::register::<Refund>();
        renox::state_machine::register::<Deposit>();
        renox::state_machine::register::<Cheque>();
        // The settings the services read (`config_of`).
        app.provide(self.config.clone());
        // The gateway the services ask about disputes (`gateway::gateway_of`).
        if let Some(gateway) = &self.gateway {
            app.provide(gateway::Installed(gateway.clone()));
        }
        // How other modules record, allocate and read payments.
        wl_kernel::payments::provide_payment_recorder(app, recorder::Payments);
        // How a vertical registers the post-dated cheques of an agreement.
        wl_kernel::cheques::provide_cheques(app, chequer::ChequeBook);
        // `FIN.RUL.03`: the cheques whose date is close go on the deposit list.
        cheque_deposit::register(app);
        // `FIN.RUL.18`: a bounced cheque is chased.
        cheque_chase::register(app);
        // The bounce fee, `ChequeBounced` and the follow-ups after a commit.
        cheque_events::register(app);
        // `InvoiceWrittenOff` completes a cheque `write_off` that waits for approvals.
        cheque_write_off::register(app);
        // The notification types of a bank return and a follow-up task.
        notices::register_types();
        // `PaymentReceived` and `PaymentReversed` after a commit.
        events::register(app);
        // `DepositSettled` after a commit.
        deposit_events::register(app);
        // The held amount follows the part refund of an adjustment.
        deposit_adjust::register(app);
        // How a vertical adjusts the deposit of an agreement that changed.
        wl_kernel::deposits::provide_deposits(app, deposit_adjust::DepositBook);
        // The published deductions the portal shows and the party's dispute.
        wl_kernel::deposits::provide_deposit_deductions(app, deposit_deductions::Deductions);
        // `RefundPaid` after a commit.
        refund_machine::register(app);
        // `RefundFailed` and its notices after a commit.
        refund_fail::register(app);
        // `FIN.RUL.21`: a failed gateway refund is retried once; otherwise
        // the accountant changes its method.
        refund_retry::register(app);
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
