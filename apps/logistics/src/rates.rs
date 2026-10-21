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
