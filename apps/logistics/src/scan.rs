//! Scanning parcels at pickup and at the hub (03 Logistics §4.1 steps 3 and
//! 4, `LOG.HUB.01`, `PLT.SCN.01`).
//!
//! - [`pick_up`] runs `pick_up` for the parcels the driver scanned.
//! - [`receive_at_hub`] runs `receive_at_hub`; from `created` it is a
//!   drop-off and needs a parcel scanned.
//! - [`scan_label`] finds the shipment of one barcode and runs one of the two.
//!
//! A label of a `voided` shipment is invalid: any scan of it is refused with
//! "label invalid", also when the void commits between the check and the
//! transition (the engine finds the state changed). A barcode that is not one
//! of the shipment's parcels is refused too.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};

use crate::shipment::{Parcel, Shipment, ShipmentStatus, parcel_state, track};
use crate::shipment_machine::{PICK_UP, RECEIVE_AT_HUB, ShipmentInput};
use crate::texts::english;

/// The barcodes in `text`, separated by spaces, commas or lines.
pub fn barcodes_in(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == ',')
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The refusal of a scan of an invalid label.
fn label_invalid() -> TransitionError {
    TransitionError::Guard(english("logistics.scan.label_invalid", &[]))
}

/// What the driver or hub staff hand in at a pickup or drop-off.
#[derive(Debug, Clone, Default)]
pub struct Handover {
    /// The parcels scanned.
    pub barcodes: Vec<String>,
    /// The parcels the client did not hand over, marked *not received*. A
    /// parcel neither scanned nor marked is not received either.
    pub not_received: Vec<String>,
    /// Why the pickup failed when no parcel was handed over (client absent,
    /// nothing ready); only written in that case.
    pub failed_reason: String,
}

impl Handover {
    /// The parcels scanned, and nothing marked.
    pub fn scanned(barcodes: &[String]) -> Self {
        Handover {
            barcodes: barcodes.to_vec(),
            ..Handover::default()
        }
    }
}

/// Checks the labels of `handover` against `shipment`.
async fn check_labels(
    state: &AppState,
    shipment: &Shipment,
    handover: &Handover,
) -> std::result::Result<(), TransitionError> {
    if shipment.status == ShipmentStatus::Voided {
        return Err(label_invalid());
    }
    if !handover.not_received.is_empty() && shipment.status != ShipmentStatus::Created {
        return Err(TransitionError::Guard(english(
            "logistics.scan.not_received_late",
            &[],
        )));
    }
    for code in handover.barcodes.iter().chain(&handover.not_received) {
        let parcel = Parcel::query()
            .where_(Parcel::BARCODE.eq(code.as_str()))
            .first(&state.db)
            .await?;
        match parcel {
            Some(parcel) if parcel.shipment_id == shipment.id => {
                if handover.barcodes.contains(code) {
                    if parcel.state == parcel_state::NOT_RECEIVED {
                        return Err(TransitionError::Guard(english(
                            "logistics.scan.not_received",
                            &[("barcode", code)],
                        )));
                    }
                    if handover.not_received.contains(code) {
                        return Err(TransitionError::Guard(english(
                            "logistics.scan.both_lists",
                            &[("barcode", code)],
                        )));
                    }
                }
            }
            _ => {
                return Err(TransitionError::Guard(english(
                    "logistics.scan.unknown_label",
                    &[("barcode", code)],
                )));
            }
        }
    }
    Ok(())
}

/// A voided shipment found by the engine is an invalid label too.
fn refused(error: TransitionError) -> TransitionError {
    match error {
        TransitionError::NotFrom(state) if state == "voided" => label_invalid(),
        other => other,
    }
}

async fn scan(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    name: &str,
    handover: &Handover,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    check_labels(state, &shipment, handover).await?;
    let result = shipment
        .transition(name)
        .by(user)
        .input(&ShipmentInput {
            barcodes: handover.barcodes.clone(),
            not_received: handover.not_received.clone(),
            ..ShipmentInput::default()
        })
        .run(state)
        .await;
    if let Err(TransitionError::Guard(message)) = &result
        && shipment.status == ShipmentStatus::Created
        && handover.barcodes.is_empty()
        && *message == english("logistics.scan.parcel_needed", &[])
    {
        // Nothing was handed over: the attempt failed, the shipment stays
        // `created` and can be picked up again or cancelled.
        let reason = match handover.failed_reason.trim() {
            "" => english("logistics.pickup.failed_default", &[]),
            text => text.to_owned(),
        };
        track(&state.db, id, "pickup_failed", &reason, Some(user.id)).await?;
    }
    let _outcome = result.map_err(refused)?;
    Ok(Shipment::find_or_404(&state.db, id).await?)
}

/// `user` (the driver, `shipments.scan`) picks up shipment `id` with the
/// parcels `barcodes` scanned; the rest of the parcels are not received.
///
/// # Errors
///
/// `Guard` (422) with "label invalid" for a voided shipment, for a barcode
/// that is not one of its parcels, and when no parcel was scanned;
/// `Forbidden` without `shipments.scan`; `NotFrom` unless it is `created`.
pub async fn pick_up(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    barcodes: &[String],
) -> std::result::Result<Shipment, TransitionError> {
    scan(state, user, id, PICK_UP, &Handover::scanned(barcodes)).await
}

/// [`pick_up`] with the parcels marked *not received* and the reason of a
/// failed pickup (BL-LOG-03).
///
/// # Errors
///
/// As [`pick_up`]; also `Guard` (422) for a parcel both scanned and marked,
/// and for a parcel already marked *not received* that is scanned.
pub async fn pick_up_handover(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    handover: &Handover,
) -> std::result::Result<Shipment, TransitionError> {
    scan(state, user, id, PICK_UP, handover).await
}

/// `user` (hub staff, `shipments.scan`) scans shipment `id` in at the hub;
/// `barcodes` are the parcels scanned (needed from `created`).
///
/// # Errors
///
/// As [`pick_up`]; a parcel marked *not received* cannot be scanned in.
pub async fn receive_at_hub(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    barcodes: &[String],
) -> std::result::Result<Shipment, TransitionError> {
    scan(state, user, id, RECEIVE_AT_HUB, &Handover::scanned(barcodes)).await
}

/// [`receive_at_hub`] with the parcels marked *not received* (a drop-off
/// from `created`) and the reason of a failed drop-off.
///
/// # Errors
///
/// As [`pick_up_handover`].
pub async fn receive_handover(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    handover: &Handover,
) -> std::result::Result<Shipment, TransitionError> {
    scan(state, user, id, RECEIVE_AT_HUB, handover).await
}

/// `user` scans one label: the shipment it belongs to is picked up (`hub`
/// false) or scanned in at the hub (`hub` true).
///
/// # Errors
///
/// `Guard` (422) with "label invalid" for the label of a voided shipment,
/// and "unknown label" for a barcode no parcel has; else as [`pick_up`].
pub async fn scan_label(
    state: &AppState,
    user: &AuthUser,
    barcode: &str,
    hub: bool,
) -> std::result::Result<Shipment, TransitionError> {
    let barcode = barcode.trim();
    let parcel = Parcel::query()
        .where_(Parcel::BARCODE.eq(barcode))
        .first(&state.db)
        .await?
        .ok_or_else(|| {
            TransitionError::Guard(english(
                "logistics.scan.unknown_label",
                &[("barcode", &barcode)],
            ))
        })?;
    let codes = [barcode.to_owned()];
    if hub {
        receive_at_hub(state, user, parcel.shipment_id, &codes).await
    } else {
        pick_up(state, user, parcel.shipment_id, &codes).await
    }
}
