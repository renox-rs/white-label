//! Cancelling a shipment (03 Logistics §4.1 *Client cancels*, §7.1 rows
//! `cancel`, `cancel_return` and `return_delivered`; `LOG.ACL.09`;
//! BL-LOG-02).
//!
//! - **Before pickup** a client user of the company, or staff with
//!   `shipments.manage`, runs [`cancel`]: a cancellation fee line only when the
//!   client's rate card has one, and the shipment's delivery is cancelled
//!   ([`crate::events`]).
//! - **After pickup** only staff with `shipments.manage` (the dispatcher, the
//!   operations manager) run [`cancel_return`], on the client's recorded
//!   instruction ([`record_instruction`]: who asked, when and how): the
//!   shipment goes back to the client (`returning`) with one return charge,
//!   its open delivery is cancelled and the client is told. A client user
//!   has no action past `created`; "ask us to return it" opens a request to
//!   the depot's dispatchers (deferred, see the audit notes).
//! - **Out for delivery** `cancel_return` is not offered: the driver first
//!   fails the stop with the reason *cancelled by client*, which does not count
//!   as an attempt (the delivery's failure reason, `WRK.RUL.08`), and the
//!   shipment follows to `attempted`.
//! - **At the client** the e-POD of the returned goods makes the shipment
//!   `returned` ([`return_delivered`]).

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};

use crate::booking;
use crate::shipment::{Shipment, track};
use crate::shipment_machine::{CANCEL, CANCEL_RETURN, RETURN_DELIVERED, ShipmentInput};
use crate::texts::english;

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// Writes the client's instruction on the shipment and a tracking event, in
/// the transaction of the transition or of [`record_instruction`].
///
/// # Errors
///
/// Database errors.
pub async fn store_instruction(
    tx: &mut Transaction,
    id: i64,
    text: &str,
    actor: Option<i64>,
) -> Result {
    let text = Some(text.trim().to_owned());
    let at = Some(renox::db::now());
    Shipment::query()
        .where_(Shipment::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("return_instruction", &text),
                ("return_instruction_by", &actor),
                ("return_instruction_at", &at),
            ],
        )
        .await?;
    track(
        &mut *tx,
        id,
        "return_instruction",
        text.as_deref().unwrap_or_default(),
        actor,
    )
    .await
}

async fn load(state: &AppState, id: i64) -> std::result::Result<Shipment, TransitionError> {
    Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// `user` (`shipments.manage`) records the client's instruction to send
/// shipment `id` back: who asked, when and how (a note, an e-mail, a call).
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) for a blank note.
pub async fn record_instruction(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    note: &str,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(crate::permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    if note.trim().is_empty() {
        return Err(guard(english("logistics.cancel.instruction_needed", &[])));
    }
    let shipment = load(state, id).await?;
    let mut tx = state.db.begin().await?;
    store_instruction(&mut tx, shipment.id, note, Some(user.id)).await?;
    tx.commit().await?;
    load(state, id).await
}

/// `user` cancels the `created` shipment `id`.
///
/// # Errors
///
/// `Forbidden` without `shipments.cancel`; `NotFrom` past `created` (a
/// client user has no cancel after pickup); `Guard` (422) for a client user
/// of another company.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let client_acts = booking::acts_for(state, user, shipment.client()).await?;
    let _outcome = shipment
        .transition(CANCEL)
        .by(user)
        .input(&ShipmentInput {
            client_acts,
            ..ShipmentInput::default()
        })
        .run(state)
        .await?;
    load(state, id).await
}

/// `user` (`shipments.manage`) sends shipment `id` back to the client on its
/// recorded instruction; `instruction`, when given, is recorded first.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `NotFrom` unless it is
/// `picked_up`, `at_hub`, `attempted` or `on_hold` (so not from
/// `out_for_delivery` or `returning`); `Guard` (422) without a recorded
/// instruction.
pub async fn cancel_return(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    instruction: &str,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let instruction = Some(instruction.trim().to_owned()).filter(|text| !text.is_empty());
    let _outcome = shipment
        .transition(CANCEL_RETURN)
        .by(user)
        .input(&ShipmentInput {
            instruction,
            ..ShipmentInput::default()
        })
        .run(state)
        .await?;
    load(state, id).await
}

/// `user` (the driver, `shipments.deliver`) records the client's e-POD for
/// the returned goods of shipment `id`.
///
/// # Errors
///
/// `Forbidden` without `shipments.deliver`; `NotFrom` unless it is
/// `returning`; `Guard` (422) without the e-POD.
pub async fn return_delivered(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    pod_reference: &str,
) -> std::result::Result<Shipment, TransitionError> {
    let shipment = load(state, id).await?;
    let _outcome = shipment
        .transition(RETURN_DELIVERED)
        .by(user)
        .input(&ShipmentInput {
            pod_reference: Some(pod_reference.trim().to_owned()),
            ..ShipmentInput::default()
        })
        .run(state)
        .await?;
    load(state, id).await
}
