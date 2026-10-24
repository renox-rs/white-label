//! Reopening a delivered dispatch (wl-inventory §4 *Dispatch* row `reverse`,
//! issue #244): when an approved delivery reversal says the goods were not
//! really delivered (`DeliveryReversed` of `wl-delivery`, approved there
//! under `WRK.ACL.10`) the dispatch goes back from `delivered` to
//! `dispatched`:
//!
//! - the delivered quantity of every line goes back to zero;
//! - the refused quantity stays as it was, *in return* or already received
//!   into quarantine, and its movements are not undone: a later
//!   `confirm_delivery` takes only the remaining quantity;
//! - the original proof is kept and marked reversed;
//! - `DispatchReversed` follows the commit, so the source reverses its
//!   fulfilment ([`crate::dispatch_events`]).
//!
//! The dispatch then ends by a new `confirm_delivery`, `return`, `discard` or
//! `mark_lost` from the delivery. A reversal for a dispatch that is not
//! `delivered` does nothing.

use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError, Transitioned};
use wl_kernel::delivery::DeliveryReversed;
use wl_kernel::inventory::DispatchReversed;

use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, lines_of};
use crate::dispatch_events::{dispatch_of, event_lines, recipient};

/// The system's `reverse` transition's name.
pub const REVERSE: &str = "reverse";

/// `reverse`.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    let reverse = Transition::new(REVERSE)
        .label("Reverse the delivery")
        .tone("danger")
        .system()
        .from([DispatchStatus::Delivered])
        .to(DispatchStatus::Dispatched)
        .effect(|dispatch: &Dispatch, _ctx, tx| {
            let id = dispatch.id;
            Box::pin(async move {
                let now = renox::db::now();
                for line in lines_of(&mut *tx, id).await? {
                    DispatchLine::query()
                        .where_(DispatchLine::ID.eq(line.id))
                        .update(
                            &mut *tx,
                            &[("delivered_milli", &0_i64), ("updated_at", &now)],
                        )
                        .await?;
                }
                Dispatch::query()
                    .where_(Dispatch::ID.eq(id))
                    .update(&mut *tx, &[("proof_reversed", &true), ("updated_at", &now)])
                    .await?;
                Ok(())
            })
        });
    machine.transition(crate::search::reindex_dispatch(reverse))
}

/// Reopens `dispatch` as the system. Does nothing, and answers `false`, when
/// it is not `delivered`.
///
/// # Errors
///
/// A database error.
pub async fn reverse(state: &AppState, dispatch: &Dispatch) -> Result<bool> {
    if dispatch.status != DispatchStatus::Delivered {
        return Ok(false);
    }
    let _outcome = dispatch
        .transition(REVERSE)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// Registers the listeners: `DispatchReversed` after `reverse`, and the
/// reversal of `wl-delivery` that asks for it.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Dispatch>, state: AppState| async move {
            if event.transition != REVERSE {
                return Ok(());
            }
            let dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            let lines = lines_of(&state.db, dispatch.id).await?;
            state
                .emit(DispatchReversed {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    lines: event_lines(&lines),
                })
                .await
        },
    );
    app.listen(|event: DeliveryReversed, state: AppState| async move {
        match dispatch_of(&state, &event.source).await? {
            Some(dispatch) => reverse(&state, &dispatch).await.map(|_| ()),
            None => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delivered_dispatch_is_reopened_to_dispatched_by_the_system() {
        let machine = Dispatch::machine();
        let reverse = machine.find(REVERSE).expect("reverse");
        assert_eq!(reverse.from_states(), ["delivered"]);
        assert_eq!(reverse.to_state(), Some("dispatched"));
        assert!(reverse.is_system());
    }
}
