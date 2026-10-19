//! The kernel events the Deposit machine gives (wl-payments §6), from a
//! listener on [`Transitioned<Deposit>`], so only after a commit and never on a
//! rollback:
//!
//! - `adjust` and `adjust_by_system` → the top-up charge and `DepositAdjusted`
//!   ([`crate::deposit_adjust::after_adjust`]).
//! - `refund`, `forfeit` and `transfer` → [`DepositSettled`] with the outcome,
//!   the deductions of the latest published version, the refund amount and the
//!   new holder of a transferred deposit; a transfer also tells the party who
//!   now holds it.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::PartyId;
use wl_kernel::deposits::{DepositOutcome, DepositSettled};

use crate::deposit::Deposit;
use crate::deposit_deductions::DepositDeduction;
use crate::deposit_adjust::{ADJUST, ADJUST_BY_SYSTEM, after_adjust};
use crate::deposit_machine::{FORFEIT, REFUND, TRANSFER};
use crate::notices;

/// The outcome the transition `name` settles a deposit with, if it does.
pub(crate) fn outcome_of(name: &str) -> Option<DepositOutcome> {
    match name {
        REFUND => Some(DepositOutcome::Refunded),
        FORFEIT => Some(DepositOutcome::Forfeited),
        TRANSFER => Some(DepositOutcome::Transferred),
        _ => None,
    }
}

/// Emits `DepositSettled` for the deposit `deposit_id` settled as `outcome`.
pub(crate) async fn settled(
    state: &AppState,
    deposit_id: i64,
    outcome: DepositOutcome,
) -> Result<()> {
    let deposit = Deposit::find_or_404(&state.db, deposit_id).await?;
    let deductions = DepositDeduction::latest(&state.db, deposit.id)
        .await?
        .map_or(0, |d| d.total);
    let refund_amount = match outcome {
        DepositOutcome::Refunded => deposit
            .linked_refund(&state.db)
            .await?
            .map_or(0, |refund| refund.amount),
        _ => 0,
    };
    state
        .emit(DepositSettled {
            deposit: deposit.deposit_id(),
            source: deposit.source(),
            party: deposit.party(),
            outcome,
            deductions,
            refund_amount,
            new_holder: deposit.transferred_to.map(PartyId),
        })
        .await?;
    if outcome == DepositOutcome::Transferred {
        notices::deposit_transferred(state, &deposit).await?;
    }
    Ok(())
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Deposit>, state: AppState| async move {
        if event.transition == ADJUST || event.transition == ADJUST_BY_SYSTEM {
            return after_adjust(&state, event.key).await;
        }
        match outcome_of(event.transition.as_str()) {
            Some(outcome) => settled(&state, event.key, outcome).await,
            None => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refund_forfeit_and_transfer_settle_a_deposit() {
        assert_eq!(outcome_of("refund"), Some(DepositOutcome::Refunded));
        assert_eq!(outcome_of("forfeit"), Some(DepositOutcome::Forfeited));
        assert_eq!(outcome_of("transfer"), Some(DepositOutcome::Transferred));
        assert_eq!(outcome_of("start_settlement"), None);
    }
}
