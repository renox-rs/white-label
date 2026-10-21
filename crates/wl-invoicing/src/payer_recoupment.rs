//! A payer takes money back: recording a recoupment (wl-invoicing §4
//! *Payer bill* `record_recoupment`, §5.8 step 7, `FIN.ACL.16`).
//!
//! A recoupment is a negative line of a later remittance that takes back
//! money the payer paid on an earlier bill's lines, or a reversed
//! wl-payments payment that was applied to the bill. The reconciler
//! (`payer_bills.reconcile`) records it on a `paid`, `partially_paid` or
//! `closed` bill; the bill goes to `denied` when every line's paid amount is
//! then zero and to `partially_paid` otherwise ([`transition_name`]).
//!
//! - **Approval.** A user's recoupment waits as an approval request
//!   (`FIN.ACL.16`, the same action as an overpayment): the input's `amount`
//!   is the amount recouped, the earlier bill is unchanged until a second
//!   person approves, and the reconciler cannot approve their own. A
//!   recoupment from a reversed payment is the system's and needs none
//!   ([`reverse`]).
//! - **Guards.** The advice names lines of the bill; each amount is above 0
//!   and not above what the line has been paid; a recoupment code or a
//!   reason is given.
//! - **Effect.** The paid amount of each line goes down and the same amount
//!   is open again as denied with the payer's code (a line that was written
//!   off keeps its write-off recorded); the money is set off from the payer
//!   credit balance first, and the rest is the part deducted from the later
//!   remittance that carried the recoupment; the receivable is back in the
//!   ageing. After the commit a listener emits `PayerRecoupmentRecorded`, to
//!   which the vertical's denial task reacts (05 `CLN.RUL.05`).
//!
//! Decisions:
//!
//! - A reversed payment is not set off from the credit balance and nothing
//!   is deducted from a remittance: the bank has already taken the money
//!   back. Each line is recouped by what that payment was applied to it, at
//!   most what the line has been paid since.
//! - Taking the part deducted from the carrying remittance "through
//!   wl-payments" is recorded here (`deducted`, the carrier's reference and
//!   date); wl-payments learns of it when that remittance is recorded as a
//!   payment.

use renox::chrono::NaiveDate;
use renox::db::Json;
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals;
use wl_kernel::machines::approved;
use wl_kernel::payer_bills::{
    PayerAnswer, PayerBillStatus, PayerRecoupmentRecorded, RecoupedLine, Recoupment,
};
use wl_kernel::payments::PaymentReversed;

use crate::payer_bill::{PayerBill, PayerBillLine, lines_of, write_totals_in};
use crate::payer_credit::{self, PayerCreditMovement};
use crate::payer_remittance::{ACTION, PayerAllocation, answer_of, approval_of};
use crate::permissions;
use crate::texts::english;

/// The prefix of the recoupment transitions.
pub const RECORD_RECOUPMENT: &str = "record_recoupment";

/// The states a recoupment leaves a bill in.
pub const OUTCOMES: [PayerBillStatus; 2] = [PayerBillStatus::PartiallyPaid, PayerBillStatus::Denied];

/// The states a recoupment leaves.
const FROM: [PayerBillStatus; 3] = [
    PayerBillStatus::Paid,
    PayerBillStatus::PartiallyPaid,
    PayerBillStatus::Closed,
];

/// The code of a recoupment that a reversed payment causes.
pub const REVERSAL_CODE: &str = "payment_reversed";

/// The name of the recoupment transition that leads to `outcome`:
/// `record_recoupment_denied`, or its `_approved` twin the kernel runs.
pub fn transition_name(outcome: PayerBillStatus, approved_twin: bool) -> String {
    let name = format!("{RECORD_RECOUPMENT}_{}", outcome.as_str());
    if approved_twin {
        approved(&name)
    } else {
        name
    }
}

/// The name of the system transition of a reversed payment.
fn reversal_name(outcome: PayerBillStatus) -> String {
    format!("{RECORD_RECOUPMENT}_reversal_{}", outcome.as_str())
}

