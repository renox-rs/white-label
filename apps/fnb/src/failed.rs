//! A failed own-rider delivery (06 F&B §4.2 rows *Own-rider delivery fails*
//! and *Rider's run is aborted*, §7.1 `fail_delivery`; BL-FNB-01, `FNB.ACL.16`,
//! `WRK.RUL.16`).
//!
//! Cooked food gets one delivery attempt. `DeliveryAttemptFailed` (the rider
//! records the failure in the driver PWA, or a run is aborted and the stop
//! cannot be handed over in time) moves the order from `dispatched` to
//! `delivery_failed` with the failure reason mapped from wl-delivery's
//! ([`crate::order::map_reason`]); the wl-delivery reason stays on the order
//! too. Nothing is collected on a cash order (wl-delivery adds only delivered
//! stops to the rider's cash) and nothing is queued for a reattempt
//! (perishable policy, `WRK.RUL.16`: the food comes back, or is discarded
//! with a photo after an abort).
//!
//! - A second event for an order that is no longer `dispatched` (a duplicate,
//!   or one that arrives after a late `DeliveryDelivered`) is ignored and
//!   logged: no second history row, no state change.
//! - The transition is the system's only: no screen offers it.
//! - The customer is told "We could not deliver your order <number>", with no
//!   blame text.
//! - `FNB.RUL.14` counts a customer-side failure of a cash order
//!   ([`crate::cod`]).
//! - The kitchen manager may change the proposed responsible party with a note
//!   ([`change_party`]).

use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::delivery::DeliveryAttemptFailed;

use crate::fulfilment::order_of_delivery;
use crate::notices::{about, tell_customer};
use crate::order::{FnbOrder, FnbOrderStatus, map_reason, party};
use crate::order_machine::{FAIL_DELIVERY, OrderInput};
use crate::permissions;
use crate::placing::refused;
use crate::texts::english;

/// The failure reasons 06 adds to wl-delivery's list: *rider issue* (not
/// counted, no reattempt), and the two causes the rider picks for *refused*
/// that are on the outlet's side. The rider picks them in the driver PWA, in
/// the same list as wl-delivery's own. As (code, English name, Arabic name,
/// counts against the attempts, allows a reattempt).
pub const FAILURE_REASONS: [(&str, &str, &str, bool, bool); 3] = [
    (
        "rider_issue",
        "Rider issue",
        "مشكلة في المندوب",
        false,
        false,
    ),
    (
        "refused_late",
        "Refused: the order was late",
        "رفض الطلب: تأخر الطلب",
        true,
        false,
    ),
    (
        "refused_wrong_item",
        "Refused: wrong item",
        "رفض الطلب: صنف خاطئ",
        true,
        false,
    ),
];

/// `DeliveryAttemptFailed`: the order whose delivery failed goes
/// `delivery_failed`, once.
///
/// # Errors
///
/// Database errors and the transition's failures.
pub(crate) async fn on_attempt_failed(state: &AppState, event: DeliveryAttemptFailed) -> Result {
    let Some(order) = order_of_delivery(state, &event.source, event.delivery).await? else {
        return Ok(());
    };
    if order.status != FnbOrderStatus::Dispatched {
        // A duplicate event, or a late one: the order has moved on.
        eprintln!(
            "fnb: attempt {} of delivery {} failed ({}) but order {} is {}: ignored",
            event.attempt,
            event.delivery,
            event.reason,
            order.number,
            order.status.as_str()
        );
        return Ok(());
    }
    let mapped = map_reason(&event.reason);
    let history = english(
        &format!("fnb.failed.reason.{}", mapped.reason),
        &[("delivery", &event.reason)],
    );
    let _outcome = order
        .transition(FAIL_DELIVERY)
        .by_system()
        .reason(history)
        .input(&OrderInput {
            delivery_reason: Some(event.reason.clone()),
            attempt: event.attempt,
            ..OrderInput::default()
        })
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// After `fail_delivery` committed: the customer is told, and `FNB.RUL.14`
/// counts the failure.
///
/// # Errors
///
/// Database errors and the notifier's.
pub(crate) async fn after_failed(state: &AppState, id: i64) -> Result {
    let Some(order) = FnbOrder::find(&state.db, id).await? else {
        return Ok(());
    };
    tell_customer(
        state,
        &order,
        &about("fnb.failed.customer_title", &order),
        &about("fnb.failed.customer_body", &order),
    )
    .await?;
    crate::cod::on_failure(state, &order).await
}

/// The kitchen manager (`orders.manage`) changes the party proposed as
/// responsible for the failed delivery of order `id` to `new_party` (one of
/// [`party::ALL`]), with a note.
///
/// # Errors
///
/// `Forbidden` without `orders.manage`; 422 for an order that did not fail, an
/// unknown party, or no note.
pub async fn change_party(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    new_party: &str,
    note: &str,
) -> Result<FnbOrder> {
    if !user.has_permission(permissions::ORDERS_MANAGE) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    if order.failure_reason.is_none() {
        return Err(refused(english("fnb.failed.not_failed", &[])));
    }
    let chosen = new_party.trim().to_owned();
    if !party::ALL.contains(&chosen.as_str()) {
        return Err(refused(english("fnb.failed.party_unknown", &[])));
    }
    let note = note.trim().to_owned();
    if note.is_empty() {
        return Err(refused(english("fnb.failed.party_note_needed", &[])));
    }
    let by = Some(user.id);
    let responsible = Some(chosen);
    let noted = Some(note);
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(id))
        .update(
            &state.db,
            &[
                ("responsible_party", &responsible),
                ("responsible_note", &noted),
                ("responsible_changed_by", &by),
            ],
        )
        .await?;
    FnbOrder::find_or_404(&state.db, id).await
}
