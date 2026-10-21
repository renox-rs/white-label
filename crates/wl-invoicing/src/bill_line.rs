//! A supplier bill line (wl-invoicing §3): what the supplier charges, its
//! quantity, price, VAT category and the amounts computed from them
//! ([`crate::vat`]). It is named `BillLine` so it does not clash with the
//! kernel's `SupplierBillLine`.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::VatCategory;

use crate::vat::{self, LineAmounts};

/// What a user enters for one line of a supplier bill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillLineDraft {
    /// What is charged.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, before VAT, in fils.
    pub unit_price: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// The day the service was done, if known; the first day of a period.
    pub service_date: Option<NaiveDate>,
    /// The last day of the period the line charges for, if it is one. A
    /// period line counts against a contract's end by this day.
    pub service_end_date: Option<NaiveDate>,
    /// The cost object the line is coded to, if any.
    pub cost_object: Option<Subject>,
    /// The purchase order line the goods were ordered on, if any.
    pub po_line: Option<Subject>,
    /// The wl-inventory landed cost a freight, duty or clearing line is
    /// coded to, if any. Such a line posts no cost to a cost object.
    pub landed_cost: Option<Subject>,
    /// The cost code, which budget vs actual maps to a budget category
    /// (`FIN.BUD.01`).
    pub cost_code: Option<String>,
}

impl BillLineDraft {
    /// A line of quantity 1.
    pub fn new(description: &str, unit_price: i64, vat_category: VatCategory) -> Self {
        Self {
            description: description.to_owned(),
            quantity_milli: 1_000,
            unit_price,
            vat_category,
            service_date: None,
            service_end_date: None,
            cost_object: None,
            po_line: None,
            landed_cost: None,
            cost_code: None,
        }
    }

    /// Coded to a cost code (`FIN.BUD.01`); blank clears it.
    pub fn cost_code(mut self, code: &str) -> Self {
        self.cost_code = Some(code.trim().to_lowercase()).filter(|c| !c.is_empty());
        self
    }

    /// With a quantity in thousandths.
    pub fn quantity_milli(mut self, quantity_milli: i64) -> Self {
        self.quantity_milli = quantity_milli;
        self
    }

    /// With the day the service was done.
    pub fn service_date(mut self, date: NaiveDate) -> Self {
        self.service_date = Some(date);
        self
    }

    /// For work over a period: the last day (the service date is the first).
    pub fn service_end_date(mut self, date: NaiveDate) -> Self {
        self.service_end_date = Some(date);
        self
    }

    /// Coded to a cost object.
    pub fn cost_object(mut self, cost_object: Subject) -> Self {
        self.cost_object = Some(cost_object);
        self
    }

    /// Ordered on a purchase order line.
    pub fn po_line(mut self, po_line: Subject) -> Self {
        self.po_line = Some(po_line);
        self
    }

    /// Coded to a wl-inventory landed cost (a freight, duty or clearing
    /// line): its cost is not posted to a cost object.
    pub fn landed_cost(mut self, landed_cost: Subject) -> Self {
        self.landed_cost = Some(landed_cost);
        self
    }

    /// Why this line (number `position`, from 1) cannot be saved, if it
    /// cannot.
    pub fn problem(&self, position: usize) -> Option<String> {
        if self.description.trim().is_empty() {
            return Some(format!("line {position} has no description"));
        }
        if self.quantity_milli <= 0 {
            return Some(format!("line {position} has a quantity of 0 or less"));
        }
        if self.unit_price < 0 {
            return Some(format!("line {position} has a negative price"));
        }
        // The cost of a landed cost line goes to the receipts, never also to
        // a cost object: it would be counted twice.
        if self.landed_cost.is_some() && self.cost_object.is_some() {
            return Some(format!(
                "line {position} is coded to a landed cost and to a cost object"
            ));
        }
        None
    }
}

