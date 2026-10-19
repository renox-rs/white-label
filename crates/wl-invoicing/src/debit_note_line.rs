//! A supplier debit note line (wl-invoicing §3): a quantity at a unit price
//! taken off one bill line, with the VAT of that net at the bill line's
//! category. A line of a note that waits for its bill has no bill line yet
//! and is priced at the PO price.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::VatCategory;

use crate::bill_line::BillLine;
use crate::vat::{self, LineAmounts};

/// Whose price a debit note line is at.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PricedAt {
    /// The purchase order's price, while the note waits for its bill.
    Po,
    /// The supplier bill's price.
    #[default]
    Bill,
}

/// One line of a [`SupplierDebitNote`](crate::supplier_debit_note::SupplierDebitNote).
/// Money is `i64` fils of the note's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_invoicing_supplier_debit_note_lines",
    unique(debit_note_id, position),
    index(bill_line_id)
)]
pub struct DebitNoteLine {
    /// The primary key.
    pub id: i64,
    /// The debit note.
    pub debit_note_id: i64,
    /// The order on the note, from 1.
    pub position: i64,
    /// The bill line it reduces; `None` while the note waits for its bill.
    pub bill_line_id: Option<i64>,
    /// The type of the PO line the goods were ordered on, if known.
    pub po_line_type: Option<String>,
    /// The id of the PO line the goods were ordered on, if known.
    pub po_line_id: Option<i64>,
    /// What is debited.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, before VAT.
    pub unit_price: i64,
    /// Quantity × unit price.
    pub net_amount: i64,
    /// The VAT treatment: the bill line's (`standard` before the link).
    pub vat_category: VatCategory,
    /// The rate, in basis points.
    pub vat_rate_bp: i64,
    /// The line VAT, rounded half-up to fils.
    pub vat_amount: i64,
    /// The line VAT in AED fils; `None` while the rate is unknown.
    pub vat_amount_aed: Option<i64>,
    /// The line net in AED fils; `None` while the rate is unknown.
    pub net_amount_aed: Option<i64>,
    /// Net plus VAT.
    pub gross_amount: i64,
    /// The type of the cost object the cost is reversed on, if any.
    pub cost_type: Option<String>,
    /// The id of the cost object the cost is reversed on, if any.
    pub cost_id: Option<i64>,
    /// Whose price the line is at.
    pub priced_at: PricedAt,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl DebitNoteLine {
    /// The line at `position` of note `debit_note_id` taking
    /// `quantity_milli` at `unit_price` off `bill_line`, its amounts
    /// computed at `rate_micro` to AED when known. The category and the
    /// cost object are the bill line's.
    pub fn of_bill_line(
        debit_note_id: i64,
        position: i64,
        bill_line: &BillLine,
        quantity_milli: i64,
        unit_price: i64,
        rate_micro: Option<i64>,
    ) -> Self {
        let mut line = DebitNoteLine {
            debit_note_id,
            position,
            bill_line_id: Some(bill_line.id),
            po_line_type: bill_line.po_line_type.clone(),
            po_line_id: bill_line.po_line_id,
            description: bill_line.description.clone(),
            quantity_milli,
            unit_price,
            vat_category: bill_line.vat_category,
            cost_type: bill_line.cost_type.clone(),
            cost_id: bill_line.cost_id,
            priced_at: PricedAt::Bill,
            ..Default::default()
        };
        line.price(rate_micro);
        line
    }

    /// Computes the amounts of this line from its quantity, price and
    /// category at `rate_micro` to AED, and stores them.
    pub fn price(&mut self, rate_micro: Option<i64>) {
        let amounts = LineAmounts::compute(
            self.quantity_milli,
            self.unit_price,
            0,
            self.vat_category,
            rate_micro,
        );
        self.net_amount = amounts.net;
        self.vat_rate_bp = amounts.rate_bp;
        self.vat_amount = amounts.vat;
        self.vat_amount_aed = amounts.vat_aed;
        self.net_amount_aed = rate_micro.map(|rate| vat::to_aed(amounts.net, rate));
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

    /// The PO line the goods were ordered on, if known.
    pub fn po_line(&self) -> Option<Subject> {
        match (&self.po_line_type, self.po_line_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The cost object the cost is reversed on, if any.
    pub fn cost_object(&self) -> Option<Subject> {
        match (&self.cost_type, self.cost_id) {
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

    fn bill_line(category: VatCategory) -> BillLine {
        BillLine {
            id: 11,
            bill_id: 3,
            position: 1,
            description: "Pump".into(),
            quantity_milli: 10_000,
            unit_price: 10_000,
            net_amount: 100_000,
            vat_category: category,
            po_line_type: Some("procurement.po_line".into()),
            po_line_id: Some(5),
            cost_type: Some("property.unit".into()),
            cost_id: Some(4),
            ..Default::default()
        }
    }

    #[test]
    fn a_line_takes_the_category_and_cost_object_of_its_bill_line() {
        let line = DebitNoteLine::of_bill_line(
            5,
            1,
            &bill_line(VatCategory::Standard),
            3_000,
            10_000,
            Some(1_000_000),
        );
        assert_eq!((line.debit_note_id, line.position), (5, 1));
        assert_eq!(line.bill_line_id, Some(11));
        assert_eq!(line.vat_category, VatCategory::Standard);
        assert_eq!(
            (line.net_amount, line.vat_amount, line.gross_amount),
            (30_000, 1_500, 31_500)
        );
        assert_eq!((line.vat_rate_bp, line.vat_amount_aed), (500, Some(1_500)));
        assert_eq!(line.net_amount_aed, Some(30_000));
        assert_eq!(line.priced_at, PricedAt::Bill);
        assert_eq!(line.po_line(), Some(Subject::new("procurement.po_line", 5)));
        assert_eq!(line.cost_object(), Some(Subject::new("property.unit", 4)));
        assert_eq!(line.amounts().gross(), 31_500);
    }

    #[test]
    fn repricing_recomputes_the_amounts_and_an_unknown_rate_has_no_aed() {
        let mut line = DebitNoteLine::of_bill_line(
            5,
            1,
            &bill_line(VatCategory::Standard),
            3_000,
            9_500,
            None,
        );
        assert_eq!((line.net_amount, line.vat_amount), (28_500, 1_425));
        assert_eq!((line.vat_amount_aed, line.net_amount_aed), (None, None));
        line.unit_price = 10_000;
        line.price(Some(3_670_000));
        assert_eq!(line.net_amount, 30_000);
        assert_eq!(line.net_amount_aed, Some(110_100));
        let zero =
            DebitNoteLine::of_bill_line(5, 2, &bill_line(VatCategory::ZeroRated), 1_000, 500, None);
        assert_eq!((zero.vat_rate_bp, zero.vat_amount), (0, 0));
        assert_eq!(zero.gross_amount, 500);
    }

    #[test]
    fn priced_at_is_stored_in_snake_case() {
        let all: Vec<_> = PricedAt::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(all, ["po", "bill"]);
    }
}
