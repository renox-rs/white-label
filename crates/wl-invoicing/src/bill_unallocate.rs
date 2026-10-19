//! Undoing money applied to a supplier bill (wl-invoicing §4 `unallocate`,
//! #75 A5 and A6), the bill-side twin of [`crate::unallocate`].
//!
//! The spec's one action has two targets and a Renox transition has one
//! `to`, so there are two system transitions, each from `paid` and
//! `partially_paid`:
//!
//! - `unallocate` -> `approved`: nothing stays applied;
//! - `unallocate_partially` -> `partially_paid`: something stays applied (a
//!   debit note counts as applied).
//!
//! [`reopen_target`] picks one and each guard re-checks it. Both are system
//! transitions, so they are never in `available()` and no route runs them.
//! Guards: the amount is more than 0; it is at most what is applied from the
//! source (active rows only); the target matches.
//!
//! The effect, in the transition's transaction: the balance goes up and the
//! allocated total down, as a compare-and-set on the loaded balance (a
//! concurrent change gives 422); the source's active rows are marked removed
//! by the shared [`plan_removal`]; a row in the [`BillUnallocation`] log
//! records it. Rows are never deleted, and approval, cost, VAT, lines,
//! totals and the number are never written.
//!
//! [`run`] is the service: it looks the idempotency key up in the log (a key
//! that is there gives `skipped`), runs the transition and logs a refusal as
//! `failed` in its own transaction.

use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::{BillBalance, BillReopenReason, UnallocateBill};

use crate::allocate::amount_text;
use crate::bill_allocation::BillAllocation;
use crate::bill_unallocation::BillUnallocation;
use crate::payables::kernel_status;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};
use crate::unallocate::{plan_removal, refused};
use crate::unallocation::UnallocationOutcome;

/// Reopens to `approved`.
pub const UNALLOCATE: &str = "unallocate";
/// Reopens to `partially_paid`.
pub const UNALLOCATE_PARTIALLY: &str = "unallocate_partially";

/// The two transitions, with the state each one leaves the bill in.
pub const REOPEN_TARGETS: [(&str, SupplierBillStatus); 2] = [
    (UNALLOCATE, SupplierBillStatus::Approved),
    (UNALLOCATE_PARTIALLY, SupplierBillStatus::PartiallyPaid),
];

/// What a bill unallocation run carries to its guards and effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UnallocateBillInput {
    /// The money's source.
    pub source: Subject,
    /// The amount to give back, in fils.
    pub amount: i64,
    /// What is applied from `source` to the bill and not removed.
    pub applied_from_source: i64,
    /// The idempotency key.
    pub key: String,
    /// Why the bill is reopened.
    pub reason: BillReopenReason,
    /// The payment's reversal reason key.
    pub reversal_reason: Option<String>,
    /// The user whose action caused it; `None` for the system.
    pub ran_by: Option<i64>,
}

/// The state a bill reopens to (#75 A5): `approved` when nothing stays
/// applied, else `partially_paid`.
pub fn reopen_target(remaining_applied: i64) -> SupplierBillStatus {
    if remaining_applied <= 0 {
        SupplierBillStatus::Approved
    } else {
        SupplierBillStatus::PartiallyPaid
    }
}

/// The transition that reopens a bill with `allocated_total` applied after
/// giving back `amount`.
fn transition_for(allocated_total: i64, amount: i64) -> &'static str {
    let target = reopen_target(allocated_total - amount);
    REOPEN_TARGETS
        .iter()
        .find(|(_, to)| *to == target)
        .map_or(UNALLOCATE, |(name, _)| name)
}

/// The `reason` as the text stored in the allocation and the log.
pub(crate) fn reason_key(reason: BillReopenReason) -> String {
    match renox::serde_json::to_value(reason) {
        Ok(Value::String(text)) => text,
        _ => String::new(),
    }
}

/// The label of transition `name`.
fn label(name: &str) -> &'static str {
    match name {
        UNALLOCATE => "Unallocate",
        _ => "Unallocate partially",
    }
}

