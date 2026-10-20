//! Where and when the goods were last traced (wl-delivery §4.1 `mark_lost`,
//! `WRK.RUL.17`): a dispatch, a visit, a review action, a scan, a note or a
//! depot receipt. The untraced clock of the lost rule starts from the latest
//! of them ([`last_traced_at`]), and a manual `mark_lost` is refused at final
//! approval when one was recorded after the request.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::TransitionError;

use crate::delivery::{Attempt, Delivery, Trace};
use crate::permissions;

/// The goods left with the run.
pub const DISPATCH: &str = "dispatch";
/// A review was opened or closed.
pub const REVIEW: &str = "review";
/// The goods were scanned at a depot or on the road.
pub const SCAN: &str = "scan";
/// A note was added to the delivery.
pub const NOTE: &str = "note";
/// The goods were received at a depot.
pub const DEPOT_RECEIPT: &str = "depot_receipt";
/// The goods of a lost delivery were found.
pub const FOUND: &str = "found";

/// Stores a trace of `kind` for delivery `delivery_id` inside `tx`.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn insert_in(
    tx: &mut Transaction,
    delivery_id: i64,
    kind: &str,
    note: &str,
    depot: &str,
    at: DateTime,
    actor: Option<i64>,
) -> Result<()> {
    let mut trace = Trace {
        delivery_id,
        kind: kind.to_owned(),
        note: note.trim().to_owned(),
        depot: depot.trim().to_owned(),
        occurred_at: at,
        actor_user_id: actor,
        ..Trace::default()
    };
    trace.insert(&mut *tx).await
}

/// Records a scan, a note or a depot receipt (`kind`) of `delivery` by
/// `user` (a holder of `deliveries.manage`), now. It resets the untraced
/// clock of the lost rule.
///
/// # Errors
///
/// `Forbidden` without `deliveries.manage`; `Guard` (422) for another kind.
pub async fn record(
    state: &AppState,
    user: &AuthUser,
    delivery: &Delivery,
    kind: &str,
    note: &str,
    depot: &str,
) -> std::result::Result<(), TransitionError> {
    if !user.has_permission(permissions::MANAGE) {
        return Err(TransitionError::Forbidden);
    }
    if ![SCAN, NOTE, DEPOT_RECEIPT].contains(&kind) {
        return Err(TransitionError::Guard(
            "A trace is a scan, a note or a depot receipt.".to_owned(),
        ));
    }
    let mut trace = Trace {
        delivery_id: delivery.id,
        kind: kind.to_owned(),
        note: note.trim().to_owned(),
        depot: depot.trim().to_owned(),
        occurred_at: renox::db::now(),
        actor_user_id: Some(user.id),
        ..Trace::default()
    };
    trace
        .insert(&state.db)
        .await
        .map_err(TransitionError::Failed)
}

/// The latest moment the goods of `delivery` were traced: the latest trace or
/// visit, else the day the delivery was created. `None` only for a delivery
/// that was never saved.
///
/// # Errors
///
/// Database errors.
pub async fn last_traced_at(db: &Db, delivery: &Delivery) -> Result<Option<DateTime>> {
    let trace = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery.id))
        .order_by_desc(Trace::OCCURRED_AT)
        .first(db)
        .await?
        .map(|t| t.occurred_at);
    let attempt = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery.id))
        .order_by_desc(Attempt::OCCURRED_AT)
        .first(db)
        .await?
        .map(|a| a.occurred_at);
    Ok([trace, attempt, delivery.created_at].into_iter().flatten().max())
}

/// Whether a trace or a visit of `delivery_id` was recorded after `since`.
///
/// # Errors
///
/// Database errors.
pub(crate) async fn recorded_after(
    tx: &mut Transaction,
    delivery_id: i64,
    since: DateTime,
) -> Result<bool> {
    let traces = Trace::query()
        .where_(Trace::DELIVERY_ID.eq(delivery_id))
        .where_(Trace::OCCURRED_AT.gt(since))
        .count(&mut *tx)
        .await?;
    let attempts = Attempt::query()
        .where_(Attempt::DELIVERY_ID.eq(delivery_id))
        .where_(Attempt::OCCURRED_AT.gt(since))
        .count(&mut *tx)
        .await?;
    Ok(traces + attempts > 0)
}
