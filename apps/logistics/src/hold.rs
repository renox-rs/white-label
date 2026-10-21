//! Holding a shipment for an exception and releasing it (03 Logistics §4.1
//! *Wrong or incomplete address* and *Parcel damaged at the hub*, §7.1 rows
//! `hold`, `release` and `return`; BL-LOG-05).
//!
//! - **Hold** ([`hold`]): hub staff from a scan, or staff with
//!   `shipments.manage`, put a `created`, `at_hub` or `attempted` shipment
//!   `on_hold` with a reason (*damaged*, *address issue*, *client
//!   instruction*, *re-weigh dispute*). A damaged parcel needs photos. The
//!   state it was held from is kept. The client is told after the commit, and
//!   for *address issue* asked to confirm the address in the portal.
//!   [`hold_for_wrong_address`] is the system's: a failed attempt with the
//!   reason *wrong address* holds the shipment (reason *address issue*)
//!   instead of queuing a reattempt ([`crate::events`]).
//! - **Company liable** ([`set_company_liable`], or ticked on the hold): a
//!   linked claim ticket is opened, once.
//! - **Address** ([`correct_address`], [`confirm_address`]): the client's
//!   user or customer service corrects the address until the shipment is out
//!   for delivery; every change is audited with the old and the new values; a
//!   zone change prices the shipment again at once and the delivery's address
//!   follows. The client's confirmation (or "address unchanged, confirmed by
//!   the client") is what an *address issue* hold waits for.
//! - **Release** ([`release`]): `shipments.manage`, back to `at_hub`, only
//!   with the confirmed address (*address issue*) or the client's recorded
//!   instruction (any other reason).
//! - **Return** ([`send_back`]): `shipments.manage` sends an attempted or held
//!   shipment back, with one return charge.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::delivery::{DeliveryId, DeliveryRequester};
use wl_kernel::tickets::{self, OpenTicket};

use crate::booking;
use crate::notices::tell_client;
use crate::permissions;
use crate::reweigh;
use crate::scan::barcodes_in;
use crate::shipment::{
    AddressChange, Parcel, Shipment, ShipmentStatus, hold_reason, parcel_state, track,
};
use crate::shipment_machine::{HOLD, HOLD_SYSTEM, RELEASE, RETURN, ShipmentInput};
use crate::texts::english;

/// The ticket category of a claim for a damaged parcel.
pub const CLAIM_CATEGORY: &str = "claim";

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Shipment, TransitionError> {
    Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// What is entered on a hold.
#[derive(Debug, Clone, Default)]
pub struct NewHold {
    /// One of [`hold_reason`].
    pub reason: String,
    /// The references of the photos of a damaged parcel.
    pub photos: Vec<String>,
    /// Whether the company is liable for the damage.
    pub company_liable: bool,
    /// The label scanned when hub staff hold from a scan.
    pub barcode: Option<String>,
}

impl NewHold {
    /// The photos in `text`, separated by lines, spaces or commas.
    pub fn photos_in(text: &str) -> Vec<String> {
        barcodes_in(text)
    }
}

/// `user` (`shipments.manage` or `shipments.scan`) holds shipment `id`.
///
/// # Errors
///
/// `Forbidden` without either permission; `NotFrom` unless the shipment is
/// `created`, `at_hub` or `attempted`; `Guard` (422) without a known reason,
/// without photos for a damaged parcel, and for a label that is not one of
/// its parcels.
pub async fn hold(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: &NewHold,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE)
        && !user.has_permission(permissions::SHIPMENTS_SCAN)
    {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    if let Some(code) = new
        .barcode
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        let parcel = Parcel::query()
            .where_(Parcel::BARCODE.eq(code))
            .first(&state.db)
            .await?;
        match parcel {
            Some(parcel)
                if parcel.shipment_id == id && parcel.state != parcel_state::NOT_RECEIVED => {}
            _ => {
                return Err(guard(english(
                    "logistics.scan.unknown_label",
                    &[("barcode", &code)],
                )));
            }
        }
    }
    let _outcome = shipment
        .transition(HOLD)
        .by(user)
        .input(&ShipmentInput {
            hold_reason: Some(new.reason.trim().to_owned()),
            photos: new.photos.clone(),
            company_liable: new.company_liable,
            ..ShipmentInput::default()
        })
        .run(state)
        .await?;
    if let Some(code) = new
        .barcode
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        track(&state.db, id, "hold_scan", code, Some(user.id)).await?;
    }
    load(state, id).await
}

