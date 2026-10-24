//! A dispatch discarded by wl-delivery (wl-inventory §4 *Dispatch* row
//! `discard`, issue #242): when wl-delivery destroys perishable goods after a
//! failed attempt (`DeliveryDiscarded`, `WRK.RUL.16`) the dispatch goes from
//! `dispatched` to `discarded` (terminal) and its value goes to wastage.
//!
//! - The system runs `discard`; there is no user twin, the approval of the
//!   wastage (`STK.ACL.02`) was given in wl-delivery and is not asked again.
//! - **No stock movement.** The goods already left with the `dispatch`
//!   movements. The value is the value of the dispatched lines at the cost
//!   they left stock at, kept on the dispatch (`discard_fils`) and posted as
//!   wastage through the cost poster after the commit, once per dispatch (the
//!   key names the dispatch), with the dispatch as source and cost object.
//! - The reason and the photo the driver gave are kept on the dispatch and
//!   carried by `DispatchDiscarded`: the cost poster takes an amount, a key
//!   and a code only, so the reason and the photo link are read from the
//!   dispatch the cost is posted against.
//! - A repeated event does nothing: the dispatch is `discarded` already.
//!   `DeliveryDiscarded` for a dispatch that is not `dispatched` changes
//!   nothing either.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::DeliveryDiscarded;
use wl_kernel::inventory::DispatchDiscarded;
use wl_kernel::invoicing::{PostCost, post_cost};

use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, lines_of};
use crate::dispatch_events::{dispatch_of, event_lines, recipient};
use crate::item::value_of;

/// The system's `discard` transition's name.
pub const DISCARD: &str = "discard";

/// The cost code of the wastage of a discarded dispatch.
pub const WASTAGE_COST_CODE: &str = "wastage";

/// What a `discard` run carries to its effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DiscardInput {
    /// The reason the driver gave.
    pub reason: String,
    /// The photo of the discarded goods.
    pub photo: Option<String>,
}

fn discard_input(ctx: &TransitionCtx) -> DiscardInput {
    ctx.input_as::<DiscardInput>().unwrap_or_default()
}

/// The value of what was dispatched, at the cost it left stock at, in fils.
pub fn discarded_value(lines: &[DispatchLine]) -> i64 {
    lines
        .iter()
        .map(|line| value_of(line.quantity_milli, line.unit_cost))
        .sum()
}

async fn discard_in(tx: &mut Transaction, dispatch: Dispatch, run: DiscardInput) -> Result {
    let lines = lines_of(&mut *tx, dispatch.id).await?;
    let value = discarded_value(&lines);
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(
            &mut *tx,
            &[
                ("discard_reason", &Some(run.reason)),
                ("discard_photo", &run.photo),
                ("discard_fils", &value),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// `discard`.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    let discard = Transition::new(DISCARD)
        .label("Discard")
        .tone("danger")
        .system()
        .from([DispatchStatus::Dispatched])
        .to(DispatchStatus::Discarded)
        .effect(|dispatch: &Dispatch, ctx, tx| {
            let dispatch = dispatch.clone();
            let run = discard_input(ctx);
            Box::pin(async move { discard_in(tx, dispatch, run).await })
        });
    machine.transition(crate::search::reindex_dispatch(discard))
}

/// Discards `dispatch` as the system. Does nothing, and answers `false`,
/// when it is not `dispatched`.
///
/// # Errors
///
/// A database error.
pub async fn discard(
    state: &AppState,
    dispatch: &Dispatch,
    reason: &str,
    photo: Option<String>,
) -> Result<bool> {
    if dispatch.status != DispatchStatus::Dispatched {
        return Ok(false);
    }
    let run = DiscardInput {
        reason: reason.trim().to_owned(),
        photo,
    };
    let _outcome = dispatch
        .transition(DISCARD)
        .by_system()
        .input(&run)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// The cost key of the wastage of `dispatch`: one per dispatch.
fn wastage_key(dispatch: &Dispatch) -> String {
    format!("inventory.dispatch_discard:{}", dispatch.id)
}

/// Registers the listeners: the wastage posted and `DispatchDiscarded`
/// emitted after `discard`, and the event of `wl-delivery` that asks for it.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Dispatch>, state: AppState| async move {
            if event.transition != DISCARD {
                return Ok(());
            }
            let dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            if dispatch.discard_fils > 0 {
                let request = PostCost::new(
                    dispatch.subject(),
                    wastage_key(&dispatch),
                    dispatch.subject(),
                    dispatch.discard_fils,
                )
                .cost_code(WASTAGE_COST_CODE);
                post_cost(&state, request).await?;
            }
            let lines = lines_of(&state.db, dispatch.id).await?;
            state
                .emit(DispatchDiscarded {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    reason: dispatch.discard_reason.clone().unwrap_or_default(),
                    photo: dispatch.discard_photo.clone(),
                    value: dispatch.discard_fils,
                    lines: event_lines(&lines),
                })
                .await
        },
    );
    app.listen(|event: DeliveryDiscarded, state: AppState| async move {
        match dispatch_of(&state, &event.source).await? {
            Some(dispatch) => discard(&state, &dispatch, &event.reason, event.photo.clone())
                .await
                .map(|_| ()),
            None => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dispatched_dispatch_is_discarded_by_the_system_and_ends() {
        let machine = Dispatch::machine();
        let discard = machine.find(DISCARD).expect("discard");
        assert_eq!(discard.from_states(), ["dispatched"]);
        assert_eq!(discard.to_state(), Some("discarded"));
        assert!(discard.is_system());
    }

    #[test]
    fn the_wastage_is_the_whole_value_that_left() {
        let line = DispatchLine {
            quantity_milli: 10_000,
            refused_milli: 2_000,
            unit_cost: 1_800,
            ..DispatchLine::default()
        };
        // 10 units at AED 1.80, refused or not: all of it is destroyed.
        assert_eq!(discarded_value(&[line]), 18_000);
    }
}
