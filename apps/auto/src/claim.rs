//! The insurance repair claim: one accident repair funded by an insurer (08
//! §6 *Insurance claim*, §7.1 *Insurance repair claim*, `AUT.INS.01`).
//!
//! A claim **funds** a job card: the job card follows its own machine, and the
//! claim records what the insurer approved and who pays what. It is registered
//! by the insurance coordinator with the insurer, the policy and claim number,
//! the accident date, the police report (the repair permit, `AUT.ACL.20`),
//! agency or non-agency repair and the excess. Its status moves only through
//! the machine in [`crate::claim_machine`]; [`crate::claims`] holds the
//! services, [`crate::claim_split`] the split billing.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

pub use crate::seed::INSURANCE_CLAIM_SUBJECT_TYPE;

/// The customer withdrew the claim and pays the repair: a private estimate is
/// sent on the same job card.
pub const CONTINUE_PRIVATELY: &str = "continue_privately";
/// The customer withdrew the claim and stops the job.
pub const STOP: &str = "stop";

/// Where a claim stands (08 §7.1 *Insurance repair claim*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClaimStatus {
    /// Claim and documents recorded; vehicle checked in.
    #[default]
    Registered,
    /// Surveyor inspection booked or photos requested.
    AwaitingSurvey,
    /// Estimate sent to the insurer.
    EstimateSubmitted,
    /// Approval or LPO received; work on approved lines may proceed.
    Approved,
    /// Hidden damage found; supplementary estimate waiting for the insurer.
    SupplementaryPending,
    /// Job card passed QC; ready for billing.
    RepairComplete,
    /// The insurer declared the vehicle a total loss.
    TotalLoss,
    /// Insurer invoice and customer excess invoice issued.
    Invoiced,
    /// Insurer paid and customer share paid (terminal).
    Settled,
    /// The insurer declined the claim (terminal).
    Rejected,
    /// Withdrawn by the customer (terminal).
    Cancelled,
    /// Registered in error (terminal).
    Voided,
}

impl ClaimStatus {
    /// Whether the claim is over.
    pub fn is_closed(self) -> bool {
        matches!(
            self,
            ClaimStatus::Settled
                | ClaimStatus::Rejected
                | ClaimStatus::Cancelled
                | ClaimStatus::Voided
        )
    }

    /// Whether the claim funds its job card: the insurer pays the approved
    /// lines, so the job card is not billed to the customer itself.
    pub fn funds_job_card(self) -> bool {
        matches!(
            self,
            ClaimStatus::Registered
                | ClaimStatus::AwaitingSurvey
                | ClaimStatus::EstimateSubmitted
                | ClaimStatus::Approved
                | ClaimStatus::SupplementaryPending
                | ClaimStatus::RepairComplete
                | ClaimStatus::TotalLoss
                | ClaimStatus::Invoiced
                | ClaimStatus::Settled
        )
    }
}

/// How the vehicle is repaired.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RepairType {
    /// At the manufacturer's agency.
    Agency,
    /// At a workshop of the insurer's panel or the customer's choice.
    #[default]
    NonAgency,
}

