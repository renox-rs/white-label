//! The re-weigh at the hub (03 Logistics §4.1 *Weight or size differs at the
//! hub*, §7.2 `LOG.RUL.07`, `LOG.HUB.01`; BL-LOG-04).
//!
//! Hub staff record the measured weight and dimensions with a photo of the
//! scale ([`record`]). A measurement is not a status change, so it is not a
//! transition: it is a [`Measurement`] row, and every one stays.
//!
//! - **Chargeable weight.** The actual weight; when the client's rate card
//!   prices by volume ([`ClientTerms::volumetric_divisor`]) the higher of the
//!   actual weight and the volumetric weight (length x width x height in cm
//!   over the divisor, in kg).
//! - **Tolerance.** [`ClientTerms::tolerance_grams`], 0 when the card sets
//!   none. The measured chargeable weight is compared with the weight the
//!   shipment is charged on (the booked weight, or the weight of the parcels
//!   received after a partial pickup).
//! - **Above the tolerance** the shipment is priced on the measured value
//!   ([`Shipment::reweigh_price`]) and the difference to the price on the
//!   booked weight is the **re-weigh surcharge**, one `reweigh` charge line
//!   for the next monthly draft; the client is told. A photo is required.
//!   Within the tolerance, or lighter than booked, the booked price stands
//!   and a surcharge is removed. The latest measurement counts; none reprices
//!   below the booked price.
//! - **Dispute.** The client (or staff for it) raises a *billing* ticket from
//!   the shipment ([`open_dispute`]); the surcharge is `held` out of the draft
//!   until customer service resolves it ([`resolve_dispute`]): confirmed, it
//!   goes on the next draft; removed, the shipment is priced on the booked
//!   weight again. The monthly draft itself is BL-LOG-20; [`crate::billing`]
//!   is the seam it reads.
//! - **Already invoiced.** A measurement after the shipment was invoiced never
//!   changes that invoice: the surcharge simply waits, unbilled, for the
//!   client's next draft.
//! - **COD** is never touched.

use renox::db::{DateTime, Executor, Transaction};
use renox::prelude::*;
use renox::state_machine::TransitionError;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::tickets::{self, OpenTicket, TaskComment};

use crate::booking;
use crate::notices::tell_client;
use crate::permissions;
use crate::pickup::kg;
use crate::rates::{self, aed};
use crate::shipment::{Charge, Shipment, ShipmentStatus, charge_kind, track};
use crate::texts::english;

/// The ticket category of a re-weigh dispute.
pub const DISPUTE_CATEGORY: &str = "billing";

/// A client's terms for the re-weigh (a stand-in for its rate card's,
/// `FIN.RAT.01`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_client_terms", unique(client_party_id))]
pub struct ClientTerms {
    /// The primary key.
    pub id: i64,
    /// The client.
    pub client_party_id: i64,
    /// The accepted difference between booked and measured weight, in
    /// grams; 0 when the rate card sets none.
    pub tolerance_grams: i64,
    /// The volumetric divisor, when the card prices by volume.
    pub volumetric_divisor: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// Sets the re-weigh terms of `client`.
///
/// # Errors
///
/// Database errors.
pub async fn set_terms(
    db: &Db,
    client: PartyId,
    tolerance_grams: i64,
    volumetric_divisor: Option<i64>,
) -> Result {
    let found = ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client.0))
        .first(db)
        .await?;
    match found {
        Some(mut terms) => {
            terms.tolerance_grams = tolerance_grams;
            terms.volumetric_divisor = volumetric_divisor;
            terms.save(db).await?;
        }
        None => {
            let mut terms = ClientTerms {
                client_party_id: client.0,
                tolerance_grams,
                volumetric_divisor,
                ..ClientTerms::default()
            };
            terms.insert(db).await?;
        }
    }
    Ok(())
}

/// The re-weigh terms of `client`: no tolerance and no volume pricing when
/// the card has none.
///
/// # Errors
///
/// Database errors.
pub async fn terms<'c, E: Executor<'c>>(executor: E, client: PartyId) -> Result<ClientTerms> {
    Ok(ClientTerms::query()
        .where_(ClientTerms::CLIENT_PARTY_ID.eq(client.0))
        .first(executor)
        .await?
        .unwrap_or_default())
}

