//! A delivered shipment whose delivery is under review (03 Logistics §4.1
//! row *Recipient denies the delivery*, §7.1 rows `open_review` and
//! `close_review`, `WRK.ACL.06`; BL-LOG-09).
//!
//! - **Raising it** ([`raise`]): staff with `shipments.manage` (the
//!   dispatcher or the operations manager; the host has no customer service
//!   role, so they raise the *delivery dispute* from the shipment) ask the
//!   delivery module to put the delivery under review; wl-delivery opens the
//!   dispute ticket and emits `DeliveryDisputed`. The shipment is never put
//!   under review by a user: `open_review` and `close_review` are the
//!   system's and run on the wl-delivery events ([`crate::events`]).
//! - **`open_review`** ([`held`]): after the shipment is `under_review`, the
//!   client is told once (naming the shipment and the ticket), and its COD is
//!   taken off the remittance that holds it: a line on a `draft` is removed
//!   and the draft's figures worked out again, an `approved` remittance goes back to `draft`
//!   (`reopen_system`), loses the line the same way, and the accountants
//!   are told. A remittance that is `paid` (or `payment_returned`) is left
//!   as it is: nothing is taken back while the delivery is under review (a
//!   later reversal is BL-LOG-10).
//! - **`close_review`**: the shipment is `delivered` again; the next draft
//!   remittance takes its COD and the next monthly invoice its billing.
//!
//! A shipment under review is never on a draft: the draft takes `delivered`
//! shipments only ([`crate::remit`]), and [`crate::shipment::Shipment::billable`]
//! leaves it out. The invoice flag (`disputed`) is wl-invoicing's.

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::delivery::DeliveryId;

use crate::notices::{tell_client, tell_reopened};
use crate::permissions;
use crate::remit;
use crate::remittance::{Remittance, RemittanceLine, RemittanceStatus};
use crate::remittance_machine::REOPEN_SYSTEM;
use crate::shipment::{Shipment, ShipmentStatus};
use crate::texts::english;

fn guard(message: String) -> TransitionError {
    TransitionError::Guard(message)
}

/// `user` (`shipments.manage`) raises the dispute of the delivered shipment
/// `id` with `reason`: its delivery goes under review in wl-delivery, which
/// opens the dispute ticket; the shipment follows on the event.
///
/// # Errors
///
/// `Forbidden` without `shipments.manage`; `Guard` (422) without a reason,
/// for a shipment that is not `delivered` or has no delivery, and the
/// delivery module's refusal.
pub async fn raise(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
) -> std::result::Result<Shipment, TransitionError> {
    if !user.has_permission(permissions::SHIPMENTS_MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    let shipment = Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))?;
    if reason.trim().is_empty() {
        return Err(guard(english("logistics.review.reason_needed", &[])));
    }
    if shipment.status != ShipmentStatus::Delivered {
        return Err(guard(english("logistics.review.not_delivered", &[])));
    }
    let Some(delivery) = shipment.delivery_id else {
        return Err(guard(english("logistics.review.no_delivery", &[])));
    };
    wl_kernel::delivery::requester(state)
        .open_review(state, DeliveryId(delivery), reason.trim(), None)
        .await
        .map_err(TransitionError::Failed)?;
    Shipment::find(&state.db, id)
        .await?
        .ok_or(TransitionError::Failed(Error::NotFound))
}

/// What follows `open_review` of shipment `id`, after its commit: the client
/// is told once, and the COD is taken off the remittance that holds it.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn held(state: &AppState, id: i64) -> Result {
    let shipment = Shipment::find_or_404(&state.db, id).await?;
    let title = english("logistics.review.title", &[("number", &shipment.number)]);
    let body = match shipment.review_ticket_id {
        Some(ticket) => english(
            "logistics.review.body",
            &[("number", &shipment.number), ("ticket", &ticket)],
        ),
        None => english(
            "logistics.review.body_no_ticket",
            &[("number", &shipment.number)],
        ),
    };
    tell_client(state, &shipment, &title, &body).await?;
    take_off_remittance(state, &shipment).await
}

/// Takes the line of `shipment` off the remittance that holds it, when that
/// remittance is not paid: a draft is rebuilt, an approved one is sent back
/// to a draft first and the accountants are told.
pub(crate) async fn take_off_remittance(state: &AppState, shipment: &Shipment) -> Result {
    let Some(line) = RemittanceLine::query()
        .where_(RemittanceLine::SHIPMENT_ID.eq(shipment.id))
        .first(&state.db)
        .await?
    else {
        return Ok(());
    };
    let remittance = Remittance::find_or_404(&state.db, line.remittance_id).await?;
    match remittance.status {
        RemittanceStatus::Draft => {}
        RemittanceStatus::Approved => {
            let _outcome = remittance
                .transition(REOPEN_SYSTEM)
                .by_system()
                .run(state)
                .await
                .map_err(TransitionError::into_error)?;
        }
        // Remitted: nothing is taken back while it is under review.
        RemittanceStatus::Paid | RemittanceStatus::PaymentReturned => return Ok(()),
    }
    let reopened = remittance.status == RemittanceStatus::Approved;
    // Its line comes off and the draft's figures are worked out again.
    line.delete(&state.db).await?;
    let draft = remit::refresh_totals(state, remittance.id).await?;
    if reopened {
        tell_reopened(state, &draft, shipment).await?;
    }
    Ok(())
}
