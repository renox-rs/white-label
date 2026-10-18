#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod allocation;
pub mod config;
pub mod invoice;
pub mod invoicer;
pub mod line;
mod machine;
mod texts;
pub mod vat;

use renox::db::Migration;
use renox::prelude::*;

pub use allocation::Allocation;
pub use config::{InvoicingConfig, WriteOffReason};
pub use invoice::{Invoice, InvoiceKind, PartySnapshot};
pub use invoicer::Invoices;
pub use line::InvoiceLine;
pub use vat::{CategoryTotal, LineAmounts, VatTotals};

/// The module's permissions (wl-invoicing §6).
pub mod permissions {
    /// See invoices and credit notes.
    pub const VIEW: &str = "invoices.view";
    /// Create, change and discard draft invoices.
    pub const CREATE: &str = "invoices.create";
    /// Issue an invoice.
    pub const ISSUE: &str = "invoices.issue";
    /// Credit an invoice and issue credit notes.
    pub const CREDIT: &str = "invoices.credit";
    /// Request a bad-debt write-off (`FIN.ACL.15`).
    pub const WRITE_OFF: &str = "invoices.write_off";
    /// Raise and resolve an invoice dispute.
    pub const DISPUTE: &str = "invoices.dispute";
    /// See receivables: balances, ageing and reminders.
    pub const RECEIVABLES_VIEW: &str = "receivables.view";
}

/// The invoicing module. The host adds it with
/// `.module(wl_invoicing::WlInvoicing::new())` when `client.toml` enables
/// `invoicing`.
#[derive(Default)]
pub struct WlInvoicing {
    config: InvoicingConfig,
}

impl WlInvoicing {
    /// The module with the §7 defaults ([`InvoicingConfig::default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// Days from the issue date to the due date when a draft has none.
    pub fn payment_terms_days(mut self, days: u32) -> Self {
        self.config.payment_terms_days = days;
        self
    }

    /// The reminder steps (`FIN.RUL.01`), in days relative to the due date
    /// (negative before it).
    pub fn reminder_steps(mut self, days: &[i32]) -> Self {
        let mut steps = days.to_vec();
        steps.sort_unstable();
        steps.dedup();
        self.config.reminder_steps = steps;
        self
    }

    /// Adds a write-off reason code, or changes whether an existing one
    /// needs a text.
    pub fn write_off_reason(mut self, code: &str, text_required: bool) -> Self {
        let reasons = &mut self.config.write_off_reasons;
        match reasons.iter_mut().find(|r| r.code == code) {
            Some(reason) => reason.text_required = text_required,
            None => reasons.push(WriteOffReason::new(code, text_required)),
        }
        self
    }

    /// Whether the customer write-off notice is sent on `write_off`.
    pub fn write_off_notice(mut self, send: bool) -> Self {
        self.config.write_off_notice = send;
        self
    }

    /// The roles whose holders approve a write-off by default (`FIN.ACL.15`).
    pub fn write_off_approvers(mut self, roles: &[&str]) -> Self {
        self.config.write_off_approvers = roles.iter().map(|r| (*r).to_owned()).collect();
        self
    }

    /// The module's settings.
    pub fn config(&self) -> &InvoicingConfig {
        &self.config
    }
}

impl Module for WlInvoicing {
    fn name(&self) -> &'static str {
        "wl-invoicing"
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Invoice>()
            .model::<InvoiceLine>()
            .model::<Allocation>();
        // `rnx state:diagram Invoice` and the admin panel find the machine.
        renox::state_machine::register::<Invoice>();
        // Other modules draft and read invoices through the kernel (§6).
        wl_kernel::invoicing::provide_invoicer(app, Invoices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builder_changes_the_defaults() {
        let module = WlInvoicing::new()
            .payment_terms_days(14)
            .reminder_steps(&[7, -3, 7])
            .write_off_reason("deceased", false)
            .write_off_reason("other", false)
            .write_off_notice(false)
            .write_off_approvers(&["admin"]);
        let config = module.config();
        assert_eq!(config.payment_terms_days, 14);
        assert_eq!(config.reminder_steps, [-3, 7]);
        assert!(config.write_off_reason("deceased").is_some());
        assert!(!config.write_off_reason("other").unwrap().text_required);
        assert!(!config.write_off_notice);
        assert_eq!(config.write_off_approvers, ["admin"]);
    }
}
