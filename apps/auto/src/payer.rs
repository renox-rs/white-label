//! Who pays an estimate line (08 §4.7, `AUT.ACL.15`, `AUT.INS.04`).
//!
//! A line of a job card funded by an insurance claim is paid by the insurer
//! once the insurer approved it, by the customer otherwise (a private item).
//! When the claim ends without the insurer paying (withdrawn, voided), every
//! line of the job card is the customer's: the job card follows its own
//! billing again and the insurer is never invoiced for it (BL-AUT-08,
//! BL-AUT-10).

use renox::prelude::*;

use crate::claim_approval::ApprovedLine;
use crate::labour_line::{LabourLine, LineStatus};
use crate::part_line::PartLine;

/// Who pays a line.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Payer {
    /// The customer: the default, and every line of a private job.
    #[default]
    Customer,
    /// The insurer, for a line it approved.
    Insurer,
}

/// Marks the lines the insurer approved as the insurer's, inside the run that
/// recorded the approval.
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn mark_insurer_lines(
    tx: &mut renox::db::Transaction,
    lines: &[ApprovedLine],
) -> Result {
    let stamp = renox::db::now();
    for line in lines {
        match line.kind.as_str() {
            "labour" => {
                LabourLine::query()
                    .where_(LabourLine::ID.eq(line.line_id))
                    .update(
                        &mut *tx,
                        &[("payer", &Payer::Insurer), ("updated_at", &stamp)],
                    )
                    .await?;
            }
            "part" => {
                PartLine::query()
                    .where_(PartLine::ID.eq(line.line_id))
                    .update(
                        &mut *tx,
                        &[("payer", &Payer::Insurer), ("updated_at", &stamp)],
                    )
                    .await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Makes every line of job card `job_card_id` that is still in the estimate the
/// customer's (a claim that ended without the insurer paying).
///
/// # Errors
///
/// The database's errors.
pub(crate) async fn make_private(tx: &mut renox::db::Transaction, job_card_id: i64) -> Result {
    let stamp = renox::db::now();
    LabourLine::query()
        .where_(LabourLine::JOB_CARD_ID.eq(job_card_id))
        .where_(LabourLine::APPROVAL.ne(LineStatus::Removed))
        .update(
            &mut *tx,
            &[("payer", &Payer::Customer), ("updated_at", &stamp)],
        )
        .await?;
    PartLine::query()
        .where_(PartLine::JOB_CARD_ID.eq(job_card_id))
        .where_(PartLine::APPROVAL.ne(LineStatus::Removed))
        .update(
            &mut *tx,
            &[("payer", &Payer::Customer), ("updated_at", &stamp)],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_is_the_customers_unless_the_insurer_approved_it() {
        assert_eq!(Payer::default(), Payer::Customer);
        assert_ne!(Payer::Insurer, Payer::Customer);
    }
}
