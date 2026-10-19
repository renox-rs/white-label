//! Allocating money to an invoice (wl-invoicing §4 `allocate`).
//!
//! The spec's one action has two targets, and a Renox transition has one
//! `to`, so there are two system transitions from `issued`,
//! `partially_paid` and `overdue`:
//!
//! - `allocate` → `partially_paid`, for an amount that leaves a balance;
//! - `allocate_in_full` → `paid`, for an amount equal to the balance.
//!
//! [`allocate`] picks one by the resulting balance. Both refuse an amount
//! ≤ 0 or above the balance. The effect writes the [`Allocation`] (its
//! source a payment or a credit note) and the invoice's allocated total and
//! balance in the transition's transaction; the balance is updated only
//! from the one the run loaded, so two allocations at once never both pass
//! (the second fails with a conflict). `InvoicePaid` is emitted after the
//! commit of `allocate_in_full` ([`crate::events`]).

use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::invoicing::InvoiceStatus;

use crate::allocation::Allocation;
use crate::invoice::Invoice;

/// The transition that leaves a balance.
pub const ALLOCATE: &str = "allocate";
/// The transition that settles the balance.
pub const ALLOCATE_IN_FULL: &str = "allocate_in_full";

/// What an allocation run carries to its guards and effect.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AllocateInput {
    /// Where the money comes from: a payment or a credit note.
    pub source: Subject,
    /// The amount, in fils of the invoice's currency.
    pub amount: i64,
    /// The user whose action caused it; `None` for the system.
    pub ran_by: Option<i64>,
}

/// `fils` as `AED 1,050.00`.
pub(crate) fn amount_text(currency: &str, fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let fils = fils.unsigned_abs();
    let units = (fils / 100).to_string();
    let mut grouped = String::new();
    for (i, digit) in units.chars().enumerate() {
        if i > 0 && (units.len() - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{currency} {sign}{grouped}.{:02}", fils % 100)
}

/// The `allocate` and `allocate_in_full` transitions, for the machine.
pub(crate) fn transitions() -> [Transition<Invoice>; 2] {
    [
        rules(
            Transition::new(ALLOCATE)
                .label("Allocate")
                .to(InvoiceStatus::PartiallyPaid),
        )
        .guard("leaves_a_balance", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount < invoice.balance => Ok(()),
                    _ => Err("The amount settles the balance: allocate it in full.".to_owned()),
                }
            })
        }),
        rules(
            Transition::new(ALLOCATE_IN_FULL)
                .label("Allocate in full")
                .to(InvoiceStatus::Paid),
        )
        .guard("settles_the_balance", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount == invoice.balance => Ok(()),
                    _ => Err("The amount does not settle the balance.".to_owned()),
                }
            })
        }),
    ]
}

/// What both transitions share: system only, the open states, the amount
/// guards and the effect.
fn rules(transition: Transition<Invoice>) -> Transition<Invoice> {
    transition
        .system()
        .from([
            InvoiceStatus::Issued,
            InvoiceStatus::PartiallyPaid,
            InvoiceStatus::Overdue,
        ])
        .guard("positive_amount", |_invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount > 0 => Ok(()),
                    _ => Err("The amount must be more than 0.".to_owned()),
                }
            })
        })
        .guard("within_balance", |invoice: &Invoice, ctx| {
            Box::pin(async move {
                match input(ctx) {
                    Some(input) if input.amount <= invoice.balance => Ok(()),
                    _ => Err(format!(
                        "The amount is above the balance of {}.",
                        amount_text(&invoice.currency, invoice.balance)
                    )),
                }
            })
        })
        .effect(|invoice: &Invoice, ctx, tx| {
            let (id, balance) = (invoice.id, invoice.balance);
            Box::pin(async move {
                // The guards passed, so the input is there.
                let input = input(ctx).ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!(
                        "an allocation run without its input"
                    ))
                })?;
                // Only from the balance this run checked: a concurrent
                // allocation moved it, so this one must not pass as well.
                // Not the status column, so outside the machine's guard.
                let moved = renox::db::sql(format!(
                    "UPDATE {} SET allocated_total = allocated_total + ?, balance = balance - ?, \
                     updated_at = ? WHERE id = ? AND balance = ?",
                    Invoice::TABLE
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
                        "The invoice's balance changed; try again.".to_owned(),
                    ));
                }
                Allocation::new(id, &input.source, input.amount, input.ran_by)
                    .insert(&mut *tx)
                    .await?;
                Ok(())
            })
        })
}

/// The run's input, if it is an [`AllocateInput`].
fn input(ctx: &TransitionCtx) -> Option<AllocateInput> {
    ctx.input_as::<AllocateInput>().ok()
}

/// The transition that allocates `amount` to an invoice with `balance`.
fn transition_for(amount: i64, balance: i64) -> &'static str {
    if amount == balance {
        ALLOCATE_IN_FULL
    } else {
        ALLOCATE
    }
}