/// One measurement at the hub.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "logistics_measurements", index(shipment_id))]
pub struct Measurement {
    /// The primary key.
    pub id: i64,
    /// The shipment.
    #[model(references = "logistics_shipments")]
    pub shipment_id: i64,
    /// The actual weight, in grams.
    pub weight_grams: i64,
    /// The length, in cm; 0 when not measured.
    pub length_cm: i64,
    /// The width, in cm.
    pub width_cm: i64,
    /// The height, in cm.
    pub height_cm: i64,
    /// The higher of the actual and the volumetric weight, in grams.
    pub chargeable_grams: i64,
    /// The photo of the scale.
    pub photo_ref: Option<String>,
    /// The user who measured.
    pub measured_by: Option<i64>,
    /// When it was measured.
    pub measured_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What hub staff enter.
#[derive(Debug, Clone, Default)]
pub struct NewMeasurement {
    /// The actual weight, in grams.
    pub weight_grams: i64,
    /// The length, in cm; 0 when not measured.
    pub length_cm: i64,
    /// The width, in cm.
    pub width_cm: i64,
    /// The height, in cm.
    pub height_cm: i64,
    /// The photo of the scale (a reference to the stored file).
    pub photo: Option<String>,
}

/// The chargeable weight of `measurement` in grams: the actual weight, or
/// the volumetric weight when the card prices by volume and it is higher.
pub fn chargeable_grams(measurement: &NewMeasurement, divisor: Option<i64>) -> i64 {
    let volumetric = match divisor {
        Some(divisor) if divisor > 0 => {
            measurement.length_cm * measurement.width_cm * measurement.height_cm * 1_000 / divisor
        }
        _ => 0,
    };
    measurement.weight_grams.max(volumetric)
}

/// What a pricing came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Repriced {
    /// The price on the weight received.
    pub price: i64,
    /// The price on the measured values, when above the tolerance.
    pub reweigh_price: Option<i64>,
    /// The surcharge, 0 for none.
    pub surcharge: i64,
}

/// Prices shipment `id` again from its weight, zone and latest measurement,
/// and keeps its surcharge line in step. A surcharge already on an issued
/// invoice is left as it is.
pub(crate) async fn reprice(tx: &mut Transaction, id: i64) -> Result<Repriced> {
    let shipment = Shipment::query()
        .where_(Shipment::ID.eq(id))
        .first_or_404(&mut *tx)
        .await?;
    let terms = terms(&mut *tx, shipment.client()).await?;
    let price = rates::band_price(&mut *tx, shipment.client(), &shipment.area, shipment.weight_grams)
        .await?
        .unwrap_or(shipment.price);
    let mut reweigh_price = None;
    let mut surcharge = 0;
    if let Some(measured) = shipment.measured_weight_grams
        && measured > shipment.weight_grams + terms.tolerance_grams
    {
        let measured_price =
            rates::band_price(&mut *tx, shipment.client(), &shipment.area, measured)
                .await?
                .unwrap_or(price);
        // A lighter parcel never reprices.
        if measured_price > price {
            reweigh_price = Some(measured_price);
            surcharge = measured_price - price;
        }
    }
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(
            &mut *tx,
            &[("price", &price), ("reweigh_price", &reweigh_price)],
        )
        .await?;
    set_surcharge(&mut *tx, &shipment, surcharge).await?;
    Ok(Repriced {
        price,
        reweigh_price,
        surcharge,
    })
}

