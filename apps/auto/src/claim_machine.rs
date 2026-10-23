//! The insurance repair claim machine (08 §7.1 *Insurance repair claim*;
//! `AUT.INS.01` .. `AUT.INS.04`, `AUT.ACL.15` .. `AUT.ACL.17`, `AUT.ACL.20`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`BOOK_SURVEY`] | registered | awaiting_survey | `insurance.manage` | `claim_details`, `police_report` (slot) |
//! | [`SUBMIT_ESTIMATE`] | registered, awaiting_survey | estimate_submitted | `insurance.manage` | `claim_details`, `estimate_priced`, `police_report` (slot) |
//! | [`APPROVE`] | estimate_submitted | approved | `insurance.manage` | `approval_reference`, `approval_document`, `approved_per_line`, `police_report` (slot) |
//! | [`REJECT`] | estimate_submitted | rejected | `insurance.manage` | a reason |
//! | [`REQUEST_SUPPLEMENTARY`] | approved | supplementary_pending | `job_cards.advise` | `findings_recorded`, `supplementary_priced` |
//! | [`APPROVE_SUPPLEMENTARY`] | supplementary_pending | approved | `insurance.manage` | `approval_reference`, `approval_document`, `approved_per_line` |
//! | [`DECLINE_SUPPLEMENTARY`] | supplementary_pending | approved | `insurance.manage` | a reason |
//! | [`COMPLETE`] | approved | repair_complete | system (job card `pass_qc`) | — |
//! | [`INVOICE`] | repair_complete, total_loss | invoiced | `invoices.issue` | `charges_approved` (from `total_loss`) |
//! | [`SETTLE`] | invoiced | settled | system (every invoice paid) | — |
//! | [`DECLARE_TOTAL_LOSS`] | awaiting_survey .. supplementary_pending | total_loss | `insurance.manage` | `total_loss_notice` (slot) |
//! | [`CANCEL`] | registered .. supplementary_pending | cancelled | `insurance.manage` | a reason; `written_withdrawal` (slot) |
//! | [`VOID`] | registered | voided | `insurance.manage` | a reason; `nothing_sent` (slot) |
//!
//! Decisions:
//!
//! - As for the job card, the guards see the claim and the run's input; the
//!   services ([`crate::claims`]) work out what a guard needs from the
//!   database (the estimate is priced, the lines the insurer approved add up)
//!   and pass it in [`ClaimInput`].
//! - The guards marked *slot* pass for now: they are where the issues of the
//!   negative flows add their rules (BL-AUT-07 the police report, BL-AUT-08
//!   the written withdrawal of `cancel`, BL-AUT-09 the total-loss notice,
//!   BL-AUT-10 *nothing sent* of `void`). A slot keeps its name so a test
//!   names the guard that refused.
//! - Effects only touch the claim's own rows (the approval rows); the job
//!   card, the payer bill and the invoices are the service's, after the
//!   commit.
//! - `cancel`, `declare_total_loss` and `void` end the claim; what happens to
//!   the job card (decline, cancel, unlink) is the issues' to add in the
//!   service.

use renox::chrono::NaiveDate;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::claim::{ClaimStatus, InsuranceClaim};
use crate::claim_approval::{
    ApprovalKind, ApprovalOutcome, ApprovedLine, ClaimApproval, LINE_KINDS,
};
use crate::db::set_claim;
use crate::permissions;
use crate::texts::english;