/// Allocates `amount` fils from `source` (a payment or a credit note) to
/// `invoice`, as the system, in its own transaction: `allocate_in_full`
/// when it settles the balance, else `allocate`. `ran_by` is the user whose
/// action caused it, stored on the allocation and in the history row's
/// `meta`.
///
/// # Errors
///
/// `NotFrom` (422) when the invoice is not `issued`, `partially_paid` or
/// `overdue`; `Guard` (422) for an amount ≤ 0 or above the balance; a 422
/// when another allocation changed the balance first. Nothing is written
/// then.
pub async fn allocate(
    state: &AppState,
    invoice: &Invoice,
    source: &Subject,
    amount: i64,
    ran_by: Option<i64>,
) -> std::result::Result<(), TransitionError> {
    let mut tx = state.db.begin().await?;
    let outcome = allocate_in(&mut tx, invoice, source, amount, ran_by).await?;
    tx.commit().await?;
    // After the commit; `InvoicePaid` follows from `allocate_in_full`.
    outcome.emit(state).await.map_err(TransitionError::Failed)
}

/// [`allocate`] inside `tx`, which the caller commits: the same guards and
/// effect, with `invoice` as the caller loaded it in `tx` (its balance is
/// checked and updated only from that value). Emit the returned outcome
/// after the commit.
///
/// # Errors
///
/// The errors of [`allocate`]; the caller rolls `tx` back.
pub async fn allocate_in(
    tx: &mut renox::db::Transaction,
    invoice: &Invoice,
    source: &Subject,
    amount: i64,
    ran_by: Option<i64>,
) -> std::result::Result<Outcome<Invoice>, TransitionError> {
    let input = AllocateInput {
        source: source.clone(),
        amount,
        ran_by,
    };
    let meta = match ran_by {
        Some(user) => renox::serde_json::json!({ "ran_by": user, "source": source.to_string() }),
        None => renox::serde_json::json!({ "source": source.to_string() }),
    };
    invoice
        .transition(transition_for(amount, invoice.balance))
        .by_system()
        .input(&input)
        .meta(meta)
        .run_in(tx)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issued(balance: i64) -> Invoice {
        Invoice {
            id: 1,
            currency: "AED".into(),
            gross_total: 105_000,
            balance,
            status: InvoiceStatus::Issued,
            ..Default::default()
        }
    }

    fn run(amount: i64) -> TransitionCtx {
        let input = AllocateInput {
            source: Subject::new("payments.payment", 3),
            amount,
            ran_by: None,
        };
        TransitionCtx::new(None).with_input(renox::serde_json::to_value(input).expect("json"))
    }

    #[test]
    fn both_go_from_the_open_states_and_are_system_only() {
        let machine = Invoice::machine();
        for (name, to) in [(ALLOCATE, "partially_paid"), (ALLOCATE_IN_FULL, "paid")] {
            let transition = machine.find(name).expect("transition");
            assert_eq!(
                transition.from_states(),
                ["issued", "partially_paid", "overdue"]
            );
            assert_eq!(transition.to_state(), Some(to));
            assert!(transition.is_system());
        }
    }

    #[test]
    fn the_resulting_balance_picks_the_transition() {
        assert_eq!(transition_for(42_000, 105_000), ALLOCATE);
        assert_eq!(transition_for(105_000, 105_000), ALLOCATE_IN_FULL);
        // Above the balance: `allocate`, whose guard refuses it.
        assert_eq!(transition_for(200_000, 105_000), ALLOCATE);
    }

    #[renox::test]
    async fn the_guards_refuse_nothing_negative_or_too_much() {
        let machine = Invoice::machine();
        let partial = machine.find(ALLOCATE).expect("allocate");
        let full = machine.find(ALLOCATE_IN_FULL).expect("in full");
        let invoice = issued(105_000);
        assert!(partial.check_guards(&invoice, &run(42_000)).await.is_ok());
        assert!(full.check_guards(&invoice, &run(105_000)).await.is_ok());
        for amount in [0, -100, 105_001] {
            assert!(matches!(
                partial.check_guards(&invoice, &run(amount)).await,
                Err(TransitionError::Guard(_))
            ));
        }
        assert!(matches!(
            partial.check_guards(&invoice, &run(105_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            full.check_guards(&invoice, &run(42_000)).await,
            Err(TransitionError::Guard(_))
        ));
        assert!(matches!(
            partial
                .check_guards(&invoice, &TransitionCtx::new(None))
                .await,
            Err(TransitionError::Guard(_))
        ));
    }

    #[test]
    fn amounts_read_as_currency_and_grouped_units() {
        assert_eq!(amount_text("AED", 105_000), "AED 1,050.00");
        assert_eq!(amount_text("AED", 33_333), "AED 333.33");
        assert_eq!(amount_text("AED", 5), "AED 0.05");
        assert_eq!(amount_text("USD", 123_456_789), "USD 1,234,567.89");
        assert_eq!(amount_text("AED", -1_000), "AED -10.00");
    }
}
