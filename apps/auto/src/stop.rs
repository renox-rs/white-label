//! What a stopped job records and what it is worth (08 §7.1 *Job card*
//! `cancel`, `AUT.ACL.21`): the reason code, the work done per line, the
//! customer's acknowledgement and the amount billed, kept in the run's input so
//! that an approval can run the cancel later with the same facts.
//!
//! The value of the work done is the labour done at the estimate's prices (a
//! line part done at the hours done times its hourly rate, a flat-rate line at
//! its flat price times the hours done over its flat hours, never above the
//! flat price), the parts fitted and the special orders the customer agreed to
//! ([`Estimate::work_done_value`]). An inspection fee agreed at check-in is
//! billed when nothing else was done (the only work was the look at the
//! vehicle). The invoice lines are those of [`draft_lines`]; a bill below the
//! value is spread over the lines as discounts ([`spread_reduction`]) so every
//! line keeps its VAT and the invoice adds up to the amount approved.

use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{DraftLine, VatCategory};

use crate::billing::{labour_draft_line, part_draft_line};
use crate::estimate::{Estimate, vat_of};
use crate::job_card::JobCard;
use crate::labour_line::{LabourLine, LabourPricing, LineStatus, WorkStatus, hours_text};
use crate::part_line::PartLine;
use crate::texts::english;

/// The customer stopped the job.
pub const CUSTOMER_STOPPED: &str = "customer_stopped";
/// The claim that funded the job was withdrawn; the customer pays.
pub const CLAIM_WITHDRAWN: &str = "claim_withdrawn";
/// The vehicle was declared a total loss.
pub const TOTAL_LOSS: &str = "total_loss";
/// The estimate was not answered in time (`AUT.RUL.17`).
pub const NO_RESPONSE: &str = "no_response";
/// The customer left before an estimate was answered.
pub const CUSTOMER_LEFT: &str = "customer_left";
/// The customer declined the estimate.
pub const ESTIMATE_DECLINED: &str = "estimate_declined";

/// The reasons the advisor picks for a cancel.
pub const USER_CODES: [&str; 2] = [CUSTOMER_STOPPED, CLAIM_WITHDRAWN];

/// The customer signed the summary of the work done.
pub const SIGNED: &str = "signed";
/// The customer refused to sign; a member of staff witnessed it.
pub const REFUSED_TO_SIGN: &str = "refused_to_sign";
/// The insurer's notice replaces the signature.
pub const INSURER_NOTICE: &str = "insurer_notice";
/// The customer's written withdrawal replaces the signature.
pub const WRITTEN_WITHDRAWAL: &str = "written_withdrawal";

/// How the labour line `line` stands when the job stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabourWork {
    /// The labour line.
    pub line: i64,
    /// Not done, part done or done.
    pub work: WorkStatus,
    /// The hours worked, in thousandths (the line's hours for a line done).
    pub hours_milli: i64,
}

/// How much of the part line `line` was fitted when the job stops.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartWork {
    /// The part line.
    pub line: i64,
    /// The quantity fitted, in thousandths.
    pub fitted_milli: i64,
}

/// What the advisor recorded for a stopped job: carried in the run's input.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopRecord {
    /// The reason code ([`USER_CODES`], [`TOTAL_LOSS`]).
    #[serde(default)]
    pub code: String,
    /// The work done per approved labour line.
    #[serde(default)]
    pub labour: Vec<LabourWork>,
    /// The parts fitted per approved part line.
    #[serde(default)]
    pub parts: Vec<PartWork>,
    /// The service checked that every approved line has its work recorded.
    #[serde(default)]
    pub work_recorded: bool,
    /// How the customer acknowledged the work done ([`SIGNED`] ..).
    #[serde(default)]
    pub ack_method: String,
    /// The customer who acknowledged it.
    #[serde(default)]
    pub ack_name: Option<String>,
    /// The customer's signature.
    #[serde(default)]
    pub ack_signature: Option<String>,
    /// The staff witness of a refusal to sign.
    #[serde(default)]
    pub ack_witness: Option<String>,
    /// The insurer's notice or the written withdrawal.
    #[serde(default)]
    pub ack_document: Option<String>,
    /// The value of the work done before VAT, in fils.
    #[serde(default)]
    pub work_value: i64,
    /// What is billed before VAT, in fils (at most the value).
    #[serde(default)]
    pub billed: i64,
}