/// The system holds shipment `id` for *address issue* after a failed attempt
/// with *wrong address*; a shipment not `attempted` is left alone.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn hold_for_wrong_address(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    if shipment.status != ShipmentStatus::Attempted {
        return Ok(());
    }
    let _outcome = shipment
        .transition(HOLD_SYSTEM)
        .by_system()
        .input(&ShipmentInput {
            hold_reason: Some(hold_reason::ADDRESS_ISSUE.to_owned()),
            ..ShipmentInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// The client asked to stop the delivery of a shipment whose re-weigh it
/// disputes: the system holds it with the reason *re-weigh dispute*. A
/// shipment that cannot be held (not `created`, `at_hub` or `attempted`) is
/// left as it is.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn hold_for_reweigh_dispute(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    if !matches!(
        shipment.status,
        ShipmentStatus::Created | ShipmentStatus::AtHub | ShipmentStatus::Attempted
    ) {
        return Ok(());
    }
    let _outcome = shipment
        .transition(HOLD_SYSTEM)
        .by_system()
        .input(&ShipmentInput {
            hold_reason: Some(hold_reason::REWEIGH_DISPUTE.to_owned()),
            ..ShipmentInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// Tells the client that shipment `id` is on hold and what is needed.
pub(crate) async fn tell_of_hold(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let reason = shipment.hold_reason.clone().unwrap_or_default();
    let title = english("logistics.hold.title", &[("number", &shipment.number)]);
    let key = format!("logistics.hold.body.{reason}");
    let body = english(&key, &[("number", &shipment.number)]);
    tell_client(state, &shipment, &title, &body).await
}

/// Opens the claim ticket of shipment `id` when the company is liable and no
/// ticket is linked yet. A refused ticket (an unknown category) is written to
/// the tracking events and the shipment stays as it is.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn open_claim(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    if !shipment.company_liable || shipment.claim_ticket_id.is_some() {
        return Ok(());
    }
    let title = english(
        "logistics.hold.claim_title",
        &[("number", &shipment.number)],
    );
    let photos = shipment
        .hold_photos
        .clone()
        .unwrap_or_default()
        .replace('\n', ", ");
    let description = english(
        "logistics.hold.claim_description",
        &[("number", &shipment.number), ("photos", &photos)],
    );
    let request = OpenTicket::new(shipment.subject(), CLAIM_CATEGORY, &title)
        .description(&description)
        .requester_party(shipment.client())
        .source_key(&format!("logistics.claim:{}", shipment.id));
    match tickets::task_creator(state).open(state, request).await {
        Ok(Some(ticket)) => {
            let ticket_id = Some(ticket.id.0);
            let number = Some(ticket.number.clone());
            Shipment::query()
                .where_(Shipment::ID.eq(id))
                .update(
                    &state.db,
                    &[
                        ("claim_ticket_id", &ticket_id),
                        ("claim_ticket_number", &number),
                    ],
                )
                .await?;
            track(&state.db, id, "claim_ticket", &ticket.number, None).await?;
        }
        Ok(None) => {
            track(
                &state.db,
                id,
                "claim_ticket_skipped",
                "ticketing is off",
                None,
            )
            .await?;
        }
        Err(error) => {
            let note = format!("{error:?}");
            track(&state.db, id, "claim_ticket_failed", &note, None).await?;
        }
    }
    Ok(())
}

/// `user` (`shipments.manage`) ticks "company liable" on the held shipment
/// `id`; a claim ticket is opened and linked, once.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) unless the shipment
/// is `on_hold` for *damaged*.
pub async fn set_company_liable(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = load(state, id).await?;
    if shipment.status != ShipmentStatus::OnHold
        || shipment.hold_reason.as_deref() != Some(hold_reason::DAMAGED)
    {
        return Err(guard(english("logistics.hold.not_damaged", &[])));
    }
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(&state.db, &[("company_liable", &true)])
        .await?;
    track(&state.db, id, "company_liable", "", Some(user.id)).await?;
    open_claim(state, id).await?;
    load(state, id).await
}

/// `user` (`shipments.manage`) records the client's instruction for the hold
/// of shipment `id` (deliver as is, return, ...): who asked, when and how.
///
/// # Errors
///
/// `Forbidden` for anybody but staff and the client's own users; `Guard`
/// (422) for a blank note and unless the shipment is `on_hold`.
pub async fn record_instruction(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    if !may_act(state, user, &shipment).await? {
        return Err(TransitionError::Forbidden);
    }
    if shipment.status != ShipmentStatus::OnHold {
        return Err(guard(english("logistics.hold.not_on_hold", &[])));
    }
    if note.trim().is_empty() {
        return Err(guard(english("logistics.hold.instruction_needed", &[])));
    }
    let text = Some(note.trim().to_owned());
    let by = Some(user.id);
    let at = Some(renox::db::now());
    let mut tx = state.db.begin().await?;
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(
            &mut tx,
            &[
                ("hold_instruction", &text),
                ("hold_instruction_by", &by),
                ("hold_instruction_at", &at),
            ],
        )
        .await?;
    track(&mut tx, id, "hold_instruction", note.trim(), by).await?;
    tx.commit().await?;
    load(state, id).await
}

/// Whether `user` may act on the shipment for its client: staff with
/// `shipments.manage`, or a client user with `shipments.book` the portal says
/// acts for the company.
async fn may_act(
    state: &AppState,
    user: &AuthUser,
    shipment: &Shipment,
) -> std::result::Result<bool, TransitionError> {
    if user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Ok(true);
    }
    Ok(user.has_permission(permissions::SHIPMENTS_BOOK)
        && booking::acts_for(state, user, shipment.client()).await?)
}

