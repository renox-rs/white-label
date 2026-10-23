//! A labour line of a job card (08 `AUT.JOB.01`, `AUT.JOB.03`): the work of
//! the estimate, priced by the hour or at the flat rate of a menu operation,
//! with the work done recorded per line.
//!
//! Prices are fils. An **hourly** line has the estimated hours (thousandths)
//! and the hourly rate; its price is the hours times the rate. A **flat-rate**
//! line has the flat hours of the operation and the flat price; its price is
//! the flat price whatever the hours worked (08 §13 question 5). The value of
//! the work done on a stopped job (`AUT.ACL.21`, 08 §7.1 *Job card* after the
//! tables) is [`LabourLine::done_value`].

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// How a labour line is priced.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LabourPricing {
    /// Hours times an hourly rate: diagnostics, body work, insurance work at
    /// the insurer's agreed rate.
    #[default]
    Hourly,
    /// A flat price for standard hours: a menu operation.
    FlatRate,
}

/// Where a line stands with the customer.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LineStatus {
    /// Being drafted; not sent to the customer yet.
    #[default]
    Draft,
    /// Sent with an estimate; waiting for the customer.
    Sent,
    /// The customer approved it.
    Approved,
    /// The customer declined it (added work).
    Declined,
    /// Taken off the estimate before it was sent.
    Removed,
}

impl LineStatus {
    /// Whether the line counts in the estimate the customer sees: drafted,
    /// sent or approved.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            LineStatus::Draft | LineStatus::Sent | LineStatus::Approved
        )
    }
}

/// How much of a labour line was done.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WorkStatus {
    /// Not started.
    #[default]
    NotDone,
    /// Part of the hours worked.
    PartDone,
    /// Done.
    Done,
}

/// A labour line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_job_labour_lines", index(job_card_id))]
pub struct LabourLine {
    /// The primary key.
    pub id: i64,
    /// The job card.
    #[model(references = "auto_job_cards")]
    pub job_card_id: i64,
    /// The menu operation it was taken from, when it was.
    pub operation_id: Option<i64>,
    /// What is done.
    pub description: String,
    /// Hourly or flat-rate.
    pub pricing: LabourPricing,
    /// The estimated hours of an hourly line, the flat hours of a flat-rate
    /// one, in thousandths of an hour.
    pub hours_milli: i64,
    /// The hourly rate, or the flat price, in fils.
    pub rate: i64,
    /// Where the line stands with the customer.
    pub approval: LineStatus,
    /// The estimate round that sent it.
    pub approval_id: Option<i64>,
    /// How much of it was done.
    pub work: WorkStatus,
    /// The hours worked so far, in thousandths of an hour.
    pub hours_done_milli: i64,
    /// When it was marked done.
    pub done_at: Option<DateTime>,
    /// The user who marked it.
    pub done_by: Option<i64>,
    /// The order of the line on the estimate.
    pub line_no: i64,
    /// The user who drafted it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// `numerator × factor ÷ divisor` rounded half up, in fils.
fn ratio(numerator: i64, factor: i64, divisor: i64) -> i64 {
    if divisor == 0 {
        return 0;
    }
    let value = i128::from(numerator) * i128::from(factor);
    let divisor = i128::from(divisor);
    let magnitude = (2 * value.abs() + divisor) / (2 * divisor);
    i64::try_from(if value < 0 { -magnitude } else { magnitude }).unwrap_or(0)
}

impl LabourLine {
    /// The price of the line in fils: the hours times the rate for an hourly
    /// line, the flat price for a flat-rate one.
    pub fn price(&self) -> i64 {
        match self.pricing {
            LabourPricing::Hourly => ratio(self.hours_milli, self.rate, 1_000),
            LabourPricing::FlatRate => self.rate,
        }
    }

