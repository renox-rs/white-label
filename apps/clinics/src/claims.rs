//! The claim services (05 §4.3 steps 1 to 3, §7.1 *Claim*; `CLN.CLM.01`;
//! base for BL-CLN-07 to BL-CLN-11): making the claim from the payer bill a
//! billed visit has, validating it, submitting it, recording the channel's
//! technical rejection and sending it again after a remittance.
//!
//! Every status change is a transition of the machine in
//! [`crate::claim_machine`]. The money side is the payer bill's and is driven
//! through the kernel's `PayerBills` (`issue`, `submission_rejected`,
//! `resubmit`); the remittance and the recoupment are in
//! [`crate::claim_money`].
//!
//! Order of a service that calls the payer bill: the claim's own guards are
//! asked first (a refusal must not leave the payer bill issued while the
//! claim is not), then the payer bill, then the claim's transition.
//!
//! Decisions:
//!
//! - **Windows.** The first-submission window counts from the visit date; the
//!   resubmission window from the remittance that denied the claim. Both are
//!   the payer contract's ([`crate::contract::PayerContract`]); a claim
//!   without a contract has no window here (the payer bill still has its
//!   agreement's).
//! - **Codes present** means every line of the claim has its service code
//!   (the charge line's code). The ICD-10 and CPT code capture and the EMR
//!   import are not built; they join this guard when they are.
//! - **Prior authorisation linked** means that where a line's service is on
//!   the payer's prior-authorisation list, the claim carries the payer
//!   reference of an authorisation (the first the bill consumed).
//! - **Deadline of a technical rejection.** The payer bill answers the last
//!   day the claim can be sent again; when it gives none (no agreement
//!   window) the contract's window is used.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionCtx, TransitionError};
use wl_kernel::documents::dubai_date;
use wl_kernel::payer_bills::{PayerBillId, payer_bills};

use crate::authorisation::PriorAuthService;
use crate::charge::ChargeLine;
use crate::claim::{Claim, ClaimLine};
use crate::claim_machine::{ClaimInput, RESUBMIT, SUBMIT, VALIDATE, clean};
use crate::contract::PayerContract;
use crate::db::refused;
use crate::permissions;
use crate::policy::InsurancePolicy;
use crate::texts::english;
use crate::visit::Visit;

/// Makes the claim of payer bill `bill`, which the billing of `visit` made:
/// one claim line for each payer bill line, with the charge it claims. A
/// second call for the same payer bill returns the claim that exists.
///
/// `contract` is the payer contract whose windows apply and `reference` the
/// payer's reference of the prior authorisation the bill carries.
///
/// # Errors
///
/// A 422 when the visit has no patient or the payer cannot be found, and the
/// database's and the payer bills' errors.
pub async fn create_for_visit(
    state: &AppState,
    visit: &Visit,
    bill: PayerBillId,
    contract: Option<&PayerContract>,
    reference: Option<String>,
    by: Option<i64>,
) -> Result<Claim> {
    if let Some(found) = Claim::of_payer_bill(&state.db, bill.0).await? {
        return Ok(found);
    }
    let patient = visit
        .patient_party_id
        .ok_or_else(|| refused(english("clinics.claim.problem.no_patient", &[])))?;
    let summary = payer_bills(state).find(&state.db, bill).await?;
    let policy = match visit.coverage_policy_id.or(visit.policy_id) {
        Some(id) => InsurancePolicy::find(&state.db, id).await?,
        None => None,
    };
    let payer = summary
        .as_ref()
        .map(|s| s.payer.0)
        .or_else(|| contract.map(|c| c.payer_party_id))
        .or_else(|| policy.as_ref().map(|p| p.payer_party_id))
        .ok_or_else(|| refused(english("clinics.claim.problem.no_payer", &[])))?;
    let charges: Vec<ChargeLine> = ChargeLine::active_of(&state.db, visit.id)
        .await?
        .into_iter()
        .filter(|line| line.payer_share > 0)
        .collect();
    let bill_lines = payer_bills(state).lines(&state.db, bill).await?;
    let claimed = summary
        .as_ref()
        .map_or_else(|| charges.iter().map(|c| c.payer_share).sum(), |s| s.claimed);
    let mut claim = Claim {
        visit_id: visit.id,
        payer_bill_id: bill.0,
        contract_id: contract.map(|c| c.id),
        patient_party_id: patient,
        payer_party_id: payer,
        policy_id: visit.coverage_policy_id.or(visit.policy_id),
        visit_date: visit.visit_date,
        payer_reference: reference,
        claimed,
        created_by: by,
        ..Claim::default()
    };
    claim.save(&state.db).await?;
    for (index, charge) in charges.iter().enumerate() {
        let on_bill = bill_lines.get(index);
        ClaimLine {
            claim_id: claim.id,
            position: i64::try_from(index + 1).unwrap_or(i64::MAX),
            payer_bill_line_id: on_bill.map(|l| l.id),
            charge_line_id: Some(charge.id),
            code: charge.code.trim().to_owned(),
            description: on_bill.map_or_else(
                || format!("{} x {}", charge.code, charge.units),
                |l| l.description.clone(),
            ),
            claimed: on_bill.map_or(charge.payer_share, |l| l.claimed),
            ..ClaimLine::default()
        }
        .save(&state.db)
        .await?;
    }
    Ok(claim)
}

