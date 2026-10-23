//! Picking a dispatch, cancelling it and re-picking a line (wl-inventory §4
//! *Dispatch* rows `pick` and `cancel`, issue #240).
//!
//! - `pick` (`dispatch.manage` in the scope of the location): `draft` to
//!   `picked`. Refused unless every lot is `released` (and not past its expiry
//!   date) and none of the quantity is on hold; the stock is then *reserved*
//!   (the lines of a picked dispatch count in [`crate::balance::reserved_at`]).
//! - `cancel` (`dispatch.manage`): `draft` or `picked` to `cancelled`; the
//!   reservation is given back.
//! - [`repick`]: the user takes a line of a picked dispatch away and picks
//!   other released lots for its quantity (first expiry first out is
//!   suggested when no lot is named; a quantity no single lot covers is
//!   split over several lines). The held quantity's reservation is released
//!   because the line is gone. The dispatch stays `picked`.
//!
//! Decisions: `pick` reads what stands in its way in a transaction of its
//! own, rolled back, and the effect reads it again; the vertical item-flag
//! guards of the spec (`MFG.ACL.11`) are the manufacturing app's and come
//! with that app.

use std::collections::BTreeMap;

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;

use crate::balance;
use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, lines_of, refused};
use crate::item::{Item, quantity_text};
use crate::lot::{Lot, LotStatus};
use crate::permissions;
use crate::texts::said;

/// The `pick` transition's name.
pub const PICK: &str = "pick";

/// The `cancel` transition's name.
pub const CANCEL: &str = "cancel";

/// What a `pick` run carries to its guards: the first thing that stands in
/// the way, `None` when nothing does.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PickInput {
    /// The user who picks it.
    pub by: Option<i64>,
    /// The dispatch has lines.
    pub lines_problem: Option<String>,
    /// A lot is not `released`, or is past its expiry date.
    pub lot_problem: Option<String>,
    /// Part of the quantity is on hold, or not available.
    pub stock_problem: Option<String>,
}

impl PickInput {
    /// The first problem, in the order the guards run.
    pub fn first_problem(&self) -> Option<&str> {
        [&self.lines_problem, &self.lot_problem, &self.stock_problem]
            .into_iter()
            .find_map(|problem| problem.as_deref())
    }
}

/// Why `lot` cannot be picked, `None` when it can: it is not `released`
/// (a lot on hold says so) or it is past its expiry date.
pub(crate) fn lot_problem(lot: &Lot, position: i64) -> Option<String> {
    let position = position.to_string();
    match lot.status {
        LotStatus::Released => {}
        LotStatus::OnHold => {
            return Some(said(
                "wl_inventory.dispatch.lot_on_hold",
                &[("position", &position), ("lot", &lot.number)],
            ));
        }
        _ => {
            return Some(said(
                "wl_inventory.dispatch.lot_not_released",
                &[("position", &position), ("lot", &lot.number)],
            ));
        }
    }
    if lot.expiry_date.is_some_and(|expiry| expiry < today()) {
        return Some(said(
            "wl_inventory.dispatch.lot_expired",
            &[("position", &position), ("lot", &lot.number)],
        ));
    }
    None
}

/// What stands in the way of picking `dispatch`, read in `tx`. `counted` is
/// true inside the transition's own transaction, where the dispatch is
/// already `picked` and its own lines count as reserved: they are added back.
///
/// # Errors
///
/// A database error.
pub async fn gather(
    tx: &mut Transaction,
    dispatch: &Dispatch,
    by: Option<i64>,
    counted: bool,
) -> Result<PickInput> {
    let lines = lines_of(&mut *tx, dispatch.id).await?;
    let mut found = PickInput {
        by,
        ..PickInput::default()
    };
    if lines.is_empty() {
        found.lines_problem = Some(said("wl_inventory.dispatch.no_lines", &[]));
        return Ok(found);
    }
    let mut claimed: BTreeMap<(i64, Option<i64>), i64> = BTreeMap::new();
    for line in &lines {
        let position = line.position.to_string();
        let item = Item::find_or_404(&mut *tx, line.item_id).await?;
        if let Some(lot_id) = line.lot_id {
            let lot = Lot::find_or_404(&mut *tx, lot_id).await?;
            if found.lot_problem.is_none() {
                found.lot_problem = lot_problem(&lot, line.position);
            }
        }
        let needed = claimed.entry((line.item_id, line.lot_id)).or_insert(0);
        *needed += line.quantity_milli;
        let needed = *needed;
        let at =
            balance::balance(&mut *tx, line.item_id, dispatch.location_id, line.lot_id).await?;
        let own = if counted { needed } else { 0 };
        let available = (at.on_hand_milli - at.reserved_milli - at.held_milli + own).max(0);
        if available >= needed || found.stock_problem.is_some() {
            continue;
        }
        let without_hold = at.on_hand_milli - at.reserved_milli + own;
        found.stock_problem = Some(if without_hold >= needed {
            said(
                "wl_inventory.dispatch.held",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("held", &quantity_text(at.held_milli)),
                ],
            )
        } else {
            said(
                "wl_inventory.dispatch.not_available",
                &[
                    ("position", &position),
                    ("item", &item.code),
                    ("available", &quantity_text(available)),
                    ("quantity", &quantity_text(needed)),
                ],
            )
        });
    }
    Ok(found)
}