/// `registered → awaiting_survey`: the surveyor is asked to inspect.
pub const BOOK_SURVEY: &str = "book_survey";
/// `registered, awaiting_survey → estimate_submitted`: the estimate goes to
/// the insurer.
pub const SUBMIT_ESTIMATE: &str = "submit_estimate";
/// `estimate_submitted → approved`: the insurer's approval or LPO.
pub const APPROVE: &str = "approve";
/// `estimate_submitted → rejected`: the insurer declines the claim.
pub const REJECT: &str = "reject";
/// `approved → supplementary_pending`: hidden damage goes back to the insurer.
pub const REQUEST_SUPPLEMENTARY: &str = "request_supplementary";
/// `supplementary_pending → approved`: the insurer approves the supplementary.
pub const APPROVE_SUPPLEMENTARY: &str = "approve_supplementary";
/// `supplementary_pending → approved`: the insurer declines the
/// supplementary.
pub const DECLINE_SUPPLEMENTARY: &str = "decline_supplementary";
/// `approved → repair_complete`: the job card passed QC.
pub const COMPLETE: &str = "complete";
/// `repair_complete, total_loss → invoiced`: the split billing.
pub const INVOICE: &str = "invoice";
/// `invoiced → settled`: every invoice of the claim is paid.
pub const SETTLE: &str = "settle";
/// `awaiting_survey .. supplementary_pending → total_loss`: the insurer
/// declared a total loss.
pub const DECLARE_TOTAL_LOSS: &str = "declare_total_loss";
/// `registered .. supplementary_pending → cancelled`: the customer withdrew
/// the claim.
pub const CANCEL: &str = "cancel";
/// `registered → voided`: registered in error.
pub const VOID: &str = "void";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimInput {
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// The day of the run (Asia/Dubai).
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The estimate has lines and every one is priced
    /// ([`SUBMIT_ESTIMATE`], [`REQUEST_SUPPLEMENTARY`]).
    #[serde(default)]
    pub priced: Option<bool>,
    /// The net total asked of the insurer, in fils ([`SUBMIT_ESTIMATE`],
    /// [`REQUEST_SUPPLEMENTARY`]).
    #[serde(default)]
    pub estimate_amount: Option<i64>,
    /// The estimate round of the job card the request is about.
    #[serde(default)]
    pub job_round_id: Option<i64>,
    /// The insurer's approval reference or LPO number.
    #[serde(default)]
    pub reference: Option<String>,
    /// The insurer's written approval, notice or the customer's withdrawal (a
    /// document reference).
    #[serde(default)]
    pub document: Option<String>,
    /// The approved amount per line ([`APPROVE`], [`APPROVE_SUPPLEMENTARY`]).
    #[serde(default)]
    pub lines: Vec<ApprovedLine>,
    /// What the technician found ([`REQUEST_SUPPLEMENTARY`]).
    #[serde(default)]
    pub findings: Option<String>,
    /// The insurer approved the charges of a total loss ([`INVOICE`]).
    #[serde(default)]
    pub charges_approved: Option<bool>,
    /// The excess the customer pays, set when the claim is approved.
    #[serde(default)]
    pub excess: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> ClaimInput {
    ctx.input_as::<ClaimInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// Whether `text` holds something.
fn given(text: &Option<String>) -> bool {
    text.as_deref().is_some_and(|t| !t.trim().is_empty())
}

/// A guard slot: it names a rule an issue adds and passes for now.
fn slot(transition: Transition<InsuranceClaim>, name: &'static str) -> Transition<InsuranceClaim> {
    transition.guard(name, |_c: &InsuranceClaim, _ctx| {
        Box::pin(async { Ok::<(), String>(()) })
    })
}

/// The lines of an approval are lines of the job card (or charges), each
/// with an amount above zero.
pub fn lines_are_valid(lines: &[ApprovedLine]) -> bool {
    !lines.is_empty()
        && lines
            .iter()
            .all(|line| line.amount > 0 && LINE_KINDS.contains(&line.kind.as_str()))
}

impl StateMachine for InsuranceClaim {
    type State = ClaimStatus;
    const COLUMN: renox::db::Col<InsuranceClaim, ClaimStatus> = InsuranceClaim::STATUS;

    fn machine() -> Machine<Self> {
        use ClaimStatus::{
            Approved, AwaitingSurvey, Cancelled, EstimateSubmitted, Invoiced, Registered, Rejected,
            RepairComplete, Settled, SupplementaryPending, TotalLoss, Voided,
        };
        Machine::new()
            .initial([Registered])
            .state(Registered)
            .tone("neutral")
            .state(AwaitingSurvey)
            .tone("info")
            .state(EstimateSubmitted)
            .tone("info")
            .state(Approved)
            .tone("success")
            .state(SupplementaryPending)
            .tone("warning")
            .state(RepairComplete)
            .tone("success")
            .state(TotalLoss)
            .tone("warning")
            .state(Invoiced)
            .tone("info")
            .state(Settled)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("error")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .state(Voided)
            .tone("neutral")
            .terminal()
            .transition(book_survey())
            .transition(submit_estimate())
            .transition(approve())
            .transition(reject())
            .transition(request_supplementary())
            .transition(approve_supplementary())
            .transition(decline_supplementary())
            .transition(complete())
            .transition(invoice())
            .transition(settle())
            .transition(declare_total_loss())
            .transition(cancel())
            .transition(void())
    }
}

/// The policy and claim numbers are recorded (`AUT.INS.01`).
fn with_claim_details(transition: Transition<InsuranceClaim>) -> Transition<InsuranceClaim> {
    transition.guard("claim_details", |c: &InsuranceClaim, _ctx| {
        let ok = !c.policy_no.trim().is_empty() && !c.claim_no.trim().is_empty();
        Box::pin(async move { check(ok, "auto.claims.problem.details") })
    })
}

/// The insurer's answer carries a reference, its written approval and the
/// approved amount per line (`AUT.ACL.16`).
fn with_approval(transition: Transition<InsuranceClaim>) -> Transition<InsuranceClaim> {
    transition
        .guard("approval_reference", |_c: &InsuranceClaim, ctx| {
            let ok = given(&input_of(ctx).reference);
            Box::pin(async move { check(ok, "auto.claims.problem.reference") })
        })
        .guard("approval_document", |_c: &InsuranceClaim, ctx| {
            let ok = given(&input_of(ctx).document);
            Box::pin(async move { check(ok, "auto.claims.problem.document") })
        })
        .guard("approved_per_line", |_c: &InsuranceClaim, ctx| {
            let ok = lines_are_valid(&input_of(ctx).lines);
            Box::pin(async move { check(ok, "auto.claims.problem.lines") })
        })
}

/// `book_survey`: the surveyor is asked to inspect on site or from photos.
fn book_survey() -> Transition<InsuranceClaim> {
    let transition = Transition::new(BOOK_SURVEY)
        .label("Book the survey")
        .tone("primary")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::Registered])
        .to(ClaimStatus::AwaitingSurvey);
    let transition = with_claim_details(transition);
    slot(transition, "police_report").effect(|c: &InsuranceClaim, _ctx, tx| {
        let claim = c.clone();
        Box::pin(async move {
            let stamp = now();
            let first = claim.sent_to_insurer_at.unwrap_or(stamp);
            set_claim(
                tx,
                claim.id,
                &[
                    ("survey_booked_at", &Some(stamp)),
                    ("sent_to_insurer_at", &Some(first)),
                ],
            )
            .await
        })
    })
}