/// Money a payer took back from a payer bill. Row of
/// `wl_invoicing_payer_recoupments`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_invoicing_payer_recoupments", index(payer_bill_id))]
pub struct PayerRecoupment {
    /// The primary key.
    pub id: i64,
    /// The payer bill.
    pub payer_bill_id: i64,
    /// The payer (a party).
    pub payer_party_id: i64,
    /// The legal entity whose books it belongs to.
    pub legal_entity_id: i64,
    /// The ISO currency code.
    pub currency: String,
    /// The payer's recoupment code (`payment_reversed` for a reversal).
    pub code: String,
    /// The payer's reason, if it gave one.
    pub reason: Option<String>,
    /// The reference of the later remittance that carried it.
    pub carrier_reference: Option<String>,
    /// The date of that remittance.
    pub carrier_date: Option<NaiveDate>,
    /// The total recouped, in fils.
    pub amount: i64,
    /// The part set off from the payer credit balance.
    pub from_credit: i64,
    /// The part deducted from the remittance that carried it.
    pub deducted: i64,
    /// The lines and the amounts.
    pub lines: Json<Vec<RecoupedLine>>,
    /// Whether a reversed payment caused it.
    pub reversal: bool,
    /// The reversed wl-payments payment, for a reversal.
    pub payment_id: Option<i64>,
    /// The approval request, if one was needed.
    pub approval_request_id: Option<i64>,
    /// The user who recorded it.
    pub recorded_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a recoupment run carries to its guards and effect; it is also the
/// approval request's payload, which the policy's condition reads
/// (`amount`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct RecoupmentInput {
    /// The recoupment.
    pub recoupment: Option<Recoupment>,
    /// Why it cannot be recorded, if it cannot.
    pub problem: Option<String>,
    /// The total recouped.
    pub total: i64,
    /// What needs a second person's approval: the total, 0 for a reversal.
    pub amount: i64,
    /// Whether a reversed payment caused it.
    pub reversal: bool,
    /// The reversed payment, for a reversal.
    pub payment: Option<i64>,
    /// The user a system run acts for.
    pub by: Option<i64>,
}

