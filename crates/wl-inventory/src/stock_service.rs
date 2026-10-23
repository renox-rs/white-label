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
//! [`consume`] is the consumption another module posts automatically (a
//! clinic visit, an F&B sale, a work order material issue) through the
//! kernel's [`StockService`](wl_kernel::inventory::StockService): the business
//! event is never blocked. What is available posts now, as `consume`
//! movements against the source and the cost object; the short part becomes a
//! [`PendingIssue`](crate::pending_issue::PendingIssue) and
//! [`StockNegative`] follows the commit. A lot balance is never negative: a
//! lot-tracked item is taken from the `released` lots only, first expiry
//! first, and held quantity is never consumed, at any location.
//!
//! At a location set to `allow_consumption` (BL-STK-19, `STK.RUL.06`), an
//! item that is **not** lot-tracked may be consumed below zero: the short
//! part posts too, at the current average cost (which a negative balance does
//! not change), and [`StockNegative`] follows without a pending issue. The
//! next receipt covers the negative quantity first and the difference
//! between its cost and the cost used goes to the price variance
//! ([`crate::price_variance`]). Held quantity is still never consumed: while
//! any quantity of the item is held at the location, what is beyond the
//! available quantity becomes a pending issue, as under `block`; a
//! lot-tracked item does the same. A manual issue never goes below zero.
//!
//! Decisions: the movement's source is the cost object when there is one,
//! else the location; a manual issue costs the item's average cost.

use renox::db::Transaction;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::inventory::{
    ConsumeRequest, Consumed, ConsumedLot, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE,
    LOT_SUBJECT_TYPE, StockChoice, StockFuture, StockLot, StockNegative, StockService,
};

use crate::balance::{self, Candidate};
use crate::item::{Item, quantity_text, value_of};
use crate::location::{Location, LocationType, NegativeStockPolicy};
use crate::lot::{Lot, LotStatus};
use crate::movement::{MovementType, StockMovement};
use crate::pending_issue;
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
        let text = shortage_text(
            &mut tx,
            &item,
            location.id,
            request.lot_id,
            request.quantity_milli,
        )
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
        moved.cost_object_id = request
            .cost_object
            .as_ref()
            .map(|subject| subject.subject_id);
        moved.insert(&mut tx).await?;
        movements.push(moved);
    }
    tx.commit().await?;
    Ok(movements)
}

/// Consumes `request` automatically (see the module docs).
///
/// # Errors
///
/// 422 for a quantity that is not above zero; a database error.
pub async fn consume(state: &AppState, request: ConsumeRequest) -> Result<Consumed> {
    if request.quantity_milli <= 0 {
        return Err(refused(said("wl_inventory.issue.bad_quantity", &[])));
    }
    let at = request.at.unwrap_or_else(renox::db::now);
    let location = Location::find_or_404(&state.db, request.location.subject_id).await?;
    let item = Item::find_or_404(&state.db, request.item.subject_id).await?;
    let mut tx = state.db.begin().await?;
    let mut candidates = balance::candidates(&mut tx, &item, location.id).await?;
    // The lot the user picked or scanned goes first; the rest follows first
    // expiry first. A lot that is not a candidate (held, not released,
    // empty) is not taken.
    if let Some(preferred) = request.lot.as_ref().map(|lot| lot.subject_id)
        && let Some(position) = candidates
            .iter()
            .position(|candidate| candidate.lot_id == Some(preferred))
    {
        let first = candidates.remove(position);
        candidates.insert(0, first);
    }
    let (mut taken, mut short) = plan(&candidates, request.quantity_milli);
    // `allow_consumption`: the short part of an item that is not lot-tracked
    // posts below zero unless some of it is held here.
    let mut below_zero = 0;
    if short > 0
        && location.negative_stock_policy == NegativeStockPolicy::AllowConsumption
        && !item.lot_tracked
    {
        let found = balance::balance(&mut tx, item.id, location.id, None).await?;
        if found.held_milli <= 0 {
            taken.push((None, short));
            below_zero = short;
            short = 0;
        }
    }
    let mut posted = 0;
    let mut lots = Vec::new();
    for (lot_id, quantity) in taken {
        lots.push(ConsumedLot {
            lot: lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
            quantity_milli: quantity,
        });
        let mut moved = StockMovement::leaving(
            MovementType::Consume,
            item.id,
            lot_id,
            location.id,
            quantity,
            item.average_cost,
            value_of(quantity, item.average_cost),
            &request.source,
            None,
            None,
        );
        moved.moved_at = at;
        moved.cost_object_type = request
            .cost_object
            .as_ref()
            .map(|subject| subject.subject_type.clone());
        moved.cost_object_id = request
            .cost_object
            .as_ref()
            .map(|subject| subject.subject_id);
        moved.theoretical = request.theoretical;
        moved.insert(&mut tx).await?;
        posted += quantity;
    }
    let pending = if short > 0 {
        Some(
            pending_issue::record_in(
                &mut tx,
                &location,
                item.id,
                short,
                &request.source,
                request.cost_object.as_ref(),
                request.theoretical,
                at,
            )
            .await?,
        )
    } else {
        None
    };
    tx.commit().await?;
    if below_zero > 0 {
        state
            .emit(StockNegative {
                item: Subject::new(ITEM_SUBJECT_TYPE, item.id),
                location: Subject::new(LOCATION_SUBJECT_TYPE, location.id),
                quantity_milli: below_zero,
                source: Some(request.source.clone()),
                pending_issue: None,
            })
            .await?;
    }
    if let Some(pending) = &pending {
        state
            .emit(StockNegative {
                item: Subject::new(ITEM_SUBJECT_TYPE, item.id),
                location: Subject::new(LOCATION_SUBJECT_TYPE, location.id),
                quantity_milli: short,
                source: Some(request.source.clone()),
                pending_issue: Some(pending.subject()),
            })
            .await?;
    }
    Ok(Consumed {
        posted_milli: posted,
        short_milli: short,
        pending_issue: pending.map(|pending| pending.subject()),
        lots,
    })
}