/// The two transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<SupplierBill>> {
    REOPEN_TARGETS
        .iter()
        .map(|&(name, to)| rules(Transition::new(name).label(label(name)).to(to), name, to))
        .collect()
}

/// What the two share: system only, the states that hold money, the guards
/// and the effect.
fn rules(
    transition: Transition<SupplierBill>,
    name: &'static str,
    target: SupplierBillStatus,
) -> Transition<SupplierBill> {
    transition
        .system()
        .from([SupplierBillStatus::Paid, SupplierBillStatus::PartiallyPaid])
        .guard("positive_amount", |_bill: &SupplierBill, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount > 0 => Ok(()),
                    _ => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("within_applied", |bill: &SupplierBill, ctx| {
            let currency = bill.currency.clone();
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount <= input.applied_from_source => Ok(()),
                    Some(input) => Err(format!(
                        "Only {} is applied from {} to this bill.",
                        amount_text(&currency, input.applied_from_source),
                        input.source
                    )),
                    None => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("target_matches", move |bill: &SupplierBill, ctx| {
            let allocated = bill.allocated_total;
            Box::pin(async move {
                let amount = input(ctx).map_or(0, |input| input.amount);
                let to = reopen_target(allocated - amount);
                if to == target {
                    Ok(())
                } else {
                    Err(format!("The bill reopens as `{}`.", to.as_str()))
                }
            })
        })
        .effect(move |bill: &SupplierBill, ctx, tx| {
            let (id, balance) = (bill.id, bill.balance);
            Box::pin(async move {
                let input = input(ctx).ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!(
                        "a bill unallocation run without its input"
                    ))
                })?;
                // Only from the balance this run checked, like `pay`.
                let moved = renox::db::sql(format!(
                    "UPDATE {} SET allocated_total = allocated_total - ?, balance = balance + ?, \
                     updated_at = ? WHERE id = ? AND balance = ?",
                    SupplierBill::TABLE
                ))
                .bind(input.amount)
                .bind(input.amount)
                .bind(renox::db::now())
                .bind(id)
                .bind(balance)
                .execute(&mut *tx)
                .await?;
                if moved == 0 {
                    return Err(refused("The bill's balance changed; try again."));
                }
                remove_in(tx, id, &input).await?;
                let mut log = BillUnallocation {
                    bill_id: id,
                    source_type: input.source.subject_type.clone(),
                    source_id: input.source.subject_id,
                    amount: input.amount,
                    key: input.key.clone(),
                    reason: input.reason,
                    reversal_reason: input.reversal_reason.clone(),
                    outcome: UnallocationOutcome::Done,
                    transition: Some(name.to_owned()),
                    balance_after: Some(balance + input.amount),
                    ran_by: input.ran_by,
                    ..Default::default()
                };
                log.insert(&mut *tx).await?;
                Ok(())
            })
        })
}

/// Marks the source's allocation rows removed for `input.amount`, and keeps
/// the rest of a split row in a new active one.
async fn remove_in(
    tx: &mut renox::db::Transaction,
    bill_id: i64,
    input: &UnallocateBillInput,
) -> Result {
    let rows = BillAllocation::active_from(tx, bill_id, &input.source).await?;
    let held: Vec<(i64, i64)> = rows.iter().map(|row| (row.id, row.amount)).collect();
    let plan = plan_removal(&held, input.amount)
        .ok_or_else(|| refused("The allocation changed; try again."))?;
    let reason = reason_key(input.reason);
    for row_id in &plan.remove {
        let marked = renox::db::sql(format!(
            "UPDATE {} SET removed_at = ?, removed_reason = ?, removed_key = ?, updated_at = ? \
             WHERE id = ? AND removed_at IS NULL",
            BillAllocation::TABLE
        ))
        .bind(renox::db::now())
        .bind(reason.clone())
        .bind(input.key.clone())
        .bind(renox::db::now())
        .bind(*row_id)
        .execute(&mut *tx)
        .await?;
        if marked == 0 {
            return Err(refused("The allocation changed; try again."));
        }
    }
    if let Some((row_id, rest)) = plan.keep
        && let Some(split) = rows.iter().find(|row| row.id == row_id)
    {
        let mut kept = BillAllocation {
            bill_id,
            source_type: split.source_type.clone(),
            source_id: split.source_id,
            amount: rest,
            allocated_at: split.allocated_at,
            created_by: split.created_by,
            ..Default::default()
        };
        kept.insert(&mut *tx).await?;
    }
    Ok(())
}