/// The run's input, if it is a [`RecoupmentInput`].
fn input(ctx: &TransitionCtx) -> Option<RecoupmentInput> {
    ctx.input_as::<RecoupmentInput>().ok()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Checks `recoupment` against `lines`: a code or a reason, lines of the
/// bill, each once, above 0 and within what the line has been paid. Gives
/// the state the bill is left in, or why it cannot be recorded.
pub fn plan(
    lines: &[PayerBillLine],
    recoupment: &Recoupment,
) -> std::result::Result<PayerBillStatus, String> {
    let reason_given = recoupment
        .reason
        .as_deref()
        .is_some_and(|r| !r.trim().is_empty());
    if recoupment.code.trim().is_empty() && !reason_given {
        return Err(english("wl_invoicing.payer_bill.errors.recoup_code").to_owned());
    }
    if recoupment.lines.is_empty() {
        return Err(english("wl_invoicing.payer_bill.errors.recoup_lines").to_owned());
    }
    let mut seen: Vec<i64> = Vec::new();
    for entry in &recoupment.lines {
        let Some(line) = lines.iter().find(|l| l.id == entry.line) else {
            return Err(english("wl_invoicing.payer_bill.errors.remit_unmatched").to_owned());
        };
        if seen.contains(&line.id) {
            return Err(english("wl_invoicing.payer_bill.errors.remit_twice").to_owned());
        }
        seen.push(line.id);
        if entry.amount <= 0 {
            return Err(english("wl_invoicing.payer_bill.errors.recoup_amount").to_owned());
        }
        if entry.amount > line.paid {
            return Err(english("wl_invoicing.payer_bill.errors.recoup_above_paid").to_owned());
        }
    }
    let nothing_left = lines.iter().all(|line| {
        let taken = recoupment
            .lines
            .iter()
            .find(|e| e.line == line.id)
            .map_or(0, |e| e.amount);
        line.paid - taken == 0
    });
    Ok(if nothing_left {
        PayerBillStatus::Denied
    } else {
        PayerBillStatus::PartiallyPaid
    })
}

/// The recoupment transitions, for the machine: the user's and the kernel's
/// approved twin, and the system's for a reversed payment.
pub(crate) fn transitions() -> Vec<Transition<PayerBill>> {
    let mut all = Vec::new();
    for outcome in OUTCOMES {
        all.push(rules(
            Transition::new(transition_name(outcome, false))
                .label("Record recoupment")
                .tone("danger")
                .permission(permissions::PAYER_BILLS_RECONCILE),
            outcome,
        ));
        all.push(rules(
            Transition::new(transition_name(outcome, true))
                .label("Record recoupment (approved)")
                .system(),
            outcome,
        ));
        all.push(rules(
            Transition::new(reversal_name(outcome))
                .label("Record recoupment (payment reversed)")
                .system(),
            outcome,
        ));
    }
    all
}

/// What the transitions share: the states, the guard and the effect.
fn rules(transition: Transition<PayerBill>, outcome: PayerBillStatus) -> Transition<PayerBill> {
    transition
        .from(FROM)
        .to(outcome)
        .guard("recoupment_matched", |_bill: &PayerBill, ctx| {
            let problem = match input(ctx) {
                Some(RecoupmentInput {
                    problem: Some(problem),
                    ..
                }) => Some(problem),
                Some(RecoupmentInput {
                    recoupment: Some(_),
                    ..
                }) => None,
                _ => Some(english("wl_invoicing.payer_bill.errors.recoup_lines").to_owned()),
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|bill: &PayerBill, ctx, tx| {
            let bill = bill.clone();
            let gathered = input(ctx);
            let approval = approval_of(ctx.meta());
            let by = ctx
                .actor()
                .map(|user| user.id)
                .or_else(|| gathered.as_ref().and_then(|i| i.by));
            Box::pin(async move {
                let gathered = gathered.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a recoupment run without its input"))
                })?;
                write_recoupment_in(tx, &bill, gathered, approval, by).await
            })
        })
}

/// What a recoupment writes besides the status, in the run's transaction.
async fn write_recoupment_in(
    tx: &mut renox::db::Transaction,
    bill: &PayerBill,
    input: RecoupmentInput,
    approval: Option<i64>,
    by: Option<i64>,
) -> Result {
    let recoupment = input
        .recoupment
        .ok_or_else(|| Error::from(renox::anyhow::anyhow!("a recoupment run without its lines")))?;
    let mut lines = lines_of(&mut *tx, bill.id).await?;
    // The lines may have changed while an approval waited.
    for entry in &recoupment.lines {
        let fits = lines
            .iter()
            .find(|l| l.id == entry.line)
            .is_some_and(|l| entry.amount > 0 && entry.amount <= l.paid);
        if !fits {
            return Err(refused(english(
                "wl_invoicing.payer_bill.errors.recoup_changed",
            )));
        }
    }
    let total: i64 = recoupment.lines.iter().map(|e| e.amount).sum();
    for entry in &recoupment.lines {
        let Some(line) = lines.iter_mut().find(|l| l.id == entry.line) else {
            continue;
        };
        line.paid -= entry.amount;
        line.denied += entry.amount;
        line.recoupment_code = Some(recoupment.code.clone()).filter(|c| !c.is_empty());
        if let Some(reason) = &recoupment.reason {
            line.denial_reason = Some(reason.clone());
        }
        PayerBillLine::query()
            .where_(PayerBillLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("paid", &line.paid),
                    ("denied", &line.denied),
                    ("recoupment_code", &line.recoupment_code),
                    ("denial_reason", &line.denial_reason),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    write_totals_in(tx, bill.id, &lines).await?;
    // The money is taken from the payer credit balance first; a reversed
    // payment is already gone from the bank, so nothing is set off or
    // deducted for it.
    let (from_credit, deducted) = if input.reversal {
        (0, 0)
    } else {
        let credit = payer_credit::balance(
            &mut *tx,
            bill.payer(),
            bill.legal_entity_id,
            &bill.currency,
        )
        .await?
        .max(0);
        let from_credit = credit.min(total);
        (from_credit, total - from_credit)
    };
    let mut row = PayerRecoupment {
        payer_bill_id: bill.id,
        payer_party_id: bill.payer_party_id,
        legal_entity_id: bill.legal_entity_id,
        currency: bill.currency.clone(),
        code: recoupment.code.clone(),
        reason: recoupment.reason.clone(),
        carrier_reference: recoupment.carrier_reference.clone(),
        carrier_date: recoupment.carrier_date,
        amount: total,
        from_credit,
        deducted,
        lines: Json(recoupment.lines.clone()),
        reversal: input.reversal,
        payment_id: input.payment,
        approval_request_id: approval,
        recorded_by: by,
        ..Default::default()
    };
    row.insert(&mut *tx).await?;
    if from_credit > 0 {
        PayerCreditMovement {
            payer_party_id: bill.payer_party_id,
            legal_entity_id: bill.legal_entity_id,
            currency: bill.currency.clone(),
            kind: payer_credit::SET_OFF.to_owned(),
            amount: -from_credit,
            payer_bill_id: Some(bill.id),
            recoupment_id: Some(row.id),
            ..Default::default()
        }
        .insert(&mut *tx)
        .await?;
    }
    if let Some(payment) = input.payment {
        PayerAllocation::query()
            .where_(PayerAllocation::PAYER_BILL_ID.eq(bill.id))
            .where_(PayerAllocation::PAYMENT_ID.eq(payment))
            .where_(PayerAllocation::REMOVED_AT.is_null())
            .update(
                &mut *tx,
                &[
                    ("removed_at", &Some(renox::db::now())),
                    ("removed_reason", &Some(REVERSAL_CODE.to_owned())),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
    }
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            &mut *tx,
            &[("in_ageing", &true), ("updated_at", &renox::db::now())],
        )
        .await?;
    Ok(())
}

/// Records `recoupment` on the payer bill `bill_id` as `user`, in its own
/// transaction. It waits as an approval request (`FIN.ACL.16`): the answer
/// is [`PayerAnswer::Pending`], the request is announced and the bill is
/// unchanged until a second person approves.
///
/// # Errors
///
/// `Forbidden` without `payer_bills.reconcile`; `NotFrom` unless the bill is
/// `paid`, `partially_paid` or `closed`; `Guard` (422) without a code or
/// reason, without lines, for a line that is not on the bill, repeated, at 0
/// or above what the line has been paid. Nothing is written then.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    bill_id: i64,
    recoupment: Recoupment,
) -> std::result::Result<(PayerBill, PayerAnswer), TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let lines = lines_of(&state.db, bill.id).await?;
    let total: i64 = recoupment.lines.iter().map(|e| e.amount).sum();
    let mut gathered = RecoupmentInput {
        recoupment: Some(recoupment.clone()),
        total,
        amount: total,
        by: Some(user.id),
        ..Default::default()
    };
    let outcome = match plan(&lines, &recoupment) {
        Ok(outcome) => outcome,
        Err(problem) => {
            gathered.problem = Some(problem);
            PayerBillStatus::PartiallyPaid
        }
    };
    let ran = bill
        .transition(&transition_name(outcome, false))
        .by(user)
        .meta(json!({}))
        .input(&gathered)
        .run(state)
        .await?;
    approvals::announce(state, &ran)
        .await
        .map_err(TransitionError::Failed)?;
    Ok((
        PayerBill::find_or_404(&state.db, bill_id).await?,
        answer_of(&ran),
    ))
}

/// A reversed payment `payment` takes back what it was applied to on payer
/// bill `bill_id`, as the system and without approval. Answers the bill, or
/// `None` when the payment was applied to nothing of it any more.
///
/// # Errors
///
/// `NotFrom` for a bill that is not `paid`, `partially_paid` or `closed`.
pub async fn reverse(
    state: &AppState,
    bill_id: i64,
    payment: i64,
) -> std::result::Result<Option<PayerBill>, TransitionError> {
    let bill = PayerBill::find_or_404(&state.db, bill_id).await?;
    let lines = lines_of(&state.db, bill.id).await?;
    let allocations = PayerAllocation::query()
        .where_(PayerAllocation::PAYER_BILL_ID.eq(bill.id))
        .where_(PayerAllocation::PAYMENT_ID.eq(payment))
        .where_(PayerAllocation::REMOVED_AT.is_null())
        .get(&state.db)
        .await?;
    let mut recoupment = Recoupment::new(REVERSAL_CODE);
    for line in &lines {
        let applied: i64 = allocations
            .iter()
            .filter(|a| a.payer_bill_line_id == line.id)
            .map(|a| a.amount)
            .sum();
        // At most what the line has been paid since.
        let amount = applied.min(line.paid);
        if amount > 0 {
            recoupment = recoupment.line(line.id, amount);
        }
    }
    if recoupment.lines.is_empty() {
        return Ok(None);
    }
    let outcome = plan(&lines, &recoupment)
        .map_err(TransitionError::Guard)?;
    let total: i64 = recoupment.lines.iter().map(|e| e.amount).sum();
    let gathered = RecoupmentInput {
        recoupment: Some(recoupment),
        total,
        // A reversal from wl-payments is exempt from the approval.
        amount: 0,
        reversal: true,
        payment: Some(payment),
        ..Default::default()
    };
    let _ran = bill
        .transition(&reversal_name(outcome))
        .by_system()
        .meta(json!({}))
        .input(&gathered)
        .run(state)
        .await?;
    Ok(Some(PayerBill::find_or_404(&state.db, bill_id).await?))
}

/// Runs [`reverse`] on every payer bill `event` took money from.
///
/// # Errors
///
/// The first unexpected error, after every bill was tried; a bill that
/// cannot be recouped from its state is left as it is.
pub async fn on_payment_reversed(state: &AppState, event: &PaymentReversed) -> Result {
    let allocations = PayerAllocation::query()
        .where_(PayerAllocation::PAYMENT_ID.eq(event.payment.0))
        .where_(PayerAllocation::REMOVED_AT.is_null())
        .get(&state.db)
        .await?;
    let mut bills: Vec<i64> = allocations.iter().map(|a| a.payer_bill_id).collect();
    bills.sort_unstable();
    bills.dedup();
    let mut first_error = None;
    for bill in bills {
        match reverse(state, bill, event.payment.0).await {
            Ok(_) => {}
            // A bill in another state is a refusal, not a failure.
            Err(TransitionError::NotFrom(_) | TransitionError::Guard(_)) => {}
            Err(other) => {
                if first_error.is_none() {
                    first_error = Some(other.into_error());
                }
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// After the commit of a recoupment (recorded at once, after the approval or
/// from a reversed payment): emits `PayerRecoupmentRecorded`.
pub(crate) async fn on_transition(state: &AppState, event: &Transitioned<PayerBill>) -> Result {
    if !event.transition.starts_with(RECORD_RECOUPMENT) {
        return Ok(());
    }
    let bill = PayerBill::find_or_404(&state.db, event.key).await?;
    let Some(row) = PayerRecoupment::query()
        .where_(PayerRecoupment::PAYER_BILL_ID.eq(bill.id))
        .order_by_desc(PayerRecoupment::ID)
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    state
        .emit(PayerRecoupmentRecorded {
            payer_bill: bill.payer_bill_id(),
            payer: bill.payer(),
            source: bill.source(),
            lines: row.lines.0.clone(),
            code: row.code.clone(),
            amount: row.amount,
            from_credit: row.from_credit,
            deducted: row.deducted,
            reversal: row.reversal,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: i64, claimed: i64, paid: i64) -> PayerBillLine {
        PayerBillLine {
            id,
            position: id,
            claimed,
            paid,
            ..Default::default()
        }
    }

    #[test]
    fn every_outcome_has_a_user_an_approved_and_a_reversal_transition() {
        let machine = PayerBill::machine();
        for outcome in OUTCOMES {
            let user = machine.find(&transition_name(outcome, false)).unwrap();
            assert!(!user.is_system());
            assert_eq!(user.from_states(), ["paid", "partially_paid", "closed"]);
            assert_eq!(user.to_state(), Some(outcome.as_str()));
            for twin in [transition_name(outcome, true), reversal_name(outcome)] {
                let twin = machine.find(&twin).unwrap();
                assert!(twin.is_system());
                assert_eq!(twin.to_state(), Some(outcome.as_str()));
            }
        }
        assert_eq!(
            transition_name(PayerBillStatus::Denied, true),
            "record_recoupment_denied_approved"
        );
    }

    #[test]
    fn a_recoupment_of_everything_paid_leaves_the_bill_denied() {
        let lines = [line(1, 40_000, 40_000)];
        let all = Recoupment::new("R07").line(1, 40_000);
        assert_eq!(plan(&lines, &all), Ok(PayerBillStatus::Denied));
        let part = Recoupment::new("R07").line(1, 10_000);
        assert_eq!(plan(&lines, &part), Ok(PayerBillStatus::PartiallyPaid));
        // Another line still holds money.
        let two = [line(1, 40_000, 40_000), line(2, 10_000, 10_000)];
        assert_eq!(plan(&two, &all), Ok(PayerBillStatus::PartiallyPaid));
    }

    #[test]
    fn a_recoupment_above_what_was_paid_is_refused() {
        let lines = [line(1, 50_000, 40_000)];
        let above = Recoupment::new("R07").line(1, 50_000);
        assert!(plan(&lines, &above).is_err());
        assert!(plan(&lines, &Recoupment::new("R07").line(1, 0)).is_err());
        assert!(plan(&lines, &Recoupment::new("R07").line(9, 1)).is_err());
        assert!(plan(&lines, &Recoupment::new("R07")).is_err());
        assert!(
            plan(
                &lines,
                &Recoupment::new("R07").line(1, 1).line(1, 1)
            )
            .is_err()
        );
    }

    #[test]
    fn a_code_or_a_reason_is_needed() {
        let lines = [line(1, 40_000, 40_000)];
        assert!(plan(&lines, &Recoupment::new(" ").line(1, 100)).is_err());
        assert!(
            plan(
                &lines,
                &Recoupment::new("").reason("taken back").line(1, 100)
            )
            .is_ok()
        );
    }
}
