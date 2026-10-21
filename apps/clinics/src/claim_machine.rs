//! The claim machine (05 §7.1 *Claim*; `CLN.CLM.01`, `CLN.ACL.09`; base for
//! BL-CLN-07 to BL-CLN-11).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`VALIDATE`] | draft | ready | `claims.manage` | codes present; prior authorisation linked where required; within the window of the attempt, or a late reason where the payer accepts late claims |
//! | [`SUBMIT`] | ready | submitted | `claims.manage` | — |
//! | [`REJECT_SUBMISSION`] | submitted, resubmitted | draft | `claims.manage` | the channel's errors given; no remittance recorded |
//! | [`RESUBMIT`] | partially_paid, denied | resubmitted | `claims.manage` | corrections recorded; within the resubmission window and limit |
//! | [`RECORD_REMITTANCE_PAID`] / [`RECORD_REMITTANCE_PARTIALLY_PAID`] / [`RECORD_REMITTANCE_DENIED`] | submitted, resubmitted | paid / partially_paid / denied | system | the payer bill recorded the advice |
//! | [`RECORD_RECOUPMENT_PARTIALLY_PAID`] / [`RECORD_RECOUPMENT_DENIED`] | paid, partially_paid, written_off | partially_paid / denied | system | recouped amount not above the paid amount; the payer's code |
//! | [`WRITE_OFF`] | draft, ready, partially_paid, denied | written_off | `claims.writeoff` | reason given; from draft or ready only after the first-submission window passed and the payer does not accept late claims (`CLN.ACL.16`); something open |
//! | [`WRITE_OFF_APPROVED`] | draft, ready, partially_paid, denied | written_off | system | reason given; something open (the owner approved the payer bill's write-off above the limit, `FIN.ACL.14`) |
//!
//! | `void` / `void_approved` | draft, ready | voided | `claims.manage` | reason given; never after a submission attempt (`CLN.ACL.14`, `PLT.ACL.09`) |
//!
//! `write_off` is BL-CLN-09's and closes the claim the way the payer bill
//! closes: it is written off, or, where the payer contract allows, moved to the
//! patient. `void` is BL-CLN-10's and the kernel's pattern (BL-KRN-03): the
//! transitions come from `wl_kernel::voids::transitions`, the claim adds the
//! guard that a claim with a stored submission attempt is never voided, and
//! it has an approval action of its own ([`VOID_ACTION`]).
//!
//! Decisions:
//!
//! - The spec's `record_remittance` ends in `paid`, `partially_paid` or
//!   `denied`, and `record_recoupment` in `partially_paid` or `denied`; a
//!   transition has one target, so each is several transitions with the
//!   state in the name (as the payer bill of wl-invoicing does).
//! - The remittance and the recoupment transitions are **system**
//!   transitions: the money is recorded on the payer bill by a user holding
//!   `claims.manage` (the service checks it) and, when it waits for a second
//!   person's approval, the claim moves later, when the approval is given.
//!   Neither the user nor the approval can run a claim transition, so the
//!   claim follows the payer bill ([`crate::claim_money`]).
//! - The guards are pure functions of the claim and the run's
//!   [`ClaimInput`], so the service asks them before it calls the payer bill
//!   (a refusal must not leave the payer bill issued while the claim is
//!   not).

use renox::chrono::NaiveDate;
use renox::db::{Json, ToDbValue, Transaction, now};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::dubai_date;
use wl_kernel::machines::SubjectMachine;
use wl_kernel::voids::{self, Voidable};

use crate::claim::{
    AnswerKind, AttemptKind, CLAIM_SUBJECT_TYPE, Claim, ClaimAttempt, ClaimError, ClaimLine,
    ClaimRemittance, ClaimStatus,
};
use crate::permissions;
use crate::texts::english;

/// `draft → ready`: the claim is checked.
pub const VALIDATE: &str = "validate";
/// `ready → submitted`: sent to the payer.
pub const SUBMIT: &str = "submit";
/// `submitted | resubmitted → draft`: the channel refused it before
/// adjudication.
pub const REJECT_SUBMISSION: &str = "reject_submission";
/// `partially_paid | denied → resubmitted`: corrected and sent again.
pub const RESUBMIT: &str = "resubmit";
/// `submitted | resubmitted → paid`: every line paid as claimed.
pub const RECORD_REMITTANCE_PAID: &str = "record_remittance_paid";
/// `submitted | resubmitted → partially_paid`.
pub const RECORD_REMITTANCE_PARTIALLY_PAID: &str = "record_remittance_partially_paid";
/// `submitted | resubmitted → denied`: nothing paid on any line.
pub const RECORD_REMITTANCE_DENIED: &str = "record_remittance_denied";
/// `paid | partially_paid | written_off → partially_paid`.
pub const RECORD_RECOUPMENT_PARTIALLY_PAID: &str = "record_recoupment_partially_paid";
/// `paid | partially_paid | written_off → denied`: nothing is paid on any
/// line after the recoupment.
pub const RECORD_RECOUPMENT_DENIED: &str = "record_recoupment_denied";

/// `draft | ready | partially_paid | denied → written_off`: the open amount
/// is written off, or moved to the patient where the payer contract allows
/// it (`CLN.ACL.16`).
pub const WRITE_OFF: &str = "write_off";
/// `draft | ready | partially_paid | denied → written_off`: the system's twin
/// of [`WRITE_OFF`], run when the owner approved the write-off the payer bill
/// held above the limit (`FIN.ACL.14`).
pub const WRITE_OFF_APPROVED: &str = "write_off_approved";

