//! Voiding a claim created in error (05 §3.3 `CLN.ACL.14`, §4.3 "When it goes
//! wrong" row *Claim created in error*, §7.1 *Claim* `void`; BL-CLN-10).
//!
//! A claim made by mistake (a duplicate visit, the wrong payer or policy) is
//! voided from `draft` or `ready`, before it was ever sent. [`void`] is the
//! billing coordinator's (`claims.manage`) and goes through the kernel's void
//! pattern (`PLT.ACL.09`, BL-KRN-03): the claim keeps its number, is shown as
//! void in the lists and is never deleted; a claim with any stored submission
//! attempt, also one back in `draft` after a technical rejection, is refused
//! by the machine ([`crate::claim_machine::never_submitted`]).
//!
//! In this order, each step only once the earlier ones are certain, and each
//! one idempotent, so that a failure in the middle is finished by running the
//! void again (the claim is void and has no `void_code` yet: the void is
//! incomplete, and the same call completes it):
//!
//! 1. The reason code (`duplicate`, `wrong_payer`, `wrong_policy`, free text
//!    optional) is checked; `duplicate` needs the claim it duplicates, picked
//!    from the same patient's other claims that are not void ([`candidates`]).
//! 2. The claim's own guards are asked ([`crate::claims::dry_run`]), and the
//!    right policy, if one is named, is priced ([`plan`]): a refusal comes
//!    before anything changed.
//! 3. The payer bill is voided through the kernel's `PayerBills::void`
//!    (`voided`, number kept; it is no longer a bill of the visit, so the
//!    visit can be split again); a bill that is void already is left.
//! 4. The claim is voided by the kernel's void.
//! 5. The visit's payer split is removed (its payer bill and payer share),
//!    once.
//!    If the right policy was named, the visit is re-split to it and a new
//!    claim is made ([`crate::billing::payer_side`]), which keeps the voided
//!    one as the claim it replaces. If none was named (and the reason is not
//!    `duplicate`), the visit is flagged *coverage to resolve* and the billing
//!    coordinator gets a task; no amount moves to the patient. The coordinator
//!    resolves it later with [`resolve_coverage`].
//!
//! Decisions:
//!
//! - **Duplicate.** The claim it duplicates carries the payer share, so the
//!   visit of a duplicate is neither re-split nor flagged.
//! - **Re-split.** The patient's share was invoiced and (maybe) paid when the
//!   visit was billed, so a re-split that would change it is refused; the
//!   charge lines keep their prices and units and are divided again by the
//!   new policy's payer contract. A policy that gives no payer share is
//!   refused too (leave the policy out and resolve the coverage).
//! - **No approval.** The payer bill's void and the claim's are drafts that
//!   were never sent: by default nothing waits for an approval. Should an
//!   admin put an approval policy on `clinics.claim_void` or on the payer
//!   bill's void, voiding a claim is **refused before anything changes**
//!   (`void_waits`): a void that waited half way would leave a voided payer
//!   bill with a live claim, or the other way round, and this app does not
//!   follow an approval to its end. Decision: refusing is the safe reading
//!   until a policy is wanted; then the approval is to be followed by a
//!   listener on `RecordVoided`.
//! - The void does not emit its own event beyond the kernel's `RecordVoided`.

use renox::db::now;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::TransitionError;
use wl_kernel::approvals::ApprovalPolicy;
use wl_kernel::payer_bills::{PAYER_BILL_SUBJECT_TYPE, PayerAnswer, PayerBillStatus, payer_bills};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::task_creator;
use wl_kernel::voids::{self, VoidOutcome};

use crate::billing::payer_side;
use crate::charge::ChargeLine;
use crate::charges::{self, Quote};
use crate::claim::{CLAIM_SUBJECT_TYPE, Claim};
use crate::claim_machine::{ClaimInput, clean};
use crate::claims::{can_manage, dry_run};
use crate::permissions::BILLING_COORDINATOR_ROLE;
use crate::policy::InsurancePolicy;
use crate::texts::english;
use crate::visit::Visit;

/// The claim duplicates another one (a duplicate visit).
pub const DUPLICATE: &str = "duplicate";
/// The claim was made for the wrong payer.
pub const WRONG_PAYER: &str = "wrong_payer";
/// The claim was made for the wrong policy.
pub const WRONG_POLICY: &str = "wrong_policy";
/// The void reason codes.
pub const REASON_CODES: [&str; 3] = [DUPLICATE, WRONG_PAYER, WRONG_POLICY];

