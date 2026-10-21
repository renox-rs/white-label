//! The claim: the clinic's request for payment to a payer (05 §6 *Claim*,
//! `CLN.CLM.01`; base for BL-CLN-07 to BL-CLN-11).
//!
//! A claim is made from the draft payer bill a billed visit has
//! ([`crate::claims::create_for_visit`]) and wraps it: the money side (the
//! lines' paid and denied amounts, the number, the credit balance) is the
//! payer bill's, in wl-invoicing, and is driven through the kernel's
//! `PayerBills`; the coding, the validation, the submission attempts and the
//! clinic-side state are kept here. The state moves only through the machine
//! in [`crate::claim_machine`].
//!
//! Decision: the claim follows its payer bill. The remittance and the
//! recoupment are recorded on the payer bill (which holds an overpayment or a
//! recoupment for a second person's approval, `FIN.ACL.16`); the claim then
//! moves to the state the payer bill reached ([`crate::claim_money`]), also
//! when that happens later, after the approval.
//!
//! Decision: the claim number is the id (`CLM-0101`) and is never changed:
//! a claim returned to `draft` by a technical rejection is sent again with
//! the same number and the same payer bill.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Json};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::payer_bills::PayerBillId;
use wl_kernel::{PartyId, Subject};

/// The subject type of a claim.
pub const CLAIM_SUBJECT_TYPE: &str = "clinics.claim";

/// Where a claim stands (05 §7.1 *Claim*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ClaimStatus {
    /// Created from charge capture, or returned after a technical rejection.
    #[default]
    Draft,
    /// Validated; waiting for submission.
    Ready,
    /// Sent to the payer; locked (`CLN.ACL.09`).
    Submitted,
    /// All lines paid as claimed; only `record_recoupment` leaves it.
    Paid,
    /// Some lines paid, some denied, reduced or recouped.
    PartiallyPaid,
    /// All lines denied or recouped.
    Denied,
    /// Corrected and sent again.
    Resubmitted,
    /// The remaining balance written off or moved to the patient; only
    /// `record_recoupment` leaves it.
    WrittenOff,
    /// Created in error before submission; never sent (terminal).
    Voided,
}

impl ClaimStatus {
    /// Whether the payer has the claim and has not answered: `submitted` or
    /// `resubmitted`.
    pub fn is_with_payer(self) -> bool {
        matches!(self, ClaimStatus::Submitted | ClaimStatus::Resubmitted)
    }

    /// Whether the claim can still be corrected by the clinic: `draft` or
    /// `ready` (a submitted claim is locked, `CLN.ACL.09`).
    pub fn is_editable(self) -> bool {
        matches!(self, ClaimStatus::Draft | ClaimStatus::Ready)
    }
}