/// The approval policies' action key of the `void` of a claim. Decision: the
/// claim has its own key, not the module's default `clinics.void`, so that the
/// void policy of the claim (the kernel's default: money or stock posted)
/// never meets another voidable type of the clinic app.
pub const VOID_ACTION: &str = "clinics.claim_void";

/// The states a close leaves.
const CLOSABLE: [ClaimStatus; 4] = [
    ClaimStatus::Draft,
    ClaimStatus::Ready,
    ClaimStatus::PartiallyPaid,
    ClaimStatus::Denied,
];

/// What a payer's answer did to one line, as the payer bill holds it now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineMoney {
    /// The claim line.
    pub line: i64,
    /// What the payer paid and kept, in fils.
    pub paid: i64,
    /// What the payer denied and is still open, in fils.
    pub denied: i64,
    /// What the payer took back, in all, in fils.
    pub recouped: i64,
    /// The payer's denial or recoupment code.
    pub code: Option<String>,
    /// The payer's reason.
    pub reason: Option<String>,
}

/// What a remittance or a recoupment carries to the claim.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MoneyInput {
    /// A remittance or a recoupment.
    pub kind: AnswerKind,
    /// The lines, as the payer bill holds them after the answer.
    pub lines: Vec<LineMoney>,
    /// What the claim has been paid in all after it, in fils.
    pub paid: i64,
    /// What is denied and open after it, in fils.
    pub denied: i64,
    /// The payer's reference of the advice.
    pub reference: Option<String>,
    /// The date of the advice (Asia/Dubai).
    pub advice_date: Option<NaiveDate>,
    /// The part that went to the payer credit balance, in fils.
    pub overpayment: i64,
    /// The amount taken back, in fils (a recoupment).
    pub recouped: i64,
    /// The payer's recoupment code.
    pub code: Option<String>,
    /// Whether a reversed payment caused it.
    pub reversal: bool,
    /// The approval request that let it be recorded.
    pub approval_request: Option<i64>,
    /// The user who recorded it.
    pub by: Option<i64>,
}

/// What a run carries to the guards and effects; the service gathers it
/// because they get no app state. The fields a transition does not read stay
/// empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimInput {
    /// The day of the run (Asia/Dubai).
    pub on: Option<NaiveDate>,
    /// The user who runs it.
    pub by: Option<i64>,
    /// How many lines the claim has (`validate`).
    pub lines: i64,
    /// How many of them have no service code (`validate`).
    pub blank_codes: i64,
    /// The service codes that need a prior authorisation the claim does not
    /// carry (`validate`).
    pub missing_authorisation: Vec<String>,
    /// The last day the attempt being prepared can be sent: the
    /// first-submission window from the visit, or, for a claim that was
    /// resubmitted, the resubmission window from the remittance
    /// (`validate`, `resubmit`).
    pub deadline: Option<NaiveDate>,
    /// Whether the payer accepts a first claim after its window
    /// (`validate`).
    pub accepts_late: bool,
    /// Why it is sent late (`validate`).
    pub late_reason: Option<String>,
    /// The reference the channel gave the submission (`submit`, `resubmit`).
    pub reference: Option<String>,
    /// The channel's errors (`reject_submission`).
    pub errors: Vec<ClaimError>,
    /// The last day to send it again after the rejection
    /// (`reject_submission`).
    pub fix_deadline: Option<NaiveDate>,
    /// The corrections written down (`resubmit`).
    pub corrections: Option<String>,
    /// How many resubmissions the payer accepts (`resubmit`).
    pub limit: Option<i64>,
    /// The payer's answer (the remittance and recoupment transitions).
    pub money: Option<MoneyInput>,
    /// Why the claim is closed (`write_off`).
    pub reason: Option<String>,
    /// Whether the payer share moves to the patient, the payer contract
    /// allowing it (`write_off`).
    pub to_patient: bool,
    /// What the close writes off or moves, in fils (`write_off`).
    pub open: i64,
}

fn input_of(ctx: &TransitionCtx) -> ClaimInput {
    ctx.input_as::<ClaimInput>().unwrap_or_default()
}

/// The text of `value`, trimmed; `None` when blank.
pub(crate) fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `validate` guard: the claim has lines and each has its service code.
///
/// # Errors
///
/// The English text of the refusal.
pub fn codes_present(input: &ClaimInput) -> std::result::Result<(), String> {
    if input.lines > 0 && input.blank_codes == 0 {
        Ok(())
    } else {
        Err(english("clinics.claim.problem.codes", &[]))
    }
}

/// `validate` guard: every service the payer asks a prior authorisation for
/// has one linked.
///
/// # Errors
///
/// The English text of the refusal, naming the codes.
pub fn authorisation_linked(input: &ClaimInput) -> std::result::Result<(), String> {
    if input.missing_authorisation.is_empty() {
        Ok(())
    } else {
        Err(english(
            "clinics.claim.problem.authorisation",
            &[("codes", &input.missing_authorisation.join(", "))],
        ))
    }
}

/// `validate` guard: the day is within the window of the attempt being
/// prepared (`CLN.ACL.16` closes a claim that missed it, BL-CLN-09), or the
/// payer accepts late claims and a late reason is recorded.
///
/// # Errors
///
/// The English text of the refusal, naming the last day.
pub fn within_window(input: &ClaimInput) -> std::result::Result<(), String> {
    let (Some(deadline), Some(on)) = (input.deadline, input.on) else {
        return Ok(());
    };
    if on <= deadline {
        return Ok(());
    }
    if !input.accepts_late {
        return Err(english(
            "clinics.claim.problem.window",
            &[("date", &deadline)],
        ));
    }
    if clean(&input.late_reason).is_none() {
        return Err(english(
            "clinics.claim.problem.late_reason",
            &[("date", &deadline)],
        ));
    }
    Ok(())
}

