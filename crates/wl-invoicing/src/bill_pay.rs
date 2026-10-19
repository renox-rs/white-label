//! Paying a supplier bill (wl-invoicing §4 `pay`, #75 A2).
//!
//! Like `allocate` on an invoice, the spec's one action has two targets, so
//! there are two system transitions from `approved` and `partially_paid`:
//!
//! - `pay` -> `partially_paid`, for an amount that leaves a balance;
//! - `pay_in_full` -> `paid`, for an amount equal to the balance.
//!
//! [`pay`] picks one by the resulting balance. Both refuse an amount <= 0 or
//! above the balance. The effect writes the [`BillAllocation`] and the bill's
//! allocated total and balance in the transition's transaction; the balance
//! is updated only from the one the run loaded, so two payments at once never
//! both pass. Neither is a user action: they are reached only through the
//! kernel contract.

use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::allocate::amount_text;
use crate::bill_allocation::BillAllocation;
use crate::supplier_bill::{SupplierBill, SupplierBillStatus};

/// The transition that leaves a balance.
pub const PAY: &str = "pay";
/// The transition that settles the balance.
pub const PAY_IN_FULL: &str = "pay_in_full";

/// What a payment run carries to its guards and effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PayInput {
    /// Where the money comes from: an outgoing payment or a debit note.
    pub source: Subject,
    /// The amount, in fils of the bill's currency.
    pub amount: i64,
    /// The user whose action caused it; `None` for the system.
    pub ran_by: Option<i64>,
    /// The idempotency key stored on the allocation; `None` for a payment
    /// made without one.
    #[serde(default)]
    pub key: Option<String>,
}

/// The `pay` and `pay_in_full` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<SupplierBill>; 2] {
    [
        rules(
            Transition::new(PAY)
                .label("Pay")
                .to(SupplierBillStatus::PartiallyPaid),
        )
        .guard("leaves_a_balance", |bill: &SupplierBill, ctx| {
            let balance = bill.balance;
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount < balance => Ok(()),
                    _ => Err("The amount settles the balance: pay it in full.".to_owned()),
                }
            })
        }),
        rules(
            Transition::new(PAY_IN_FULL)
                .label("Pay in full")
                .to(SupplierBillStatus::Paid),
        )
        .guard("settles_the_balance", |bill: &SupplierBill, ctx| {
            let balance = bill.balance;
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount == balance => Ok(()),
                    _ => Err("The amount does not settle the balance.".to_owned()),
                }
            })
        }),
    ]
}

/// What both transitions share: system only, the open states, the amount
/// guards and the effect.
fn rules(transition: Transition<SupplierBill>) -> Transition<SupplierBill> {
    transition
        .system()
        .from([
            SupplierBillStatus::Approved,
            SupplierBillStatus::PartiallyPaid,
        ])
        .guard("positive_amount", |_bill: &SupplierBill, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount > 0 => Ok(()),
                    _ => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("within_balance", |bill: &SupplierBill, ctx| {
            let (balance, currency) = (bill.balance, bill.currency.clone());
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount <= balance => Ok(()),
                    _ => Err(format!(
                        "The amount is above the balance of {}.",
                        amount_text(&currency, balance)
                    )),
                }
            })
        })
        .effect(|bill: &SupplierBill, ctx, tx| {
            let (id, balance) = (bill.id, bill.balance);
            Box::pin(async move {
                // The guards passed, so the input is there.
                let input = input(ctx).ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("a payment run without its input"))
                })?;
                // Only from the balance this run checked: a concurrent
                // payment moved it, so this one must not pass as well. Not
                // the status column, so outside the machine's guard.
                let moved = renox::db::sql(format!(
                    "UPDATE {} SET allocated_total = allocated_total + ?, balance = balance - ?, \
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
                    return Err(Error::Status(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "The bill's balance changed; try again.".to_owned(),
                    ));
                }
                let mut row = BillAllocation::new(id, &input.source, input.amount, input.ran_by);
                row.key = input.key.clone();
                row.insert(&mut *tx).await?;
                Ok(())
            })
        })
}

/// The run's input, if it is a [`PayInput`].
fn input(ctx: &TransitionCtx) -> Option<PayInput> {
    ctx.input_as::<PayInput>().ok()
}

/// The transition that pays `amount` on a bill with `balance`.
fn transition_for(amount: i64, balance: i64) -> &'static str {
    if amount == balance { PAY_IN_FULL } else { PAY }
}

