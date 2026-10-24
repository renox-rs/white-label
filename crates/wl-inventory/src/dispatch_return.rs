//! A dispatch that came back (wl-inventory §4 *Dispatch* row `return`, §5.4):
//! the customer refused it whole or it was brought back, so it goes from
//! `dispatched` to `returned` (terminal).
//!
//! - **Who:** a `dispatch.manage` holder at the location with a reason
//!   (`return`), or the system when wl-delivery says the delivery came back
//!   (`DeliveryReturned`, `return_by_delivery`; the reason is the delivery's
//!   last failure reason).
//! - **Effect:** a `return_in` movement per line puts the whole quantity into
//!   **quarantine** (never free stock), with the picked lot and the reason; the
//!   reason is kept on the dispatch.
//! - **After the commit:** a complaint ticket is opened for the dispatch
//!   (task category, requester the customer's party when there is one) and its
//!   id kept; `DispatchReturned` carries the ticket and the lines, and
//!   `DispatchGoodsQuarantined` says which lots are now in quarantine, so a
//!   vertical can inspect them (07 `MFG.RUL.10`). The source keeps its
//!   reservation request and tells the rep and the customer (wl-sales).
//! - A `DeliveryReturned` for a dispatch that is not `dispatched`, or one that
//!   returned already, changes nothing.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError, Transitioned};
use wl_kernel::delivery::DeliveryReturned;
use wl_kernel::inventory::{
    DispatchGoodsQuarantined, DispatchReturned, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE,
    QuarantinedLine,
};
use wl_kernel::tickets::{OpenTicket, TASK_CATEGORY, task_creator};

use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, lines_of};
use crate::dispatch_events::{dispatch_of, event_lines, recipient};
use crate::item::value_of;
use crate::location;
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::texts::said;

/// The user's `return` transition's name.
pub const RETURN: &str = "return";

/// The system's `return` transition's name (`DeliveryReturned`).
pub const RETURN_BY_DELIVERY: &str = "return_by_delivery";

/// The effect the two transitions share: the goods into quarantine, the
/// reason kept.
fn rules(transition: Transition<Dispatch>) -> Transition<Dispatch> {
    transition
        .from([DispatchStatus::Dispatched])
        .to(DispatchStatus::Returned)
        .effect(|dispatch: &Dispatch, ctx, tx| {
            let dispatch = dispatch.clone();
            let reason = ctx.reason().map(str::to_owned).unwrap_or_default();
            let by = ctx.actor().map(|user| user.id);
            Box::pin(async move { return_in(tx, dispatch, reason, by).await })
        })
}

async fn return_in(
    tx: &mut Transaction,
    dispatch: Dispatch,
    reason: String,
    by: Option<i64>,
) -> Result {
    let now = renox::db::now();
    let quarantine = location::quarantine_location(&mut *tx).await?;
    for line in lines_of(&mut *tx, dispatch.id).await? {
        // What the customer already took (a delivery that was reversed) is
        // not on the way back; a returned dispatch brings back what was sent
        // less what is in quarantine already as refused.
        let quantity = (line.quantity_milli - line.returned_milli - line.missing_milli).max(0);
        if quantity == 0 {
            continue;
        }
        let mut moved = StockMovement::moving(
            MovementType::ReturnIn,
            line.item_id,
            line.lot_id,
            None,
            Some(quarantine),
            quantity,
            line.unit_cost,
            value_of(quantity, line.unit_cost),
            &dispatch.subject(),
            by,
        );
        moved.reason = Some(reason.clone()).filter(|reason| !reason.is_empty());
        moved.moved_at = now;
        moved.insert(&mut *tx).await?;
        DispatchLine::query()
            .where_(DispatchLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("returned_milli", &(line.returned_milli + quantity)),
                    ("updated_at", &now),
                ],
            )
            .await?;
    }
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(
            &mut *tx,
            &[("return_reason", &Some(reason)), ("updated_at", &now)],
        )
        .await?;
    Ok(())
}

/// `return` for a user and for the system.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    let by_user = rules(
        Transition::new(RETURN)
            .label("Return")
            .tone("danger")
            .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
                dispatch.scope()
            })
            .reason_required(),
    );
    let by_system = rules(
        Transition::new(RETURN_BY_DELIVERY)
            .label("Return (delivery came back)")
            .system(),
    );
    machine.transition(by_user).transition(by_system)
}