/// `reject_submission` guard: at least one error, each with its code and
/// message.
///
/// # Errors
///
/// The English text of the refusal.
pub fn errors_given(input: &ClaimInput) -> std::result::Result<(), String> {
    let ok = !input.errors.is_empty()
        && input
            .errors
            .iter()
            .all(|e| !e.code.trim().is_empty() && !e.message.trim().is_empty());
    if ok {
        Ok(())
    } else {
        Err(english("clinics.claim.problem.errors", &[]))
    }
}

/// `reject_submission` guard: the payer has not answered the submission.
/// A claim that was resubmitted has the remittance it was resubmitted for on
/// record, and the channel can still refuse the resubmission.
///
/// # Errors
///
/// The English text of the refusal.
pub fn no_remittance_recorded(claim: &Claim) -> std::result::Result<(), String> {
    if claim.has_remittance && claim.resubmission_count == 0 {
        Err(english("clinics.claim.problem.remitted", &[]))
    } else {
        Ok(())
    }
}

/// `resubmit` guard: the corrections are written down.
///
/// # Errors
///
/// The English text of the refusal.
pub fn corrections_recorded(input: &ClaimInput) -> std::result::Result<(), String> {
    if clean(&input.corrections).is_some() {
        Ok(())
    } else {
        Err(english("clinics.claim.problem.corrections", &[]))
    }
}

/// `resubmit` guard: the day is within the payer's resubmission window.
///
/// # Errors
///
/// The English text of the refusal, naming the last day.
pub fn within_resubmission_window(input: &ClaimInput) -> std::result::Result<(), String> {
    match (input.deadline, input.on) {
        (Some(deadline), Some(on)) if on > deadline => Err(english(
            "clinics.claim.problem.resubmit_window",
            &[("date", &deadline)],
        )),
        _ => Ok(()),
    }
}

/// `resubmit` guard: the claim was not resubmitted as often as the payer
/// accepts already.
///
/// # Errors
///
/// The English text of the refusal, naming the limit.
pub fn within_resubmission_limit(
    claim: &Claim,
    input: &ClaimInput,
) -> std::result::Result<(), String> {
    match input.limit {
        Some(limit) if claim.resubmission_count >= limit => Err(english(
            "clinics.claim.problem.resubmit_limit",
            &[("limit", &limit)],
        )),
        _ => Ok(()),
    }
}

/// Remittance guard: the payer bill recorded the advice.
///
/// # Errors
///
/// The English text of the refusal.
pub fn answer_given(input: &ClaimInput, kind: AnswerKind) -> std::result::Result<(), String> {
    match &input.money {
        Some(money) if money.kind == kind => Ok(()),
        _ => Err(english("clinics.claim.problem.answer", &[])),
    }
}

/// Recoupment guard: something is taken back, and not more than the claim
/// has been paid.
///
/// # Errors
///
/// The English text of the refusal.
pub fn recouped_within_paid(claim: &Claim, input: &ClaimInput) -> std::result::Result<(), String> {
    match &input.money {
        Some(money) if money.recouped > 0 && money.recouped <= claim.paid => Ok(()),
        _ => Err(english("clinics.claim.problem.recoup_above", &[])),
    }
}

/// Recoupment guard: the payer's code is given.
///
/// # Errors
///
/// The English text of the refusal.
pub fn recoupment_code_given(input: &ClaimInput) -> std::result::Result<(), String> {
    match &input.money {
        Some(money) if clean(&money.code).is_some() => Ok(()),
        _ => Err(english("clinics.claim.problem.recoup_code", &[])),
    }
}

/// `write_off` guard: a reason is given.
///
/// # Errors
///
/// The English text of the refusal.
pub fn reason_given(input: &ClaimInput) -> std::result::Result<(), String> {
    if clean(&input.reason).is_some() {
        Ok(())
    } else {
        Err(english("clinics.claim.problem.close_reason", &[]))
    }
}

/// `write_off` guard: from `draft` or `ready` only after the first-submission
/// window of the attempt passed (today is after its last day) and when the
/// payer does not accept late claims (`CLN.ACL.16`); a claim the payer has
/// answered is not tied to the window.
///
/// # Errors
///
/// The English text of the refusal, naming the last day.
pub fn window_missed(claim: &Claim, input: &ClaimInput) -> std::result::Result<(), String> {
    if !claim.status.is_editable() {
        return Ok(());
    }
    match (input.deadline, input.on) {
        (Some(deadline), Some(on)) if on > deadline => {
            if input.accepts_late {
                Err(english("clinics.claim.problem.accepts_late", &[]))
            } else {
                Ok(())
            }
        }
        (Some(deadline), Some(_)) => Err(english(
            "clinics.claim.problem.window_open",
            &[("date", &deadline)],
        )),
        _ => Err(english("clinics.claim.problem.window_unknown", &[])),
    }
}

/// `write_off` guard: something is open on the claim.
///
/// # Errors
///
/// The English text of the refusal.
pub fn something_open(input: &ClaimInput) -> std::result::Result<(), String> {
    if input.open > 0 {
        Ok(())
    } else {
        Err(english("clinics.claim.problem.close_nothing", &[]))
    }
}