    /// The value of the work done, in fils (08 §7.1 *Job card*): a line that
    /// is done at its price; a line part done at the hours done times its
    /// hourly rate, or, for a flat-rate line, its flat price times the hours
    /// done over its flat hours, never above the flat price; a line not done
    /// at nothing.
    pub fn done_value(&self) -> i64 {
        match self.work {
            WorkStatus::NotDone => 0,
            WorkStatus::Done => self.price(),
            WorkStatus::PartDone => match self.pricing {
                LabourPricing::Hourly => ratio(self.hours_done_milli, self.rate, 1_000),
                LabourPricing::FlatRate => {
                    ratio(self.rate, self.hours_done_milli, self.hours_milli).min(self.rate)
                }
            },
        }
    }

    /// The price as the estimate prints it: `2.5 h × AED 150.00` for an
    /// hourly line, `flat rate for 1 h` for a flat one is the description's
    /// business; this is the hours.
    pub fn hours_text(&self) -> String {
        hours_text(self.hours_milli)
    }
}

/// Hours in thousandths as people read them: `2.5`, `1`, `0.25`.
pub fn hours_text(milli: i64) -> String {
    let whole = milli / 1_000;
    let fraction = milli.abs() % 1_000;
    if fraction == 0 {
        return whole.to_string();
    }
    let digits = format!("{fraction:03}");
    format!("{whole}.{}", digits.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hourly(hours_milli: i64, rate: i64) -> LabourLine {
        LabourLine {
            pricing: LabourPricing::Hourly,
            hours_milli,
            rate,
            ..LabourLine::default()
        }
    }

    fn flat(hours_milli: i64, price: i64) -> LabourLine {
        LabourLine {
            pricing: LabourPricing::FlatRate,
            hours_milli,
            rate: price,
            ..LabourLine::default()
        }
    }

    #[test]
    fn an_hourly_line_costs_hours_times_rate_and_a_flat_line_its_flat_price() {
        assert_eq!(hourly(3_000, 15_000).price(), 45_000);
        assert_eq!(hourly(2_500, 15_000).price(), 37_500);
        assert_eq!(hourly(333, 10_000).price(), 3_330);
        assert_eq!(flat(1_000, 12_000).price(), 12_000);
        assert_eq!(flat(4_000, 12_000).price(), 12_000);
    }

    #[test]
    fn the_work_done_is_valued_as_the_spec_says() {
        // 08 §7.1: service at AED 150/hour, 3 hours estimated, 2 done = AED 300;
        // flat-rate alignment AED 120 for 1 flat hour, 0.5 done = AED 60.
        let mut service = hourly(3_000, 15_000);
        service.work = WorkStatus::PartDone;
        service.hours_done_milli = 2_000;
        assert_eq!(service.done_value(), 30_000);
        let mut alignment = flat(1_000, 12_000);
        alignment.work = WorkStatus::PartDone;
        alignment.hours_done_milli = 500;
        assert_eq!(alignment.done_value(), 6_000);
        assert_eq!(service.done_value() + alignment.done_value(), 36_000);
    }

    #[test]
    fn a_flat_line_is_never_valued_above_its_flat_price() {
        let mut line = flat(1_000, 12_000);
        line.work = WorkStatus::PartDone;
        line.hours_done_milli = 2_500;
        assert_eq!(line.done_value(), 12_000);
    }

    #[test]
    fn a_done_line_is_worth_its_price_and_one_not_done_nothing() {
        let mut line = hourly(2_000, 20_000);
        assert_eq!(line.done_value(), 0);
        line.work = WorkStatus::Done;
        assert_eq!(line.done_value(), 40_000);
    }

    #[test]
    fn hours_read_without_trailing_zeros() {
        assert_eq!(hours_text(2_500), "2.5");
        assert_eq!(hours_text(1_000), "1");
        assert_eq!(hours_text(250), "0.25");
        assert_eq!(hours_text(0), "0");
    }

    #[test]
    fn a_removed_or_declined_line_is_not_live() {
        assert!(LineStatus::Draft.is_live());
        assert!(LineStatus::Sent.is_live());
        assert!(LineStatus::Approved.is_live());
        assert!(!LineStatus::Declined.is_live());
        assert!(!LineStatus::Removed.is_live());
    }
}
