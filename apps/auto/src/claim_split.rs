//! The split of an insurance repair between the insurer and the customer
//! (08 §4.7 step 7, `AUT.INS.04`, `FIN.TPP.01`).
//!
//! The insurer pays what it approved, line by line, less the **excess**; the
//! customer pays the excess and the **private items** (lines the customer
//! approved that the insurer did not). Decisions:
//!
//! - The insurer is invoiced for the approved amount net of the excess, so
//!   that the two invoices together are the price of the repair and the
//!   excess is not billed twice. The excess is taken off the approved lines
//!   in their order; a line the excess covers entirely is left off the
//!   insurer's bill.
//! - The excess cannot be more than the approved amount.
//! - When the insurer approves a line for less than its price, the insurer is
//!   billed what it approved; the difference is not billed to anyone unless
//!   the customer approves it as a private item on a new estimate round
//!   (`AUT.ACL.15`).
//! - The amounts are before VAT, in fils; VAT is added by the invoice
//!   (5 % on every line).

use crate::claim_approval::ApprovedLine;

/// One line of a bill: what it is for and its amount before VAT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Share {
    /// The line as the invoice will say.
    pub description: String,
    /// The amount before VAT, in fils.
    pub amount: i64,
}

/// The lines the insurer is billed for, and the excess that was taken off
/// them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsurerShare {
    /// The lines of the payer bill.
    pub lines: Vec<Share>,
    /// The excess taken off the approved lines (never more than they are
    /// worth).
    pub excess_applied: i64,
}

impl InsurerShare {
    /// The amount the insurer is billed, before VAT.
    pub fn total(&self) -> i64 {
        self.lines.iter().map(|line| line.amount).sum()
    }
}

/// The insurer's share of `approved` lines when the customer pays `excess`.
pub fn insurer_share(approved: &[ApprovedLine], excess: i64) -> InsurerShare {
    let worth: i64 = approved.iter().map(|line| line.amount).sum();
    let excess_applied = excess.max(0).min(worth);
    let mut left = excess_applied;
    let mut lines = Vec::new();
    for line in approved {
        let taken = left.min(line.amount);
        left -= taken;
        let amount = line.amount - taken;
        if amount > 0 {
            lines.push(Share {
                description: line.description.clone(),
                amount,
            });
        }
    }
    InsurerShare {
        lines,
        excess_applied,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(description: &str, amount: i64) -> ApprovedLine {
        ApprovedLine {
            kind: "labour".to_owned(),
            line_id: 1,
            description: description.to_owned(),
            amount,
        }
    }

    #[test]
    fn without_an_excess_the_insurer_pays_every_approved_line() {
        let share = insurer_share(&[line("Door", 80_000), line("Bumper", 120_000)], 0);
        assert_eq!(share.total(), 200_000);
        assert_eq!(share.excess_applied, 0);
        assert_eq!(share.lines.len(), 2);
    }

    #[test]
    fn the_excess_comes_off_the_first_lines() {
        let share = insurer_share(&[line("Door", 80_000), line("Bumper", 120_000)], 100_000);
        assert_eq!(share.excess_applied, 100_000);
        assert_eq!(share.lines.len(), 1, "the door is covered by the excess");
        assert_eq!(share.lines[0].description, "Bumper");
        assert_eq!(share.lines[0].amount, 100_000);
        assert_eq!(share.total(), 100_000);
    }

    #[test]
    fn the_excess_is_never_more_than_the_approved_amount() {
        let share = insurer_share(&[line("Door", 30_000)], 50_000);
        assert_eq!(share.excess_applied, 30_000);
        assert!(share.lines.is_empty());
        assert_eq!(share.total(), 0);
    }

    #[test]
    fn a_negative_excess_is_nothing() {
        let share = insurer_share(&[line("Door", 30_000)], -5);
        assert_eq!(share.excess_applied, 0);
        assert_eq!(share.total(), 30_000);
    }
}
