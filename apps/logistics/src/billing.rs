//! What the monthly invoice reads from the shipments (03 Logistics §7.2
//! `LOG.RUL.07`, BL-LOG-20). The draft invoice itself is built by BL-LOG-20
//! on the usage billing of wl-invoicing (`FIN.USG.01`); until then these
//! are the seams it calls, and the tests stand in for it.

use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::PartyId;

use crate::shipment::{Charge, Shipment, charge_kind};

/// The re-weigh surcharges of `client` still to be billed on its next draft:
/// not on an invoice yet and not held by an open re-weigh ticket. A surcharge
/// recorded after its shipment was invoiced is among them; it goes on the
/// next draft and never changes the issued invoice.
///
/// # Errors
///
/// Database errors.
pub async fn unbilled_surcharges(db: &Db, client: PartyId) -> Result<Vec<Charge>> {
    Charge::query()
        .where_(Charge::CLIENT_PARTY_ID.eq(client.0))
        .where_(Charge::KIND.eq(charge_kind::REWEIGH))
        .where_(Charge::HELD.eq(false))
        .where_(Charge::BILLED_ON.is_null())
        .order_by(Charge::ID)
        .get(db)
        .await
}

/// Marks the charge lines `ids` as billed `at` (the draft they went on).
///
/// # Errors
///
/// Database errors.
pub async fn mark_billed(db: &Db, ids: &[i64], at: DateTime) -> Result {
    let at = Some(at);
    for id in ids {
        Charge::query()
            .where_(Charge::ID.eq(*id))
            .update(db, &[("billed_on", &at)])
            .await?;
    }
    Ok(())
}

/// Marks shipment `id` as on an issued invoice `at`.
///
/// # Errors
///
/// Database errors.
pub async fn mark_shipment_invoiced(db: &Db, id: i64, at: DateTime) -> Result {
    let at = Some(at);
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(db, &[("invoiced_on", &at)])
        .await?;
    Ok(())
}
