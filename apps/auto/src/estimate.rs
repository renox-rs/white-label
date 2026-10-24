//! The estimate of a job card: its labour and part lines and what they add up
//! to (08 `AUT.JOB.01`, `FIN.QUO.01`).
//!
//! The totals count the lines that are still in the estimate (drafted, sent
//! or approved); a removed or declined line is not billed. VAT is worked out
//! line by line at 5% and rounded half up, as the invoice does, so the sheet
//! and the invoice agree to the fils.

use renox::prelude::*;

use crate::labour_line::{LabourLine, LineStatus};
use crate::part_line::PartLine;

/// The VAT rate on workshop services and parts, in basis points.
pub const VAT_BP: i64 = 500;

/// The VAT on a net amount in fils: 5% rounded half up.
pub fn vat_of(net: i64) -> i64 {
    let value = i128::from(net) * i128::from(VAT_BP);
    let magnitude = (2 * value.abs() + 10_000) / 20_000;
    i64::try_from(if value < 0 { -magnitude } else { magnitude }).unwrap_or(0)
}

/// What a set of lines add up to, in fils.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Totals {
    /// The labour before VAT.
    pub labour: i64,
    /// The parts before VAT.
    pub parts: i64,
    /// The VAT, line by line.
    pub vat: i64,
}

impl Totals {
    /// Labour and parts before VAT.
    pub fn net(&self) -> i64 {
        self.labour + self.parts
    }

    /// The total with VAT.
    pub fn gross(&self) -> i64 {
        self.net() + self.vat
    }
}

/// The estimate of a job card with VAT, in fils, as SQL for a list column:
/// the sum of the lines still in the estimate (drafted, sent or approved),
/// each line's VAT rounded half up as [`vat_of`] does. `job_card` is the SQL
/// of the job card's id (`{T}."id"` in a grid column). The lines' prices are
/// those of [`LabourLine::price`] and [`PartLine::price`].
pub fn gross_sql(job_card: &str) -> String {
    let live = "('draft', 'sent', 'approved')";
    let vat = |price: &str| format!("({price} + (2 * {price} * {VAT_BP} + 10000) / 20000)");
    let labour = "CASE \"l\".\"pricing\" WHEN 'hourly' THEN \
                  (2 * \"l\".\"hours_milli\" * \"l\".\"rate\" + 1000) / 2000 \
                  ELSE \"l\".\"rate\" END";
    let part = "(2 * \"p\".\"quantity_milli\" * \"p\".\"unit_price\" + 1000) / 2000";
    format!(
        "(COALESCE((SELECT SUM({labour_vat}) FROM \"auto_job_labour_lines\" \"l\" \
         WHERE \"l\".\"job_card_id\" = {job_card} AND \"l\".\"approval\" IN {live}), 0) + \
         COALESCE((SELECT SUM({part_vat}) FROM \"auto_job_part_lines\" \"p\" \
         WHERE \"p\".\"job_card_id\" = {job_card} AND \"p\".\"approval\" IN {live}), 0))",
        labour_vat = vat(labour),
        part_vat = vat(part),
    )
}

/// The lines of a job card.
#[derive(Debug, Clone, Default)]
pub struct Estimate {
    /// The labour lines, in order.
    pub labour: Vec<LabourLine>,
    /// The part lines, in order.
    pub parts: Vec<PartLine>,
}