fn filled(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

impl StopRecord {
    /// Whether the acknowledgement is complete: the customer's name for a
    /// signature, the witness for a refusal, the document for a notice.
    pub fn acknowledged(&self) -> bool {
        match self.ack_method.as_str() {
            SIGNED => filled(&self.ack_name),
            REFUSED_TO_SIGN => filled(&self.ack_witness),
            INSURER_NOTICE | WRITTEN_WITHDRAWAL => filled(&self.ack_document),
            _ => false,
        }
    }

    /// Whether the bill is below the work done (a lowered or waived bill).
    pub fn lowered(&self) -> bool {
        self.billed < self.work_value
    }

    /// Whether the branch manager must approve (`AUT.ACL.21`): the bill is
    /// below the work done, or the customer refused to sign.
    pub fn needs_approval(&self) -> bool {
        self.lowered() || self.ack_method == REFUSED_TO_SIGN
    }
}

/// The estimate with the work done of `record` applied to its lines.
pub fn applied(estimate: &Estimate, record: &StopRecord) -> Estimate {
    let mut next = estimate.clone();
    for work in &record.labour {
        if let Some(line) = next.labour.iter_mut().find(|l| l.id == work.line) {
            line.work = work.work;
            line.hours_done_milli = match work.work {
                WorkStatus::NotDone => 0,
                WorkStatus::Done | WorkStatus::PartDone => work.hours_milli,
            };
        }
    }
    for part in &record.parts {
        if let Some(line) = next.parts.iter_mut().find(|l| l.id == part.line) {
            line.fitted_milli = part.fitted_milli;
        }
    }
    next
}

/// The inspection fee due on a job that stopped with `work_value` of work
/// done: the fee agreed at check-in, when nothing else was done.
pub fn fee_due(card: &JobCard, work_value: i64) -> i64 {
    if work_value == 0 {
        card.inspection_fee.max(0)
    } else {
        0
    }
}

/// The value of the work done on `estimate` (with the work applied) and the
/// inspection fee due, before VAT, in fils.
pub fn value_of(card: &JobCard, estimate: &Estimate) -> i64 {
    let work = estimate.work_done_value();
    work + fee_due(card, work)
}

/// The invoice line of the labour line `line` for the work done, `None` when
/// nothing of it was done.
fn labour_line_billed(line: &LabourLine, origin: &Subject) -> Option<DraftLine> {
    match line.work {
        WorkStatus::NotDone => None,
        WorkStatus::Done => Some(labour_draft_line(line, origin.clone())),
        WorkStatus::PartDone => {
            let done = hours_text(line.hours_done_milli);
            let all = hours_text(line.hours_milli);
            match line.pricing {
                LabourPricing::Hourly => {
                    let description = english(
                        "auto.stop.labour_part",
                        &[
                            ("description", &line.description),
                            ("done", &done),
                            ("hours", &all),
                        ],
                    );
                    Some(
                        DraftLine::new(&description, line.rate, VatCategory::Standard)
                            .quantity_milli(line.hours_done_milli)
                            .origin(origin.clone()),
                    )
                }
                LabourPricing::FlatRate => {
                    let description = english(
                        "auto.stop.labour_part_flat",
                        &[
                            ("description", &line.description),
                            ("done", &done),
                            ("hours", &all),
                        ],
                    );
                    Some(
                        DraftLine::new(&description, line.done_value(), VatCategory::Standard)
                            .origin(origin.clone()),
                    )
                }
            }
        }
    }
}

/// The invoice line of the part line `line` for what is billed: the quantity
/// fitted, or the whole quantity of a special order the customer agreed to.
fn part_line_billed(line: &PartLine, origin: &Subject) -> Option<DraftLine> {
    let quantity = if line.special_order && line.special_order_agreed {
        line.quantity_milli
    } else {
        line.fitted_milli
    };
    if quantity <= 0 {
        return None;
    }
    let billed = PartLine {
        quantity_milli: quantity,
        ..line.clone()
    };
    Some(part_draft_line(&billed, origin.clone()))
}

/// The invoice lines of a stopped job: the labour done, the parts fitted and
/// the special orders agreed, then the inspection fee when it is due. The
/// estimate has the work done applied ([`applied`]).
pub fn draft_lines(card: &JobCard, estimate: &Estimate) -> Vec<DraftLine> {
    let origin = card.subject();
    let mut lines = Vec::new();
    for line in estimate
        .labour
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
    {
        lines.extend(labour_line_billed(line, &origin));
    }
    for line in estimate
        .parts
        .iter()
        .filter(|l| l.approval == LineStatus::Approved)
    {
        lines.extend(part_line_billed(line, &origin));
    }
    let fee = fee_due(card, estimate.work_done_value());
    if fee > 0 {
        lines.push(
            DraftLine::new(
                &english("auto.stop.inspection_fee", &[]),
                fee,
                VatCategory::Standard,
            )
            .origin(origin),
        );
    }
    lines
}

/// Takes `reduction` fils off the lines as discounts, first line first, never
/// more than a line's net; what is left over (a reduction above the total)
/// is ignored. Every line keeps its VAT, worked out on what it is billed.
pub fn spread_reduction(lines: &mut [DraftLine], reduction: i64) {
    let mut left = reduction.max(0);
    for line in lines.iter_mut() {
        if left == 0 {
            break;
        }
        let cut = left.min(line.net().max(0));
        line.discount += cut;
        left -= cut;
    }
}

/// The total of `lines` with VAT, in fils.
pub fn gross_of(lines: &[DraftLine]) -> i64 {
    lines.iter().map(DraftLine::gross).sum()
}

/// The VAT-inclusive amount of `net` fils billed as one line.
pub fn gross_of_net(net: i64) -> i64 {
    net + vat_of(net)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part_line::PartCondition;

    fn card() -> JobCard {
        JobCard {
            id: 10,
            inspection_fee: 15_000,
            ..JobCard::default()
        }
    }

    fn hourly(id: i64, hours: i64, rate: i64, work: WorkStatus, done: i64) -> LabourLine {
        LabourLine {
            id,
            description: "Service".to_owned(),
            pricing: LabourPricing::Hourly,
            hours_milli: hours,
            rate,
            approval: LineStatus::Approved,
            work,
            hours_done_milli: done,
            ..LabourLine::default()
        }
    }

    fn flat(id: i64, hours: i64, price: i64, work: WorkStatus, done: i64) -> LabourLine {
        LabourLine {
            id,
            description: "Alignment".to_owned(),
            pricing: LabourPricing::FlatRate,
            hours_milli: hours,
            rate: price,
            approval: LineStatus::Approved,
            work,
            hours_done_milli: done,
            ..LabourLine::default()
        }
    }

    #[test]
    fn the_invoice_lines_add_up_to_the_value_of_the_work_done() {
        // 08 §7.1: service at AED 150/hour, 3 hours estimated, 2 done = AED 300;
        // flat-rate alignment AED 120 for 1 flat hour, 0.5 done = AED 60.
        let estimate = Estimate {
            labour: vec![
                hourly(1, 3_000, 15_000, WorkStatus::PartDone, 2_000),
                flat(2, 1_000, 12_000, WorkStatus::PartDone, 500),
                hourly(3, 1_000, 20_000, WorkStatus::NotDone, 0),
            ],
            parts: Vec::new(),
        };
        let lines = draft_lines(&card(), &estimate);
        assert_eq!(lines.len(), 2, "what was not done is not billed");
        let net: i64 = lines.iter().map(DraftLine::net).sum();
        assert_eq!(net, 36_000);
        assert_eq!(net, value_of(&card(), &estimate));
        assert_eq!(gross_of(&lines), 37_800, "AED 360 and 5% VAT");
    }

    #[test]
    fn parts_fitted_and_special_orders_agreed_are_billed_the_rest_is_not() {
        let fitted = PartLine {
            id: 1,
            description: "Brake pad set".to_owned(),
            quantity_milli: 1_000,
            unit_price: 30_000,
            issued_milli: 1_000,
            fitted_milli: 1_000,
            approval: LineStatus::Approved,
            part_condition: PartCondition::New,
            ..PartLine::default()
        };
        let issued = PartLine {
            id: 2,
            description: "Oil filter".to_owned(),
            quantity_milli: 1_000,
            unit_price: 4_000,
            issued_milli: 1_000,
            approval: LineStatus::Approved,
            ..PartLine::default()
        };
        let special = PartLine {
            id: 3,
            description: "Gearbox".to_owned(),
            quantity_milli: 1_000,
            unit_price: 90_000,
            special_order: true,
            special_order_agreed: true,
            approval: LineStatus::Approved,
            ..PartLine::default()
        };
        let estimate = Estimate {
            labour: Vec::new(),
            parts: vec![fitted, issued, special],
        };
        let lines = draft_lines(&card(), &estimate);
        let nets: Vec<i64> = lines.iter().map(DraftLine::net).collect();
        assert_eq!(nets, [30_000, 90_000]);
        assert_eq!(value_of(&card(), &estimate), 120_000);
    }

    #[test]
    fn the_inspection_fee_is_billed_when_nothing_else_was_done() {
        let estimate = Estimate::default();
        assert_eq!(fee_due(&card(), 0), 15_000);
        assert_eq!(fee_due(&card(), 20_000), 0);
        let lines = draft_lines(&card(), &estimate);
        assert_eq!(lines.len(), 1);
        assert_eq!(gross_of(&lines), 15_750, "AED 150 and 5% VAT");
        assert_eq!(value_of(&card(), &estimate), 15_000);
    }

    #[test]
    fn a_reduction_is_spread_over_the_lines_and_keeps_the_vat_right() {
        let estimate = Estimate {
            labour: vec![hourly(1, 1_000, 20_000, WorkStatus::Done, 1_000)],
            parts: vec![PartLine {
                id: 1,
                description: "Brake pad set".to_owned(),
                quantity_milli: 1_000,
                unit_price: 30_000,
                fitted_milli: 1_000,
                issued_milli: 1_000,
                approval: LineStatus::Approved,
                ..PartLine::default()
            }],
        };
        let mut lines = draft_lines(&card(), &estimate);
        spread_reduction(&mut lines, 20_000);
        let nets: Vec<i64> = lines.iter().map(DraftLine::net).collect();
        assert_eq!(nets, [0, 30_000]);
        assert_eq!(gross_of(&lines), 31_500, "AED 300 and 5% VAT");
        let mut waived = draft_lines(&card(), &estimate);
        spread_reduction(&mut waived, 50_000);
        assert_eq!(gross_of(&waived), 0);
    }

    #[test]
    fn the_acknowledgement_needs_its_name_witness_or_document() {
        let mut record = StopRecord::default();
        assert!(!record.acknowledged());
        record.ack_method = SIGNED.to_owned();
        assert!(!record.acknowledged(), "a signature needs the customer");
        record.ack_name = Some("Mona Hassan".to_owned());
        assert!(record.acknowledged());
        record.ack_method = REFUSED_TO_SIGN.to_owned();
        assert!(!record.acknowledged(), "a refusal needs its witness");
        record.ack_witness = Some("Sami".to_owned());
        assert!(record.acknowledged());
        assert!(record.needs_approval(), "a refused signature is approved");
        record.ack_method = INSURER_NOTICE.to_owned();
        assert!(!record.acknowledged());
        record.ack_document = Some("Notice 88".to_owned());
        assert!(record.acknowledged());
        assert!(!record.needs_approval());
    }

    #[test]
    fn billing_even_a_fil_less_than_the_work_done_needs_approval() {
        let mut record = StopRecord {
            work_value: 50_000,
            billed: 50_000,
            ack_method: SIGNED.to_owned(),
            ..StopRecord::default()
        };
        assert!(!record.lowered());
        assert!(!record.needs_approval());
        record.billed = 49_999;
        assert!(record.lowered());
        assert!(record.needs_approval());
    }

    #[test]
    fn the_work_done_is_applied_to_the_lines_it_names() {
        let estimate = Estimate {
            labour: vec![hourly(4, 2_000, 10_000, WorkStatus::NotDone, 0)],
            parts: Vec::new(),
        };
        let record = StopRecord {
            labour: vec![LabourWork {
                line: 4,
                work: WorkStatus::PartDone,
                hours_milli: 500,
            }],
            ..StopRecord::default()
        };
        let next = applied(&estimate, &record);
        assert_eq!(next.labour[0].work, WorkStatus::PartDone);
        assert_eq!(next.labour[0].done_value(), 5_000);
        assert_eq!(gross_of_net(5_000), 5_250);
    }
}