/// The run's input, if it is an [`UnallocateBillInput`].
fn input(ctx: &TransitionCtx) -> Option<UnallocateBillInput> {
    ctx.input_as::<UnallocateBillInput>().ok()
}

/// The `meta` of the history row, which the `SupplierBillReopened` listener
/// reads (#75 part 4).
fn meta_of(input: &UnallocateBillInput) -> Value {
    json!({
        "source": input.source.to_string(),
        "amount": input.amount,
        "key": input.key,
        "reason": reason_key(input.reason),
        "reversal_reason": input.reversal_reason,
        "ran_by": input.ran_by,
    })
}

/// Runs the unallocation of `request` on `bill` inside `tx`, which the
/// caller commits. Emit the returned outcome after the commit.
///
/// # Errors
///
/// `NotFrom` (422) when the bill is not `paid` or `partially_paid`; `Guard`
/// (422) for an amount <= 0 or above what is applied from the source; a 422
/// when another change moved the balance first. The caller rolls `tx` back.
pub async fn unallocate_in(
    tx: &mut renox::db::Transaction,
    bill: &SupplierBill,
    request: &UnallocateBill,
    ran_by: Option<i64>,
) -> std::result::Result<Outcome<SupplierBill>, TransitionError> {
    let applied_from_source: i64 = BillAllocation::active_from(tx, bill.id, &request.source)
        .await?
        .iter()
        .map(|row| row.amount)
        .sum();
    let input = UnallocateBillInput {
        source: request.source.clone(),
        amount: request.amount,
        applied_from_source,
        key: request.key.clone(),
        reason: request.reason,
        reversal_reason: request.reversal_reason.clone(),
        ran_by,
    };
    bill.transition(transition_for(bill.allocated_total, request.amount))
        .by_system()
        .input(&input)
        .meta(meta_of(&input))
        .run_in(tx)
        .await
}

/// What a `skipped` call answers: the bill as it is.
async fn skipped(state: &AppState, id: i64) -> Result<BillBalance> {
    let bill = SupplierBill::find_or_404(&state.db, id).await?;
    Ok(BillBalance {
        new_status: kernel_status(bill.status),
        balance: bill.balance,
        skipped: true,
    })
}

/// Logs a refused `request` as `failed`, in its own transaction. A key that
/// is already in the log is left as it is.
async fn log_failed(state: &AppState, request: &UnallocateBill, message: String) -> Result {
    let mut log = BillUnallocation {
        bill_id: request.bill.0,
        source_type: request.source.subject_type.clone(),
        source_id: request.source.subject_id,
        amount: request.amount,
        key: request.key.clone(),
        reason: request.reason,
        reversal_reason: request.reversal_reason.clone(),
        outcome: UnallocationOutcome::Failed,
        message: Some(message),
        ..Default::default()
    };
    match log.insert(&state.db).await {
        Err(error) if !error.is_unique_violation() => Err(error),
        _ => Ok(()),
    }
}

