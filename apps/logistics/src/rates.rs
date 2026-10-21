//! The client's prices that the negative flows of a shipment read: the
//! cancellation fee of `cancel` and the return charge of `cancel_return`
//! (03 Logistics §7.1, `FIN.RAT.01`).
//!
//! The rate cards of wl-invoicing (`FIN.RAT.01`) are not built yet. Until they
//! are, a client's two lines are kept here as [`ClientRate`] rows, one per
//! kind, and [`price`] is the only place that reads them: replacing it with a
//! call to the `RateCards` contract of wl-kernel changes nothing else. A
//! client without a row for a kind has no such line on its rate card.

use renox::db::{DateTime, Executor};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;

use crate::shipment::{Charge, Shipment};

/// One line of a client's prices (a stand-in for a rate card line).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_client_rates", index(client_party_id))]
pub struct ClientRate {
    /// The primary key.
    pub id: i64,
    /// The client.
    pub client_party_id: i64,
    /// `cancellation` or `return` ([`crate::shipment::charge_kind`]).
    pub kind: String,
    /// The price, in fils.
    pub amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// One band of a client's zone prices (a stand-in for the rate card's
/// weight bands, `FIN.RAT.01`): shipments up to `up_to_grams` cost `amount`.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_weight_bands", index(client_party_id))]
pub struct WeightBand {
    /// The primary key.
    pub id: i64,
    /// The client.
    pub client_party_id: i64,
    /// The zone (a shipment's area); blank is every zone.
    pub zone: String,
    /// The band covers weights up to this many grams.
    pub up_to_grams: i64,
    /// The price, in fils.
    pub amount: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Sets the band of `client` in `zone` up to `up_to_grams`, replacing the
/// one there is.
///
/// # Errors
///
/// Database errors.
pub async fn set_band(
    db: &Db,
    client: PartyId,
    zone: &str,
    up_to_grams: i64,
    amount: i64,
) -> Result {
    let zone = zone.trim().to_lowercase();
    let found = WeightBand::query()
        .where_(WeightBand::CLIENT_PARTY_ID.eq(client.0))
        .where_(WeightBand::ZONE.eq(zone.as_str()))
        .where_(WeightBand::UP_TO_GRAMS.eq(up_to_grams))
        .first(db)
        .await?;
    match found {
        Some(mut band) => {
            band.amount = amount;
            band.save(db).await?;
        }
        None => {
            let mut band = WeightBand {
                client_party_id: client.0,
                zone,
                up_to_grams,
                amount,
                ..WeightBand::default()
            };
            band.insert(db).await?;
        }
    }
    Ok(())
}

/// The price of `grams` in `zone` on `client`'s bands: the first band of the
/// zone (else of the blank zone) that covers the weight, the heaviest band
/// when the weight is above all of them; `None` when the client has none.
///
/// # Errors
///
/// Database errors.
pub async fn band_price<'c, E: Executor<'c>>(
    executor: E,
    client: PartyId,
    zone: &str,
    grams: i64,
) -> Result<Option<i64>> {
    let bands = WeightBand::query()
        .where_(WeightBand::CLIENT_PARTY_ID.eq(client.0))
        .order_by(WeightBand::UP_TO_GRAMS)
        .get(executor)
        .await?;
    let zone = zone.trim().to_lowercase();
    let in_zone: Vec<&WeightBand> = bands.iter().filter(|band| band.zone == zone).collect();
    let usable = if in_zone.is_empty() {
        bands.iter().filter(|band| band.zone.is_empty()).collect()
    } else {
        in_zone
    };
    Ok(usable
        .iter()
        .find(|band| band.up_to_grams >= grams)
        .or_else(|| usable.last())
        .map(|band| band.amount))
}

/// Fils as text: `AED 25.00`.
pub fn aed(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    format!("AED {sign}{}.{:02}", fils.abs() / 100, fils.abs() % 100)
}

/// Sets the price of `kind` for `client`, replacing the one there is.
///
/// # Errors
///
/// Database errors.
pub async fn set_price(db: &Db, client: PartyId, kind: &str, amount: i64) -> Result {
    let found = ClientRate::query()
        .where_(ClientRate::CLIENT_PARTY_ID.eq(client.0))
        .where_(ClientRate::KIND.eq(kind))
        .first(db)
        .await?;
    match found {
        Some(mut rate) => {
            rate.amount = amount;
            rate.save(db).await?;
        }
        None => {
            let mut rate = ClientRate {
                client_party_id: client.0,
                kind: kind.to_owned(),
                amount,
                ..ClientRate::default()
            };
            rate.insert(db).await?;
        }
    }
    Ok(())
}

/// The price of `kind` on `client`'s rate card, or `None` when the card has
/// no such line.
///
/// # Errors
///
/// Database errors.
pub async fn price<'c, E: Executor<'c>>(
    executor: E,
    client: PartyId,
    kind: &str,
) -> Result<Option<i64>> {
    Ok(ClientRate::query()
        .where_(ClientRate::CLIENT_PARTY_ID.eq(client.0))
        .where_(ClientRate::KIND.eq(kind))
        .first(executor)
        .await?
        .map(|rate| rate.amount))
}

/// Records the charge `kind` of `amount` fils for `shipment`, once: a second
/// call for the same kind records nothing (a shipment has exactly one return
/// charge, whichever path sent it back).
///
/// # Errors
///
/// Database errors.
pub async fn record_charge(
    tx: &mut renox::db::Transaction,
    shipment: &Shipment,
    kind: &str,
    amount: i64,
    note: &str,
) -> Result<bool> {
    let existing = Charge::query()
        .where_(Charge::SHIPMENT_ID.eq(shipment.id))
        .where_(Charge::KIND.eq(kind))
        .count(&mut *tx)
        .await?;
    if existing > 0 {
        return Ok(false);
    }
    let mut charge = Charge {
        shipment_id: shipment.id,
        client_party_id: shipment.client_party_id,
        kind: kind.to_owned(),
        amount,
        note: note.to_owned(),
        recorded_at: renox::db::now(),
        ..Charge::default()
    };
    charge.insert(&mut *tx).await?;
    Ok(true)
}