/// Applies `amount` fils from `source` (an outgoing payment or a debit note)
/// to `bill`, as the system, in its own transaction: `pay_in_full` when it
/// settles the balance, else `pay`. `ran_by` is the user whose action caused
/// it, stored on the allocation and in the history row's `meta`.
///
/// # Errors
///
/// `NotFrom` (422) when the bill is not `approved` or `partially_paid`;
/// `Guard` (422) for an amount <= 0 or above the balance; a 422 when another
/// payment changed the balance first. Nothing is written then.
pub async fn pay(
    state: &AppState,
    bill: &SupplierBill,
    source: &Subject,
    amount: i64,
    ran_by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    let mut tx = state.db.begin().await?;
    let outcome = pay_in(&mut tx, bill, source, amount, ran_by).await?;
    tx.commit().await?;
    outcome.emit(state).await.map_err(TransitionError::Failed)
}

/// [`pay`] inside `tx`, which the caller commits: the same guards and
/// effect, with `bill` as the caller loaded it in `tx` (its balance is
/// checked and updated only from that value). Emit the returned outcome
/// after the commit.
///
/// # Errors
///
/// The errors of [`pay`]; the caller rolls `tx` back.
pub async fn pay_in(
    tx: &mut renox::db::Transaction,
    bill: &SupplierBill,
    source: &Subject,
    amount: i64,
    ran_by: Option<i64>,
) -> std::result::Result<Outcome<SupplierBill>, TransitionError> {
    pay_keyed_in(tx, bill, source, amount, ran_by, None).await
}

/// [`pay_in`] with an idempotency `key`, stored on the allocation (unique:
/// a second payment under the same key fails with a unique violation, which
/// the caller reads as `skipped`).
///
/// # Errors
///
/// The errors of [`pay`]; the caller rolls `tx` back.
pub async fn pay_keyed_in(
    tx: &mut renox::db::Transaction,
    bill: &SupplierBill,
    source: &Subject,
    amount: i64,
    ran_by: Option<i64>,
    key: Option<String>,
) -> std::result::Result<Outcome<SupplierBill>, TransitionError> {
    let input = PayInput {
        source: source.clone(),
        amount,
        ran_by,
        key,
    };
    let meta = match ran_by {
        Some(user) => renox::serde_json::json!({ "ran_by": user, "source": source.to_string() }),
        None => renox::serde_json::json!({ "source": source.to_string() }),
    };
    bill.transition(transition_for(amount, bill.balance))
        .by_system()
        .input(&input)
        .meta(meta)
        .run_in(tx)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approved(balance: i64) -> SupplierBill {
        SupplierBill {
            id: 1,
            currency: "AED".into(),
            gross_total: 1_050_000,
            balance,
            status: SupplierBillStatus::Approved,
            ..Default::default()
        }
    }

    fn run(amount: i64) -> TransitionCtx {
        let input = PayInput {
            source: Subject::new("payments.payment", 3),
            amount,
            ran_by: None,
            key: None,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn both_go_from_the_open_states_and_are_system_only() {
        let machine = SupplierBill::machine();
        for (name, to) in [(PAY, "partially_paid"), (PAY_IN_FULL, "paid")] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(transition.from_states(), ["approved", "partially_paid"]);
            assert_eq!(transition.to_state(), Some(to));
            assert!(transition.is_system());
        }
    }

    #[test]
    fn the_resulting_balance_picks_the_transition() {
        assert_eq!(transition_for(600_000, 1_050_000), PAY);
        assert_eq!(transition_for(1_050_000, 1_050_000), PAY_IN_FULL);
        // Above the balance: `pay`, whose guard refuses it.
        assert_eq!(transition_for(2_000_000, 1_050_000), PAY);
    }

    #[renox::test]
    async fn the_guards_refuse_nothing_negative_or_too_much() {
        let machine = SupplierBill::machine();
        let partial = machine.find(PAY).expect("pay");
        let full = machine.find(PAY_IN_FULL).expect("in full");
        let bill = approved(1_050_000);
        assert!(partial.check_guards(&bill, &run(600_000)).await.is_ok());
        assert!(full.check_guards(&bill, &run(1_050_000)).await.is_ok());
        for amount in [0, -100, 1_050_001] {
            assert!(matches!(
                partial.check_guards(&bill, &run(amount)).await,
                Err(TransitionError::Guard(_))
            ));
        }
        assert!(matches!(
            partial.check_guards(&bill, &run(1_050_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            full.check_guards(&bill, &run(600_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            partial.check_guards(&bill, &TransitionCtx::new(None)).await,
            Err(TransitionError::Guard(_))
        ));
    }
}