async fn set_surcharge(tx: &mut Transaction, shipment: &Shipment, surcharge: i64) -> Result {
    let existing = Charge::query()
        .where_(Charge::SHIPMENT_ID.eq(shipment.id))
        .where_(Charge::KIND.eq(charge_kind::REWEIGH))
        .first(&mut *tx)
        .await?;
    match existing {
        Some(charge) if charge.billed_on.is_some() => {}
        Some(charge) if surcharge > 0 => {
            Charge::query()
                .where_(Charge::ID.eq(charge.id))
                .update(&mut *tx, &[("amount", &surcharge)])
                .await?;
        }
        Some(charge) => {
            Charge::query()
                .where_(Charge::ID.eq(charge.id))
                .delete(&mut *tx)
                .await?;
        }
        None if surcharge > 0 => {
            let mut charge = Charge {
                shipment_id: shipment.id,
                client_party_id: shipment.client_party_id,
                kind: charge_kind::REWEIGH.to_owned(),
                amount: surcharge,
                note: english("logistics.reweigh.charge_note", &[]),
                recorded_at: renox::db::now(),
                ..Charge::default()
            };
            charge.insert(&mut *tx).await?;
        }
        None => {}
    }
    Ok(())
}

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Shipment, TransitionError> {
    Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (hub staff, `shipments.scan`, or `shipments.manage`) records a
/// measurement of shipment `id`.
///
/// # Errors
///
/// `Forbidden` without either permission; `Guard` (422) for a shipment not
/// yet in our hands (`created`) or closed (`cancelled`, `voided`,
/// `disposed`), for a weight that is not above zero, and for a measured
/// chargeable weight above the tolerance without a photo of the scale.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: &NewMeasurement,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_SCAN)
        && !user.has_permission(permissions::SHIPMENTS_MANAGE)
    {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    if matches!(
        shipment.status,
        ShipmentStatus::Created
            | ShipmentStatus::Cancelled
            | ShipmentStatus::Voided
            | ShipmentStatus::Disposed
    ) {
        return Err(guard(english("logistics.reweigh.not_now", &[])));
    }
    if new.weight_grams <= 0 || new.length_cm < 0 || new.width_cm < 0 || new.height_cm < 0 {
        return Err(guard(english("logistics.reweigh.bad_values", &[])));
    }
    let terms = terms(&state.db, shipment.client()).await?;
    let chargeable = chargeable_grams(new, terms.volumetric_divisor);
    let above = chargeable > shipment.weight_grams + terms.tolerance_grams;
    let photo = new
        .photo
        .as_deref()
        .map(str::trim)
        .filter(|photo| !photo.is_empty())
        .map(str::to_owned);
    if above && photo.is_none() {
        return Err(guard(english("logistics.reweigh.photo_needed", &[])));
    }
    let mut tx = state.db.begin().await?;
    let mut row = Measurement {
        shipment_id: id,
        weight_grams: new.weight_grams,
        length_cm: new.length_cm,
        width_cm: new.width_cm,
        height_cm: new.height_cm,
        chargeable_grams: chargeable,
        photo_ref: photo,
        measured_by: Some(user.id),
        measured_at: renox::db::now(),
        ..Measurement::default()
    };
    row.insert(&mut tx).await?;
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(&mut tx, &[("measured_weight_grams", &Some(chargeable))])
        .await?;
    let outcome = reprice(&mut tx, id).await?;
    let note = english(
        "logistics.reweigh.event",
        &[
            ("weight", &kg(new.weight_grams)),
            ("chargeable", &kg(chargeable)),
            ("surcharge", &aed(outcome.surcharge)),
        ],
    );
    track(&mut tx, id, "reweighed", &note, Some(user.id)).await?;
    tx.commit().await?;
    let after = load(state, id).await?;
    if outcome.surcharge > 0 {
        let title = english(
            "logistics.reweigh.notice_title",
            &[("number", &after.number)],
        );
        let body = english(
            "logistics.reweigh.notice_body",
            &[
                ("number", &after.number),
                ("booked", &kg(after.weight_grams)),
                ("measured", &kg(chargeable)),
                ("price", &aed(after.charged_price())),
                ("surcharge", &aed(outcome.surcharge)),
            ],
        );
        tell_client(state, &after, &title, &body).await?;
    }
    Ok(after)
}

/// The measurements of `shipment`, oldest first.
///
/// # Errors
///
/// Database errors.
pub async fn measurements(db: &Db, shipment: &Shipment) -> Result<Vec<Measurement>> {
    Measurement::query()
        .where_(Measurement::SHIPMENT_ID.eq(shipment.id))
        .order_by(Measurement::ID)
        .get(db)
        .await
}

/// The re-weigh surcharge line of `shipment`, if it has one.
///
/// # Errors
///
/// Database errors.
pub async fn surcharge_of(db: &Db, shipment: &Shipment) -> Result<Option<Charge>> {
    Charge::query()
        .where_(Charge::SHIPMENT_ID.eq(shipment.id))
        .where_(Charge::KIND.eq(charge_kind::REWEIGH))
        .first(db)
        .await
}

/// `user` raises a *billing* ticket from shipment `id` to dispute its
/// surcharge before the invoice is issued: the client's own user (the portal
/// says they act for it) or staff with `shipments.manage`. The surcharge is
/// held out of the draft while the ticket is open. Without the ticketing
/// module there is no ticket, and the surcharge is held all the same.
///
/// # Errors
///
/// `Forbidden` for anybody else; `Guard` (422) without a surcharge still to
/// be invoiced (after the invoice it is an invoice dispute, BL-INV-04), and
/// when it is held already.
pub async fn open_dispute(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let allowed = user.has_permission(permissions::SHIPMENTS_MANAGE)
        || (user.has_permission(permissions::SHIPMENTS_BOOK)
            && booking::acts_for(state, user, shipment.client()).await?);
    if !allowed {
        return Err(TransitionError::Forbidden);
    }
    let charge = match surcharge_of(&state.db, &shipment).await? {
        Some(charge) if charge.billed_on.is_none() => charge,
        _ => return Err(guard(english("logistics.reweigh.no_surcharge", &[]))),
    };
    if charge.held {
        return Err(guard(english("logistics.reweigh.already_disputed", &[])));
    }
    let title = english(
        "logistics.reweigh.ticket_title",
        &[("number", &shipment.number)],
    );
    let request = OpenTicket::new(shipment.subject(), DISPUTE_CATEGORY, &title)
        .description(note.trim())
        .requester_user(user.id)
        .source_key(&format!("logistics.reweigh:{}", shipment.id));
    let ticket = tickets::task_creator(state).open(state, request).await?;
    let ticket_id = ticket.map(|ticket| ticket.id.0);
    let mut tx = state.db.begin().await?;
    Charge::query()
        .where_(Charge::ID.eq(charge.id))
        .update(&mut tx, &[("held", &true), ("ticket_id", &ticket_id)])
        .await?;
    track(&mut tx, id, "reweigh_disputed", note.trim(), Some(user.id)).await?;
    tx.commit().await?;
    load(state, id).await
}

/// `user` (`shipments.manage`, customer service) resolves the open re-weigh
/// ticket of shipment `id`: `confirm` keeps the surcharge for the next draft,
/// otherwise it is removed and the shipment is priced on the weight received
/// again. The ticket is resolved from here.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) without a held
/// surcharge.
pub async fn resolve_dispute(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    confirm: bool,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    let charge = match surcharge_of(&state.db, &shipment).await? {
        Some(charge) if charge.held => charge,
        _ => return Err(guard(english("logistics.reweigh.no_dispute", &[]))),
    };
    let mut tx = state.db.begin().await?;
    if confirm {
        Charge::query()
            .where_(Charge::ID.eq(charge.id))
            .update(&mut tx, &[("held", &false)])
            .await?;
        track(&mut tx, id, "reweigh_confirmed", "", Some(user.id)).await?;
    } else {
        Charge::query()
            .where_(Charge::ID.eq(charge.id))
            .delete(&mut tx)
            .await?;
        Shipment::query()
            .where_(Shipment::ID.eq(id))
            .update(&mut tx, &[("reweigh_price", &None::<i64>)])
            .await?;
        track(&mut tx, id, "reweigh_removed", "", Some(user.id)).await?;
    }
    tx.commit().await?;
    if let Some(ticket) = charge.ticket_id {
        let key = if confirm {
            "logistics.reweigh.resolved_confirmed"
        } else {
            "logistics.reweigh.resolved_removed"
        };
        let note = english(key, &[]);
        let creator = tickets::task_creator(state);
        creator
            .resolve_from_source(state, tickets::TicketId(ticket), &note)
            .await?;
        creator
            .comment(state, tickets::TicketId(ticket), TaskComment::visible(&note))
            .await?;
    }
    load(state, id).await
}