/// The last day the attempt being prepared can be sent (Asia/Dubai,
/// inclusive): the first-submission window from the visit for a claim never
/// sent again after a remittance, the resubmission window from the last
/// remittance for one that was.
pub fn attempt_deadline(claim: &Claim, contract: Option<&PayerContract>) -> Option<NaiveDate> {
    let contract = contract?;
    if claim.resubmission_count > 0 {
        claim
            .last_remittance_on
            .and_then(|on| contract.resubmission_deadline(on))
    } else {
        contract.first_submission_deadline(claim.visit_date)
    }
}

/// The last day a resubmission is accepted: the resubmission window from the
/// last remittance.
pub fn resubmission_deadline(claim: &Claim, contract: Option<&PayerContract>) -> Option<NaiveDate> {
    claim
        .last_remittance_on
        .and_then(|on| contract?.resubmission_deadline(on))
}

/// The contract of `claim`, if it has one.
async fn contract_of(db: &Db, claim: &Claim) -> Result<Option<PayerContract>> {
    match claim.contract_id {
        Some(id) => PayerContract::find(db, id).await,
        None => Ok(None),
    }
}

/// The service codes of `claim` that the payer asks a prior authorisation
/// for when the claim carries none.
async fn missing_authorisations(db: &Db, claim: &Claim, lines: &[ClaimLine]) -> Result<Vec<String>> {
    if claim
        .payer_reference
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty())
    {
        return Ok(Vec::new());
    }
    let mut missing = Vec::new();
    for line in lines {
        if PriorAuthService::needed(db, claim.payer_party_id, &line.code).await?
            && !missing.contains(&line.code)
        {
            missing.push(line.code.clone());
        }
    }
    Ok(missing)
}

/// What the guards of the claim's own transitions need, gathered once.
async fn gather(state: &AppState, claim: &Claim, at: DateTime, by: Option<i64>) -> Result<ClaimInput> {
    let contract = contract_of(&state.db, claim).await?;
    let lines = ClaimLine::of_claim(&state.db, claim.id).await?;
    Ok(ClaimInput {
        on: Some(dubai_date(at)),
        by,
        lines: i64::try_from(lines.len()).unwrap_or(i64::MAX),
        blank_codes: i64::try_from(lines.iter().filter(|l| l.code.trim().is_empty()).count())
            .unwrap_or(i64::MAX),
        missing_authorisation: missing_authorisations(&state.db, claim, &lines).await?,
        deadline: attempt_deadline(claim, contract.as_ref()),
        accepts_late: contract.as_ref().is_some_and(|c| c.accepts_late_claims),
        ..ClaimInput::default()
    })
}

/// Asks what running `name` as `by` would ask (who, from where, the guards)
/// without writing, so that a refusal comes before the payer bill is
/// called.
///
/// # Errors
///
/// The refusal the run would give.
pub async fn dry_run(
    claim: &Claim,
    name: &str,
    by: &AuthUser,
    input: &ClaimInput,
) -> std::result::Result<(), TransitionError> {
    let machine = Claim::machine();
    let transition = machine.find(name)?;
    transition.check_access(claim, Some(by))?;
    transition.check_from(claim.status.as_str())?;
    let value = renox::serde_json::to_value(input).unwrap_or(renox::serde_json::Value::Null);
    let ctx = TransitionCtx::new(Some(by.clone())).with_input(value);
    transition.check_guards(claim, &ctx).await
}