/// A claim.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "clinics_claims",
    state_machine,
    unique(payer_bill_id),
    index(visit_id),
    index(status)
)]
pub struct Claim {
    /// The primary key.
    pub id: i64,
    /// The visit it claims for.
    #[model(references = "clinics_visits")]
    pub visit_id: i64,
    /// The payer bill (`invoicing.payer_bill`) that carries the money.
    pub payer_bill_id: i64,
    /// The payer contract whose windows apply.
    pub contract_id: Option<i64>,
    /// The patient, a party.
    pub patient_party_id: i64,
    /// The payer, a party.
    pub payer_party_id: i64,
    /// The insurance policy of the visit.
    pub policy_id: Option<i64>,
    /// The day of the visit (Asia/Dubai); the first-submission window counts
    /// from it.
    pub visit_date: NaiveDate,
    /// The payer's reference of the prior authorisation the claim carries.
    pub payer_reference: Option<String>,
    /// What the payer is claimed for, in fils.
    pub claimed: i64,
    /// What the payer paid and kept, in fils.
    pub paid: i64,
    /// What the payer denied and is still open, in fils.
    pub denied: i64,
    /// How many times it was sent again after a remittance (a technical
    /// rejection does not count).
    pub resubmission_count: i64,
    /// How many times it was sent to the channel, resubmissions and the
    /// attempts a technical rejection sent back included.
    pub attempts: i64,
    /// The reference the channel gave the last submission.
    pub submission_reference: Option<String>,
    /// The day it was last submitted (Asia/Dubai).
    pub submitted_on: Option<NaiveDate>,
    /// Why it is sent after the window to a payer that accepts late claims.
    pub late_reason: Option<String>,
    /// The corrections written down for the last resubmission.
    pub corrections: Option<String>,
    /// Whether a remittance was recorded on it.
    pub has_remittance: bool,
    /// The date of the last remittance advice that paid or denied it.
    pub last_remittance_on: Option<NaiveDate>,
    /// The payer's reference of that advice.
    pub last_remittance_reference: Option<String>,
    /// The last day the claim can be sent again after a technical rejection
    /// (Asia/Dubai, inclusive).
    pub fix_deadline: Option<NaiveDate>,
    /// What waits for a second person's approval on the payer bill:
    /// `remittance` or `recoupment`.
    pub held_kind: Option<String>,
    /// The approval request that holds it.
    pub held_request_id: Option<i64>,
    /// The payer's reference of the held advice.
    pub held_reference: Option<String>,
    /// The date of the held advice.
    pub held_date: Option<NaiveDate>,
    /// The user who recorded the advice that is held, or being recorded.
    pub held_by: Option<i64>,
    /// How the last hold ended without a yes (`rejected`, `returned`,
    /// `withdrawn`, `expired`, `failed`); cleared by the next hold.
    pub held_result: Option<String>,
    /// The day the billing coordinator was told the first-submission window
    /// is running (`CLN.RUL.03`, N days after the visit).
    pub window_notified_on: Option<NaiveDate>,
    /// The day the clinic manager got the task for the window about to end
    /// (`CLN.RUL.03`, D days before it ends).
    pub window_task_on: Option<NaiveDate>,
    /// The lifecycle state; changed only by a transition.
    pub status: ClaimStatus,
    /// The user who made it, if any.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Claim {
    /// The claim as a [`Subject`]: `clinics.claim:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(CLAIM_SUBJECT_TYPE, self.id)
    }

    /// The number shown to staff: `CLM-0101`.
    pub fn number(&self) -> String {
        format!("CLM-{:04}", self.id)
    }

    /// The payer bill as a [`PayerBillId`].
    pub fn payer_bill(&self) -> PayerBillId {
        PayerBillId(self.payer_bill_id)
    }

    /// The payer as a [`PartyId`].
    pub fn payer(&self) -> PartyId {
        PartyId(self.payer_party_id)
    }

    /// The patient as a [`PartyId`].
    pub fn patient(&self) -> PartyId {
        PartyId(self.patient_party_id)
    }

    /// Whether a remittance or a recoupment waits for approval.
    pub fn is_held(&self) -> bool {
        self.held_request_id.is_some()
    }

    /// The claim of payer bill `bill`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_payer_bill(db: &Db, bill: i64) -> Result<Option<Claim>> {
        Claim::query()
            .where_(Claim::PAYER_BILL_ID.eq(bill))
            .first(db)
            .await
    }

    /// The claims of visit `visit`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_visit(db: &Db, visit: i64) -> Result<Vec<Claim>> {
        Claim::query()
            .where_(Claim::VISIT_ID.eq(visit))
            .order_by(Claim::ID)
            .get(db)
            .await
    }
}

/// A line of a claim: one line of the payer bill, with the charge it claims
/// and what the payer paid, denied and recouped on it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_claim_lines", index(claim_id))]
pub struct ClaimLine {
    /// The primary key.
    pub id: i64,
    /// The claim.
    #[model(references = "clinics_claims")]
    pub claim_id: i64,
    /// The order on the claim, from 1.
    pub position: i64,
    /// The line of the payer bill (what a remittance names).
    pub payer_bill_line_id: Option<i64>,
    /// The charge line of the visit it claims.
    pub charge_line_id: Option<i64>,
    /// The service or consumable code on the claim.
    pub code: String,
    /// What the line says.
    pub description: String,
    /// What the payer is asked to pay, in fils.
    pub claimed: i64,
    /// What the payer paid and kept, in fils.
    pub paid: i64,
    /// What the payer denied and is still open, in fils.
    pub denied: i64,
    /// What the payer took back, in all, in fils.
    pub recouped: i64,
    /// The payer's denial or recoupment code.
    pub denial_code: Option<String>,
    /// The payer's reason.
    pub denial_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ClaimLine {
    /// The lines of claim `claim`, in order.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_claim(db: &Db, claim: i64) -> Result<Vec<ClaimLine>> {
        ClaimLine::query()
            .where_(ClaimLine::CLAIM_ID.eq(claim))
            .order_by(ClaimLine::POSITION)
            .get(db)
            .await
    }
}