/// An insurance repair claim.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "auto_insurance_claims",
    state_machine,
    unique(insurer_id, claim_no),
    index(vehicle_id),
    index(job_card_id),
    index(status)
)]
pub struct InsuranceClaim {
    /// The primary key.
    pub id: i64,
    /// The insurer.
    #[model(references = "auto_insurers")]
    pub insurer_id: i64,
    /// The policyholder billed for the excess and the private items.
    pub customer_party_id: i64,
    /// The vehicle.
    #[model(references = "auto_vehicles")]
    pub vehicle_id: i64,
    /// The job card the claim funds.
    pub job_card_id: Option<i64>,
    /// The policy number.
    pub policy_no: String,
    /// The insurer's claim number.
    pub claim_no: String,
    /// The day of the accident.
    pub accident_date: NaiveDate,
    /// The police accident report number (`AUT.ACL.20`).
    pub police_report_no: Option<String>,
    /// The date of the police report.
    pub police_report_date: Option<NaiveDate>,
    /// Agency or non-agency repair.
    pub repair_type: RepairType,
    /// The excess the customer pays, in fils.
    pub excess: i64,
    /// The rate card of the insurer in force at registration (wl-invoicing).
    pub rate_card_id: Option<i64>,
    /// The version of that card.
    pub rate_card_version: Option<i64>,
    /// The net total of the estimate sent to the insurer, in fils.
    pub estimate_amount: i64,
    /// What the insurer approved before VAT, in fils: the estimate and the
    /// supplementaries it approved.
    pub approved_amount: i64,
    /// When the surveyor was asked.
    pub survey_booked_at: Option<DateTime>,
    /// When anything was first sent to the insurer (a void is refused after).
    pub sent_to_insurer_at: Option<DateTime>,
    /// When the estimate was sent.
    pub submitted_at: Option<DateTime>,
    /// When the insurer approved it.
    pub approved_at: Option<DateTime>,
    /// When the supplementary in waiting was sent (`AUT.RUL.11`).
    pub supplementary_at: Option<DateTime>,
    /// When the repair passed QC.
    pub completed_at: Option<DateTime>,
    /// When the insurer declared a total loss.
    pub total_loss_at: Option<DateTime>,
    /// The insurer's total-loss notice (document reference).
    pub total_loss_notice: Option<String>,
    /// The customer's written withdrawal (document reference).
    pub withdrawal_document: Option<String>,
    /// What the customer chose when work had started: `continue_privately` or
    /// `stop` ([`CONTINUE_PRIVATELY`], [`STOP`]).
    pub withdrawal_choice: Option<String>,
    /// The job card a claim registered in error was unlinked from.
    pub unlinked_job_card_id: Option<i64>,
    /// When the letter to the insurer about the withdrawal was queued.
    pub letter_sent_at: Option<DateTime>,
    /// Why the claim was rejected, cancelled or voided.
    pub closed_reason: Option<String>,
    /// When it was rejected, cancelled or voided.
    pub closed_at: Option<DateTime>,
    /// The payer bill of the insurer's share (wl-invoicing).
    pub payer_bill_id: Option<i64>,
    /// The invoice of the customer's share (wl-invoicing).
    pub customer_invoice_id: Option<i64>,
    /// When the claim was invoiced.
    pub invoiced_at: Option<DateTime>,
    /// When the claim was settled.
    pub settled_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: ClaimStatus,
    /// The user who registered it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl InsuranceClaim {
    /// The number people read: `CLM-00007`.
    pub fn number(&self) -> String {
        format!("CLM-{:05}", self.id)
    }

    /// The claim as a [`Subject`]: `auto.insurance_claim:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(INSURANCE_CLAIM_SUBJECT_TYPE, self.id)
    }

    /// Whether the police accident report (the repair permit) is recorded.
    pub fn has_police_report(&self) -> bool {
        self.police_report_no
            .as_deref()
            .is_some_and(|no| !no.trim().is_empty())
    }

    /// The open claim that funds job card `job_card_id`, if there is one.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn funding(db: &Db, job_card_id: i64) -> Result<Option<InsuranceClaim>> {
        let claims = InsuranceClaim::query()
            .where_(InsuranceClaim::JOB_CARD_ID.eq(Some(job_card_id)))
            .order_by_desc(InsuranceClaim::ID)
            .get(db)
            .await?;
        Ok(claims
            .into_iter()
            .find(|claim| claim.status.funds_job_card()))
    }

    /// Every claim of job card `job_card_id`, newest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_job_card(db: &Db, job_card_id: i64) -> Result<Vec<InsuranceClaim>> {
        InsuranceClaim::query()
            .where_(InsuranceClaim::JOB_CARD_ID.eq(Some(job_card_id)))
            .order_by_desc(InsuranceClaim::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_is_numbered_and_is_a_subject() {
        let claim = InsuranceClaim {
            id: 7,
            ..InsuranceClaim::default()
        };
        assert_eq!(claim.number(), "CLM-00007");
        assert_eq!(claim.subject().to_string(), "auto.insurance_claim:7");
    }

    #[test]
    fn a_claim_that_ended_without_the_insurer_paying_no_longer_funds_the_job() {
        for open in [
            ClaimStatus::Registered,
            ClaimStatus::Approved,
            ClaimStatus::TotalLoss,
            ClaimStatus::Invoiced,
        ] {
            assert!(open.funds_job_card(), "{open:?}");
        }
        for ended in [
            ClaimStatus::Rejected,
            ClaimStatus::Cancelled,
            ClaimStatus::Voided,
        ] {
            assert!(!ended.funds_job_card(), "{ended:?}");
            assert!(ended.is_closed());
        }
    }

    #[test]
    fn the_police_report_must_not_be_blank() {
        let mut claim = InsuranceClaim::default();
        assert!(!claim.has_police_report());
        claim.police_report_no = Some("  ".to_owned());
        assert!(!claim.has_police_report());
        claim.police_report_no = Some("DXB-2026-88".to_owned());
        assert!(claim.has_police_report());
    }
}