/// What to void a claim with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VoidRequest {
    /// One of [`REASON_CODES`].
    pub code: String,
    /// Free text, optional.
    pub note: Option<String>,
    /// For [`DUPLICATE`], the claim it duplicates.
    pub duplicate_of: Option<i64>,
    /// The right policy of the patient to re-split the visit to; `None`
    /// leaves the coverage to resolve.
    pub policy: Option<i64>,
}

impl VoidRequest {
    /// A void with reason `code`.
    pub fn new(code: &str) -> Self {
        VoidRequest {
            code: code.trim().to_owned(),
            note: None,
            duplicate_of: None,
            policy: None,
        }
    }

    /// With this free text.
    pub fn note(mut self, note: &str) -> Self {
        self.note = clean(&Some(note.to_owned()));
        self
    }

    /// Duplicating claim `claim`.
    pub fn duplicate_of(mut self, claim: i64) -> Self {
        self.duplicate_of = Some(claim);
        self
    }

    /// Re-split to the patient's policy `policy`.
    pub fn policy(mut self, policy: i64) -> Self {
        self.policy = Some(policy);
        self
    }
}

/// What a void came to.
#[derive(Debug, Clone)]
pub struct Voided {
    /// The voided claim.
    pub claim: Claim,
    /// The claim the re-split made, when a right policy was named.
    pub replacement: Option<Claim>,
    /// Whether the visit is left without a payer split, flagged *coverage to
    /// resolve*.
    pub to_resolve: bool,
}

/// The source key of the task to resolve the coverage of visit `visit`.
pub fn coverage_task_key(visit: i64) -> String {
    format!("clinics.coverage_to_resolve:{visit}")
}

fn guard(key: &str) -> TransitionError {
    TransitionError::Guard(english(key, &[]))
}

/// The claims `claim` can be said to duplicate: the same patient's other
/// claims that are not void, oldest first.
///
/// # Errors
///
/// The database's errors.
pub async fn candidates(db: &Db, claim: &Claim) -> Result<Vec<Claim>> {
    Ok(Claim::query()
        .where_(Claim::PATIENT_PARTY_ID.eq(claim.patient_party_id))
        .order_by(Claim::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|other| other.id != claim.id && !other.is_void())
        .collect())
}

/// The policies of the patient of `claim` that are valid on the day of its
/// visit: the ones a visit can be re-split to.
///
/// # Errors
///
/// The database's errors.
pub async fn policies_for(db: &Db, claim: &Claim) -> Result<Vec<InsurancePolicy>> {
    Ok(InsurancePolicy::query()
        .where_(InsurancePolicy::PATIENT_PARTY_ID.eq(claim.patient_party_id))
        .order_by(InsurancePolicy::ID)
        .get(db)
        .await?
        .into_iter()
        .filter(|policy| policy.is_valid_on(claim.visit_date))
        .collect())
}

/// The text the payer bill and the Void record keep: the code, then the note.
fn reason_text(request: &VoidRequest) -> String {
    match &request.note {
        Some(note) => format!("{}: {note}", request.code),
        None => request.code.clone(),
    }
}

/// Prices the visit under policy `policy`, for a re-split: the policy is the
/// patient's and valid on the day of the visit, its payer contract gives a
/// payer share, and the patient's share is the one invoiced.
async fn plan(
    state: &AppState,
    visit: &Visit,
    policy: i64,
) -> std::result::Result<(InsurancePolicy, Quote), TransitionError> {
    let policy = InsurancePolicy::find_or_404(&state.db, policy).await?;
    if visit.patient_party_id != Some(policy.patient_party_id) {
        return Err(guard("clinics.claim.problem.resplit_other_patient"));
    }
    if !policy.is_valid_on(visit.visit_date) {
        return Err(guard("clinics.claim.problem.resplit_not_valid"));
    }
    let mut moved = visit.clone();
    moved.coverage_policy_id = Some(policy.id);
    let quote = charges::quote(state, &moved).await?;
    if quote.payer_total <= 0 {
        return Err(guard("clinics.claim.problem.resplit_no_share"));
    }
    if quote.patient_total != visit.patient_total {
        return Err(guard("clinics.claim.problem.resplit_patient_share"));
    }
    Ok((policy, quote))
}

