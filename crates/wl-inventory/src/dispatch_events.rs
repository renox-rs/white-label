//! The events of a dispatch (wl-inventory §6) that every transition shares:
//! the lines as an event carries them, the dispatch a `wl-delivery` event is
//! about, and `DispatchSent` after `dispatch` committed.
//!
//! The other events are emitted where their transition is, each module
//! registering the listener of its own: [`crate::dispatch_deliver`]
//! (`DispatchDelivered`, `DeliveryDelivered`, `DeliveryRefusedReceived`),
//! [`crate::dispatch_loss`] (`DispatchLost`, `DeliveryLost`, `DeliveryFound`)
//! and [`crate::dispatch_reverse`] (`DispatchReversed`, `DeliveryReversed`).
//!
//! A `wl-delivery` event names the record the delivery was requested for;
//! only those of type `inventory.dispatch` are ours, the others are ignored.
//! The events are read as committed, after the transition, so a rollback
//! emits nothing.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::inventory::{DispatchSent, DispatchedLine, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE};

use crate::dispatch::{Dispatch, DispatchLine, lines_of};
use crate::dispatch_send::DISPATCH;
use crate::item::value_of;

/// The lines of `lines` as the events carry them.
pub fn event_lines(lines: &[DispatchLine]) -> Vec<DispatchedLine> {
    lines
        .iter()
        .map(|line| DispatchedLine {
            line: line.subject(),
            item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
            lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
            source_line: line.source_line(),
            quantity_milli: line.quantity_milli,
            delivered_milli: line.delivered_milli,
            refused_milli: line.refused_milli,
            refusal_reason: line.refusal_reason.clone(),
            value: value_of(line.quantity_milli, line.unit_cost),
        })
        .collect()
}

/// The recipient of `dispatch`, when it is a party.
pub(crate) fn recipient(dispatch: &Dispatch) -> Option<wl_kernel::PartyId> {
    dispatch.recipient_party_id.map(wl_kernel::PartyId)
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<Dispatch>, state: AppState| async move {
            if event.transition != DISPATCH {
                return Ok(());
            }
            let dispatch = Dispatch::find_or_404(&state.db, event.key).await?;
            let lines = lines_of(&state.db, dispatch.id).await?;
            state
                .emit(DispatchSent {
                    dispatch: dispatch.subject(),
                    source: dispatch.source(),
                    recipient: recipient(&dispatch),
                    handover: dispatch.handover.clone().unwrap_or_default(),
                    delivery: dispatch.delivery_id,
                    tracking_number: dispatch.tracking_number.clone(),
                    lines: event_lines(&lines),
                })
                .await
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_line_names_the_line_the_item_the_lot_and_the_value() {
        let line = DispatchLine {
            id: 4,
            item_id: 9,
            lot_id: Some(2),
            quantity_milli: 10_000,
            delivered_milli: 8_000,
            refused_milli: 2_000,
            refusal_reason: Some("damaged".into()),
            unit_cost: 5_000,
            source_line_type: Some("sales.sales_order_line".into()),
            source_line_id: Some(7),
            ..DispatchLine::default()
        };
        let lines = event_lines(&[line]);
        assert_eq!(lines[0].line.to_string(), "inventory.dispatch_line:4");
        assert_eq!(
            lines[0].lot.as_ref().unwrap().to_string(),
            "inventory.lot:2"
        );
        assert_eq!(
            lines[0].source_line.as_ref().unwrap().to_string(),
            "sales.sales_order_line:7"
        );
        assert_eq!(
            (lines[0].delivered_milli, lines[0].refused_milli),
            (8_000, 2_000)
        );
        assert_eq!(lines[0].value, 50_000);
    }
}
