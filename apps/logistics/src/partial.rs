//! Delivering part of a multi-parcel shipment and sending the refused parcels
//! back (03 Logistics §4.1 row *Recipient refuses some parcels of a
//! multi-parcel shipment*, §6 *Shipment*, §7.1 rows `deliver`, `create_return`
//! and `return_delivered`; `WRK.POD.01`; BL-LOG-07).
//!
//! - **At the stop.** Each parcel is one delivery line of quantity 1. The
//!   delivery of a shipment of several parcels may be partly accepted (the
//!   shipment asks for it when it is booked); that of a single parcel may
//!   not, so refusing it is a failed attempt. A refused parcel needs a reason
//!   (wl-delivery checks it at the stop, the shipment's `deliver` guard
//!   again), and a photo when it is *damaged* (wl-delivery).
//! - **`deliver` with refused parcels** ([`create_return`], run inside the
//!   transition): a new shipment linked to the original
//!   ([`Shipment::return_of_id`]) is created directly in `returning` with
//!   [`StateMachine::insert_in_state`] (its first history row is the insert
//!   with the reason `create_return`, there is no `created` row), because
//!   the parcels are on the vehicle. The refused parcels move to it with
//!   their barcodes and their refusal reason, its client is the original's
//!   client, its address is the client's pickup address
//!   ([`crate::ClientTerms::pickup_address`]), its COD is 0, and the return
//!   charge is recorded on it only. The original keeps the accepted parcels,
//!   its price and its COD amount: the expected COD stays the full amount
//!   (parcels carry no COD amount), and a shortfall goes through the COD
//!   difference flow (BL-LOG-08).
//! - **`ShipmentDelivered`** is emitted after the commit with the parcels
//!   delivered, the parcels refused and the return shipment.
//! - **At the depot.** `DeliveryRefusedReceived` (the run closed) writes a
//!   tracking event on the return shipment for each refused parcel: received
//!   back, or reported missing. A missing parcel stays on the return
//!   shipment; tracing it is BL-LOG-12.
//! - **At the client.** The client's e-POD makes the return shipment
//!   `returned` (`return_delivered`, as for any returning shipment).

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::PartyId;
use wl_kernel::delivery::{DeliveryDelivered, DeliveryRefusedReceived};
use wl_kernel::documents::today;
use wl_kernel::parties;

use crate::reweigh::{self, ClientTerms};
use crate::shipment::{
    PARCEL_SUBJECT_TYPE, Parcel, SHIPMENT_SERIES, Shipment, ShipmentStatus, parcel_state, track,
};
use crate::shipment_machine::{RefusedParcel, ReturnPlan, ShipmentInput, start_return};
use crate::texts::english;

/// The reason written on the history row of the return shipment's creation
/// (`create_return`, §7.1).
pub const CREATE_RETURN: &str = "create_return";

/// The parcels `event` says the recipient refused: the lines that are
/// parcels of a shipment with a refused quantity.
pub(crate) fn refused_in(event: &DeliveryDelivered) -> Vec<RefusedParcel> {
    event
        .lines
        .iter()
        .filter(|line| line.refused_milli > 0)
        .filter_map(|line| {
            let subject = line.line.as_ref()?;
            if subject.subject_type != PARCEL_SUBJECT_TYPE {
                return None;
            }
            Some(RefusedParcel {
                parcel_id: subject.subject_id,
                reason: line.refusal_reason.clone().unwrap_or_default(),
            })
        })
        .collect()
}

/// What the return shipment of `shipment` is made of, worked out before the
/// transition: its number, who it goes to and where (the client's pickup
/// address; a text that says it is missing when the client has none).
///
/// # Errors
///
/// Database errors.
pub(crate) async fn plan_return(state: &AppState, shipment: &Shipment) -> Result<ReturnPlan> {
    let number = SHIPMENT_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let terms = reweigh::terms(&state.db, shipment.client()).await?;
    let name = parties::directory(state)
        .display_name(&state.db, shipment.client(), "en")
        .await?
        .unwrap_or_else(|| {
            english(
                "logistics.partial.client_name",
                &[("client", &shipment.client_party_id)],
            )
        });
    let address = terms
        .pickup_address
        .clone()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| {
            english(
                "logistics.partial.no_pickup_address",
                &[("client", &shipment.client_party_id)],
            )
        });
    Ok(ReturnPlan {
        number,
        recipient_name: name,
        address,
        area: terms.pickup_area.clone().unwrap_or_default(),
    })
}