/// `validate`: the claim is checked and waits for submission. A claim sent
/// after the window to a payer that accepts late claims needs `late_reason`.
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `NotFrom` unless the claim is a
/// `draft`; `Guard` (422) for a line without a code, a prior authorisation
/// the payer asks for and the claim lacks, and a window that has passed.
pub async fn validate(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    late_reason: Option<&str>,
    at: DateTime,
) -> std::result::Result<Claim, TransitionError> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    let mut input = gather(state, &claim, at, Some(by.id)).await?;
    input.late_reason = clean(&late_reason.map(str::to_owned));
    let _ = claim
        .transition(VALIDATE)
        .by(by)
        .input(&input)
        .run(state)
        .await?;
    Ok(Claim::find_or_404(&state.db, id).await?)
}

/// `submit`: the claim is sent to the payer; the submission reference and
/// date are recorded and the payer bill is issued (it keeps its number when
/// issued again after a technical rejection).
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `NotFrom` unless the claim is
/// `ready`; the payer bill's refusals.
pub async fn submit(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    reference: Option<&str>,
    at: DateTime,
) -> std::result::Result<Claim, TransitionError> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    let input = ClaimInput {
        on: Some(dubai_date(at)),
        by: Some(by.id),
        reference: clean(&reference.map(str::to_owned)),
        ..ClaimInput::default()
    };
    dry_run(&claim, SUBMIT, by, &input).await?;
    payer_bills(state)
        .issue(state, claim.payer_bill(), Some(by))
        .await?;
    let _ = claim
        .transition(SUBMIT)
        .by(by)
        .input(&input)
        .run(state)
        .await?;
    Ok(Claim::find_or_404(&state.db, id).await?)
}

/// `resubmit`: a `partially_paid` or `denied` claim is corrected and sent
/// again; the payer bill runs `resubmit` and the count goes up by one. The
/// payer's window and limit are the contract's.
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `NotFrom` unless the claim is
/// `partially_paid` or `denied` (a `paid` claim is not resubmitted);
/// `Guard` (422) without corrections, past the window or the limit.
pub async fn resubmit(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    corrections: &str,
    reference: Option<&str>,
    at: DateTime,
) -> std::result::Result<Claim, TransitionError> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    let contract = contract_of(&state.db, &claim).await?;
    let input = ClaimInput {
        on: Some(dubai_date(at)),
        by: Some(by.id),
        reference: clean(&reference.map(str::to_owned)),
        corrections: clean(&Some(corrections.to_owned())),
        deadline: resubmission_deadline(&claim, contract.as_ref()),
        limit: contract.as_ref().map(|c| c.resubmission_limit),
        ..ClaimInput::default()
    };
    dry_run(&claim, RESUBMIT, by, &input).await?;
    payer_bills(state)
        .resubmit(state, claim.payer_bill(), corrections.trim(), Some(by))
        .await?;
    let _ = claim
        .transition(RESUBMIT)
        .by(by)
        .input(&input)
        .run(state)
        .await?;
    Ok(Claim::find_or_404(&state.db, id).await?)
}

/// Whether `user` may manage claims.
pub fn can_manage(user: &AuthUser) -> bool {
    user.has_permission(permissions::CLAIMS_MANAGE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn contract() -> PayerContract {
        PayerContract {
            first_submission_days: 30,
            resubmission_days: 30,
            resubmission_limit: 2,
            ..PayerContract::default()
        }
    }

    #[test]
    fn the_first_window_counts_from_the_visit() {
        let claim = Claim {
            visit_date: day("2026-11-01"),
            ..Claim::default()
        };
        assert_eq!(
            attempt_deadline(&claim, Some(&contract())),
            Some(day("2026-12-01"))
        );
        assert_eq!(attempt_deadline(&claim, None), None);
    }

    #[test]
    fn a_resubmitted_claim_is_checked_against_the_resubmission_window() {
        let claim = Claim {
            visit_date: day("2026-11-01"),
            resubmission_count: 1,
            last_remittance_on: Some(day("2026-12-10")),
            ..Claim::default()
        };
        assert_eq!(
            attempt_deadline(&claim, Some(&contract())),
            Some(day("2027-01-09"))
        );
        assert_eq!(
            resubmission_deadline(&claim, Some(&contract())),
            Some(day("2027-01-09"))
        );
        let never_remitted = Claim {
            resubmission_count: 1,
            ..Claim::default()
        };
        assert_eq!(attempt_deadline(&never_remitted, Some(&contract())), None);
    }
}