/// `submit_estimate`: the estimate goes to the insurer as a request waiting
/// for its answer.
fn submit_estimate() -> Transition<InsuranceClaim> {
    let transition = Transition::new(SUBMIT_ESTIMATE)
        .label("Submit the estimate")
        .tone("primary")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::Registered, ClaimStatus::AwaitingSurvey])
        .to(ClaimStatus::EstimateSubmitted);
    let transition =
        with_claim_details(transition).guard("estimate_priced", |_c: &InsuranceClaim, ctx| {
            let ok = input_of(ctx).priced == Some(true);
            Box::pin(async move { check(ok, "auto.claims.problem.not_priced") })
        });
    slot(transition, "police_report").effect(|c: &InsuranceClaim, ctx, tx| {
        let claim = c.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            let stamp = now();
            let mut request = ClaimApproval {
                claim_id: claim.id,
                kind: ApprovalKind::Estimate,
                round_no: 1,
                job_round_id: input.job_round_id,
                estimate_amount: input.estimate_amount.unwrap_or(0),
                outcome: ApprovalOutcome::Pending,
                requested_by: input.by,
                requested_at: Some(stamp),
                ..ClaimApproval::default()
            };
            request.insert(&mut *tx).await?;
            let first = claim.sent_to_insurer_at.unwrap_or(stamp);
            set_claim(
                tx,
                claim.id,
                &[
                    ("estimate_amount", &input.estimate_amount.unwrap_or(0)),
                    ("submitted_at", &Some(stamp)),
                    ("sent_to_insurer_at", &Some(first)),
                ],
            )
            .await
        })
    })
}