/// Creates the return shipment of `original` for `input.refused`, inside the
/// transaction of its `deliver` (`create_return`, §7.1): the shipment is
/// born `returning`, the refused parcels move to it, the return charge is
/// recorded on it, and the original keeps the parcels accepted.
///
/// # Errors
///
/// Database errors, and a refusal when the plan is missing, a refused parcel
/// is not a parcel of the shipment or has no reason.
pub(crate) async fn create_return(
    tx: &mut Transaction,
    original: &Shipment,
    input: &ShipmentInput,
    actor: Option<i64>,
) -> Result {
    let unprocessable = |message: String| Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message);
    let Some(plan) = input.return_plan.clone() else {
        return Err(unprocessable(english(
            "logistics.partial.plan_missing",
            &[],
        )));
    };
    let parcels = Parcel::query()
        .where_(Parcel::SHIPMENT_ID.eq(original.id))
        .get(&mut *tx)
        .await?;
    let mut moved: Vec<(Parcel, String)> = Vec::new();
    for refused in &input.refused {
        let reason = refused.reason.trim().to_owned();
        if reason.is_empty() {
            return Err(unprocessable(english(
                "logistics.partial.reason_needed",
                &[],
            )));
        }
        let Some(parcel) = parcels.iter().find(|p| p.id == refused.parcel_id) else {
            return Err(unprocessable(english(
                "logistics.partial.not_a_parcel",
                &[],
            )));
        };
        moved.push((parcel.clone(), reason));
    }
    if moved.is_empty() {
        return Ok(());
    }
    let weight: i64 = moved.iter().map(|(parcel, _)| parcel.weight_grams).sum();
    let count = i64::try_from(moved.len()).unwrap_or(i64::MAX);
    let mut ret = Shipment {
        number: plan.number.clone(),
        client_party_id: original.client_party_id,
        recipient_name: plan.recipient_name.clone(),
        address: plan.address.clone(),
        area: plan.area.clone(),
        cod_amount: 0,
        booked_weight_grams: weight,
        weight_grams: weight,
        price: 0,
        parcel_count: count,
        parcels_scanned: count,
        return_of_id: Some(original.id),
        status: ShipmentStatus::Returning,
        ..Shipment::default()
    };
    ret.insert_in_state(&mut *tx, ShipmentStatus::Returning, CREATE_RETURN)
        .await?;
    wl_kernel::search::index_in(&mut *tx, &ret.subject()).await?;
    let mut codes = Vec::new();
    for (parcel, reason) in &moved {
        let reason = Some(reason.clone());
        Parcel::query()
            .where_(Parcel::ID.eq(parcel.id))
            .update(
                &mut *tx,
                &[("shipment_id", &ret.id), ("refusal_reason", &reason)],
            )
            .await?;
        codes.push(parcel.barcode.clone());
    }
    // The original keeps the parcels accepted, its price and its COD.
    let scanned_moved = i64::try_from(
        moved
            .iter()
            .filter(|(parcel, _)| parcel.state == parcel_state::SCANNED)
            .count(),
    )
    .unwrap_or(i64::MAX);
    let remaining = (original.parcel_count - count).max(0);
    let remaining_scanned = (original.parcels_scanned - scanned_moved).max(0);
    Shipment::query()
        .where_(Shipment::ID.eq(original.id))
        .update(
            &mut *tx,
            &[
                ("parcel_count", &remaining),
                ("parcels_scanned", &remaining_scanned),
            ],
        )
        .await?;
    // One return charge, on the return shipment only.
    start_return(&mut *tx, &ret, actor).await?;
    let list = codes.join(", ");
    track(
        &mut *tx,
        ret.id,
        "return_created",
        &english(
            "logistics.partial.created_note",
            &[("number", &original.number), ("parcels", &list)],
        ),
        actor,
    )
    .await?;
    track(
        &mut *tx,
        original.id,
        "parcels_refused",
        &english(
            "logistics.partial.refused_note",
            &[("number", &ret.number), ("parcels", &list)],
        ),
        actor,
    )
    .await
}

/// Writes, on the return shipments of the shipment `event` is about, what the
/// depot found when the run closed: each refused parcel received back or
/// reported missing.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn note_received(state: &AppState, event: &DeliveryRefusedReceived) -> Result {
    if event.source.subject_type != crate::shipment::SUBJECT_TYPE {
        return Ok(());
    }
    let Some(original) = Shipment::find(&state.db, event.source.subject_id).await? else {
        return Ok(());
    };
    let returns = original.returns(&state.db).await?;
    for line in &event.lines {
        let Some(subject) = line.line.as_ref() else {
            continue;
        };
        if subject.subject_type != PARCEL_SUBJECT_TYPE {
            continue;
        }
        let Some(parcel) = Parcel::find(&state.db, subject.subject_id).await? else {
            continue;
        };
        let Some(ret) = returns.iter().find(|r| r.id == parcel.shipment_id) else {
            continue;
        };
        if line.received_milli > 0 {
            track(&state.db, ret.id, "refused_received", &parcel.barcode, None).await?;
        }
        if line.missing_milli > 0 {
            track(&state.db, ret.id, "refused_missing", &parcel.barcode, None).await?;
        }
    }
    Ok(())
}

/// Sets where the refused parcels of `client` go back to (a stand-in for the
/// address on the client's party record).
///
/// # Errors
///
/// Database errors.
pub async fn set_pickup_address(db: &Db, client: PartyId, address: &str, area: &str) -> Result {
    let found = ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client.0))
        .first(db)
        .await?;
    let address = Some(address.trim().to_owned());
    let area = Some(area.trim().to_owned());
    match found {
        Some(mut terms) => {
            terms.pickup_address = address;
            terms.pickup_area = area;
            terms.save(db).await?;
        }
        None => {
            let mut terms = ClientTerms {
                client_party_id: client.0,
                pickup_address: address,
                pickup_area: area,
                ..ClientTerms::default()
            };
            terms.insert(db).await?;
        }
    }
    Ok(())
}
