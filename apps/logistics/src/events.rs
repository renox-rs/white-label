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
//! - `DeliveryDelivered` with refused parcels also creates the linked return
//!   shipment (`create_return`, inside `deliver`), and `ShipmentDelivered` is
//!   emitted after the commit; `DeliveryRefusedReceived` writes what the depot
//!   found on the return shipment ([`crate::partial`], BL-LOG-07).
//! - `DeliveryDisputed` and `DeliveryDisputeClosed` (outcome *upheld*) run
//!   `open_review` and `close_review` as the system (BL-LOG-09); after
//!   `open_review` commits, the client is told and the COD comes off its
//!   remittance ([`crate::review`]). A second event finds the shipment in the
//!   target state and does nothing.
//! - `cancel`, `cancel_return` and `void` cancel the shipment's open
//!   delivery; a delivery that cannot be cancelled (it is on a dispatched
//!   run, or already attempted) is written to the tracking events. A voided
//!   shipment gets its `voided` tracking event, and a shipment sent back
//!   tells its client.

use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::delivery::{
    DeliveryAttemptFailed, DeliveryDelivered, DeliveryDisputeClosed, DeliveryDisputed, DeliveryId,
    DeliveryOutForDelivery, DeliveryRefusedReceived, DeliveryRequester, ReviewOutcome,
};
use wl_kernel::{Subject, voids};

use crate::notices::tell_client;
use crate::shipment::{SUBJECT_TYPE, Shipment, ShipmentDelivered, ShipmentStatus, track};
use crate::shipment_machine::{
    CANCEL, CANCEL_RETURN, CLOSE_REVIEW, DELIVER, DISPATCH, FAIL_ATTEMPT, HOLD, HOLD_SYSTEM,
    OPEN_REVIEW, PICK_UP, RECEIVE_AT_HUB, RELEASE, RETURN, ShipmentInput,
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

/// Emits `ShipmentDelivered` for shipment `id`, which was just delivered: the
/// parcels it kept, the parcels that went back and the return shipment.
async fn emit_delivered(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let returns = shipment.returns(&state.db).await?;
    let mut refused_parcels = Vec::new();
    for ret in &returns {
        for parcel in ret.parcels(&state.db).await? {
            refused_parcels.push(parcel.barcode);
        }
    }
    let delivered_parcels = shipment
        .parcels(&state.db)
        .await?
        .into_iter()
        .filter(|parcel| parcel.state != crate::shipment::parcel_state::NOT_RECEIVED)
        .map(|parcel| parcel.barcode)
        .collect();
    state
        .emit(ShipmentDelivered {
            shipment: shipment.id,
            number: shipment.number.clone(),
            client: shipment.client(),
            delivered_parcels,
            refused_parcels,
            return_shipment: returns.first().map(|ret| ret.id),
            cod_expected: shipment.cod_amount,
        })
        .await
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
            // Refused parcels go back on a linked return shipment, whose
            // number is taken before the transition (BL-LOG-07).
            let refused = if shipment.status == ShipmentStatus::OutForDelivery {
                crate::partial::refused_in(&event)
            } else {
                Vec::new()
            };
            let return_plan = if refused.is_empty() {
                None
            } else {
                Some(crate::partial::plan_return(&state, &shipment).await?)
            };
            follow(
                &state,
                shipment,
                ShipmentStatus::OutForDelivery,
                DELIVER,
                ShipmentInput {
                    cod_difference: event.cod_difference,
                    cod_difference_reason: event.cod_difference_reason.clone(),
                    absorbed_by: event.absorbed_by.clone(),
                    refused,
                    return_plan,
                    ..ShipmentInput::default()
                },
            )
            .await?;
        }
        Ok(())
    });
    app.listen(|event: DeliveryRefusedReceived, state: AppState| async move {
        crate::partial::note_received(&state, &event).await
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
    app.listen(|event: DeliveryDisputed, state: AppState| async move {
        if let Some(shipment) = shipment_of(&state, &event.source).await? {
            follow(
                &state,
                shipment,
                ShipmentStatus::Delivered,
                OPEN_REVIEW,
                ShipmentInput {
                    review_ticket: event.ticket.map(|ticket| ticket.0),
                    ticketing_off: event.ticket.is_none(),
                    ..ShipmentInput::default()
                },
            )
            .await?;
        }
        Ok(())
    });
    app.listen(|event: DeliveryDisputeClosed, state: AppState| async move {
        // The only outcome of the event is *upheld*; a reversal or a loss
        // arrives as its own event (BL-LOG-10, BL-LOG-11).
        let ReviewOutcome::Upheld = event.outcome;
        if let Some(shipment) = shipment_of(&state, &event.source).await? {
            follow(
                &state,
                shipment,
                ShipmentStatus::UnderReview,
                CLOSE_REVIEW,
                ShipmentInput::default(),
            )
            .await?;
        }
        Ok(())
    });
    app.listen(
        |event: Transitioned<Shipment>, state: AppState| async move {
            let name = event.transition.as_str();
            if name == OPEN_REVIEW {
                return crate::review::held(&state, event.key).await;
            }
            if name == DELIVER {
                return emit_delivered(&state, event.key).await;
            }
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
