//! What happens after a transition or a delivery event commits (03 §7.1):
//! work that reaches other modules runs here, in listeners that re-read the
//! row, never inside an effect.
//!
//! - `DeliveryOutForDelivery`, `DeliveryDelivered` and `DeliveryAttemptFailed`
//!   of a delivery whose source is a shipment run the shipment's `dispatch`,
//!   `deliver` and `fail_attempt` as the system. A shipment that is not in a
//!   state the transition leaves is left alone (a delivery planned before the
//!   shipment reached the hub is a gap of the planning, recorded in the
//!   audit notes).
//! - `cancel`, `cancel_return` and `void` cancel the shipment's open
//!   delivery; a delivery that cannot be cancelled (it is on a dispatched
//!   run, or already attempted) is written to the tracking events. A voided
//!   shipment gets its `voided` tracking event, and a shipment sent back
//!   tells its client.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::delivery::{
    DeliveryAttemptFailed, DeliveryDelivered, DeliveryId, DeliveryOutForDelivery,
    DeliveryRequester,
};
use wl_kernel::{Subject, voids};

use crate::notices::tell_client;
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentStatus, track};
use crate::shipment_machine::{
    CANCEL, CANCEL_RETURN, DELIVER, DISPATCH, FAIL_ATTEMPT, ShipmentInput,
};
use crate::texts::english;

/// The shipment a delivery was requested for, if its source is one.
async fn shipment_of(state: &AppState, source: &Subject) -> Result<Option<Shipment>> {
    if source.subject_type != SUBJECT_TYPE {
        return Ok(None);
    }
    Shipment::find(&state.db, source.subject_id).await
}

/// Runs the system transition `name` of `shipment` when it is in `from`.
async fn follow(
    state: &AppState,
    shipment: Shipment,
    from: ShipmentStatus,
    name: &str,
    input: ShipmentInput,
) -> Result {
    if shipment.status != from {
        return Ok(());
    }
    let _outcome = shipment
        .transition(name)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(renox::state_machine::TransitionError::into_error)?;
    Ok(())
}

/// Cancels the open delivery of `shipment`, when it has one.
async fn cancel_delivery(state: &AppState, shipment: &Shipment, reason: &str) -> Result {
    let Some(id) = shipment.delivery_id else {
        return Ok(());
    };
    if state
        .provided::<std::sync::Arc<dyn DeliveryRequester>>()
        .is_none()
    {
        return Ok(());
    }
    if let Err(error) = wl_kernel::delivery::requester(state)
        .cancel(state, DeliveryId(id), reason)
        .await
    {
        let note = format!("{error:?}");
        track(&state.db, shipment.id, "delivery_cancel_failed", &note, None).await?;
    }
    Ok(())
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: DeliveryOutForDelivery, state: AppState| async move {
            if let Some(shipment) = shipment_of(&state, &event.source).await? {
                follow(
                    &state,
                    shipment,
                    ShipmentStatus::AtHub,
                    DISPATCH,
                    ShipmentInput::default(),
                )
                .await?;
            }
            Ok(())
        },
    );
    app.listen(|event: DeliveryDelivered, state: AppState| async move {
        if let Some(shipment) = shipment_of(&state, &event.source).await? {
            follow(
                &state,
                shipment,
                ShipmentStatus::OutForDelivery,
                DELIVER,
                ShipmentInput::default(),
            )
            .await?;
        }
        Ok(())
    });
    app.listen(|event: DeliveryAttemptFailed, state: AppState| async move {
        if let Some(shipment) = shipment_of(&state, &event.source).await? {
            follow(
                &state,
                shipment,
                ShipmentStatus::OutForDelivery,
                FAIL_ATTEMPT,
                ShipmentInput {
                    reason: Some(event.reason.clone()),
                    ..ShipmentInput::default()
                },
            )
            .await?;
        }
        Ok(())
    });
    app.listen(|event: Transitioned<Shipment>, state: AppState| async move {
        let name = event.transition.as_str();
        let voided = name == voids::VOID || name == voids::VOID_APPROVED;
        if name != CANCEL && name != CANCEL_RETURN && !voided {
            return Ok(());
        }
        let shipment = Shipment::find_or_404(&state.db, event.key).await?;
        if voided {
            let reason = event.reason.clone().unwrap_or_default();
            track(&state.db, shipment.id, "voided", &reason, event.actor_id).await?;
            cancel_delivery(&state, &shipment, &english("logistics.void.delivery_reason", &[]))
                .await?;
        } else if name == CANCEL {
            cancel_delivery(
                &state,
                &shipment,
                &english("logistics.cancel.delivery_reason", &[]),
            )
            .await?;
        } else {
            cancel_delivery(
                &state,
                &shipment,
                &english("logistics.cancel.delivery_reason", &[]),
            )
            .await?;
            let title = english(
                "logistics.cancel.returning_title",
                &[("number", &shipment.number)],
            );
            let body = english(
                "logistics.cancel.returning_body",
                &[("number", &shipment.number)],
            );
            tell_client(&state, &shipment, &title, &body).await?;
        }
        Ok(())
    });
}