/// Undoes `request.amount` fils of the money from `request.source` on
/// `request.bill`, as the system, in one transaction, and reopens the bill.
/// Idempotent per `request.key`: a key that is in the log, `done` or
/// `failed`, answers `skipped` and changes nothing. A refusal is logged as
/// `failed` (with its message) and returned as an error.
///
/// # Errors
///
/// 404 for an unknown bill; 422 for an amount <= 0 or above what is applied
/// from the source, a bill in a state that cannot be reopened (`approved`,
/// `void`, `exception`, ...), or a balance that changed under the run.
/// Nothing is written then, apart from the `failed` row of a refusal.
pub async fn run(state: &AppState, request: UnallocateBill) -> Result<BillBalance> {
    if BillUnallocation::by_key(&state.db, &request.key)
        .await?
        .is_some()
    {
        return skipped(state, request.bill.0).await;
    }
    let Some(bill) = SupplierBill::find(&state.db, request.bill.0).await? else {
        log_failed(state, &request, "The bill does not exist.".to_owned()).await?;
        return Err(Error::NotFound);
    };
    let mut tx = state.db.begin().await?;
    let outcome = match unallocate_in(&mut tx, &bill, &request, None).await {
        Ok(outcome) => outcome,
        Err(error) => {
            drop(tx);
            return match error {
                TransitionError::Failed(error) if error.is_unique_violation() => {
                    skipped(state, bill.id).await
                }
                TransitionError::Guard(_) | TransitionError::NotFrom(_) => {
                    log_failed(state, &request, error.to_string()).await?;
                    Err(error.into_error())
                }
                other => Err(other.into_error()),
            };
        }
    };
    tx.commit().await?;
    outcome.emit(state).await?;
    let bill = SupplierBill::find_or_404(&state.db, bill.id).await?;
    Ok(BillBalance {
        new_status: kernel_status(bill.status),
        balance: bill.balance,
        skipped: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paid(allocated: i64) -> SupplierBill {
        SupplierBill {
            id: 1,
            currency: "AED".into(),
            gross_total: 1_050_000,
            allocated_total: allocated,
            balance: 1_050_000 - allocated,
            status: SupplierBillStatus::Paid,
            ..Default::default()
        }
    }

    fn run_of(amount: i64, held: i64) -> TransitionCtx {
        let input = UnallocateBillInput {
            source: Subject::new("payments.payment", 3),
            amount,
            applied_from_source: held,
            key: "payments.allocation:1".into(),
            reason: BillReopenReason::PaymentReversed,
            reversal_reason: Some("bank_return".into()),
            ran_by: None,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn nothing_left_applied_reopens_to_approved_else_partially_paid() {
        assert_eq!(reopen_target(0), SupplierBillStatus::Approved);
        assert_eq!(reopen_target(-1), SupplierBillStatus::Approved);
        assert_eq!(reopen_target(1), SupplierBillStatus::PartiallyPaid);
        assert_eq!(transition_for(1_050_000, 1_050_000), UNALLOCATE);
        assert_eq!(transition_for(1_050_000, 450_000), UNALLOCATE_PARTIALLY);
    }

    #[test]
    fn both_go_from_the_money_states_and_are_system_only() {
        let machine = SupplierBill::machine();
        for (name, to) in [
            (UNALLOCATE, "approved"),
            (UNALLOCATE_PARTIALLY, "partially_paid"),
        ] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(transition.from_states(), ["paid", "partially_paid"]);
            assert_eq!(transition.to_state(), Some(to));
            assert!(transition.is_system());
        }
    }

    #[test]
    fn the_reason_is_stored_as_its_key() {
        assert_eq!(
            reason_key(BillReopenReason::PaymentReversed),
            "payment_reversed"
        );
        assert_eq!(
            reason_key(BillReopenReason::DebitNoteCancelled),
            "debit_note_cancelled"
        );
    }

    #[renox::test]
    async fn the_guards_refuse_a_bad_amount_and_a_wrong_target() {
        let machine = SupplierBill::machine();
        let full = machine.find(UNALLOCATE).expect("unallocate");
        let part = machine.find(UNALLOCATE_PARTIALLY).expect("partially");
        let bill = paid(1_050_000);
        assert!(
            full.check_guards(&bill, &run_of(1_050_000, 1_050_000))
                .await
                .is_ok()
        );
        assert!(
            part.check_guards(&bill, &run_of(450_000, 450_000))
                .await
                .is_ok()
        );
        // Zero, above what the source applied, a missing input.
        for ctx in [run_of(0, 100), run_of(200, 100), TransitionCtx::new(None)] {
            assert!(matches!(
                full.check_guards(&bill, &ctx).await,
                Err(TransitionError::Guard(_))
            ));
        }
        // Something stays applied, so `unallocate` is the wrong target, and
        // the other way round.
        assert!(matches!(
            full.check_guards(&bill, &run_of(450_000, 450_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            part.check_guards(&bill, &run_of(1_050_000, 1_050_000))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }
}
