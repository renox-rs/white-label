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

use crate::shipment::{Parcel, Shipment, ShipmentStatus};
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

/// Checks the labels of `barcodes` against `shipment`.
async fn check_labels(
    state: &AppState,
    shipment: &Shipment,
    barcodes: &[String],
) -> std::result::Result<(), TransitionError> {
    if shipment.status == ShipmentStatus::Voided {
        return Err(label_invalid());
    }
    for code in barcodes {
        let parcel = Parcel::query()
            .where_(Parcel::BARCODE.eq(code.as_str()))
            .first(&state.db)
            .await?;
        match parcel {
            Some(parcel) if parcel.shipment_id == shipment.id => {}
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
    barcodes: &[String],
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    check_labels(state, &shipment, barcodes).await?;
    let _outcome = shipment
        .transition(name)
        .by(user)
        .input(&ShipmentInput {
            barcodes: barcodes.to_vec(),
            ..ShipmentInput::default()
        })
        .run(state)
        .await
        .map_err(refused)?;
    Ok(Shipment::find_or_404(&state.db, id).await?)
}

/// `user` (the driver, `shipments.scan`) picks up shipment `id` with the
/// parcels `barcodes` scanned.
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
    scan(state, user, id, PICK_UP, barcodes).await
}

/// `user` (hub staff, `shipments.scan`) scans shipment `id` in at the hub;
/// `barcodes` are the parcels scanned (needed from `created`).
///
/// # Errors
///
/// As [`pick_up`].
pub async fn receive_at_hub(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    barcodes: &[String],
) -> std::result::Result<Shipment, TransitionError> {
    scan(state, user, id, RECEIVE_AT_HUB, barcodes).await
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
