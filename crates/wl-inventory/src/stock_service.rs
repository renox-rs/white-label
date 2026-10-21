//! Moving stock out of a location (wl-inventory §3 *Picking order*,
//! *Negative stock*, §6 `StockService`).
//!
//! [`issue`] is the manual issue to a cost object (`stock.issue` in the
//! scope of the location): it takes only what is available, first expiry
//! first out among the `released` lots of a lot-tracked item, and is refused
//! when the quantity is short, when part of it is on hold (held quantity is
//! never issued) or when the lot named is not `released`. It never takes the
//! balance below zero: under every negative stock policy a manual issue
//! stops there.
//!
//! Decisions: the movement's source is the cost object when there is one,
//! else the location; a manual issue costs the item's average cost.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;

use crate::balance::{self, Candidate};
use crate::item::{Item, quantity_text, value_of};
use crate::location::Location;
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::texts::said;

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// Takes `quantity_milli` from `candidates` in order: `(lot, quantity)` for
/// each lot used, and what is still short.
pub fn plan(candidates: &[Candidate], quantity_milli: i64) -> (Vec<(Option<i64>, i64)>, i64) {
    let mut left = quantity_milli;
    let mut taken = Vec::new();
    for candidate in candidates {
        if left <= 0 {
            break;
        }
        let part = left.min(candidate.available_milli);
        if part > 0 {
            taken.push((candidate.lot_id, part));
            left -= part;
        }
    }
    (taken, left.max(0))
}

/// A manual issue to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRequest {
    /// The item.
    pub item_id: i64,
    /// The location it is taken from.
    pub location_id: i64,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The lot to take it from; first expiry first out when `None`.
    pub lot_id: Option<i64>,
    /// The record the cost is charged to.
    pub cost_object: Option<Subject>,
    /// Why, when it needs a reason.
    pub reason: Option<String>,
}

/// Why `quantity_milli` of `item` is not available at `location_id`: on hold,
/// or short.
pub(crate) async fn shortage_text(
    tx: &mut Transaction,
    item: &Item,
    location_id: i64,
    lot_id: Option<i64>,
    quantity_milli: i64,
) -> Result<String> {
    let found = balance::balance(&mut *tx, item.id, location_id, lot_id).await?;
    let without_hold = found.on_hand_milli - found.reserved_milli;
    Ok(if found.held_milli > 0 && without_hold >= quantity_milli {
        said(
            "wl_inventory.issue.held",
            &[
                ("item", &item.code),
                ("held", &quantity_text(found.held_milli)),
            ],
        )
    } else {
        said(
            "wl_inventory.issue.not_available",
            &[
                ("item", &item.code),
                ("available", &quantity_text(found.available_milli())),
                ("quantity", &quantity_text(quantity_milli)),
            ],
        )
    })
}

/// The movements of a manual issue by `by`.
///
/// # Errors
///
/// `Forbidden` without `stock.issue` in the location's scope; 422 for a
/// quantity that is not above zero, a lot of another item or one that is
/// not `released`, or a quantity that is not available (the held part is
/// named).
pub async fn issue(
    state: &AppState,
    user: &AuthUser,
    request: IssueRequest,
) -> Result<Vec<StockMovement>> {
    let location = Location::find_or_404(&state.db, request.location_id).await?;
    if !user.has_permission_in(permissions::STOCK_ISSUE, &location.scope()) {
        return Err(Error::Forbidden);
    }
    if request.quantity_milli <= 0 {
        return Err(refused(said("wl_inventory.issue.bad_quantity", &[])));
    }
    let item = Item::find_or_404(&state.db, request.item_id).await?;
    let mut tx = state.db.begin().await?;
    let candidates = match request.lot_id {
        Some(lot_id) => {
            let lot = Lot::find_or_404(&mut tx, lot_id).await?;
            if lot.item_id != item.id {
                return Err(refused(said("wl_inventory.issue.wrong_lot", &[])));
            }
            if lot.status != LotStatus::Released {
                return Err(refused(said(
                    "wl_inventory.issue.lot_not_released",
                    &[("lot", &lot.number)],
                )));
            }
            let found = balance::balance(&mut tx, item.id, location.id, Some(lot.id)).await?;
            vec![Candidate {
                lot_id: Some(lot.id),
                available_milli: found.available_milli(),
            }]
        }
        None => balance::candidates(&mut tx, &item, location.id).await?,
    };
    let (taken, short) = plan(&candidates, request.quantity_milli);
    if short > 0 {
        let text =
            shortage_text(&mut tx, &item, location.id, request.lot_id, request.quantity_milli)
                .await?;
        return Err(refused(text));
    }
    let source = request
        .cost_object
        .clone()
        .unwrap_or_else(|| location.subject());
    let mut movements = Vec::new();
    for (lot_id, quantity) in taken {
        let mut moved = StockMovement::leaving(
            MovementType::Issue,
            item.id,
            lot_id,
            location.id,
            quantity,
            item.average_cost,
            value_of(quantity, item.average_cost),
            &source,
            request.reason.clone(),
            Some(user.id),
        );
        moved.cost_object_type = request
            .cost_object
            .as_ref()
            .map(|subject| subject.subject_type.clone());
        moved.cost_object_id = request.cost_object.as_ref().map(|subject| subject.subject_id);
        moved.insert(&mut tx).await?;
        movements.push(moved);
    }
    tx.commit().await?;
    Ok(movements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(lot: i64, available: i64) -> Candidate {
        Candidate {
            lot_id: Some(lot),
            available_milli: available,
        }
    }

    #[test]
    fn a_plan_takes_from_the_first_candidate_and_names_what_is_short() {
        let candidates = [candidate(1, 3_000), candidate(2, 4_000)];
        let (taken, short) = plan(&candidates, 5_000);
        assert_eq!(taken, vec![(Some(1), 3_000), (Some(2), 2_000)]);
        assert_eq!(short, 0);
        let (taken, short) = plan(&candidates, 9_000);
        assert_eq!(taken, vec![(Some(1), 3_000), (Some(2), 4_000)]);
        assert_eq!(short, 2_000);
        assert_eq!(plan(&[], 1_000), (Vec::new(), 1_000));
    }
}