/// Records the insurer's answer on the request of `kind` waiting for it.
async fn answer(
    tx: &mut renox::db::Transaction,
    claim_id: i64,
    kind: ApprovalKind,
    outcome: ApprovalOutcome,
    input: &ClaimInput,
    reason: Option<String>,
) -> Result {
    let pending = ClaimApproval::query()
        .where_(ClaimApproval::CLAIM_ID.eq(claim_id))
        .where_(ClaimApproval::KIND.eq(kind))
        .where_(ClaimApproval::OUTCOME.eq(ApprovalOutcome::Pending))
        .order_by_desc(ClaimApproval::ID)
        .first(&mut *tx)
        .await?;
    let Some(pending) = pending else {
        return Ok(());
    };
    let stamp = now();
    let approved = if outcome == ApprovalOutcome::Approved {
        ClaimApproval::sum(&input.lines)
    } else {
        0
    };
    let lines = renox::db::Json(if outcome == ApprovalOutcome::Approved {
        input.lines.clone()
    } else {
        Vec::new()
    });
    ClaimApproval::query()
        .where_(ClaimApproval::ID.eq(pending.id))
        .update(
            &mut *tx,
            &[
                ("outcome", &outcome),
                ("reference", &input.reference),
                ("approved_amount", &approved),
                ("lines", &lines),
                ("document", &input.document),
                ("reason", &reason),
                ("decided_by", &input.by),
                ("decided_at", &Some(stamp)),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    Ok(())
}

/// `approve`: the insurer approved the estimate; the amount approved per line
/// and its written approval are recorded (`AUT.ACL.16`).
fn approve() -> Transition<InsuranceClaim> {
    let transition = Transition::new(APPROVE)
        .label("Record the approval")
        .tone("primary")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::EstimateSubmitted])
        .to(ClaimStatus::Approved);
    let transition = with_approval(transition);
    slot(transition, "police_report").effect(|c: &InsuranceClaim, ctx, tx| {
        let claim = c.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            answer(
                tx,
                claim.id,
                ApprovalKind::Estimate,
                ApprovalOutcome::Approved,
                &input,
                None,
            )
            .await?;
            let approved = ClaimApproval::sum(&input.lines);
            let excess = input.excess.unwrap_or(claim.excess);
            set_claim(
                tx,
                claim.id,
                &[
                    ("approved_amount", &approved),
                    ("excess", &excess),
                    ("approved_at", &Some(now())),
                ],
            )
            .await
        })
    })
}

/// `reject`: the insurer declined the claim; the customer is offered a
/// private estimate on the same job card.
fn reject() -> Transition<InsuranceClaim> {
    Transition::new(REJECT)
        .label("Record the rejection")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::EstimateSubmitted])
        .to(ClaimStatus::Rejected)
        .reason_required()
        .effect(|c: &InsuranceClaim, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                answer(
                    tx,
                    id,
                    ApprovalKind::Estimate,
                    ApprovalOutcome::Declined,
                    &input,
                    reason.clone(),
                )
                .await?;
                set_claim(
                    tx,
                    id,
                    &[("closed_reason", &reason), ("closed_at", &Some(now()))],
                )
                .await
            })
        })
}