/// How an attempt reached the payer.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AttemptKind {
    /// The first submission, or the same claim sent again after the channel
    /// refused it (a technical rejection).
    #[default]
    Submission,
    /// A resubmission after a remittance (counts against the payer's limit).
    Resubmission,
}

/// One error the channel gave a rejected attempt.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClaimError {
    /// The channel's error code, e.g. `E104`.
    pub code: String,
    /// The channel's message, e.g. `invalid member ID`.
    pub message: String,
}

/// One submission of a claim to the channel; each attempt keeps its own
/// errors.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_claim_attempts", unique(claim_id, number))]
pub struct ClaimAttempt {
    /// The primary key.
    pub id: i64,
    /// The claim.
    #[model(references = "clinics_claims")]
    pub claim_id: i64,
    /// The order of the attempt, from 1.
    pub number: i64,
    /// A submission or a resubmission.
    pub kind: AttemptKind,
    /// The reference the channel gave it.
    pub reference: Option<String>,
    /// The day it was sent (Asia/Dubai).
    pub submitted_on: NaiveDate,
    /// The channel's errors when it refused the attempt before adjudication.
    pub errors: Json<Vec<ClaimError>>,
    /// The day the rejection was recorded.
    pub rejected_on: Option<NaiveDate>,
    /// The user who recorded the attempt.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ClaimAttempt {
    /// The attempts of claim `claim`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_claim(db: &Db, claim: i64) -> Result<Vec<ClaimAttempt>> {
        ClaimAttempt::query()
            .where_(ClaimAttempt::CLAIM_ID.eq(claim))
            .order_by(ClaimAttempt::NUMBER)
            .get(db)
            .await
    }

    /// Whether the channel refused the attempt.
    pub fn is_rejected(&self) -> bool {
        self.rejected_on.is_some()
    }
}

/// What a payer's answer was.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnswerKind {
    /// A remittance advice paid and denied lines.
    #[default]
    Remittance,
    /// The payer took money back.
    Recoupment,
}

/// A remittance or a recoupment recorded on a claim.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "clinics_claim_remittances", index(claim_id))]
pub struct ClaimRemittance {
    /// The primary key.
    pub id: i64,
    /// The claim.
    #[model(references = "clinics_claims")]
    pub claim_id: i64,
    /// A remittance or a recoupment.
    pub kind: AnswerKind,
    /// The payer's reference of the advice.
    pub reference: Option<String>,
    /// The date of the advice (Asia/Dubai).
    pub advice_date: NaiveDate,
    /// What the claim had been paid after it, in fils.
    pub paid: i64,
    /// What was denied and open after it, in fils.
    pub denied: i64,
    /// The part of the advice that went to the payer credit balance, in fils.
    pub overpayment: i64,
    /// The amount taken back, in fils (a recoupment).
    pub recouped: i64,
    /// The payer's recoupment code.
    pub code: Option<String>,
    /// Whether a reversed payment caused it.
    pub reversal: bool,
    /// The approval request that let it be recorded.
    pub approval_request_id: Option<i64>,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ClaimRemittance {
    /// The answers recorded on claim `claim`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_claim(db: &Db, claim: i64) -> Result<Vec<ClaimRemittance>> {
        ClaimRemittance::query()
            .where_(ClaimRemittance::CLAIM_ID.eq(claim))
            .order_by(ClaimRemittance::ID)
            .get(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_stored_in_snake_case() {
        let names: Vec<_> = ClaimStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            names,
            [
                "draft",
                "ready",
                "submitted",
                "paid",
                "partially_paid",
                "denied",
                "resubmitted",
                "written_off",
                "voided"
            ]
        );
    }

    #[test]
    fn a_claim_is_numbered_and_is_a_subject() {
        let claim = Claim {
            id: 101,
            ..Claim::default()
        };
        assert_eq!(claim.number(), "CLM-0101");
        assert_eq!(claim.subject().to_string(), "clinics.claim:101");
        assert!(!claim.is_held());
    }

    #[test]
    fn only_a_claim_with_the_payer_is_locked() {
        assert!(ClaimStatus::Submitted.is_with_payer());
        assert!(ClaimStatus::Resubmitted.is_with_payer());
        assert!(!ClaimStatus::Draft.is_with_payer());
        assert!(ClaimStatus::Draft.is_editable());
        assert!(ClaimStatus::Ready.is_editable());
        assert!(!ClaimStatus::Submitted.is_editable());
    }
}
