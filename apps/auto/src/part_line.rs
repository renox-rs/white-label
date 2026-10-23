//! A part line of a job card (08 `AUT.JOB.01`, `AUT.JOB.02`): a part of the
//! estimate, its condition (new, used or renovated, which the invoice must
//! state, 08 §2 and §9), whether it was ordered for this job (special order),
//! and what became of it: issued from stock, fitted, returned.
//!
//! Quantities are thousandths; money is fils. A part is **issued** to the job
//! from the workshop's store through the kernel's stock service
//! ([`crate::parts`]); what the store could not give is the **short**
//! quantity, which wl-inventory keeps as a pending issue. What was issued and
//! not fitted goes **back** to stock when the job stops.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

pub use crate::labour_line::LineStatus;

/// The condition of a part the invoice states.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PartCondition {
    /// A new part.
    #[default]
    New,
    /// A used part.
    Used,
    /// A renovated (refurbished) part.
    Renovated,
}

impl PartCondition {
    /// The condition written as `text`: its stored name, in any case.
    pub fn parse(text: &str) -> Option<PartCondition> {
        PartCondition::ALL
            .iter()
            .find(|condition| condition.as_str().eq_ignore_ascii_case(text.trim()))
            .copied()
    }
}

/// A part line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_job_part_lines", index(job_card_id))]
pub struct PartLine {
    /// The primary key.
    pub id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// The code of the item in the stock catalogue, when it is a stocked part.
    pub item_code: Option<String>,
    /// What the part is.
    pub description: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit in fils, before VAT.
    pub unit_price: i64,
    /// New, used or renovated.
    pub part_condition: PartCondition,
    /// Ordered for this job; the supplier will not take it back.
    pub special_order: bool,
    /// The customer agreed to the special order in the estimate.
    pub special_order_agreed: bool,
    /// Where the line stands with the customer.
    pub approval: LineStatus,
    /// The estimate round that sent it.
    pub approval_id: Option<i64>,
    /// The inventory location the part was issued from.
    pub store_location_id: Option<i64>,
    /// The quantity handed to the job.
    pub issued_milli: i64,
    /// The part of it that posted against stock.
    pub posted_milli: i64,
    /// The part of it the store could not give (a pending issue).
    pub short_milli: i64,
    /// The quantity fitted to the vehicle.
    pub fitted_milli: i64,
    /// The quantity given back.
    pub returned_milli: i64,
    /// The order of the line on the estimate.
    pub line_no: i64,
    /// The user who drafted it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl PartLine {
    /// The price of the line in fils: the quantity times the unit price,
    /// rounded half up.
    pub fn price(&self) -> i64 {
        let value = i128::from(self.quantity_milli) * i128::from(self.unit_price);
        i64::try_from((2 * value + 1_000) / 2_000).unwrap_or(0)
    }

    /// The price of `quantity_milli` of the part, in fils.
    pub fn price_of(&self, quantity_milli: i64) -> i64 {
        let value = i128::from(quantity_milli) * i128::from(self.unit_price);
        i64::try_from((2 * value + 1_000) / 2_000).unwrap_or(0)
    }

    /// The value of the part on a stopped job (`AUT.ACL.21`): the fitted
    /// quantity, plus the whole special order the customer agreed to, at the
    /// unit price. A part not fitted is not billed.
    pub fn done_value(&self) -> i64 {
        if self.special_order && self.special_order_agreed {
            return self.price();
        }
        self.price_of(self.fitted_milli)
    }

    /// What was issued and has neither been fitted nor given back.
    pub fn unfitted_milli(&self) -> i64 {
        (self.issued_milli - self.fitted_milli - self.returned_milli).max(0)
    }

    /// What is still to issue to the job.
    pub fn outstanding_milli(&self) -> i64 {
        (self.quantity_milli - self.issued_milli).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(quantity_milli: i64, unit_price: i64) -> PartLine {
        PartLine {
            quantity_milli,
            unit_price,
            ..PartLine::default()
        }
    }

    #[test]
    fn a_part_costs_quantity_times_unit_price() {
        assert_eq!(part(1_000, 30_000).price(), 30_000);
        assert_eq!(part(4_000, 4_500).price(), 18_000);
        assert_eq!(part(500, 1_001).price(), 501);
    }

    #[test]
    fn a_stopped_job_bills_the_fitted_parts_and_the_agreed_special_orders() {
        let mut fitted = part(2_000, 15_000);
        fitted.issued_milli = 2_000;
        fitted.fitted_milli = 1_000;
        assert_eq!(fitted.done_value(), 15_000);

        let mut not_fitted = part(1_000, 8_000);
        not_fitted.issued_milli = 1_000;
        assert_eq!(not_fitted.done_value(), 0);
        assert_eq!(not_fitted.unfitted_milli(), 1_000);

        let mut special = part(1_000, 90_000);
        special.special_order = true;
        assert_eq!(special.done_value(), 0, "not agreed: not billed");
        special.special_order_agreed = true;
        assert_eq!(special.done_value(), 90_000);
    }

    #[test]
    fn what_is_left_to_issue_and_to_return() {
        let mut line = part(3_000, 100);
        line.issued_milli = 2_000;
        line.fitted_milli = 1_000;
        line.returned_milli = 500;
        assert_eq!(line.outstanding_milli(), 1_000);
        assert_eq!(line.unfitted_milli(), 500);
    }

    #[test]
    fn a_condition_is_read_by_its_stored_name() {
        assert_eq!(PartCondition::parse(" Used "), Some(PartCondition::Used));
        assert_eq!(
            PartCondition::parse("renovated"),
            Some(PartCondition::Renovated)
        );
        assert_eq!(PartCondition::parse("broken"), None);
    }
}
