//! An export sale and its exit evidence (08 §2 re-export, §4.1 *When it goes
//! wrong* last row, `AUT.EXP.01`, `AUT.RUL.15`, BL-AUT-14).
//!
//! Re-exported parts are zero-rated only if they leave the UAE within 90 days
//! with a customs exit certificate, a bill of lading or an airway bill. The
//! accountant registers the zero-rated invoice as an export sale with its
//! **supply date** (the delivery or collection date), and attaches the
//! evidence to it as it arrives. The rule that watches the days is
//! [`crate::export_rule`]; the services are [`crate::export_sales`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;

/// The subject type of an export sale (`auto.export_sale:7`).
pub const EXPORT_SALE_SUBJECT_TYPE: &str = "auto.export_sale";

/// A customs exit certificate.
pub const EXIT_CERTIFICATE: &str = "exit_certificate";
/// A bill of lading.
pub const BILL_OF_LADING: &str = "bill_of_lading";
/// An airway bill.
pub const AIRWAY_BILL: &str = "airway_bill";
/// The kinds of exit evidence: any one of them stops the rule.
pub const EVIDENCE_KINDS: &[&str] = &[EXIT_CERTIFICATE, BILL_OF_LADING, AIRWAY_BILL];

/// A zero-rated export sale.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_export_sales", unique(invoice_id), index(party_id))]
pub struct ExportSale {
    /// The primary key.
    pub id: i64,
    /// The zero-rated invoice (wl-invoicing).
    pub invoice_id: i64,
    /// The invoice's number, when it has one.
    pub invoice_number: Option<String>,
    /// The customer.
    pub party_id: i64,
    /// The invoice's currency.
    pub currency: String,
    /// The invoice's total, in fils of its currency.
    pub gross_total: i64,
    /// The delivery or collection date: the day the 60 and 90 days count from.
    pub supply_date: NaiveDate,
    /// When the accountant was told at 60 days.
    pub notified_at: Option<DateTime>,
    /// When the sale was flagged *5% VAT due* at 90 days.
    pub vat_flagged_at: Option<DateTime>,
    /// When the accountant was told that evidence came after the flag.
    pub late_evidence_notified_at: Option<DateTime>,
    /// The user who registered it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ExportSale {
    /// What people read: the invoice's number, else `invoice #7`.
    pub fn number(&self) -> String {
        self.invoice_number
            .clone()
            .unwrap_or_else(|| format!("#{}", self.invoice_id))
    }

    /// The sale as a [`Subject`]: `auto.export_sale:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(EXPORT_SALE_SUBJECT_TYPE, self.id)
    }

    /// The invoice as a [`Subject`], which the accountant's task links.
    pub fn invoice_subject(&self) -> Subject {
        Subject::new(INVOICE_SUBJECT_TYPE, self.invoice_id)
    }

    /// The calendar days from the supply date to `on`.
    pub fn days_since_supply(&self, on: NaiveDate) -> i64 {
        (on - self.supply_date).num_days()
    }

    /// The evidence attached so far, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn evidence(&self, db: &Db) -> Result<Vec<ExportEvidence>> {
        ExportEvidence::query()
            .where_(ExportEvidence::EXPORT_SALE_ID.eq(self.id))
            .order_by(ExportEvidence::ID)
            .get(db)
            .await
    }
}

/// A piece of exit evidence attached to an export sale.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_export_evidence", index(export_sale_id))]
pub struct ExportEvidence {
    /// The primary key.
    pub id: i64,
    /// The export sale.
    #[model(references = "auto_export_sales")]
    pub export_sale_id: i64,
    /// One of [`EVIDENCE_KINDS`].
    pub kind: String,
    /// The reference of the document: its number or its file name.
    pub reference: String,
    /// The user who attached it.
    pub attached_by: Option<i64>,
    /// When it was attached.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap()
    }

    #[test]
    fn the_days_count_from_the_supply_date() {
        let sale = ExportSale {
            id: 3,
            invoice_id: 140,
            supply_date: day(8, 1),
            ..ExportSale::default()
        };
        assert_eq!(sale.days_since_supply(day(9, 30)), 60);
        assert_eq!(sale.days_since_supply(day(10, 30)), 90);
        assert_eq!(sale.number(), "#140");
        assert_eq!(sale.subject().to_string(), "auto.export_sale:3");
        assert_eq!(sale.invoice_subject().to_string(), "invoicing.invoice:140");
    }

    #[test]
    fn three_kinds_of_evidence_are_known() {
        assert_eq!(EVIDENCE_KINDS.len(), 3);
    }
}