/// One line of a [`SupplierBill`](crate::supplier_bill::SupplierBill). Money
/// is `i64` fils of the bill's currency.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_supplier_bill_lines", unique(bill_id, position))]
pub struct BillLine {
    /// The primary key.
    pub id: i64,
    /// The bill.
    pub bill_id: i64,
    /// The order on the bill, from 1.
    pub position: i64,
    /// What is charged.
    pub description: String,
    /// The quantity in thousandths (1000 = 1).
    pub quantity_milli: i64,
    /// The price of one unit, before VAT.
    pub unit_price: i64,
    /// Quantity × unit price.
    pub net_amount: i64,
    /// The VAT treatment.
    pub vat_category: VatCategory,
    /// The rate, in basis points (500 = 5%).
    pub vat_rate_bp: i64,
    /// The line VAT, rounded half-up to fils.
    pub vat_amount: i64,
    /// The line VAT in AED fils; `None` while the rate is unknown.
    pub vat_amount_aed: Option<i64>,
    /// The line net in AED fils; `None` while the rate is unknown.
    pub net_amount_aed: Option<i64>,
    /// Net plus VAT.
    pub gross_amount: i64,
    /// The day the service was done, if known; the first day of a period.
    pub service_date: Option<NaiveDate>,
    /// The last day of the period the line charges for, if it is one.
    pub service_end_date: Option<NaiveDate>,
    /// The type of the cost object the line is coded to, if any.
    pub cost_type: Option<String>,
    /// The id of the cost object the line is coded to, if any.
    pub cost_id: Option<i64>,
    /// The type of the purchase order line the goods were ordered on.
    pub po_line_type: Option<String>,
    /// The id of the purchase order line the goods were ordered on.
    pub po_line_id: Option<i64>,
    /// The type of the landed cost the line is coded to, if any.
    pub landed_cost_type: Option<String>,
    /// The id of the landed cost the line is coded to, if any.
    pub landed_cost_id: Option<i64>,
    /// The cost code, which budget vs actual maps to a budget category.
    pub cost_code: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BillLine {
    /// The day the work counts on against a contract: the last day of a
    /// period line, else its service date.
    pub fn counted_day(&self) -> Option<NaiveDate> {
        self.service_end_date.or(self.service_date)
    }

    /// The line at `position` of bill `bill_id` for `draft`, its amounts
    /// computed at `rate_micro` to AED when known.
    pub fn from_draft(
        bill_id: i64,
        position: i64,
        draft: &BillLineDraft,
        rate_micro: Option<i64>,
    ) -> Self {
        let mut line = BillLine {
            bill_id,
            position,
            description: draft.description.trim().to_owned(),
            quantity_milli: draft.quantity_milli,
            unit_price: draft.unit_price,
            vat_category: draft.vat_category,
            service_date: draft.service_date,
            service_end_date: draft.service_end_date,
            cost_type: draft.cost_object.as_ref().map(|c| c.subject_type.clone()),
            cost_id: draft.cost_object.as_ref().map(|c| c.subject_id),
            po_line_type: draft.po_line.as_ref().map(|c| c.subject_type.clone()),
            po_line_id: draft.po_line.as_ref().map(|c| c.subject_id),
            landed_cost_type: draft.landed_cost.as_ref().map(|c| c.subject_type.clone()),
            landed_cost_id: draft.landed_cost.as_ref().map(|c| c.subject_id),
            cost_code: draft.cost_code.clone(),
            ..Default::default()
        };
        let amounts = line.compute(rate_micro);
        line.set_amounts(&amounts, rate_micro);
        line
    }

    /// The amounts of this line at `rate_micro` to AED.
    pub fn compute(&self, rate_micro: Option<i64>) -> LineAmounts {
        LineAmounts::compute(
            self.quantity_milli,
            self.unit_price,
            0,
            self.vat_category,
            rate_micro,
        )
    }

    /// Stores `amounts` as the line's computed columns.
    pub fn set_amounts(&mut self, amounts: &LineAmounts, rate_micro: Option<i64>) {
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

    /// The cost object the line is coded to, if any.
    pub fn cost_object(&self) -> Option<Subject> {
        match (&self.cost_type, self.cost_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The landed cost the line is coded to, if any.
    pub fn landed_cost(&self) -> Option<Subject> {
        match (&self.landed_cost_type, self.landed_cost_id) {
            (Some(subject_type), Some(subject_id)) => Some(Subject {
                subject_type: subject_type.clone(),
                subject_id,
            }),
            _ => None,
        }
    }

    /// The purchase order line the goods were ordered on, if any.
    pub fn po_line(&self) -> Option<Subject> {
        match (&self.po_line_type, self.po_line_id) {
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
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let draft = BillLineDraft::new(" Pump repair ", 25_000, VatCategory::Standard)
            .quantity_milli(2_000)
            .service_date(day)
            .cost_object(Subject::new("property.unit", 4));
        let line = BillLine::from_draft(3, 1, &draft, Some(1_000_000));
        assert_eq!(line.description, "Pump repair");
        assert_eq!((line.bill_id, line.position), (3, 1));
        assert_eq!(
            (line.net_amount, line.vat_amount, line.gross_amount),
            (50_000, 2_500, 52_500)
        );
        assert_eq!((line.vat_rate_bp, line.vat_amount_aed), (500, Some(2_500)));
        assert_eq!(line.net_amount_aed, Some(50_000));
        assert_eq!(line.service_date, Some(day));
        assert_eq!(line.cost_object(), Some(Subject::new("property.unit", 4)));
        assert_eq!(line.amounts(), line.compute(Some(1_000_000)));
    }

    #[test]
    fn a_foreign_line_is_converted_and_one_without_a_rate_is_not() {
        let draft = BillLineDraft::new("Freight", 10_000, VatCategory::ZeroRated);
        let line = BillLine::from_draft(1, 1, &draft, Some(3_670_000));
        assert_eq!(line.net_amount_aed, Some(36_700));
        let line = BillLine::from_draft(1, 2, &draft, None);
        assert_eq!((line.net_amount_aed, line.vat_amount_aed), (None, None));
        assert_eq!(line.cost_object(), None);
        assert_eq!(line.po_line(), None);
    }

    #[test]
    fn a_line_keeps_the_po_line_it_was_ordered_on() {
        let draft = BillLineDraft::new("Pump", 100, VatCategory::Standard)
            .po_line(Subject::new("procurement.po_line", 12));
        let line = BillLine::from_draft(1, 1, &draft, None);
        assert_eq!(line.po_line_id, Some(12));
        assert_eq!(
            line.po_line(),
            Some(Subject::new("procurement.po_line", 12))
        );
    }

    #[test]
    fn a_line_keeps_the_landed_cost_it_is_coded_to() {
        let draft = BillLineDraft::new("Freight", 40_000, VatCategory::Standard)
            .landed_cost(Subject::new("inventory.landed_cost", 12));
        let line = BillLine::from_draft(1, 2, &draft, Some(1_000_000));
        assert_eq!(
            line.landed_cost(),
            Some(Subject::new("inventory.landed_cost", 12))
        );
        assert_eq!(line.cost_object(), None);
        let plain = BillLine::from_draft(
            1,
            1,
            &BillLineDraft::new("Pump", 100, VatCategory::Standard),
            None,
        );
        assert_eq!(plain.landed_cost(), None);
    }

    #[test]
    fn a_landed_cost_line_has_no_cost_object() {
        let both = BillLineDraft::new("Freight", 40_000, VatCategory::Standard)
            .landed_cost(Subject::new("inventory.landed_cost", 12))
            .cost_object(Subject::new("property.unit", 4));
        assert!(both.problem(2).unwrap().contains("landed cost"));
    }

    #[test]
    fn a_line_is_checked() {
        let ok = BillLineDraft::new("Rent", 100, VatCategory::Exempt);
        assert_eq!(ok.problem(1), None);
        let blank = BillLineDraft::new("  ", 100, VatCategory::Standard);
        assert!(blank.problem(1).unwrap().contains("description"));
        let none = ok.clone().quantity_milli(0);
        assert!(none.problem(2).unwrap().contains("line 2"));
        let negative = BillLineDraft::new("Fee", -1, VatCategory::Standard);
        assert!(negative.problem(1).unwrap().contains("negative"));
    }
}