/// `user` records that dispatch `id` came back, for `reason`.
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` in the location's scope; `NotFrom`
/// unless `dispatched`; `Guard` (422) without a reason.
pub async fn return_dispatch(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let _outcome = dispatch
        .transition(RETURN)
        .by(user)
        .reason(reason.trim())
        .run(state)
        .await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

/// Returns `dispatch` as the system. Does nothing, and answers `false`, when
/// it is not `dispatched`.
///
/// # Errors
///
/// A database error.
pub async fn return_by_delivery(
    state: &AppState,
    dispatch: &Dispatch,
    reason: &str,
) -> Result<bool> {
    if dispatch.status != DispatchStatus::Dispatched {
        return Ok(false);
    }
    let _outcome = dispatch
        .transition(RETURN_BY_DELIVERY)
        .by_system()
        .reason(reason.trim())
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(true)
}

/// Opens the complaint ticket of a returned dispatch and keeps its id.
async fn open_complaint(state: &AppState, dispatch: &Dispatch) -> Result<Option<i64>> {
    let number = dispatch.number.clone().unwrap_or_default();
    let title = said(
        "wl_inventory.dispatch.complaint_title",
        &[("number", &number)],
    );
    let body = said(
        "wl_inventory.dispatch.complaint_body",
        &[
            ("number", &number),
            (
                "reason",
                dispatch.return_reason.as_deref().unwrap_or_default(),
            ),
        ],
    );
    let mut ticket = OpenTicket::new(dispatch.subject(), TASK_CATEGORY, &title)
        .description(&body)
        .source(dispatch.source())
        .source_key(&format!("inventory.dispatch_return:{}", dispatch.id));
    if let Some(party) = recipient(dispatch) {
        ticket = ticket.requester_party(party);
    }
    let opened = task_creator(state).open(state, ticket).await?;
    let Some(opened) = opened else {
        return Ok(None);
    };
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(&state.db, &[("complaint_ticket_id", &Some(opened.id.0))])
        .await?;
    Ok(Some(opened.id.0))
}

/// The lines of a returned dispatch as the quarantine event carries them.
pub(crate) fn quarantined(
    dispatch: &Dispatch,
    lines: &[DispatchLine],
    reason: &str,
) -> Vec<QuarantinedLine> {
    let _ = dispatch;
    lines
        .iter()
        .filter(|line| line.returned_milli > 0)
        .map(|line| QuarantinedLine {
            line: line.subject(),
            item: wl_kernel::Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
            lot: line
                .lot_id
                .map(|id| wl_kernel::Subject::new(LOT_SUBJECT_TYPE, id)),
            source_line: line.source_line(),
            quantity_milli: line.returned_milli,
            reason: Some(reason.to_owned()).filter(|reason| !reason.is_empty()),
        })
        .collect()
}

/// Registers the listeners: the ticket and the events after `return`, and the
/// event of `wl-delivery` that asks for it.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Dispatch>, state: AppState| async move {
            if ![RETURN, RETURN_BY_DELIVERY].contains(&event.transition.as_str()) {
                return Ok(());
            }
            let mut dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            if open_complaint(&state, &dispatch).await?.is_some() {
                dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            }
            let complaint = dispatch
                .complaint_ticket_id
                .map(|id| wl_kernel::Subject::new(wl_kernel::tickets::TICKET_SUBJECT_TYPE, id));
            let lines = lines_of(&state.db, dispatch.id).await?;
            let reason = dispatch.return_reason.clone().unwrap_or_default();
            state
                .emit(DispatchReturned {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    reason: reason.clone(),
                    complaint: complaint.clone(),
                    lines: event_lines(&lines),
                })
                .await?;
            state
                .emit(DispatchGoodsQuarantined {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    returned: true,
                    complaint,
                    lines: quarantined(&dispatch, &lines, &reason),
                })
                .await
        },
    );
    app.listen(|event: DeliveryReturned, state: AppState| async move {
        match dispatch_of(&state, &event.source).await? {
            Some(dispatch) => return_by_delivery(&state, &dispatch, &event.reason)
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
    fn a_dispatched_dispatch_returns_by_a_user_or_the_system_and_ends() {
        let machine = Dispatch::machine();
        for (name, system) in [(RETURN, false), (RETURN_BY_DELIVERY, true)] {
            let found = machine.find(name).expect(name);
            assert_eq!(found.from_states(), ["dispatched"]);
            assert_eq!(found.to_state(), Some("returned"));
            assert_eq!(found.is_system(), system);
        }
    }
}