/// Sets the columns of claim `id` other than the status, and stamps
/// `updated_at`.
async fn set(tx: &mut Transaction, id: i64, columns: &[(&str, &(dyn ToDbValue + Sync))]) -> Result {
    let mut all: Vec<(&str, &(dyn ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Claim::query()
        .where_(Claim::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for Claim {
    type State = ClaimStatus;
    const COLUMN: renox::db::Col<Claim, ClaimStatus> = Claim::STATUS;

    fn machine() -> Machine<Self> {
        use ClaimStatus::{
            Denied, Draft, Paid, PartiallyPaid, Ready, Resubmitted, Submitted, Voided, WrittenOff,
        };
        let machine = Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(Ready)
            .tone("info")
            .state(Submitted)
            .tone("info")
            .state(Paid)
            .tone("success")
            .state(PartiallyPaid)
            .tone("warning")
            .state(Denied)
            .tone("error")
            .state(Resubmitted)
            .tone("info")
            .state(WrittenOff)
            .tone("neutral")
            .state(Voided)
            .tone("neutral")
            .terminal()
            .transition(validate())
            .transition(submit())
            .transition(reject_submission())
            .transition(resubmit())
            .transition(remittance(RECORD_REMITTANCE_PAID, Paid))
            .transition(remittance(RECORD_REMITTANCE_PARTIALLY_PAID, PartiallyPaid))
            .transition(remittance(RECORD_REMITTANCE_DENIED, Denied))
            .transition(recoupment(RECORD_RECOUPMENT_PARTIALLY_PAID, PartiallyPaid))
            .transition(recoupment(RECORD_RECOUPMENT_DENIED, Denied))
            .transition(write_off())
            .transition(write_off_approved());
        // `void` and `void_approved`, the kernel's, with the claim's rule on
        // top (`CLN.ACL.14`).
        voids::transitions::<Claim, _>(CLAIM_SUBJECT_TYPE, [Draft, Ready])
            .into_iter()
            .map(void_rules)
            .fold(machine, Machine::transition)
    }
}

/// `validate`: the claim is checked and waits for submission.
fn validate() -> Transition<Claim> {
    Transition::new(VALIDATE)
        .label("Validate")
        .tone("primary")
        .permission(permissions::CLAIMS_MANAGE)
        .from([ClaimStatus::Draft])
        .to(ClaimStatus::Ready)
        .guard("codes_present", |_c: &Claim, ctx| {
            let result = codes_present(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("authorisation_linked", |_c: &Claim, ctx| {
            let result = authorisation_linked(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("within_window", |_c: &Claim, ctx| {
            let result = within_window(&input_of(ctx));
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let id = c.id;
            let input = input_of(ctx);
            let late = matches!((input.deadline, input.on), (Some(d), Some(on)) if on > d);
            let reason = if late {
                clean(&input.late_reason)
            } else {
                c.late_reason.clone()
            };
            Box::pin(async move { set(tx, id, &[("late_reason", &reason)]).await })
        })
}

/// `submit`: sent to the payer; the attempt is recorded.
fn submit() -> Transition<Claim> {
    Transition::new(SUBMIT)
        .label("Submit")
        .tone("primary")
        .permission(permissions::CLAIMS_MANAGE)
        .from([ClaimStatus::Ready])
        .to(ClaimStatus::Submitted)
        .effect(|c: &Claim, ctx, tx| {
            let claim = c.clone();
            let input = input_of(ctx);
            Box::pin(async move { send_in(tx, &claim, &input, false).await })
        })
}

/// Records one attempt to reach the payer: `submit`, or `resubmit` when
/// `again` is set (the resubmission count goes up).
async fn send_in(tx: &mut Transaction, claim: &Claim, input: &ClaimInput, again: bool) -> Result {
    let on = input.on.unwrap_or_else(|| dubai_date(now()));
    let attempts = claim.attempts + 1;
    let count = claim.resubmission_count + i64::from(again);
    let reference = clean(&input.reference);
    ClaimAttempt {
        claim_id: claim.id,
        number: attempts,
        kind: if count > 0 {
            AttemptKind::Resubmission
        } else {
            AttemptKind::Submission
        },
        reference: reference.clone(),
        submitted_on: on,
        recorded_by: input.by,
        ..ClaimAttempt::default()
    }
    .insert(&mut *tx)
    .await?;
    let reference = reference.or_else(|| claim.submission_reference.clone());
    set(
        tx,
        claim.id,
        &[
            ("attempts", &attempts),
            ("resubmission_count", &count),
            ("submitted_on", &Some(on)),
            ("submission_reference", &reference),
            ("fix_deadline", &None::<NaiveDate>),
        ],
    )
    .await?;
    if again {
        // The payer is asked again for what it denied: it is open, not denied.
        ClaimLine::query()
            .where_(ClaimLine::CLAIM_ID.eq(claim.id))
            .update(&mut *tx, &[("denied", &0_i64), ("updated_at", &now())])
            .await?;
        set(
            tx,
            claim.id,
            &[
                ("denied", &0_i64),
                ("corrections", &clean(&input.corrections)),
            ],
        )
        .await?;
    }
    Ok(())
}

/// `reject_submission`: the channel refused the attempt before adjudication;
/// the claim is a draft again with the attempt's errors, and the resubmission
/// count and the numbers stay.
fn reject_submission() -> Transition<Claim> {
    Transition::new(REJECT_SUBMISSION)
        .label("Record rejection")
        .tone("danger")
        .permission(permissions::CLAIMS_MANAGE)
        .from([ClaimStatus::Submitted, ClaimStatus::Resubmitted])
        .to(ClaimStatus::Draft)
        .guard("errors_given", |_c: &Claim, ctx| {
            let result = errors_given(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("no_remittance_recorded", |c: &Claim, _ctx| {
            let result = no_remittance_recorded(c);
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let id = c.id;
            let attempt = c.attempts;
            let input = input_of(ctx);
            Box::pin(async move {
                let on = input.on.unwrap_or_else(|| dubai_date(now()));
                let errors: Vec<ClaimError> = input
                    .errors
                    .iter()
                    .map(|e| ClaimError {
                        code: e.code.trim().to_owned(),
                        message: e.message.trim().to_owned(),
                    })
                    .collect();
                ClaimAttempt::query()
                    .where_(ClaimAttempt::CLAIM_ID.eq(id))
                    .where_(ClaimAttempt::NUMBER.eq(attempt))
                    .update(
                        &mut *tx,
                        &[
                            ("errors", &Json(errors)),
                            ("rejected_on", &Some(on)),
                            ("updated_at", &now()),
                        ],
                    )
                    .await?;
                set(tx, id, &[("fix_deadline", &input.fix_deadline)]).await
            })
        })
}

/// `resubmit`: corrected and sent again; the payer's count goes up by one.
fn resubmit() -> Transition<Claim> {
    Transition::new(RESUBMIT)
        .label("Resubmit")
        .tone("primary")
        .permission(permissions::CLAIMS_MANAGE)
        .from([ClaimStatus::PartiallyPaid, ClaimStatus::Denied])
        .to(ClaimStatus::Resubmitted)
        .guard("corrections_recorded", |_c: &Claim, ctx| {
            let result = corrections_recorded(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("within_resubmission_window", |_c: &Claim, ctx| {
            let result = within_resubmission_window(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("within_resubmission_limit", |c: &Claim, ctx| {
            let result = within_resubmission_limit(c, &input_of(ctx));
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let claim = c.clone();
            let input = input_of(ctx);
            Box::pin(async move { send_in(tx, &claim, &input, true).await })
        })
}

/// The system transition that records a remittance on the claim and leads to
/// `to`.
fn remittance(name: &'static str, to: ClaimStatus) -> Transition<Claim> {
    Transition::new(name)
        .label("Record remittance")
        .from([ClaimStatus::Submitted, ClaimStatus::Resubmitted])
        .to(to)
        .system()
        .guard("remittance_recorded", |_c: &Claim, ctx| {
            let result = answer_given(&input_of(ctx), AnswerKind::Remittance);
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let claim_id = c.id;
            let input = input_of(ctx);
            Box::pin(async move { write_money_in(tx, claim_id, input).await })
        })
}

/// The system transition that records a recoupment on the claim and leads to
/// `to`.
fn recoupment(name: &'static str, to: ClaimStatus) -> Transition<Claim> {
    Transition::new(name)
        .label("Record recoupment")
        .from([
            ClaimStatus::Paid,
            ClaimStatus::PartiallyPaid,
            ClaimStatus::WrittenOff,
        ])
        .to(to)
        .system()
        .guard("recouped_within_paid", |c: &Claim, ctx| {
            let result = answer_given(&input_of(ctx), AnswerKind::Recoupment)
                .and_then(|()| recouped_within_paid(c, &input_of(ctx)));
            Box::pin(async move { result })
        })
        .guard("recoupment_code_given", |_c: &Claim, ctx| {
            let result = recoupment_code_given(&input_of(ctx));
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let claim_id = c.id;
            let input = input_of(ctx);
            Box::pin(async move { write_money_in(tx, claim_id, input).await })
        })
}

/// `write_off`: the claim is closed. From `draft` or `ready` it is the claim
/// that missed the payer's window (`CLN.ACL.16`); from `partially_paid` or
/// `denied` it is the balance the payer will not pay.
fn write_off() -> Transition<Claim> {
    Transition::new(WRITE_OFF)
        .label("Write off")
        .tone("danger")
        .permission(permissions::CLAIMS_WRITEOFF)
        .from(CLOSABLE)
        .to(ClaimStatus::WrittenOff)
        .guard("reason_given", |_c: &Claim, ctx| {
            let result = reason_given(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("first_window_missed", |c: &Claim, ctx| {
            let result = window_missed(c, &input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("something_open", |_c: &Claim, ctx| {
            let result = something_open(&input_of(ctx));
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let claim = c.clone();
            let input = input_of(ctx);
            Box::pin(async move { close_in(tx, &claim, &input).await })
        })
}

/// `write_off_approved`: the system closes the claim once the owner approved
/// the write-off the payer bill held. The window was asked when the write-off
/// was requested.
fn write_off_approved() -> Transition<Claim> {
    Transition::new(WRITE_OFF_APPROVED)
        .label("Write off (approved)")
        .from(CLOSABLE)
        .to(ClaimStatus::WrittenOff)
        .system()
        .guard("reason_given", |_c: &Claim, ctx| {
            let result = reason_given(&input_of(ctx));
            Box::pin(async move { result })
        })
        .guard("something_open", |_c: &Claim, ctx| {
            let result = something_open(&input_of(ctx));
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let claim = c.clone();
            let input = input_of(ctx);
            Box::pin(async move { close_in(tx, &claim, &input).await })
        })
}

/// What a close writes besides the status: how and why it was closed, the
/// amount, who and when, whether it is *not submitted in time*, and that the
/// payer owes nothing more (the denied amounts are closed with it), and the
/// end of any hold.
async fn close_in(tx: &mut Transaction, claim: &Claim, input: &ClaimInput) -> Result {
    let how = if input.to_patient {
        "transferred"
    } else {
        "written_off"
    };
    let on = input.on.unwrap_or_else(|| dubai_date(now()));
    ClaimLine::query()
        .where_(ClaimLine::CLAIM_ID.eq(claim.id))
        .update(&mut *tx, &[("denied", &0_i64), ("updated_at", &now())])
        .await?;
    let none: Option<String> = None;
    let no_date: Option<NaiveDate> = None;
    let no_id: Option<i64> = None;
    let missed = claim.status.is_editable();
    set(
        tx,
        claim.id,
        &[
            ("close_how", &Some(how.to_owned())),
            ("close_reason", &clean(&input.reason)),
            ("close_amount", &input.open),
            ("closed_on", &Some(on)),
            ("closed_by", &input.by),
            ("not_submitted_in_time", &missed),
            ("denied", &0_i64),
            ("held_kind", &none),
            ("held_request_id", &no_id),
            ("held_reference", &none),
            ("held_date", &no_date),
            ("held_by", &no_id),
        ],
    )
    .await
}

/// Whether the claim was ever sent: a stored submission attempt proves it,
/// also for a claim back in `draft` after a technical rejection.
///
/// # Errors
///
/// The English text of the refusal.
pub fn never_submitted(claim: &Claim) -> std::result::Result<(), String> {
    if claim.attempts > 0 || claim.submitted_on.is_some() {
        Err(english("clinics.claim.problem.void_submitted", &[]))
    } else {
        Ok(())
    }
}

/// The claim's rule on top of the kernel's `void` (`CLN.ACL.14`): never after
/// a submission attempt; and what `void` writes besides the Void record.
fn void_rules(transition: Transition<Claim>) -> Transition<Claim> {
    transition
        .guard("never_submitted", |c: &Claim, _ctx| {
            let result = never_submitted(c);
            Box::pin(async move { result })
        })
        .effect(|c: &Claim, ctx, tx| {
            let id = c.id;
            let by = ctx
                .actor()
                .map(|user| user.id)
                .or_else(|| ctx.meta().get("requester_id").and_then(Value::as_i64));
            Box::pin(async move {
                set(
                    tx,
                    id,
                    &[
                        ("voided_on", &Some(dubai_date(now()))),
                        ("voided_by", &by),
                    ],
                )
                .await
            })
        })
}

/// Registers the claim as the subject `clinics.claim` with its own approval
/// action for the void, as voidable (permission `claims.manage`, the number
/// kept) and for `rnx state:diagram`.
pub(crate) fn register() {
    // Declared before the `Voidable`, so the void policy takes the key.
    SubjectMachine::of::<Claim>(CLAIM_SUBJECT_TYPE)
        .action(voids::VOID, VOID_ACTION)
        .register();
    renox::state_machine::register::<Claim>();
    Voidable::of::<Claim>(CLAIM_SUBJECT_TYPE)
        .permission(permissions::CLAIMS_MANAGE)
        .number("claim", |claim: &Claim| Some(claim.number()))
        .register();
}

/// What a remittance or a recoupment writes besides the status: the lines'
/// paid, denied and recouped amounts with the payer's codes, the totals, the
/// record of the advice, and the end of the hold.
async fn write_money_in(tx: &mut Transaction, claim_id: i64, input: ClaimInput) -> Result {
    let Some(money) = input.money else {
        return Ok(());
    };
    let on = money.advice_date.unwrap_or_else(|| dubai_date(now()));
    for line in &money.lines {
        ClaimLine::query()
            .where_(ClaimLine::ID.eq(line.line))
            .update(
                &mut *tx,
                &[
                    ("paid", &line.paid),
                    ("denied", &line.denied),
                    ("recouped", &line.recouped),
                    ("denial_code", &line.code),
                    ("denial_reason", &line.reason),
                    ("updated_at", &now()),
                ],
            )
            .await?;
    }
    ClaimRemittance {
        claim_id,
        kind: money.kind,
        reference: clean(&money.reference),
        advice_date: on,
        paid: money.paid,
        denied: money.denied,
        overpayment: money.overpayment,
        recouped: money.recouped,
        code: clean(&money.code),
        reversal: money.reversal,
        approval_request_id: money.approval_request,
        recorded_by: money.by,
        ..ClaimRemittance::default()
    }
    .insert(&mut *tx)
    .await?;
    let none: Option<String> = None;
    let no_date: Option<NaiveDate> = None;
    let no_request: Option<i64> = None;
    set(
        tx,
        claim_id,
        &[
            ("paid", &money.paid),
            ("denied", &money.denied),
            ("has_remittance", &true),
            ("last_remittance_on", &Some(on)),
            ("last_remittance_reference", &clean(&money.reference)),
            ("held_kind", &none),
            ("held_request_id", &no_request),
            ("held_reference", &none),
            ("held_date", &no_date),
            ("held_by", &no_request),
            ("held_result", &none),
        ],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn input() -> ClaimInput {
        ClaimInput {
            on: Some(day("2026-11-03")),
            ..ClaimInput::default()
        }
    }

    #[test]
    fn every_state_tone_and_transition_is_as_the_spec_lists() {
        let machine = Claim::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("draft", Some("neutral"), false),
                ("ready", Some("info"), false),
                ("submitted", Some("info"), false),
                ("paid", Some("success"), false),
                ("partially_paid", Some("warning"), false),
                ("denied", Some("error"), false),
                ("resubmitted", Some("info"), false),
                ("written_off", Some("neutral"), false),
                ("voided", Some("neutral"), true),
            ]
        );
        assert_eq!(machine.transitions().len(), 13);
        let expected: [(&str, &[&str], &str, bool, &[&str]); 13] = [
            (
                VALIDATE,
                &["draft"],
                "ready",
                false,
                &["codes_present", "authorisation_linked", "within_window"],
            ),
            (SUBMIT, &["ready"], "submitted", false, &[]),
            (
                REJECT_SUBMISSION,
                &["submitted", "resubmitted"],
                "draft",
                false,
                &["errors_given", "no_remittance_recorded"],
            ),
            (
                RESUBMIT,
                &["partially_paid", "denied"],
                "resubmitted",
                false,
                &[
                    "corrections_recorded",
                    "within_resubmission_window",
                    "within_resubmission_limit",
                ],
            ),
            (
                RECORD_REMITTANCE_PAID,
                &["submitted", "resubmitted"],
                "paid",
                true,
                &["remittance_recorded"],
            ),
            (
                RECORD_REMITTANCE_PARTIALLY_PAID,
                &["submitted", "resubmitted"],
                "partially_paid",
                true,
                &["remittance_recorded"],
            ),
            (
                RECORD_REMITTANCE_DENIED,
                &["submitted", "resubmitted"],
                "denied",
                true,
                &["remittance_recorded"],
            ),
            (
                RECORD_RECOUPMENT_PARTIALLY_PAID,
                &["paid", "partially_paid", "written_off"],
                "partially_paid",
                true,
                &["recouped_within_paid", "recoupment_code_given"],
            ),
            (
                RECORD_RECOUPMENT_DENIED,
                &["paid", "partially_paid", "written_off"],
                "denied",
                true,
                &["recouped_within_paid", "recoupment_code_given"],
            ),
            (
                WRITE_OFF,
                &["draft", "ready", "partially_paid", "denied"],
                "written_off",
                false,
                &["reason_given", "first_window_missed", "something_open"],
            ),
            (
                WRITE_OFF_APPROVED,
                &["draft", "ready", "partially_paid", "denied"],
                "written_off",
                true,
                &["reason_given", "something_open"],
            ),
            (
                voids::VOID,
                &["draft", "ready"],
                "voided",
                false,
                &[
                    "permission_held",
                    "not_given_to_third_party",
                    "never_submitted",
                ],
            ),
            (
                voids::VOID_APPROVED,
                &["draft", "ready"],
                "voided",
                true,
                &[
                    "permission_held",
                    "not_given_to_third_party",
                    "never_submitted",
                ],
            ),
        ];
        for (name, from, to, system, guards) in expected {
            let transition = machine.find(name).expect("the transition exists");
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
            assert_eq!(transition.guard_names(), guards, "{name}");
        }
    }

    #[test]
    fn paid_and_written_off_are_left_only_by_a_recoupment() {
        let machine = Claim::machine();
        for state in ["paid", "written_off"] {
            let leaving: Vec<_> = machine
                .from_state(state)
                .map(|t| t.name().to_owned())
                .collect();
            assert_eq!(
                leaving,
                [RECORD_RECOUPMENT_PARTIALLY_PAID, RECORD_RECOUPMENT_DENIED],
                "{state}"
            );
        }
    }

    #[test]
    fn a_claim_is_closed_only_with_a_reason_and_something_open() {
        let closing = ClaimInput {
            reason: Some(" missed window ".to_owned()),
            open: 60_000,
            ..input()
        };
        assert!(reason_given(&closing).is_ok());
        assert!(reason_given(&input()).is_err());
        assert!(something_open(&closing).is_ok());
        assert!(something_open(&input()).is_err());
    }

    #[test]
    fn a_draft_is_written_off_only_after_a_missed_window_of_a_payer_that_does_not_accept_late() {
        let draft = Claim::default();
        let missed = ClaimInput {
            deadline: Some(day("2026-11-02")),
            ..input()
        };
        assert!(window_missed(&draft, &missed).is_ok());
        let open = ClaimInput {
            deadline: Some(day("2026-11-03")),
            ..input()
        };
        let refusal = window_missed(&draft, &open).unwrap_err();
        assert!(refusal.contains("2026-11-03"), "the last day counts: {refusal}");
        let accepting = ClaimInput {
            accepts_late: true,
            ..missed.clone()
        };
        assert!(window_missed(&draft, &accepting).unwrap_err().contains("late"));
        assert!(window_missed(&draft, &input()).is_err(), "no window to count");
        // The window does not matter once the payer answered.
        let denied = Claim {
            status: ClaimStatus::Denied,
            ..Claim::default()
        };
        assert!(window_missed(&denied, &open).is_ok());
        let ready = Claim {
            status: ClaimStatus::Ready,
            ..Claim::default()
        };
        assert!(window_missed(&ready, &open).is_err());
    }

    #[test]
    fn a_claim_with_a_submission_attempt_is_never_voided() {
        assert!(never_submitted(&Claim::default()).is_ok());
        let sent = Claim {
            attempts: 1,
            ..Claim::default()
        };
        assert!(never_submitted(&sent).unwrap_err().contains("sent"));
        let dated = Claim {
            submitted_on: Some(day("2026-11-03")),
            ..Claim::default()
        };
        assert!(never_submitted(&dated).is_err());
    }

    #[test]
    fn a_voided_claim_is_left_by_no_transition_and_the_void_has_its_own_action() {
        let machine = Claim::machine();
        assert_eq!(machine.from_state("voided").count(), 0);
        assert_eq!(VOID_ACTION, "clinics.claim_void");
        assert_eq!(
            SubjectMachine::of::<Claim>(CLAIM_SUBJECT_TYPE)
                .action(voids::VOID, VOID_ACTION)
                .build()
                .action_key(voids::VOID),
            "clinics.claim_void"
        );
    }

    #[test]
    fn codes_and_authorisations_are_checked() {
        let mut checked = ClaimInput {
            lines: 2,
            ..input()
        };
        assert!(codes_present(&checked).is_ok());
        checked.blank_codes = 1;
        assert!(codes_present(&checked).is_err());
        assert!(codes_present(&input()).is_err(), "a claim without lines");
        assert!(authorisation_linked(&input()).is_ok());
        let missing = ClaimInput {
            missing_authorisation: vec!["laser".to_owned()],
            ..input()
        };
        let refusal = authorisation_linked(&missing).unwrap_err();
        assert!(refusal.contains("laser"));
    }

    #[test]
    fn the_window_ends_on_its_last_day_and_a_late_reason_opens_it() {
        let inside = ClaimInput {
            deadline: Some(day("2026-11-03")),
            ..input()
        };
        assert!(within_window(&inside).is_ok(), "the last day counts");
        let late = ClaimInput {
            deadline: Some(day("2026-11-02")),
            ..input()
        };
        let refusal = within_window(&late).unwrap_err();
        assert!(refusal.contains("2026-11-02"));
        let accepting = ClaimInput {
            accepts_late: true,
            ..late.clone()
        };
        assert!(
            within_window(&accepting)
                .unwrap_err()
                .contains("late reason")
        );
        let with_reason = ClaimInput {
            late_reason: Some(" payer portal was down ".to_owned()),
            ..accepting
        };
        assert!(within_window(&with_reason).is_ok());
        assert!(within_window(&input()).is_ok(), "no window to count");
    }

    #[test]
    fn a_rejection_needs_an_error_with_code_and_message() {
        let error = |code: &str, message: &str| ClaimError {
            code: code.to_owned(),
            message: message.to_owned(),
        };
        let ok = ClaimInput {
            errors: vec![error("E104", "invalid member ID")],
            ..input()
        };
        assert!(errors_given(&ok).is_ok());
        assert!(errors_given(&input()).is_err());
        let blank = ClaimInput {
            errors: vec![error("E104", " ")],
            ..input()
        };
        assert!(errors_given(&blank).is_err());
    }

    #[test]
    fn a_rejection_is_refused_only_after_the_first_remittance() {
        let remitted = Claim {
            has_remittance: true,
            ..Claim::default()
        };
        assert!(no_remittance_recorded(&remitted).is_err());
        let resubmitted = Claim {
            has_remittance: true,
            resubmission_count: 1,
            ..Claim::default()
        };
        assert!(no_remittance_recorded(&resubmitted).is_ok());
        assert!(no_remittance_recorded(&Claim::default()).is_ok());
    }

    #[test]
    fn a_resubmission_needs_corrections_the_window_and_the_limit() {
        assert!(corrections_recorded(&input()).is_err());
        let corrected = ClaimInput {
            corrections: Some("member ID fixed".to_owned()),
            ..input()
        };
        assert!(corrections_recorded(&corrected).is_ok());
        let late = ClaimInput {
            deadline: Some(day("2026-11-02")),
            ..input()
        };
        assert!(within_resubmission_window(&late).is_err());
        let on_time = ClaimInput {
            deadline: Some(day("2026-11-03")),
            ..input()
        };
        assert!(within_resubmission_window(&on_time).is_ok());
        let twice = Claim {
            resubmission_count: 2,
            ..Claim::default()
        };
        let limited = ClaimInput {
            limit: Some(2),
            ..input()
        };
        assert!(within_resubmission_limit(&twice, &limited).is_err());
        let once = Claim {
            resubmission_count: 1,
            ..Claim::default()
        };
        assert!(within_resubmission_limit(&once, &limited).is_ok());
        assert!(within_resubmission_limit(&twice, &input()).is_ok());
    }

    #[test]
    fn a_recoupment_is_within_what_was_paid_and_has_the_payers_code() {
        let claim = Claim {
            paid: 40_000,
            ..Claim::default()
        };
        let money = |recouped: i64, code: &str| ClaimInput {
            money: Some(MoneyInput {
                kind: AnswerKind::Recoupment,
                recouped,
                code: Some(code.to_owned()),
                ..MoneyInput::default()
            }),
            ..input()
        };
        assert!(recouped_within_paid(&claim, &money(40_000, "R07")).is_ok());
        assert!(recouped_within_paid(&claim, &money(50_000, "R07")).is_err());
        assert!(recouped_within_paid(&claim, &money(0, "R07")).is_err());
        assert!(recoupment_code_given(&money(1, "R07")).is_ok());
        assert!(recoupment_code_given(&money(1, " ")).is_err());
        assert!(answer_given(&money(1, "R07"), AnswerKind::Recoupment).is_ok());
        assert!(answer_given(&money(1, "R07"), AnswerKind::Remittance).is_err());
        assert!(answer_given(&input(), AnswerKind::Remittance).is_err());
    }
}