/// The store of scope `kind`/`id`: the location of that scope of the type
/// store (else the first location of the scope that is not in transit,
/// quarantine or scrap), oldest first.
///
/// # Errors
///
/// A database error.
pub async fn store_of(db: &Db, kind: &str, id: i64) -> Result<Option<Location>> {
    let found = Location::query()
        .where_(Location::SCOPE_TYPE.eq(Some(kind.to_owned())))
        .where_(Location::SCOPE_ID.eq(Some(id)))
        .order_by(Location::ID)
        .get(db)
        .await?;
    let usable = |location: &&Location| {
        !matches!(
            location.location_type,
            LocationType::InTransit | LocationType::Quarantine | LocationType::Scrap
        )
    };
    Ok(found
        .iter()
        .find(|location| location.location_type == LocationType::Store)
        .or_else(|| found.iter().find(usable))
        .cloned())
}

/// What can be taken of the item `code` at `location_id` (see
/// [`StockService::choices`]): a lot past its expiry date is left out.
///
/// # Errors
///
/// A database error.
pub async fn choices(db: &Db, code: &str, location_id: i64) -> Result<Option<StockChoice>> {
    let Some(item) = Item::query()
        .where_(Item::CODE.eq(code.trim()))
        .first(db)
        .await?
    else {
        return Ok(None);
    };
    let mut lots = Vec::new();
    if item.lot_tracked {
        let day = today();
        for candidate in balance::candidates(db, &item, location_id).await? {
            let Some(lot_id) = candidate.lot_id else {
                continue;
            };
            let lot = Lot::find_or_404(db, lot_id).await?;
            if lot.expiry_date.is_some_and(|expiry| expiry < day) {
                continue;
            }
            lots.push(StockLot {
                lot: Subject::new(LOT_SUBJECT_TYPE, lot.id),
                number: lot.number,
                expiry_date: lot.expiry_date,
                available_milli: candidate.available_milli,
            });
        }
    }
    Ok(Some(StockChoice {
        item: item.subject(),
        lot_tracked: item.lot_tracked,
        lots,
    }))
}

/// The module's [`StockService`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryStock;

impl StockService for InventoryStock {
    fn consume<'a>(
        &'a self,
        state: &'a AppState,
        request: ConsumeRequest,
    ) -> StockFuture<'a, Consumed> {
        Box::pin(async move { consume(state, request).await })
    }

    fn store_of<'a>(
        &'a self,
        state: &'a AppState,
        scope_kind: &'a str,
        scope_id: i64,
    ) -> StockFuture<'a, Option<Subject>> {
        Box::pin(async move {
            Ok(store_of(&state.db, scope_kind, scope_id)
                .await?
                .map(|location| location.subject()))
        })
    }

    fn choices<'a>(
        &'a self,
        state: &'a AppState,
        code: &'a str,
        location: &'a Subject,
    ) -> StockFuture<'a, Option<StockChoice>> {
        Box::pin(async move { choices(&state.db, code, location.subject_id).await })
    }

    fn source_cancelled<'a>(
        &'a self,
        state: &'a AppState,
        source: &'a Subject,
    ) -> StockFuture<'a, usize> {
        Box::pin(async move { pending_issue::cancel_for_source(state, source).await })
    }
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
