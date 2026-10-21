//! Fewer parcels handed over than booked (03 Logistics §4.1 *Fewer parcels
//! handed over than booked*, §7.1 rows `pick_up` and `receive_at_hub`;
//! BL-LOG-03).
//!
//! At `pick_up`, and at `receive_at_hub` from `created` (a drop-off), the
//! parcels scanned are the parcels received. The parcels the driver marked
//! *not received*, and any other parcel that was neither scanned nor
//! marked, get the mark `not_received`: they stay on the shipment, are left
//! out of its weight and price, and are never scanned in later. The weight
//! is recalculated as the sum of the booked weights of the parcels received
//! and the price is read again from the client's bands. The COD amount is
//! not touched. After the commit the client is told which parcels were
//! missing ([`crate::events`]).
//!
//! With no parcel scanned the pickup fails ([`crate::scan`] writes a
//! *pickup failed* event with the reason) and the shipment stays `created`.

use renox::db::Transaction;
use renox::prelude::*;

use crate::rates::{self, aed};
use crate::shipment::{Parcel, Shipment, parcel_state};
use crate::shipment_machine::ShipmentInput;

/// Marks the parcels not handed over and recalculates the weight and price
/// from the parcels received, in the transaction of the transition.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn settle(
    tx: &mut Transaction,
    shipment: &Shipment,
    input: &ShipmentInput,
) -> Result {
    let missing = parcel_state::NOT_RECEIVED.to_owned();
    let booked = parcel_state::BOOKED.to_owned();
    for code in &input.not_received {
        Parcel::query()
            .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
            .where_(Parcel::BARCODE.eq(code.as_str()))
            .where_(Parcel::STATE.eq(parcel_state::BOOKED))
            .update(&mut *tx, &[("state", &missing)])
            .await?;
    }
    // The rest: neither scanned nor marked.
    Parcel::query()
        .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
        .where_(Parcel::STATE.eq(booked.as_str()))
        .update(&mut *tx, &[("state", &missing)])
        .await?;
    let parcels = Parcel::query()
        .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
        .get(&mut *tx)
        .await?;
    let weight: i64 = parcels
        .iter()
        .filter(|parcel| parcel.state != parcel_state::NOT_RECEIVED)
        .map(|parcel| parcel.weight_grams)
        .sum();
    let price = rates::band_price(&mut *tx, shipment.client(), &shipment.area, weight)
        .await?
        .unwrap_or(0);
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(&mut *tx, &[("weight_grams", &weight), ("price", &price)])
        .await?;
    Ok(())
}

/// The barcodes of the parcels of `shipment` marked *not received*.
///
/// # Errors
///
/// Database errors.
pub async fn missing_parcels(db: &Db, shipment: &Shipment) -> Result<Vec<String>> {
    Ok(Parcel::query()
        .where_(Parcel::SHIPMENT_ID.eq(shipment.id))
        .where_(Parcel::STATE.eq(parcel_state::NOT_RECEIVED))
        .order_by(Parcel::ID)
        .get(db)
        .await?
        .into_iter()
        .map(|parcel| parcel.barcode)
        .collect())
}

/// The weight as kilograms: `10 kg`, `2.4 kg`, `2.45 kg`.
pub(crate) fn kg(grams: i64) -> String {
    let whole = grams / 1000;
    let hundredths = grams % 1000 / 10;
    if hundredths == 0 {
        format!("{whole} kg")
    } else if hundredths % 10 == 0 {
        format!("{whole}.{} kg", hundredths / 10)
    } else {
        format!("{whole}.{hundredths:02} kg")
    }
}

/// The weight and price the client is told of, as text.
pub(crate) fn weight_and_price(shipment: &Shipment) -> (String, String) {
    (kg(shipment.weight_grams), aed(shipment.price))
}