/// The address a client or customer service enters.
#[derive(Debug, Clone, Default)]
pub struct NewAddress {
    /// The address as text.
    pub address: String,
    /// The area (the zone of the rate card).
    pub area: String,
}

/// Writes the correction, the audit row and the new price in one
/// transaction, then tells the delivery.
async fn change_address(
    state: &AppState,
    user: &AuthUser,
    shipment: &Shipment,
    new: &NewAddress,
) -> std::result::Result<(), TransitionError> {
    let address = new.address.trim().to_owned();
    let area = new.area.trim().to_owned();
    if address.is_empty() {
        return Err(guard(english("logistics.hold.address_blank", &[])));
    }
    let mut tx = state.db.begin().await?;
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(&mut tx, &[("address", &address), ("area", &area)])
        .await?;
    let repriced = reweigh::reprice(&mut tx, shipment.id).await?;
    let mut change = AddressChange {
        shipment_id: shipment.id,
        old_address: shipment.address.clone(),
        old_area: shipment.area.clone(),
        new_address: address.clone(),
        new_area: area.clone(),
        old_price: shipment.price,
        new_price: repriced.price,
        changed_by: Some(user.id),
        changed_at: renox::db::now(),
        ..AddressChange::default()
    };
    change.insert(&mut tx).await?;
    let note = english(
        "logistics.hold.address_event",
        &[("address", &address), ("area", &area)],
    );
    track(
        &mut tx,
        shipment.id,
        "address_corrected",
        &note,
        Some(user.id),
    )
    .await?;
    tx.commit().await?;
    if let Some(delivery) = shipment.delivery_id
        && state
            .provided::<std::sync::Arc<dyn DeliveryRequester>>()
            .is_some()
        && let Err(error) = wl_kernel::delivery::requester(state)
            .update_address(state, DeliveryId(delivery), &address, &area)
            .await
    {
        let note = format!("{error:?}");
        track(
            &state.db,
            shipment.id,
            "delivery_address_failed",
            &note,
            None,
        )
        .await?;
    }
    Ok(())
}

/// `user` corrects the address of shipment `id` before it is out for
/// delivery: the client's own user or staff with `shipments.manage`. The
/// change is audited and a zone change prices the shipment again at once.
///
/// # Errors
///
/// `Forbidden` for anybody else; `Guard` (422) for a blank address and when
/// the shipment is out for delivery or closed.
pub async fn correct_address(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: &NewAddress,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    if !may_act(state, user, &shipment).await? {
        return Err(TransitionError::Forbidden);
    }
    if !matches!(
        shipment.status,
        ShipmentStatus::Created
            | ShipmentStatus::PickedUp
            | ShipmentStatus::AtHub
            | ShipmentStatus::Attempted
            | ShipmentStatus::OnHold
    ) {
        return Err(guard(english("logistics.hold.address_locked", &[])));
    }
    change_address(state, user, &shipment, new).await?;
    load(state, id).await
}

/// `user` confirms the address of the shipment `id` held for *address issue*:
/// with a new address (corrected and audited, repriced when the zone changes),
/// or with none, as "address unchanged, confirmed by the client".
///
/// # Errors
///
/// `Forbidden` for anybody but the client's own user or staff with
/// `shipments.manage`; `Guard` (422) unless the shipment is `on_hold` for
/// *address issue*.
pub async fn confirm_address(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new: Option<&NewAddress>,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    if !may_act(state, user, &shipment).await? {
        return Err(TransitionError::Forbidden);
    }
    if shipment.status != ShipmentStatus::OnHold
        || shipment.hold_reason.as_deref() != Some(hold_reason::ADDRESS_ISSUE)
    {
        return Err(guard(english("logistics.hold.not_address_hold", &[])));
    }
    let note = match new {
        Some(new) => {
            change_address(state, user, &shipment, new).await?;
            english(
                "logistics.hold.address_event",
                &[("address", &new.address.trim()), ("area", &new.area.trim())],
            )
        }
        None => english("logistics.hold.address_unchanged", &[]),
    };
    let by = Some(user.id);
    let at = Some(renox::db::now());
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(
            &state.db,
            &[("address_confirmed_at", &at), ("address_confirmed_by", &by)],
        )
        .await?;
    track(&state.db, id, "address_confirmed", &note, by).await?;
    load(state, id).await
}

/// `user` (`shipments.manage`) releases the held shipment `id` back to the
/// hub.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `NotFrom` unless `on_hold`;
/// `Guard` (422) without the confirmed address or the recorded instruction.
pub async fn release(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let _outcome = shipment
        .transition(RELEASE)
        .by(user)
        .input(&ShipmentInput::default())
        .run(state)
        .await?;
    load(state, id).await
}

/// `user` (`shipments.manage`) sends the attempted or held shipment `id`
/// back to the client, with one return charge.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `NotFrom` unless `attempted` or
/// `on_hold`.
pub async fn send_back(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let _outcome = shipment
        .transition(RETURN)
        .by(user)
        .input(&ShipmentInput::default())
        .run(state)
        .await?;
    load(state, id).await
}
