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
    DeliveryAttemptFailed, DeliveryDelivered, DeliveryId, DeliveryOutForDelivery, DeliveryRequester,
};
use wl_kernel::{Subject, voids};

use crate::notices::tell_client;
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentStatus, track};
use crate::shipment_machine::{
    CANCEL, CANCEL_RETURN, DELIVER, DISPATCH, FAIL_ATTEMPT, HOLD, HOLD_SYSTEM, PICK_UP,
    RECEIVE_AT_HUB, RELEASE, RETURN, ShipmentInput,
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
        track(
            &state.db,
            shipment.id,
            "delivery_cancel_failed",
            &note,
            None,
        )
        .await?;
    }
    Ok(())
}

/// Holds (or releases) the open delivery of shipment `id` in the delivery
/// module, so a held shipment's delivery cannot go on a run or leave on one
/// (03 §7.1 `hold`, `release`; LOG.RTE.01). A refusal is written to the
/// tracking events.
async fn hold_delivery(state: &AppState, id: i64, held: bool) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let Some(delivery) = shipment.delivery_id else {
        return Ok(());
    };
    if state
        .provided::<std::sync::Arc<dyn DeliveryRequester>>()
        .is_none()
    {
        return Ok(());
    }
    if let Err(error) = wl_kernel::delivery::requester(state)
        .hold(state, DeliveryId(delivery), held)
        .await
    {
        let note = format!("{error:?}");
        track(&state.db, id, "delivery_hold_failed", &note, None).await?;
    }
    Ok(())
}

/// Tells the client which parcels were not handed over, when some were
/// (BL-LOG-03): the shipment goes ahead with the parcels received, the COD
/// amount is unchanged.
async fn tell_of_missing_parcels(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let missing = crate::pickup::missing_parcels(&state.db, &shipment).await?;
    if missing.is_empty() {
        return Ok(());
    }
    let (weight, price) = crate::pickup::weight_and_price(&shipment);
    let title = english(
        "logistics.pickup.missing_title",
        &[("number", &shipment.number), ("count", &missing.len())],
    );
    let body = english(
        "logistics.pickup.missing_body",
        &[
            ("number", &shipment.number),
            ("parcels", &missing.join(", ")),
            ("weight", &weight),
            ("price", &price),
            ("cod", &crate::rates::aed(shipment.cod_amount)),
        ],
    );
    tell_client(state, &shipment, &title, &body).await
}

/// The failure reason that holds the shipment instead of a reattempt.
const WRONG_ADDRESS: &str = "wrong_address";

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
                ShipmentInput {
                    cod_difference: event.cod_difference,
                    cod_difference_reason: event.cod_difference_reason.clone(),
                    absorbed_by: event.absorbed_by.clone(),
                    ..ShipmentInput::default()
                },
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
            if event.reason == WRONG_ADDRESS {
                // Not queued for a reattempt: held until the client confirms
                // the address (BL-LOG-05).
                crate::hold::hold_for_wrong_address(&state, event.source.subject_id).await?;
            }
        }
        Ok(())
    });
    app.listen(
        |event: Transitioned<Shipment>, state: AppState| async move {
            let name = event.transition.as_str();
            if name == HOLD || name == HOLD_SYSTEM {
                hold_delivery(&state, event.key, true).await?;
                crate::hold::tell_of_hold(&state, event.key).await?;
                return crate::hold::open_claim(&state, event.key).await;
            }
            if name == RELEASE {
                return hold_delivery(&state, event.key, false).await;
            }
            if name == PICK_UP || (name == RECEIVE_AT_HUB && event.from == "created") {
                return tell_of_missing_parcels(&state, event.key).await;
            }
            let voided = name == voids::VOID || name == voids::VOID_APPROVED;
            if name != CANCEL && name != CANCEL_RETURN && name != RETURN && !voided {
                return Ok(());
            }
            let shipment = Shipment::find_or_404(&state.db, event.key).await?;
            if voided {
                let reason = event.reason.clone().unwrap_or_default();
                track(&state.db, shipment.id, "voided", &reason, event.actor_id).await?;
                cancel_delivery(
                    &state,
                    &shipment,
                    &english("logistics.void.delivery_reason", &[]),
                )
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
        },
    );
}