/// `request_supplementary`: hidden damage found during strip-down goes back to
/// the insurer, with the findings; the affected lines stay on hold on the job
/// card until the answer.
fn request_supplementary() -> Transition<InsuranceClaim> {
    Transition::new(REQUEST_SUPPLEMENTARY)
        .label("Request a supplementary")
        .tone("primary")
        .permission(permissions::JOB_CARDS_ADVISE)
        .from([ClaimStatus::Approved])
        .to(ClaimStatus::SupplementaryPending)
        .guard("findings_recorded", |_c: &InsuranceClaim, ctx| {
            let ok = given(&input_of(ctx).findings);
            Box::pin(async move { check(ok, "auto.claims.problem.findings") })
        })
        .guard("supplementary_priced", |_c: &InsuranceClaim, ctx| {
            let input = input_of(ctx);
            let ok = input.priced == Some(true) && input.estimate_amount.unwrap_or(0) > 0;
            Box::pin(async move { check(ok, "auto.claims.problem.not_priced") })
        })
        .effect(|c: &InsuranceClaim, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            Box::pin(async move {
                let before = ClaimApproval::query()
                    .where_(ClaimApproval::CLAIM_ID.eq(id))
                    .count(&mut *tx)
                    .await?;
                let stamp = now();
                let mut request = ClaimApproval {
                    claim_id: id,
                    kind: ApprovalKind::Supplementary,
                    round_no: i64::try_from(before).unwrap_or(0) + 1,
                    job_round_id: input.job_round_id,
                    estimate_amount: input.estimate_amount.unwrap_or(0),
                    findings: input.findings.clone(),
                    outcome: ApprovalOutcome::Pending,
                    requested_by: input.by,
                    requested_at: Some(stamp),
                    ..ClaimApproval::default()
                };
                request.insert(&mut *tx).await?;
                set_claim(tx, id, &[("supplementary_at", &Some(stamp))]).await
            })
        })
}

/// `approve_supplementary`: the insurer approved the hidden damage; the
/// approved amount goes up and the job card lines are released.
fn approve_supplementary() -> Transition<InsuranceClaim> {
    let transition = Transition::new(APPROVE_SUPPLEMENTARY)
        .label("Record the supplementary approval")
        .tone("primary")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::SupplementaryPending])
        .to(ClaimStatus::Approved);
    with_approval(transition).effect(|c: &InsuranceClaim, ctx, tx| {
        let claim = c.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            answer(
                tx,
                claim.id,
                ApprovalKind::Supplementary,
                ApprovalOutcome::Approved,
                &input,
                None,
            )
            .await?;
            let approved = claim.approved_amount + ClaimApproval::sum(&input.lines);
            let none: Option<renox::db::DateTime> = None;
            set_claim(
                tx,
                claim.id,
                &[("approved_amount", &approved), ("supplementary_at", &none)],
            )
            .await
        })
    })
}

/// `decline_supplementary`: the insurer declined the hidden damage; the lines
/// are removed or offered to the customer as private items (`AUT.ACL.15`).
fn decline_supplementary() -> Transition<InsuranceClaim> {
    Transition::new(DECLINE_SUPPLEMENTARY)
        .label("Record the supplementary decline")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::SupplementaryPending])
        .to(ClaimStatus::Approved)
        .reason_required()
        .effect(|c: &InsuranceClaim, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                answer(
                    tx,
                    id,
                    ApprovalKind::Supplementary,
                    ApprovalOutcome::Declined,
                    &input,
                    reason,
                )
                .await?;
                let none: Option<renox::db::DateTime> = None;
                set_claim(tx, id, &[("supplementary_at", &none)]).await
            })
        })
}

/// `complete`: the job card passed QC; the claim is ready for billing.
fn complete() -> Transition<InsuranceClaim> {
    Transition::new(COMPLETE)
        .label("Repair complete")
        .system()
        .from([ClaimStatus::Approved])
        .to(ClaimStatus::RepairComplete)
        .effect(|c: &InsuranceClaim, _ctx, tx| {
            let id = c.id;
            Box::pin(async move { set_claim(tx, id, &[("completed_at", &Some(now()))]).await })
        })
}

