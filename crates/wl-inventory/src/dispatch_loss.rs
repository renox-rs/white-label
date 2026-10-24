//! A lost dispatch (wl-inventory §4 *Dispatch* rows `mark_lost` and `found`,
//! issue #243): when `wl-delivery` marks a delivery lost (`DeliveryLost`) its
//! dispatch goes `lost`, also when it was already `delivered` (the delivery
//! was under review), and its value is held as a loss while the claim runs;
//! when the goods are found (`DeliveryFound`) the loss is reversed and the
//! dispatch is `dispatched` again to finish its delivery.
//!
//! - `mark_lost` (the system; from `dispatched` or `delivered`): the value of
//!   the lost lines (the quantity that left and was not refused, at the cost it
//!   left stock at) is kept on the dispatch as the loss pending the claim.
//!   From `delivered` the delivered quantity goes back to zero; the refused
//!   quantity stays *in return* or in quarantine. No stock movement is posted:
//!   the goods already left. After the commit the loss is posted through the
//!   cost poster and `DispatchLost` is emitted ([`crate::dispatch_events`]);
//!   from `delivered` the source reverses its fulfilment.
//! - `found` (the system; from `lost`): the loss is no longer open and the
//!   cost poster takes it back.
//!
//! Idempotency: a `DeliveryLost` for a dispatch that is lost already, and a
//! `DeliveryFound` for one that is not lost, do nothing. The cost posts under
//! one key per loss (the dispatch and the number of times it was lost), so a
//! repeat posts nothing twice.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::delivery::{DeliveryFound, DeliveryLost};
use wl_kernel::inventory::DispatchLost;
use wl_kernel::invoicing::{PostCost, ReverseCost, cost_poster, post_cost};

use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, lines_of};
use crate::dispatch_events::{dispatch_of, event_lines, recipient};
use crate::item::value_of;

/// The system's `mark_lost` transition's name.
pub const MARK_LOST: &str = "mark_lost";

/// The system's `found` transition's name.
pub const FOUND: &str = "found";

/// What a `mark_lost` run carries to its effect.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LostInput {
    /// Whether the dispatch was `delivered` (under review) when it was lost.
    pub was_delivered: bool,
}

fn lost_input(ctx: &TransitionCtx) -> LostInput {
    ctx.input_as::<LostInput>().unwrap_or_default()
}

/// The value of what was lost: the quantity that left and was not refused,
/// at the cost it left stock at, in fils.
pub fn lost_value(lines: &[DispatchLine]) -> i64 {
    lines
        .iter()
        .map(|line| value_of(line.accepted_candidate_milli(), line.unit_cost))
        .sum()
}

