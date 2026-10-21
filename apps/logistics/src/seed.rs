//! The demo company's logistics data (DEMO-DATA.md, 03: *Najm Express Cargo
//! LLC*): a client's prices for the cancellation fee and the return charge,
//! and a few shipments booked through the app's own booking service, so the
//! permissions, the numbering and the guards are the real ones.
//!
//! [`demo_shipments`] is idempotent: a shipment whose client reference is
//! already used is left alone.

use renox::prelude::*;
use wl_kernel::PartyId;

use crate::booking::{self, NewShipment};
use crate::rates;
use crate::shipment::charge_kind;

/// The name of the demo company.
pub const DEMO_COMPANY: &str = "Najm Express Cargo LLC";

/// The demo online shop that books shipments.
pub const DEMO_CLIENT: &str = "Souq Express Trading LLC";

/// The demo client's prices in fils: the cancellation fee and the return
/// charge.
pub const DEMO_RATES: [(&str, i64); 2] = [
    (charge_kind::CANCELLATION, 1_000),
    (charge_kind::RETURN, 1_500),
];

/// The demo client's weight bands in any zone: up to this many grams, the
/// price in fils.
pub const DEMO_BANDS: [(i64, i64); 3] = [(5_000, 2_000), (10_000, 2_500), (15_000, 3_000)];

/// The demo shipments of the client: reference, recipient, area, parcels
/// and the cash to collect in fils.
pub const DEMO_SHIPMENTS: [(&str, &str, &str, usize, i64); 4] = [
    ("ORD-1001", "Mariam Al Hosani", "Al Barsha", 1, 15_000),
    ("ORD-1002", "Omar Khalid", "Business Bay", 2, 0),
    ("ORD-1003", "Sara Nasser", "Jumeirah", 1, 8_950),
    ("ORD-1004", "Yousef Ibrahim", "Deira", 3, 0),
];

/// Sets the demo client's prices.
///
/// # Errors
///
/// Database errors.
pub async fn demo_rates(db: &Db, client: PartyId) -> Result {
    for (kind, amount) in DEMO_RATES {
        rates::set_price(db, client, kind, amount).await?;
    }
    for (up_to_grams, amount) in DEMO_BANDS {
        rates::set_band(db, client, "", up_to_grams, amount).await?;
    }
    Ok(())
}

/// Books the demo shipments of `client` for the user `booked_by`; returns
/// how many were new.
///
/// # Errors
///
/// The booking's refusals, as errors.
pub async fn demo_shipments(state: &AppState, booked_by: i64, client: PartyId) -> Result<usize> {
    let mut booked = 0;
    for (reference, recipient, area, parcels, cod_amount) in DEMO_SHIPMENTS {
        if booking::using_reference(&state.db, client, reference)
            .await?
            .is_some()
        {
            continue;
        }
        let new = NewShipment {
            client,
            client_reference: Some(reference.to_owned()),
            recipient_name: recipient.to_owned(),
            recipient_phone: "+971500000000".to_owned(),
            address: format!("Villa 12, {area}"),
            area: area.to_owned(),
            parcel_weights_grams: vec![1_200; parcels],
            cod_amount,
        };
        booking::create(state, Some(booked_by), &new)
            .await
            .map_err(renox::state_machine::TransitionError::into_error)?;
        booked += 1;
    }
    Ok(booked)
}
