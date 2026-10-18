//! A credit note line (wl-invoicing §3): the net credited on one invoice
//! line, with the VAT of that net at the invoice line's category.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::invoicing::VatCategory;

use crate::line::InvoiceLine;
use crate::vat::LineAmounts;

/// One line of a [`CreditNote`](crate::credit_note::CreditNote). Money is
/// `i64` fils of the credit note's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_credit_note_lines",
    unique(credit_note_id, position),
    index(invoice_line_id)
)]
pub struct CreditNoteLine {
    /// The primary key.
    pub id: i64,
    /// The credit note.
    pub credit_note_id: i64,
    /// The order on the credit note, from 1.
    pub position: i64,
    /// The invoice line it credits.
    pub invoice_line_id: i64,
    /// The invoice line's description.
    pub description: String,
    /// The net credited, in fils.
    pub net_amount: i64,
    /// The VAT treatment: always the invoice line's.
    pub vat_category: VatCategory,
    /// The rate charged, in basis points.
    pub vat_rate_bp: i64,
    /// The line VAT, rounded half-up to fils.
    pub vat_amount: i64,
    /// The line VAT in AED fils; `None` while the rate is unknown.
    pub vat_amount_aed: Option<i64>,
    /// Net plus VAT.
    pub gross_amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CreditNoteLine {
    /// The line at `position` of credit note `credit_note_id` crediting
    /// `net` fils of `invoice_line`, its amounts computed at `rate_micro`
    /// to AED when known.
    pub fn of(
        credit_note_id: i64,
        position: i64,
        invoice_line: &InvoiceLine,
        net: i64,
        rate_micro: Option<i64>,
    ) -> Self {
        let amounts = LineAmounts::for_net(net, invoice_line.vat_category, rate_micro);
        CreditNoteLine {
            credit_note_id,
            position,
            invoice_line_id: invoice_line.id,
            description: invoice_line.description.clone(),
            net_amount: amounts.net,
            vat_category: amounts.category,
            vat_rate_bp: amounts.rate_bp,
            vat_amount: amounts.vat,
            vat_amount_aed: amounts.vat_aed,
            gross_amount: amounts.gross(),
            ..Default::default()
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoice_line(category: VatCategory) -> InvoiceLine {
        InvoiceLine {
            id: 11,
            invoice_id: 3,
            position: 1,
            description: "Rent".into(),
            net_amount: 100_000,
            vat_category: category,
            ..Default::default()
        }
    }

    #[test]
    fn a_credit_line_takes_the_category_of_its_invoice_line() {
        let line = CreditNoteLine::of(
            5,
            1,
            &invoice_line(VatCategory::Standard),
            20_000,
            Some(1_000_000),
        );
        assert_eq!((line.credit_note_id, line.position), (5, 1));
        assert_eq!(line.invoice_line_id, 11);
        assert_eq!(line.description, "Rent");
        assert_eq!(line.vat_category, VatCategory::Standard);
        assert_eq!(
            (line.net_amount, line.vat_amount, line.gross_amount),
            (20_000, 1_000, 21_000)
        );
        assert_eq!((line.vat_rate_bp, line.vat_amount_aed), (500, Some(1_000)));
        assert_eq!(line.amounts().gross(), 21_000);
    }

    #[test]
    fn a_zero_rated_credit_line_has_no_vat_and_an_unknown_rate_no_aed() {
        let line = CreditNoteLine::of(5, 2, &invoice_line(VatCategory::ZeroRated), 50_000, None);
        assert_eq!((line.vat_rate_bp, line.vat_amount), (0, 0));
        assert_eq!(line.vat_amount_aed, None);
        assert_eq!(line.gross_amount, 50_000);
    }
}