/// The effect of `mark_lost`: the loss is held, the delivered quantity goes
/// back to zero.
async fn lost_in(tx: &mut Transaction, dispatch: Dispatch, run: LostInput) -> Result {
    let now = renox::db::now();
    let lines = lines_of(&mut *tx, dispatch.id).await?;
    let value = lost_value(&lines);
    if run.was_delivered {
        for line in &lines {
            DispatchLine::query()
                .where_(DispatchLine::ID.eq(line.id))
                .update(
                    &mut *tx,
                    &[("delivered_milli", &0_i64), ("updated_at", &now)],
                )
                .await?;
        }
    }
    let from = if run.was_delivered {
        DispatchStatus::Delivered
    } else {
        DispatchStatus::Dispatched
    };
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(
            &mut *tx,
            &[
                ("lost_from", &Some(from.as_str().to_owned())),
                ("loss_fils", &value),
                ("loss_count", &(dispatch.loss_count + 1)),
                ("loss_open", &true),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// `mark_lost` and `found`.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    use DispatchStatus::{Delivered, Dispatched, Lost};
    let lost = Transition::new(MARK_LOST)
        .label("Mark lost")
        .tone("danger")
        .system()
        .from([Dispatched, Delivered])
        .to(Lost)
        .effect(|dispatch: &Dispatch, ctx, tx| {
            let dispatch = dispatch.clone();
            let run = lost_input(ctx);
            Box::pin(async move { lost_in(tx, dispatch, run).await })
        });
    let found = Transition::new(FOUND)
        .label("Found")
        .system()
        .from([Lost])
        .to(Dispatched)
        .effect(|dispatch: &Dispatch, _ctx, tx| {
            let id = dispatch.id;
            Box::pin(async move {
                let now = renox::db::now();
                Dispatch::query()
                    .where_(Dispatch::ID.eq(id))
                    .update(&mut *tx, &[("loss_open", &false), ("updated_at", &now)])
                    .await?;
                Ok(())
            })
        });
    machine.transition(lost).transition(found)
}

/// Marks `dispatch` lost as the system. Does nothing, and answers `false`,
/// when it is not `dispatched` or `delivered`.
///
/// # Errors
///
/// A database error.
pub async fn mark_lost(state: &AppState, dispatch: &Dispatch) -> Result<bool> {
    if !matches!(
        dispatch.status,
        DispatchStatus::Dispatched | DispatchStatus::Delivered
    ) {
        return Ok(false);
    }
    let run = LostInput {
        was_delivered: dispatch.status == DispatchStatus::Delivered,
    };
    let _outcome = dispatch
        .transition(MARK_LOST)
        .by_system()
        .input(&run)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// The goods of a lost `dispatch` were found: it is `dispatched` again and
/// the loss is reversed. Does nothing, and answers `false`, when it is not
/// `lost`.
///
/// # Errors
///
/// A database error.
pub async fn found(state: &AppState, dispatch: &Dispatch) -> Result<bool> {
    if dispatch.status != DispatchStatus::Lost {
        return Ok(false);
    }
    let _outcome = dispatch
        .transition(FOUND)
        .by_system()
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// The cost code of a loss held while the claim runs.
pub const LOSS_COST_CODE: &str = "loss_pending_claim";

/// The cost key of the loss number `loss_count` of `dispatch`; `what` is
/// `loss` when it is held and `found` when it is taken back.
fn loss_key(dispatch: &Dispatch, what: &str) -> String {
    format!(
        "inventory.dispatch_{what}:{}:{}",
        dispatch.id, dispatch.loss_count
    )
}

/// Registers the listeners: the loss posted and `DispatchLost` emitted after
/// `mark_lost`, the loss taken back after `found`, and the events of
/// `wl-delivery` that lose and find a delivery.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Dispatch>, state: AppState| async move {
            let name = event.transition.as_str();
            if ![MARK_LOST, FOUND].contains(&name) {
                return Ok(());
            }
            let dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            if name == FOUND {
                if dispatch.loss_fils > 0 {
                    let request =
                        ReverseCost::new(dispatch.subject(), loss_key(&dispatch, "found"))
                            .line(dispatch.subject(), dispatch.loss_fils);
                    cost_poster(&state).reverse(&state, request).await?;
                }
                return Ok(());
            }
            if dispatch.loss_fils > 0 {
                let request = PostCost::new(
                    dispatch.subject(),
                    loss_key(&dispatch, "loss"),
                    dispatch.subject(),
                    dispatch.loss_fils,
                )
                .cost_code(LOSS_COST_CODE);
                post_cost(&state, request).await?;
            }
            let lines = lines_of(&state.db, dispatch.id).await?;
            state
                .emit(DispatchLost {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    was_delivered: dispatch.lost_from.as_deref()
                        == Some(DispatchStatus::Delivered.as_str()),
                    value: dispatch.loss_fils,
                    lines: event_lines(&lines),
                })
                .await
        },
    );
    app.listen(|event: DeliveryLost, state: AppState| async move {
        match dispatch_of(&state, &event.source).await? {
            Some(dispatch) => mark_lost(&state, &dispatch).await.map(|_| ()),
            None => Ok(()),
        }
    });
    app.listen(|event: DeliveryFound, state: AppState| async move {
        match dispatch_of(&state, &event.source).await? {
            Some(dispatch) => found(&state, &dispatch).await.map(|_| ()),
            None => Ok(()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dispatch_is_lost_from_dispatched_or_delivered_and_found_back_to_dispatched() {
        let machine = Dispatch::machine();
        let lost = machine.find(MARK_LOST).expect("mark_lost");
        assert_eq!(lost.from_states(), ["dispatched", "delivered"]);
        assert_eq!(lost.to_state(), Some("lost"));
        let found = machine.find(FOUND).expect("found");
        assert_eq!(found.from_states(), ["lost"]);
        assert_eq!(found.to_state(), Some("dispatched"));
        assert!(lost.is_system() && found.is_system());
    }

    #[test]
    fn the_loss_is_what_left_and_was_not_refused() {
        let line = DispatchLine {
            quantity_milli: 10_000,
            refused_milli: 2_000,
            unit_cost: 5_000,
            ..DispatchLine::default()
        };
        // 8 units at AED 5.00.
        assert_eq!(lost_value(&[line]), 40_000);
    }
}
