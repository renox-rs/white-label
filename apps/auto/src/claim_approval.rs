//! The insurer's approval: what the insurer was asked and what it answered
//! (08 §6 *Insurer approval*, `AUT.INS.02`, `AUT.INS.03`, `AUT.ACL.16`).
//!
//! One row is one request to the insurer and its answer: the estimate, a
//! supplementary for hidden damage, or the charges of a total loss. The
//! answer is the insurer's reference (or LPO), the document it came in
//! (an email, kept as a reference) and the approved amount **per line**. An
//! approved amount is changed only by attaching the insurer's written
//! approval, so a row is never edited: a new request is a new row.

use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// What the insurer was asked.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApprovalKind {
    /// The estimate of the repair.
    #[default]
    Estimate,
    /// Hidden damage found during strip-down.
    Supplementary,
    /// Strip-down, labour done and storage after a total loss.
    Charges,
}

/// How the insurer answered.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ApprovalOutcome {
    /// Sent; no answer yet.
    #[default]
    Pending,
    /// The insurer approved it, in whole or in part.
    Approved,
    /// The insurer declined it.
    Declined,
}

/// One line the insurer approved, with its amount before VAT in fils.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ApprovedLine {
    /// `labour`, `part` or `charge`.
    pub kind: String,
    /// The job card line (0 for a charge).
    pub line_id: i64,
    /// What the line is, as the invoice will say.
    pub description: String,
    /// The approved amount before VAT, in fils.
    pub amount: i64,
}

/// The kinds of line an approval has.
pub const LINE_KINDS: [&str; 3] = ["labour", "part", "charge"];

/// A request to the insurer and its answer.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "auto_claim_approvals", index(claim_id))]
pub struct ClaimApproval {
    /// The primary key.
    pub id: i64,
    /// The claim.
    #[model(references = "auto_insurance_claims")]
    pub claim_id: i64,
    /// Estimate, supplementary or charges.
    pub kind: ApprovalKind,
    /// 1 for the estimate, 2 and up for each request after it.
    pub round_no: i64,
    /// The estimate round of the job card the request is about.
    pub job_round_id: Option<i64>,
    /// The net total asked for, in fils.
    pub estimate_amount: i64,
    /// What was found (a supplementary).
    pub findings: Option<String>,
    /// Pending, approved or declined.
    pub outcome: ApprovalOutcome,
    /// The insurer's approval reference or LPO number.
    pub reference: Option<String>,
    /// The approved amount before VAT, in fils.
    pub approved_amount: i64,
    /// The approved amount per line.
    pub lines: Json<Vec<ApprovedLine>>,
    /// The insurer's written approval (a document reference).
    pub document: Option<String>,
    /// Why the insurer declined.
    pub reason: Option<String>,
    /// The user who made the request.
    pub requested_by: Option<i64>,
    /// When it was made.
    pub requested_at: Option<DateTime>,
    /// The user who recorded the answer.
    pub decided_by: Option<i64>,
    /// When the insurer answered.
    pub decided_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ClaimApproval {
    /// The requests of `claim_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_claim(db: &Db, claim_id: i64) -> Result<Vec<ClaimApproval>> {
        ClaimApproval::query()
            .where_(ClaimApproval::CLAIM_ID.eq(claim_id))
            .order_by(ClaimApproval::ID)
            .get(db)
            .await
    }

    /// The request of `kind` that waits for the insurer's answer, if any.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn pending(
        db: &Db,
        claim_id: i64,
        kind: ApprovalKind,
    ) -> Result<Option<ClaimApproval>> {
        ClaimApproval::query()
            .where_(ClaimApproval::CLAIM_ID.eq(claim_id))
            .where_(ClaimApproval::KIND.eq(kind))
            .where_(ClaimApproval::OUTCOME.eq(ApprovalOutcome::Pending))
            .order_by_desc(ClaimApproval::ID)
            .first(db)
            .await
    }

    /// The sum of the amounts of `lines`.
    pub fn sum(lines: &[ApprovedLine]) -> i64 {
        lines.iter().map(|line| line.amount).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_amounts_of_the_lines_add_up() {
        let lines = vec![
            ApprovedLine {
                kind: "labour".to_owned(),
                line_id: 1,
                description: "Door".to_owned(),
                amount: 80_000,
            },
            ApprovedLine {
                kind: "part".to_owned(),
                line_id: 2,
                description: "Bumper".to_owned(),
                amount: 120_000,
            },
        ];
        assert_eq!(ClaimApproval::sum(&lines), 200_000);
        assert_eq!(ClaimApproval::sum(&[]), 0);
    }

    #[test]
    fn a_new_request_waits_for_the_insurer() {
        assert_eq!(ClaimApproval::default().outcome, ApprovalOutcome::Pending);
    }
}