impl Estimate {
    /// The lines of job card `job_card_id`, in the order they were drafted.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn load(db: &Db, job_card_id: i64) -> Result<Estimate> {
        let labour = LabourLine::query()
            .where_(LabourLine::JOB_CARD_ID.eq(job_card_id))
            .order_by(LabourLine::LINE_NO)
            .order_by(LabourLine::ID)
            .get(db)
            .await?;
        let parts = PartLine::query()
            .where_(PartLine::JOB_CARD_ID.eq(job_card_id))
            .order_by(PartLine::LINE_NO)
            .order_by(PartLine::ID)
            .get(db)
            .await?;
        Ok(Estimate { labour, parts })
    }

    /// The totals of the lines `keep` accepts.
    pub fn totals(&self, keep: impl Fn(LineStatus) -> bool) -> Totals {
        let mut totals = Totals::default();
        for line in self.labour.iter().filter(|l| keep(l.approval)) {
            totals.labour += line.price();
            totals.vat += vat_of(line.price());
        }
        for line in self.parts.iter().filter(|l| keep(l.approval)) {
            totals.parts += line.price();
            totals.vat += vat_of(line.price());
        }
        totals
    }

    /// The totals of the estimate as it stands: every line still in it.
    pub fn live_totals(&self) -> Totals {
        self.totals(LineStatus::is_live)
    }

    /// The totals of what the customer approved.
    pub fn approved_totals(&self) -> Totals {
        self.totals(|status| status == LineStatus::Approved)
    }

    /// Whether any line is still in the estimate (not removed or declined).
    pub fn has_live_lines(&self) -> bool {
        self.labour.iter().any(|l| l.approval.is_live())
            || self.parts.iter().any(|l| l.approval.is_live())
    }

    /// How many lines are recorded: removed ones are not counted.
    pub fn recorded(&self) -> usize {
        self.labour
            .iter()
            .filter(|l| l.approval != LineStatus::Removed)
            .count()
            + self
                .parts
                .iter()
                .filter(|l| l.approval != LineStatus::Removed)
                .count()
    }

    /// Whether any line was recorded at all, removed ones too.
    pub fn has_lines(&self) -> bool {
        !self.labour.is_empty() || !self.parts.is_empty()
    }

    /// How many lines were drafted and not sent yet.
    pub fn drafts(&self) -> usize {
        self.labour
            .iter()
            .filter(|l| l.approval == LineStatus::Draft)
            .count()
            + self
                .parts
                .iter()
                .filter(|l| l.approval == LineStatus::Draft)
                .count()
    }

    /// How many lines the customer has not answered: drafted or sent.
    pub fn unanswered(&self) -> usize {
        let open = |status: LineStatus| matches!(status, LineStatus::Draft | LineStatus::Sent);
        self.labour.iter().filter(|l| open(l.approval)).count()
            + self.parts.iter().filter(|l| open(l.approval)).count()
    }

    /// Whether every drafted line has a price above zero and the labour lines
    /// have hours: the estimate can be sent.
    pub fn drafts_priced(&self) -> bool {
        let labour = self
            .labour
            .iter()
            .filter(|l| l.approval == LineStatus::Draft)
            .all(|l| l.price() > 0 && l.hours_milli > 0);
        let parts = self
            .parts
            .iter()
            .filter(|l| l.approval == LineStatus::Draft)
            .all(|l| l.price() > 0 && l.quantity_milli > 0);
        labour && parts
    }

    /// How many approved labour lines are not done yet.
    pub fn labour_open(&self) -> i64 {
        i64::try_from(
            self.labour
                .iter()
                .filter(|l| {
                    l.approval == LineStatus::Approved
                        && l.work != crate::labour_line::WorkStatus::Done
                })
                .count(),
        )
        .unwrap_or(i64::MAX)
    }

    /// The value of the work done on a stopped job, in fils: labour done at
    /// the estimate's prices and parts fitted or agreed special orders
    /// (`AUT.ACL.21`).
    pub fn work_done_value(&self) -> i64 {
        let labour: i64 = self
            .labour
            .iter()
            .filter(|l| l.approval == LineStatus::Approved)
            .map(LabourLine::done_value)
            .sum();
        let parts: i64 = self
            .parts
            .iter()
            .filter(|l| l.approval == LineStatus::Approved)
            .map(PartLine::done_value)
            .sum();
        labour + parts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::labour_line::{LabourPricing, WorkStatus};

    fn labour(status: LineStatus, hours: i64, rate: i64) -> LabourLine {
        LabourLine {
            pricing: LabourPricing::Hourly,
            hours_milli: hours,
            rate,
            approval: status,
            ..LabourLine::default()
        }
    }

    fn part(status: LineStatus, quantity: i64, unit: i64) -> PartLine {
        PartLine {
            quantity_milli: quantity,
            unit_price: unit,
            approval: status,
            ..PartLine::default()
        }
    }

    #[test]
    fn vat_is_five_percent_rounded_half_up() {
        assert_eq!(vat_of(10_000), 500);
        assert_eq!(vat_of(30_050), 1_503);
        assert_eq!(vat_of(10), 1);
        assert_eq!(vat_of(0), 0);
    }

    #[test]
    fn the_totals_count_the_lines_still_in_the_estimate() {
        let estimate = Estimate {
            labour: vec![
                labour(LineStatus::Approved, 2_000, 15_000),
                labour(LineStatus::Declined, 1_000, 15_000),
            ],
            parts: vec![
                part(LineStatus::Approved, 1_000, 30_000),
                part(LineStatus::Removed, 1_000, 99_000),
            ],
        };
        let live = estimate.live_totals();
        assert_eq!((live.labour, live.parts, live.vat), (30_000, 30_000, 3_000));
        assert_eq!(live.net(), 60_000);
        assert_eq!(live.gross(), 63_000);
        assert_eq!(estimate.approved_totals(), live);
    }

    #[test]
    fn a_draft_is_priced_when_every_drafted_line_has_a_price() {
        let mut estimate = Estimate {
            labour: vec![labour(LineStatus::Draft, 1_000, 15_000)],
            parts: vec![part(LineStatus::Draft, 1_000, 5_000)],
        };
        assert!(estimate.drafts_priced());
        assert_eq!(estimate.drafts(), 2);
        estimate.parts.push(part(LineStatus::Draft, 1_000, 0));
        assert!(!estimate.drafts_priced());
        // A line already sent does not need a price check.
        estimate.parts[1].approval = LineStatus::Sent;
        assert!(estimate.drafts_priced());
        assert_eq!(estimate.unanswered(), 3);
    }

    #[test]
    fn the_work_done_is_the_labour_done_and_the_parts_fitted() {
        let mut done = labour(LineStatus::Approved, 2_000, 10_000);
        done.work = WorkStatus::Done;
        let not_done = labour(LineStatus::Approved, 1_000, 10_000);
        let mut fitted = part(LineStatus::Approved, 1_000, 30_000);
        fitted.issued_milli = 1_000;
        fitted.fitted_milli = 1_000;
        let mut issued = part(LineStatus::Approved, 1_000, 7_000);
        issued.issued_milli = 1_000;
        let estimate = Estimate {
            labour: vec![done, not_done],
            parts: vec![fitted, issued],
        };
        assert_eq!(estimate.work_done_value(), 20_000 + 30_000);
        assert_eq!(estimate.labour_open(), 1);
    }
}