fn pick_input(ctx: &TransitionCtx) -> Option<PickInput> {
    ctx.input_as::<PickInput>().ok()
}

/// A `pick` guard that refuses with the problem `pick` finds in the input.
fn with_problem(
    transition: Transition<Dispatch>,
    name: &'static str,
    pick: fn(&PickInput) -> Option<String>,
) -> Transition<Dispatch> {
    transition.guard(name, move |_dispatch: &Dispatch, ctx| {
        let problem = pick_input(ctx).map_or_else(
            || Some(said("wl_inventory.dispatch.not_draft", &[])),
            |input| pick(&input),
        );
        Box::pin(async move { problem.map_or(Ok(()), Err) })
    })
}

/// The effect of `pick`: the stock is reserved (its lines count as reserved
/// while the dispatch is `picked`).
async fn pick_in(tx: &mut Transaction, dispatch: Dispatch, by: Option<i64>) -> Result {
    // Stock may have moved or been held since the guards looked.
    let found = gather(tx, &dispatch, by, true).await?;
    if let Some(problem) = found.first_problem() {
        return Err(refused(problem));
    }
    let now = renox::db::now();
    Dispatch::query()
        .where_(Dispatch::ID.eq(dispatch.id))
        .update(
            &mut *tx,
            &[
                ("picked_by", &by),
                ("picked_at", &Some(now)),
                ("updated_at", &now),
            ],
        )
        .await?;
    Ok(())
}

/// `pick` and `cancel`.
pub(crate) fn add_to(machine: Machine<Dispatch>) -> Machine<Dispatch> {
    use DispatchStatus::{Cancelled, Draft, Picked};
    let pick = Transition::new(PICK)
        .label("Pick")
        .tone("primary")
        .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
            dispatch.scope()
        })
        .from([Draft])
        .to(Picked);
    let pick = with_problem(pick, "has_lines", |i| i.lines_problem.clone());
    let pick = with_problem(pick, "released_lots_only", |i| i.lot_problem.clone());
    let pick = with_problem(pick, "no_held_quantity", |i| i.stock_problem.clone());
    let pick = pick.effect(|dispatch: &Dispatch, ctx, tx| {
        let dispatch = dispatch.clone();
        let by = pick_input(ctx).and_then(|input| input.by);
        Box::pin(async move { pick_in(tx, dispatch, by).await })
    });
    let cancel = Transition::new(CANCEL)
        .label("Cancel")
        .permission_in(permissions::DISPATCH_MANAGE, |dispatch: &Dispatch| {
            dispatch.scope()
        })
        .from([Draft, Picked])
        .to(Cancelled)
        .effect(|dispatch: &Dispatch, ctx, tx| {
            let id = dispatch.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                let now = renox::db::now();
                Dispatch::query()
                    .where_(Dispatch::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[("cancel_reason", &reason), ("updated_at", &now)],
                    )
                    .await?;
                Ok(())
            })
        });
    machine.transition(pick).transition(cancel)
}

/// `user` picks dispatch `id`. Refused with the first problem before
/// anything is written.
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` in the location's scope; `NotFrom`
/// unless `draft`; `Guard` (422) when a lot is not released, part of the
/// quantity is on hold or the stock is not there.
pub async fn pick(
    state: &AppState,
    user: &AuthUser,
    id: i64,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let mut tx = state.db.begin().await?;
    let found = gather(&mut tx, &dispatch, Some(user.id), false).await?;
    tx.rollback().await?;
    if dispatch.status == DispatchStatus::Draft
        && user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope())
        && let Some(problem) = found.first_problem()
    {
        return Err(TransitionError::Guard(problem.to_owned()));
    }
    let _outcome = dispatch
        .transition(PICK)
        .by(user)
        .input(&found)
        .run(state)
        .await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

/// `user` cancels dispatch `id` (`draft` or `picked`), with an optional
/// `reason`; the reservation is given back.
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` in the location's scope; `NotFrom`
/// once it has left.
pub async fn cancel(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: Option<&str>,
) -> std::result::Result<Dispatch, TransitionError> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    let mut run = dispatch.transition(CANCEL).by(user);
    if let Some(reason) = reason.map(str::trim).filter(|reason| !reason.is_empty()) {
        run = run.reason(reason);
    }
    let _outcome = run.run(state).await?;
    Ok(Dispatch::find_or_404(&state.db, id).await?)
}

