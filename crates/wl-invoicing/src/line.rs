//! An invoice line (wl-invoicing §3): what is billed, its quantity, price,
//! discount, VAT category and the amounts computed from them
//! ([`crate::vat`]).

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, VatCategory};

use crate::vat::LineAmounts;

/// One line of an [`Invoice`](crate::invoice::Invoice). Money is `i64` fils
/// of the invoice's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_invoice_lines", unique(invoice_id, position))]
pub struct InvoiceLine {
    /// The primary key.
    pub id: i64,
    /// The invoice.
    pub invoice_id: i64,
    /// The order on the invoice, from 1.
    pub position: i64,
    /// What is billed.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, before VAT.
    pub unit_price: i64,
    /// The discount on the line, before VAT.
    pub discount: i64,
    /// Quantity × unit price − discount.
    pub net_amount: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// The rate charged, in basis points (500 = 5%).
    pub vat_rate_bp: i64,
    /// The line VAT, rounded half-up to fils.
    pub vat_amount: i64,
    /// The line VAT in AED fils; `None` while the rate is unknown.
    pub vat_amount_aed: Option<i64>,
    /// Net plus VAT.
    pub gross_amount: i64,
    /// The type of the record the line bills, if any.
    pub origin_type: Option<String>,
    /// The id of the record the line bills, if any.
    pub origin_id: Option<i64>,
    /// The charge type code, which budget vs actual maps to a budget
    /// category (`FIN.BUD.01`).
    pub charge_type: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl InvoiceLine {
    /// The line at `position` of invoice `invoice_id` for `draft`, its
    /// amounts computed at `rate_micro` to AED when known.
    pub fn from_draft(
        invoice_id: i64,
        position: i64,
        draft: &DraftLine,
        rate_micro: Option<i64>,
    ) -> Self {
        let mut line = InvoiceLine {
            invoice_id,
            position,
            description: draft.description.trim().to_owned(),
            quantity_milli: draft.quantity_milli,
            unit_price: draft.unit_price,
            discount: draft.discount,
            vat_category: draft.vat_category,
            origin_type: draft.origin.as_ref().map(|o| o.subject_type.clone()),
            origin_id: draft.origin.as_ref().map(|o| o.subject_id),
            charge_type: draft.charge_type.as_deref().map(str::to_lowercase),
            ..Default::default()
        };
        line.set_amounts(&line.compute(rate_micro));
        line
    }

    /// The amounts of this line at `rate_micro` to AED.
    pub fn compute(&self, rate_micro: Option<i64>) -> LineAmounts {
        LineAmounts::compute(
            self.quantity_milli,
            self.unit_price,
            self.discount,
            self.vat_category,
            rate_micro,
        )
    }

    /// Stores `amounts` as the line's computed columns.
    pub fn set_amounts(&mut self, amounts: &LineAmounts) {
        self.net_amount = amounts.net;
        self.vat_rate_bp = amounts.rate_bp;
        self.vat_amount = amounts.vat;
        self.vat_amount_aed = amounts.vat_aed;
        self.gross_amount = amounts.gross();
    }

    /// The stored amounts of this line.
    pub fn amounts(&self) -> LineAmounts {
        LineAmounts {
            category: self.vat_category,
            rate_bp: self.vat_rate_bp,
            net: self.net_amount,
            vat: self.vat_amount,
            vat_aed: self.vat_amount_aed,
        }
    }

    /// The record the line bills, if any.
    pub fn origin(&self) -> Option<Subject> {
        match (&self.origin_type, self.origin_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_line_gets_its_amounts() {
        let draft = DraftLine::new(" Parking ", 25_000, VatCategory::Standard)
            .quantity_milli(2_000)
            .discount(5_000)
            .origin(Subject::new("property.charge", 4));
        let line = InvoiceLine::from_draft(3, 1, &draft, Some(1_000_000));
        assert_eq!(line.description, "Parking");
        assert_eq!((line.invoice_id, line.position), (3, 1));
        assert_eq!(
            (line.net_amount, line.vat_amount, line.gross_amount),
            (45_000, 2_250, 47_250)
        );
        assert_eq!((line.vat_rate_bp, line.vat_amount_aed), (500, Some(2_250)));
        assert_eq!(line.origin(), Some(Subject::new("property.charge", 4)));
        assert_eq!(line.amounts(), line.compute(Some(1_000_000)));
    }

    #[test]
    fn a_line_without_vat_or_rate() {
        let draft = DraftLine::new("Rent", 100_000, VatCategory::Exempt);
        let line = InvoiceLine::from_draft(3, 2, &draft, None);
        assert_eq!((line.vat_rate_bp, line.vat_amount), (0, 0));
        assert_eq!(line.vat_amount_aed, None);
        assert_eq!(line.gross_amount, 100_000);
        assert_eq!(line.origin(), None);
    }
}
