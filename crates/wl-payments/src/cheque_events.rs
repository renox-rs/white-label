//! The kernel events and follow-ups the Cheque machine gives (wl-payments
//! §6), from a listener on [`Transitioned<Cheque>`], so only after a commit
//! and never on a rollback:
//!
//! - `bounce` → the bounce fee, [`ChequeBounced`], the notices and the tasks
//!   ([`crate::cheque_bounce::after_bounce`]).
//! - `clear` after a re-presentation (the cheque was deposited more than
//!   once) → [`ChequeBounceResolved`] (cleared); a replacement cheque that
//!   clears also lets the original it replaces be handed back
//!   ([`crate::cheque_settle::after_clear`]).
//! - `replace` → [`ChequeBounceResolved`] (replaced), the legal case task
//!   closed and the hand-back task ([`crate::cheque_settle::after_replace`]).
//! - `represent` → the party is told the new presentation date.
//! - `refer_to_legal` → the case owner's task, the formal notice and the
//!   manager's notice ([`crate::cheque_legal::after_referral`]).
//!
//! The module never changes another module's record; wl-sales reacts to the
//! events.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::cheques::{ChequeBounceResolved, ChequeId, ChequeOutcome};

use crate::cheque::Cheque;
use crate::cheque_bounce::{self, BOUNCE};
use crate::cheque_deposit::CLEAR;
use crate::cheque_represent::{self, REPRESENT};

/// The event that says `cheque` was resolved as `outcome`, with the other
/// bounced cheques of the party still open.
pub(crate) async fn resolved(
    state: &AppState,
    cheque: &Cheque,
    outcome: ChequeOutcome,
) -> Result<()> {
    let others_open: Vec<ChequeId> = Cheque::owed_of_party(&state.db, cheque.party(), Some(cheque.id))
        .await?
        .iter()
        .map(Cheque::cheque_id)
        .collect();
    state
        .emit(ChequeBounceResolved {
            cheque: cheque.cheque_id(),
            party: cheque.party(),
            outcome,
            others_open,
        })
        .await
}

/// Registers the listener.
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: Transitioned<Cheque>, state: AppState| async move {
        let name = event.transition.as_str();
        if name == BOUNCE {
            return cheque_bounce::after_bounce(&state, event.key).await;
        }
        if name == REPRESENT {
            return cheque_represent::after_represent(&state, event.key).await;
        }
        if name == CLEAR {
            let cheque = Cheque::find_or_404(&state.db, event.key).await?;
            if cheque.presentations > 1 {
                resolved(&state, &cheque, ChequeOutcome::Cleared).await?;
            }
        }
        Ok(())
    });
}