/// `invoice`: the split billing; from `total_loss` the insurer's approval of
/// the charges comes first. The payer bill and the customer invoice are made
/// by the service before the run and kept on the claim.
fn invoice() -> Transition<InsuranceClaim> {
    Transition::new(INVOICE)
        .label("Invoice the insurer and the customer")
        .tone("primary")
        .permission(permissions::INVOICES_ISSUE)
        .from([ClaimStatus::RepairComplete, ClaimStatus::TotalLoss])
        .to(ClaimStatus::Invoiced)
        .guard("charges_approved", |c: &InsuranceClaim, ctx| {
            let ok =
                c.status != ClaimStatus::TotalLoss || input_of(ctx).charges_approved == Some(true);
            Box::pin(async move { check(ok, "auto.claims.problem.charges") })
        })
        .effect(|c: &InsuranceClaim, _ctx, tx| {
            let id = c.id;
            Box::pin(async move { set_claim(tx, id, &[("invoiced_at", &Some(now()))]).await })
        })
}

/// `settle`: the insurer and the customer have paid.
fn settle() -> Transition<InsuranceClaim> {
    Transition::new(SETTLE)
        .label("Settle")
        .system()
        .from([ClaimStatus::Invoiced])
        .to(ClaimStatus::Settled)
        .effect(|c: &InsuranceClaim, _ctx, tx| {
            let id = c.id;
            Box::pin(async move { set_claim(tx, id, &[("settled_at", &Some(now()))]).await })
        })
}

/// `declare_total_loss`: the insurer's notice stops the work.
fn declare_total_loss() -> Transition<InsuranceClaim> {
    let transition = Transition::new(DECLARE_TOTAL_LOSS)
        .label("Declare a total loss")
        .permission(permissions::INSURANCE_MANAGE)
        .from([
            ClaimStatus::AwaitingSurvey,
            ClaimStatus::EstimateSubmitted,
            ClaimStatus::Approved,
            ClaimStatus::SupplementaryPending,
        ])
        .to(ClaimStatus::TotalLoss);
    slot(transition, "total_loss_notice").effect(|c: &InsuranceClaim, ctx, tx| {
        let id = c.id;
        let notice = input_of(ctx).document;
        Box::pin(async move {
            set_claim(
                tx,
                id,
                &[
                    ("total_loss_at", &Some(now())),
                    ("total_loss_notice", &notice),
                ],
            )
            .await
        })
    })
}

/// `cancel`: the customer withdrew the claim.
fn cancel() -> Transition<InsuranceClaim> {
    let transition = Transition::new(CANCEL)
        .label("Withdraw the claim")
        .permission(permissions::INSURANCE_MANAGE)
        .from([
            ClaimStatus::Registered,
            ClaimStatus::AwaitingSurvey,
            ClaimStatus::EstimateSubmitted,
            ClaimStatus::Approved,
            ClaimStatus::SupplementaryPending,
        ])
        .to(ClaimStatus::Cancelled)
        .reason_required();
    slot(transition, "written_withdrawal").effect(|c: &InsuranceClaim, ctx, tx| {
        let id = c.id;
        let reason = ctx.reason().map(str::to_owned);
        let document = input_of(ctx).document;
        Box::pin(async move {
            set_claim(
                tx,
                id,
                &[
                    ("closed_reason", &reason),
                    ("closed_at", &Some(now())),
                    ("withdrawal_document", &document),
                ],
            )
            .await
        })
    })
}