/// Where `quantity_milli` of `item` can be picked at `location_id`: the
/// named `lot` when given, else first expiry first out over the released
/// lots, as `(lot, quantity)` pieces. `Err` is the text of what is short.
async fn allocate(
    tx: &mut Transaction,
    item: &Item,
    location_id: i64,
    quantity_milli: i64,
    lot: Option<i64>,
) -> Result<std::result::Result<Vec<(Option<i64>, i64)>, String>> {
    let position = "1";
    if let Some(lot_id) = lot {
        let lot = Lot::find_or_404(&mut *tx, lot_id).await?;
        if lot.item_id != item.id {
            return Ok(Err(said(
                "wl_inventory.dispatch.lot_other_item",
                &[("lot", &lot.number), ("item", &item.code)],
            )));
        }
        if let Some(problem) = lot_problem(&lot, 1) {
            return Ok(Err(problem));
        }
        let at = balance::balance(&mut *tx, item.id, location_id, Some(lot_id)).await?;
        if at.available_milli() < quantity_milli {
            return Ok(Err(said(
                "wl_inventory.dispatch.not_available",
                &[
                    ("position", position),
                    ("item", &item.code),
                    ("available", &quantity_text(at.available_milli())),
                    ("quantity", &quantity_text(quantity_milli)),
                ],
            )));
        }
        return Ok(Ok(vec![(Some(lot_id), quantity_milli)]));
    }
    let mut left = quantity_milli;
    let mut pieces = Vec::new();
    for candidate in balance::candidates(&mut *tx, item, location_id).await? {
        if left == 0 {
            break;
        }
        let take = candidate.available_milli.min(left);
        pieces.push((candidate.lot_id, take));
        left -= take;
    }
    if left > 0 {
        let available = quantity_milli - left;
        return Ok(Err(said(
            "wl_inventory.dispatch.not_available",
            &[
                ("position", position),
                ("item", &item.code),
                ("available", &quantity_text(available)),
                ("quantity", &quantity_text(quantity_milli)),
            ],
        )));
    }
    Ok(Ok(pieces))
}

/// `user` re-picks line `line_id` of picked dispatch `id`: the line is taken
/// away and its quantity is picked again from `lot` (first expiry first out
/// over the released lots when none is named). The held quantity's
/// reservation is released with the line; the dispatch stays `picked`.
///
/// # Errors
///
/// `Forbidden` without `dispatch.manage` in the location's scope; 422 unless
/// the dispatch is `picked` and the line is its own, or when the released
/// lots do not cover the quantity (nothing is changed then).
pub async fn repick(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    line_id: i64,
    lot: Option<i64>,
) -> Result<Vec<DispatchLine>> {
    let dispatch = Dispatch::find_or_404(&state.db, id).await?;
    if !user.has_permission_in(permissions::DISPATCH_MANAGE, &dispatch.scope()) {
        return Err(Error::Forbidden);
    }
    if dispatch.status != DispatchStatus::Picked {
        return Err(refused(said("wl_inventory.dispatch.not_picked", &[])));
    }
    let mut tx = state.db.begin().await?;
    let lines = lines_of(&mut tx, id).await?;
    let Some(line) = lines.iter().find(|line| line.id == line_id).cloned() else {
        tx.rollback().await?;
        return Err(Error::NotFound);
    };
    // The old line goes first: its reservation must not count against the
    // lots the quantity is picked from.
    DispatchLine::query()
        .where_(DispatchLine::ID.eq(line.id))
        .delete(&mut tx)
        .await?;
    let item = Item::find_or_404(&mut tx, line.item_id).await?;
    let pieces = match allocate(
        &mut tx,
        &item,
        dispatch.location_id,
        line.quantity_milli,
        lot,
    )
    .await?
    {
        Ok(pieces) => pieces,
        Err(problem) => {
            tx.rollback().await?;
            return Err(refused(problem));
        }
    };
    let last = lines.iter().map(|l| l.position).max().unwrap_or(0);
    for (position, (lot_id, quantity_milli)) in (last + 1..).zip(pieces) {
        let mut piece = DispatchLine {
            dispatch_id: id,
            position,
            item_id: line.item_id,
            lot_id,
            quantity_milli,
            source_line_type: line.source_line_type.clone(),
            source_line_id: line.source_line_id,
            ..DispatchLine::default()
        };
        piece.insert(&mut tx).await?;
    }
    tx.commit().await?;
    lines_of(&state.db, id).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_problem_follows_the_order_of_the_guards() {
        let mut input = PickInput {
            stock_problem: Some("stock".into()),
            ..PickInput::default()
        };
        assert_eq!(input.first_problem(), Some("stock"));
        input.lot_problem = Some("lot".into());
        assert_eq!(input.first_problem(), Some("lot"));
        assert_eq!(PickInput::default().first_problem(), None);
    }

    #[test]
    fn a_draft_is_picked_or_cancelled_and_a_picked_one_cancelled() {
        let machine = Dispatch::machine();
        let pick = machine.find(PICK).expect("pick");
        assert_eq!(pick.from_states(), ["draft"]);
        assert_eq!(pick.to_state(), Some("picked"));
        assert_eq!(
            pick.guard_names(),
            ["has_lines", "released_lots_only", "no_held_quantity"]
        );
        let cancel = machine.find(CANCEL).expect("cancel");
        assert_eq!(cancel.from_states(), ["draft", "picked"]);
        assert_eq!(cancel.to_state(), Some("cancelled"));
        assert!(!pick.is_system() && !cancel.is_system());
    }
}