/// Takes the payer split away from the visit: no payer bill, no payer share on
/// the visit and its lines.
async fn remove_split(state: &AppState, visit: &Visit) -> Result {
    ChargeLine::query()
        .where_(ChargeLine::VISIT_ID.eq(visit.id))
        .where_(ChargeLine::REMOVED_ON.is_null())
        .update(
            &state.db,
            &[("payer_share", &0_i64), ("updated_at", &now())],
        )
        .await?;
    let no_bill: Option<i64> = None;
    Visit::query()
        .where_(Visit::ID.eq(visit.id))
        .update(
            &state.db,
            &[
                ("payer_bill_id", &no_bill),
                ("payer_total", &0_i64),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(())
}

/// Divides the visit by `quote` under `policy`: the lines keep their parts,
/// the visit its payer total and policy of the split; a draft payer bill and
/// the claim around it are made, replacing `replaces`.
async fn apply_split(
    state: &AppState,
    visit_id: i64,
    policy: &InsurancePolicy,
    quote: &Quote,
    replaces: i64,
    by: Option<i64>,
) -> Result<Claim> {
    for line in &quote.lines {
        ChargeLine::query()
            .where_(ChargeLine::ID.eq(line.id))
            .update(
                &state.db,
                &[
                    ("self_pay_units", &line.self_pay_units),
                    ("authorised_units", &line.authorised_units),
                    ("patient_share", &line.patient_share),
                    ("payer_share", &line.payer_share),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    Visit::query()
        .where_(Visit::ID.eq(visit_id))
        .update(
            &state.db,
            &[
                ("coverage_policy_id", &Some(policy.id)),
                ("payer_total", &quote.payer_total),
                ("coverage_to_resolve", &false),
                ("updated_at", &now()),
            ],
        )
        .await?;
    let mut visit = Visit::find_or_404(&state.db, visit_id).await?;
    // A run that stopped after the claim was made finds it again.
    let made = match payer_side(state, &mut visit, quote, by).await? {
        Some(made) => Some(made),
        None => match visit.payer_bill_id {
            Some(bill) => Claim::of_payer_bill(&state.db, bill).await?,
            None => None,
        },
    }
    .ok_or_else(|| crate::db::refused(english("clinics.claim.problem.resplit_no_share", &[])))?;
    Claim::query()
        .where_(Claim::ID.eq(made.id))
        .update(
            &state.db,
            &[
                ("replaces_claim_id", &Some(replaces)),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Claim::find_or_404(&state.db, made.id).await
}

/// Voids claim `id` as `by` (who needs `claims.manage`) with `request`; see
/// the module for the order of the steps.
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `NotFrom` unless the claim is `draft`
/// or `ready`; `Guard` (422) for a reason code that is none of the three, a
/// `duplicate` without a claim of the same patient that is not void, a claim
/// with any stored submission attempt, a policy that is not the patient's, not
/// valid on the visit day, without a payer contract, without a payer share or
/// that would change the patient's share, and the payer bill's refusals.
pub async fn void(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    request: VoidRequest,
) -> std::result::Result<Voided, TransitionError> {
    let claim = Claim::find_or_404(&state.db, id).await?;
    if !REASON_CODES.contains(&request.code.as_str()) {
        return Err(guard("clinics.claim.problem.void_code"));
    }
    let duplicate = request.code == DUPLICATE;
    let duplicate_of = if duplicate {
        let Some(other) = request.duplicate_of else {
            return Err(guard("clinics.claim.problem.void_duplicate_needed"));
        };
        if !candidates(&state.db, &claim)
            .await?
            .iter()
            .any(|c| c.id == other)
        {
            return Err(guard("clinics.claim.problem.void_duplicate_other"));
        }
        Some(other)
    } else {
        None
    };
    // A void that stopped half way (the claim is void, nothing marks it
    // complete) is finished by the same call.
    let resuming = claim.is_void() && claim.void_code.is_none();
    if resuming {
        if !can_manage(by) {
            return Err(TransitionError::Forbidden);
        }
    } else {
        dry_run(&claim, voids::VOID, by, &ClaimInput::default()).await?;
    }
    let visit = Visit::find_or_404(&state.db, claim.visit_id).await?;
    let resplit = match (duplicate, request.policy) {
        (false, Some(policy)) => Some(plan(state, &visit, policy).await?),
        _ => None,
    };
    let reason = reason_text(&request);

    if !resuming {
        if void_needs_approval(state).await? {
            return Err(guard("clinics.claim.problem.void_waits"));
        }
        let bill = payer_bills(state)
            .find(&state.db, claim.payer_bill())
            .await?;
        if bill.is_none_or(|bill| bill.status != PayerBillStatus::Voided) {
            match payer_bills(state)
                .void(state, claim.payer_bill(), &reason, by)
                .await?
            {
                PayerAnswer::Done => {}
                PayerAnswer::Pending { .. } => {
                    return Err(guard("clinics.claim.problem.void_waits"));
                }
            }
        }
        match voids::void(state, by, &claim.subject(), &reason).await? {
            VoidOutcome::Voided(_) => {}
            VoidOutcome::Pending(_) => return Err(guard("clinics.claim.problem.void_waits")),
        }
    }

    // The split is taken away once: a visit that no longer holds this
    // claim's payer bill has been through it.
    if visit.payer_bill_id == Some(claim.payer_bill_id) {
        remove_split(state, &visit).await?;
    }
    let mut replacement = Claim::query()
        .where_(Claim::REPLACES_CLAIM_ID.eq(id))
        .first(&state.db)
        .await?;
    if replacement.is_none()
        && let Some((policy, quote)) = &resplit
    {
        replacement = Some(apply_split(state, visit.id, policy, quote, id, Some(by.id)).await?);
    }
    let to_resolve = !duplicate && resplit.is_none();
    if to_resolve {
        Visit::query()
            .where_(Visit::ID.eq(visit.id))
            .update(
                &state.db,
                &[("coverage_to_resolve", &true), ("updated_at", &now())],
            )
            .await?;
        let number = claim.number();
        let visit_number = visit.number();
        let title = english(
            "clinics.claim.task.coverage_title",
            &[("visit", &visit_number)],
        );
        let description = english(
            "clinics.claim.task.coverage_description",
            &[
                ("visit", &visit_number),
                ("number", &number),
                ("reason", &reason),
            ],
        );
        let task = RoleTask::new(
            visit.subject(),
            BILLING_COORDINATOR_ROLE,
            &title,
            &coverage_task_key(visit.id),
        )
        .description(&description);
        role_tasks::open(state, task).await?;
    }
    // Last: the mark that the void is complete.
    Claim::query()
        .where_(Claim::ID.eq(id))
        .update(
            &state.db,
            &[
                ("void_code", &Some(request.code.clone())),
                ("void_note", &request.note),
                ("void_duplicate_of", &duplicate_of),
                ("updated_at", &now()),
            ],
        )
        .await?;
    Ok(Voided {
        claim: Claim::find_or_404(&state.db, id).await?,
        replacement,
        to_resolve,
    })
}

/// Whether an admin put an approval policy on the void of a claim or of a
/// payer bill: the void would then wait half way, so it is refused up front.
async fn void_needs_approval(state: &AppState) -> Result<bool> {
    let input = json!({ voids::POSTED_KEY: 0 });
    for subject_type in [CLAIM_SUBJECT_TYPE, PAYER_BILL_SUBJECT_TYPE] {
        let action = voids::approval::action_key(subject_type);
        if ApprovalPolicy::for_action(&state.db, &action, &input)
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Resolves the coverage of the visit of the voided claim `id`: the visit is
/// split again under the patient's policy `policy` and a new claim is made,
/// which replaces the voided one; the task goes.
///
/// # Errors
///
/// `Forbidden` without `claims.manage`; `Guard` (422) unless the claim is
/// void and its visit flagged *coverage to resolve*, and for a policy that
/// cannot be used (see [`void`]).
pub async fn resolve_coverage(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    policy: i64,
) -> std::result::Result<Claim, TransitionError> {
    if !can_manage(by) {
        return Err(TransitionError::Forbidden);
    }
    let claim = Claim::find_or_404(&state.db, id).await?;
    let visit = Visit::find_or_404(&state.db, claim.visit_id).await?;
    if !claim.is_void() || !visit.coverage_to_resolve {
        return Err(guard("clinics.claim.problem.resolve_nothing"));
    }
    let (policy, quote) = plan(state, &visit, policy).await?;
    let made = apply_split(state, visit.id, &policy, &quote, id, Some(by.id)).await?;
    let key = coverage_task_key(visit.id);
    if let Some(task) = task_creator(state).find_open(state, &key).await? {
        let note = english("clinics.claim.task.coverage_resolved", &[]);
        task_creator(state)
            .resolve_from_source(state, task.id, &note)
            .await?;
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_void_names_its_reason_and_note() {
        let request = VoidRequest::new(" wrong_payer ").note("  billed to Daman ");
        assert_eq!(request.code, "wrong_payer");
        assert_eq!(reason_text(&request), "wrong_payer: billed to Daman");
        assert_eq!(reason_text(&VoidRequest::new("duplicate")), "duplicate");
        assert!(REASON_CODES.contains(&request.code.as_str()));
        let duplicate = VoidRequest::new("duplicate").duplicate_of(7).policy(3);
        assert_eq!(
            (duplicate.duplicate_of, duplicate.policy),
            (Some(7), Some(3))
        );
    }

    #[test]
    fn the_coverage_task_is_found_by_its_visit() {
        assert_eq!(coverage_task_key(12), "clinics.coverage_to_resolve:12");
    }
}