/// `void`: registered in error, before anything was sent to the insurer.
fn void() -> Transition<InsuranceClaim> {
    let transition = Transition::new(VOID)
        .label("Void")
        .permission(permissions::INSURANCE_MANAGE)
        .from([ClaimStatus::Registered])
        .to(ClaimStatus::Voided)
        .reason_required();
    slot(transition, "nothing_sent").effect(|c: &InsuranceClaim, ctx, tx| {
        let id = c.id;
        let reason = ctx.reason().map(str::to_owned);
        Box::pin(async move {
            set_claim(
                tx,
                id,
                &[("closed_reason", &reason), ("closed_at", &Some(now()))],
            )
            .await
        })
    })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<InsuranceClaim>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = InsuranceClaim::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("registered", Some("neutral"), false),
                ("awaiting_survey", Some("info"), false),
                ("estimate_submitted", Some("info"), false),
                ("approved", Some("success"), false),
                ("supplementary_pending", Some("warning"), false),
                ("repair_complete", Some("success"), false),
                ("total_loss", Some("warning"), false),
                ("invoiced", Some("info"), false),
                ("settled", Some("success"), true),
                ("rejected", Some("error"), true),
                ("cancelled", Some("neutral"), true),
                ("voided", Some("neutral"), true),
            ]
        );
        let open = vec![
            "registered",
            "awaiting_survey",
            "estimate_submitted",
            "approved",
            "supplementary_pending",
        ];
        let expected: Vec<(&str, Vec<&str>, &str)> = vec![
            (BOOK_SURVEY, vec!["registered"], "awaiting_survey"),
            (
                SUBMIT_ESTIMATE,
                vec!["registered", "awaiting_survey"],
                "estimate_submitted",
            ),
            (APPROVE, vec!["estimate_submitted"], "approved"),
            (REJECT, vec!["estimate_submitted"], "rejected"),
            (
                REQUEST_SUPPLEMENTARY,
                vec!["approved"],
                "supplementary_pending",
            ),
            (
                APPROVE_SUPPLEMENTARY,
                vec!["supplementary_pending"],
                "approved",
            ),
            (
                DECLINE_SUPPLEMENTARY,
                vec!["supplementary_pending"],
                "approved",
            ),
            (COMPLETE, vec!["approved"], "repair_complete"),
            (INVOICE, vec!["repair_complete", "total_loss"], "invoiced"),
            (SETTLE, vec!["invoiced"], "settled"),
            (
                DECLARE_TOTAL_LOSS,
                vec![
                    "awaiting_survey",
                    "estimate_submitted",
                    "approved",
                    "supplementary_pending",
                ],
                "total_loss",
            ),
            (CANCEL, open, "cancelled"),
            (VOID, vec!["registered"], "voided"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(
                transition.is_system(),
                [COMPLETE, SETTLE].contains(&name),
                "{name}"
            );
        }
    }

    #[test]
    fn the_guards_and_the_guard_slots_are_declared() {
        let machine = InsuranceClaim::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards(BOOK_SURVEY), ["claim_details", "police_report"]);
        assert_eq!(
            guards(SUBMIT_ESTIMATE),
            ["claim_details", "estimate_priced", "police_report"]
        );
        assert_eq!(
            guards(APPROVE),
            [
                "approval_reference",
                "approval_document",
                "approved_per_line",
                "police_report"
            ]
        );
        assert_eq!(
            guards(APPROVE_SUPPLEMENTARY),
            [
                "approval_reference",
                "approval_document",
                "approved_per_line"
            ]
        );
        assert_eq!(
            guards(REQUEST_SUPPLEMENTARY),
            ["findings_recorded", "supplementary_priced"]
        );
        assert_eq!(guards(INVOICE), ["charges_approved"]);
        assert_eq!(guards(DECLARE_TOTAL_LOSS), ["total_loss_notice"]);
        assert_eq!(guards(CANCEL), ["written_withdrawal"]);
        assert_eq!(guards(VOID), ["nothing_sent"]);
        for name in [REJECT, DECLINE_SUPPLEMENTARY, CANCEL, VOID] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
    }

    #[test]
    fn approved_lines_have_a_kind_and_an_amount_above_zero() {
        let good = ApprovedLine {
            kind: "labour".to_owned(),
            line_id: 1,
            description: "Door".to_owned(),
            amount: 100,
        };
        assert!(lines_are_valid(std::slice::from_ref(&good)));
        assert!(!lines_are_valid(&[]));
        let zero = ApprovedLine {
            amount: 0,
            ..good.clone()
        };
        assert!(!lines_are_valid(&[good.clone(), zero]));
        let odd = ApprovedLine {
            kind: "tyre".to_owned(),
            ..good
        };
        assert!(!lines_are_valid(&[odd]));
    }
}
