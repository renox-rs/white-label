//! Placing a Stock hold and adding lines to it (wl-inventory §4 *Stock hold*
//! `place` and `add_lines`, issue #230); the rows and the machine are in
//! [`crate::stock_hold`].
//!
//! - [`place`]: a holder of `stock.hold` (or a vertical automation, as the
//!   system, through the kernel's `StockHoldService`) places a hold with a
//!   reason and a reference. The lines are made from the balances in scope
//!   (item, location, lot, quantity); a whole-lot scope runs the lot's `hold`;
//!   the holders of `lots.release` in the locations' scope are told;
//!   [`StockHoldPlaced`] follows the commit. Nothing is moved.
//! - [`add_lines`]: the quantity that moved into the location of an open hold
//!   joins it as new lines; the deciders are told.
//! - [`warnings_for`]: what a goods receipt or transfer screen warns about
//!   before stock is moved into a held location.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_permission_in;
use renox::db::{Executor, Transaction};
use renox::prelude::*;
use renox::state_machine::{Outcome, StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::inventory::{
    HeldLine, HoldDecision, HoldLineView, HoldView, ITEM_SUBJECT_TYPE, LOCATION_SUBJECT_TYPE,
    LocationInfo, LOT_SUBJECT_TYPE, STOCK_HOLD_LINE_SUBJECT_TYPE, STOCK_HOLD_SUBJECT_TYPE, StockHoldPlaced,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use crate::balance;
use crate::hold_decide::{DecideOutcome, Decision, decide};
use crate::item::{Item, value_of};
use crate::location::Location;
use crate::lot::{self, Lot, LotStatus};
use crate::movement::StockMovement;
use crate::permissions;
use crate::stock_hold::{
    ADD_LINES, ADD_LINES_BY_SYSTEM, AddLinesInput, AddedLine, HoldScope, HoldStatus, LINES_NOTICE,
    PLACED_NOTICE, REASONS, STOCK_HOLD_SERIES, StockHold, StockHoldItem, StockHoldLine, lines_of,
    new_line, scope_items,
};
use crate::texts::{english, said};

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// What a hold covers, when it is placed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewScope {
    /// These lots, wherever they are.
    Lots(Vec<i64>),
    /// The stock at a location: the items named, every item when empty.
    Location {
        /// The location.
        location_id: i64,
        /// The items held; all of them when empty.
        item_ids: Vec<i64>,
    },
}

/// A hold to place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHold {
    /// One of [`REASONS`].
    pub reason: String,
    /// The record that caused it.
    pub reference: Subject,
    /// What it covers.
    pub scope: NewScope,
    /// The user who places it; `None` for the system.
    pub by: Option<i64>,
}

/// The notification type of the notice to the deciders of a new hold.
pub fn placed_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(PLACED_NOTICE, "inventory")
        .group("holds")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.hold.placed_subject")),
            english("wl_inventory.hold.placed_body"),
        )
}

/// The notification type of the notice about lines added to an open hold.
pub fn lines_notice_type() -> NotificationTypeDef {
    NotificationTypeDef::new(LINES_NOTICE, "inventory")
        .group("holds")
        .category(Category::Transactional)
        .template(
            NotifyChannel::Email,
            "en",
            Some(english("wl_inventory.hold.lines_subject")),
            english("wl_inventory.hold.lines_body"),
        )
}

/// Registers the notification types with the kernel registry.
pub(crate) fn register_types() {
    notify::register_type(placed_notice_type());
    notify::register_type(lines_notice_type());
}

/// The open holds on `location_id` that cover `item_id`.
///
/// # Errors
///
/// A database error.
pub async fn open_on_location<'c>(
    db: impl Executor<'c>,
    location_id: i64,
    item_id: i64,
) -> Result<Vec<StockHold>> {
    let mut conn = db.into_conn();
    let holds = StockHold::query()
        .where_(StockHold::STATUS.eq(HoldStatus::Open))
        .where_(StockHold::SCOPE_KIND.eq(HoldScope::Location))
        .where_(StockHold::LOCATION_ID.eq(Some(location_id)))
        .order_by(StockHold::ID)
        .get(conn.reborrow())
        .await?;
    let mut covering = Vec::new();
    for hold in holds {
        let items = scope_items(conn.reborrow(), hold.id).await?;
        if items.is_empty() || items.contains(&item_id) {
            covering.push(hold);
        }
    }
    Ok(covering)
}

/// The warnings to show before stock is received into `location_id`:
/// "Fridge 2 is on temperature hold" for each open hold on the location that
/// covers one of `item_ids`. Empty when nothing is held there.
///
/// # Errors
///
/// A database error.
pub async fn warnings_for(db: &Db, location_id: i64, item_ids: &[i64]) -> Result<Vec<String>> {
    let location = Location::find_or_404(db, location_id).await?;
    let mut seen = BTreeSet::new();
    let mut warnings = Vec::new();
    for item_id in item_ids {
        for hold in open_on_location(db, location_id, *item_id).await? {
            if seen.insert(hold.id) {
                warnings.push(said(
                    "wl_inventory.hold.warning",
                    &[("location", &location.name_en), ("reason", &hold.reason)],
                ));
            }
        }
    }
    Ok(warnings)
}

/// The lines a hold on the stock at `location_id` is made of: one per item
/// (and lot) with something on hand, for `item_ids` or for every item.
async fn location_lines(
    tx: &mut renox::db::Transaction,
    location_id: i64,
    item_ids: &[i64],
) -> Result<Vec<AddedLine>> {
    let ids: Vec<i64> = if item_ids.is_empty() {
        let mut ids: BTreeSet<i64> = BTreeSet::new();
        for movement in StockMovement::query()
            .where_(StockMovement::TO_LOCATION_ID.eq(location_id))
            .get(&mut *tx)
            .await?
        {
            ids.insert(movement.item_id);
        }
        ids.into_iter().collect()
    } else {
        item_ids.to_vec()
    };
    let mut lines = Vec::new();
    for item_id in ids {
        let item = Item::find_or_404(&mut *tx, item_id).await?;
        if item.lot_tracked {
            let lots = Lot::query()
                .where_(Lot::ITEM_ID.eq(item.id))
                .order_by(Lot::ID)
                .get(&mut *tx)
                .await?;
            for lot in lots {
                let quantity =
                    balance::on_hand_at(&mut *tx, item.id, location_id, Some(lot.id)).await?;
                if quantity > 0 {
                    lines.push(AddedLine {
                        item_id: item.id,
                        location_id,
                        lot_id: Some(lot.id),
                        quantity_milli: quantity,
                    });
                }
            }
        } else {
            let quantity = balance::on_hand_at(&mut *tx, item.id, location_id, None).await?;
            if quantity > 0 {
                lines.push(AddedLine {
                    item_id: item.id,
                    location_id,
                    lot_id: None,
                    quantity_milli: quantity,
                });
            }
        }
    }
    Ok(lines)
}

/// The users who decide the lines in `locations`: the holders of
/// `lots.release` in the scope of each.
async fn deciders(db: &Db, locations: &BTreeSet<i64>) -> Result<Vec<i64>> {
    let mut ids = BTreeSet::new();
    for location_id in locations {
        let Some(location) = Location::find(db, *location_id).await? else {
            continue;
        };
        for user in
            users_with_permission_in(db, permissions::LOTS_RELEASE, &location.scope()).await?
        {
            ids.insert(user.id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// Tells the deciders of `locations` about `hold` with the notification
/// `kind` and `data`.
pub(crate) async fn tell_deciders(
    state: &AppState,
    hold: &StockHold,
    kind: &str,
    locations: &BTreeSet<i64>,
    data: renox::serde_json::Value,
) -> Result {
    let people = deciders(&state.db, locations).await?;
    if people.is_empty() {
        return Ok(());
    }
    let number = hold.number.clone().unwrap_or_default();
    let mut request = NotifyRequest::new(kind, hold.subject())
        .subject_label(&number)
        .data(data);
    for id in people {
        request = request.to_user(id);
    }
    notify::notifier(state).notify(state, request).await?;
    Ok(())
}

/// The kernel's view of a line.
fn held_line(line: &StockHoldLine) -> HeldLine {
    HeldLine {
        item: Subject::new(ITEM_SUBJECT_TYPE, line.item_id),
        location: Subject::new(LOCATION_SUBJECT_TYPE, line.location_id),
        lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
        quantity_milli: line.quantity_milli,
    }
}

/// A hold placed inside a transaction that is not committed yet: the hold and
/// the lots that went `on_hold`, whose events follow the commit
/// ([`announce_placed`]).
pub(crate) struct Placed {
    /// The hold.
    pub(crate) hold: StockHold,
    /// The `hold` runs of the lots, to emit after the commit.
    outcomes: Vec<Outcome<Lot>>,
}

/// Places a hold in `tx` without committing (§4 `place`): the lines from the
/// balances in scope, the hold's items, and each lot of a whole-lot scope
/// `on_hold`. With `allow_empty` a scope with no balance gives a hold with no
/// line (a recall of lots that are all used up); otherwise it is refused.
///
/// # Errors
///
/// 422 for a reason that is not one of [`REASONS`], an empty scope or (unless
/// `allow_empty`) a scope with no balance; a database error.
pub(crate) async fn place_in(
    tx: &mut Transaction,
    new: &NewHold,
    allow_empty: bool,
) -> Result<Placed> {
    let reason = new.reason.trim().to_lowercase();
    if !REASONS.contains(&reason.as_str()) {
        return Err(refused(said("wl_inventory.hold.bad_reason", &[])));
    }
    let (scope_kind, location_id, item_ids, lot_ids) = match &new.scope {
        NewScope::Lots(lots) => (HoldScope::Lots, None, Vec::new(), lots.clone()),
        NewScope::Location {
            location_id,
            item_ids,
        } => (
            HoldScope::Location,
            Some(*location_id),
            item_ids.clone(),
            Vec::new(),
        ),
    };
    if scope_kind == HoldScope::Lots && lot_ids.is_empty() {
        return Err(refused(said("wl_inventory.hold.no_scope", &[])));
    }
    let mut lines: Vec<AddedLine> = Vec::new();
    match &new.scope {
        NewScope::Lots(lots) => {
            for lot_id in lots {
                let lot = Lot::find_or_404(&mut *tx, *lot_id).await?;
                for (place, quantity) in balance::lot_locations(&mut *tx, lot.id).await? {
                    lines.push(AddedLine {
                        item_id: lot.item_id,
                        location_id: place,
                        lot_id: Some(lot.id),
                        quantity_milli: quantity,
                    });
                }
            }
        }
        NewScope::Location {
            location_id,
            item_ids,
        } => {
            Location::find_or_404(&mut *tx, *location_id).await?;
            lines = location_lines(&mut *tx, *location_id, item_ids).await?;
        }
    }
    if lines.is_empty() && !allow_empty {
        return Err(refused(said("wl_inventory.hold.no_balance", &[])));
    }
    // The scope of the location of a hold on a location decides who may
    // cancel it.
    let (scope_type, scope_id) = match location_id {
        Some(id) => {
            let location = Location::find_or_404(&mut *tx, id).await?;
            (location.scope_type, location.scope_id)
        }
        None => (None, None),
    };
    let mut hold = StockHold {
        number: Some(
            STOCK_HOLD_SERIES
                .next(&mut *tx, None, None, today())
                .await?,
        ),
        reason: reason.clone(),
        reference_type: Some(new.reference.subject_type.clone()),
        reference_id: Some(new.reference.subject_id),
        scope_kind,
        location_id,
        status: HoldStatus::Open,
        placed_by: new.by,
        scope_type,
        scope_id,
        placed_at: Some(renox::db::now()),
        ..StockHold::default()
    };
    hold.insert(&mut *tx).await?;
    for added in &lines {
        new_line(
            &mut *tx,
            hold.id,
            added.item_id,
            added.location_id,
            added.lot_id,
            added.quantity_milli,
        )
        .await?;
    }
    for item_id in &item_ids {
        let mut row = StockHoldItem {
            stock_hold_id: hold.id,
            item_id: *item_id,
            ..StockHoldItem::default()
        };
        row.insert(&mut *tx).await?;
    }
    // A lot that can be held goes `on_hold`; one that is held or expired
    // already stays as it is, with its balance as a line all the same.
    let mut outcomes = Vec::new();
    for lot_id in &lot_ids {
        let lot = Lot::find_or_404(&mut *tx, *lot_id).await?;
        if matches!(lot.status, LotStatus::Quarantine | LotStatus::Released) {
            let outcome = lot
                .transition(lot::HOLD)
                .by_system()
                .reason(reason.clone())
                .run_in(&mut *tx)
                .await
                .map_err(TransitionError::into_error)?;
            outcomes.push(outcome);
        }
    }
    Ok(Placed { hold, outcomes })
}

/// After the commit of [`place_in`]: emits `LotHeld` for each lot held now
/// and [`StockHoldPlaced`], and tells the holders of `lots.release`.
pub(crate) async fn announce_placed(state: &AppState, placed: Placed) -> Result<StockHold> {
    let Placed { hold, outcomes } = placed;
    for outcome in outcomes {
        outcome.emit(state).await?;
    }
    let rows = lines_of(&state.db, hold.id).await?;
    let number = hold.number.clone().unwrap_or_default();
    state
        .emit(StockHoldPlaced {
            hold: hold.subject(),
            number: number.clone(),
            reason: hold.reason.clone(),
            reference: hold.reference(),
            lines: rows.iter().map(held_line).collect(),
        })
        .await?;
    let locations: BTreeSet<i64> = rows.iter().map(|line| line.location_id).collect();
    tell_deciders(
        state,
        &hold,
        PLACED_NOTICE,
        &locations,
        renox::serde_json::json!({
            "number": number,
            "reason": hold.reason,
            "lines": rows.len().to_string(),
        }),
    )
    .await?;
    Ok(hold)
}

/// Places a hold (§4 `place`). Moves no stock: it makes the lines from the
/// balances in scope, puts each lot of a whole-lot scope `on_hold`, tells the
/// holders of `lots.release` and emits [`StockHoldPlaced`] (and `LotHeld`
/// for each lot held now) after the commit.
///
/// # Errors
///
/// 422 for a reason that is not one of [`REASONS`], an empty scope or a
/// scope with no balance; a database error.
pub async fn place(state: &AppState, new: NewHold) -> Result<StockHold> {
    let mut tx = state.db.begin().await?;
    let placed = place_in(&mut tx, &new, false).await?;
    tx.commit().await?;
    announce_placed(state, placed).await
}

/// `user` (a holder of `stock.hold`) places `new`: the permission is checked
/// in the scope of the location for a hold on a location, globally for
/// whole lots.
///
/// # Errors
///
/// `Forbidden` without the permission, and the errors of [`place`].
pub async fn place_as(state: &AppState, user: &AuthUser, mut new: NewHold) -> Result<StockHold> {
    let allowed = match &new.scope {
        NewScope::Lots(_) => user.has_permission(permissions::STOCK_HOLD),
        NewScope::Location { location_id, .. } => {
            let location = Location::find_or_404(&state.db, *location_id).await?;
            user.has_permission_in(permissions::STOCK_HOLD, &location.scope())
        }
    };
    if !allowed {
        return Err(Error::Forbidden);
    }
    new.by = Some(user.id);
    place(state, new).await
}

/// Adds `lines` that moved into the location of the open hold `hold_id` to
/// it (§4 `add_lines`), as `by` (a holder of `stock.hold`) or as the system.
/// The deciders are told by the listener on the transition
/// ([`crate::events`]).
///
/// # Errors
///
/// `Guard` (422) when the hold is not on a location, a line is at another
/// location or of an item the hold does not cover, or a quantity is not
/// above zero; `NotFrom` unless the hold is `open`.
pub async fn add_lines(
    state: &AppState,
    by: Option<&AuthUser>,
    hold_id: i64,
    lines: Vec<AddedLine>,
) -> std::result::Result<StockHold, TransitionError> {
    let hold = StockHold::find_or_404(&state.db, hold_id).await?;
    let scope_items = scope_items(&state.db, hold.id).await?;
    let count = lines.len();
    let run = AddLinesInput { lines, scope_items };
    let meta = renox::serde_json::json!({ "added": count });
    match by {
        Some(user) => {
            let _outcome = hold
                .transition(ADD_LINES)
                .by(user)
                .input(&run)
                .meta(meta)
                .run(state)
                .await?;
        }
        None => {
            let _outcome = hold
                .transition(ADD_LINES_BY_SYSTEM)
                .by_system()
                .input(&run)
                .meta(meta)
                .run(state)
                .await?;
        }
    }
    Ok(StockHold::find_or_404(&state.db, hold_id).await?)
}

/// Tells the deciders that lines were added to hold `hold_id` (called by the
/// listener on `add_lines`).
pub(crate) async fn announce_added(state: &AppState, hold_id: i64, added: usize) -> Result {
    let hold = StockHold::find_or_404(&state.db, hold_id).await?;
    let rows = lines_of(&state.db, hold.id).await?;
    let locations: BTreeSet<i64> = rows.iter().map(|line| line.location_id).collect();
    tell_deciders(
        state,
        &hold,
        LINES_NOTICE,
        &locations,
        renox::serde_json::json!({
            "number": hold.number.clone().unwrap_or_default(),
            "reason": hold.reason,
            "added": added.to_string(),
        }),
    )
    .await
}

/// The system's [`StockHoldService`](wl_kernel::inventory::StockHoldService).
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryHolds;

impl wl_kernel::inventory::StockHoldService for InventoryHolds {
    fn place<'a>(
        &'a self,
        state: &'a AppState,
        hold: wl_kernel::inventory::PlaceHold,
    ) -> wl_kernel::inventory::StockFuture<'a, Option<Subject>> {
        Box::pin(async move {
            let item_ids = hold.items.iter().map(|item| item.subject_id).collect();
            let placed = place(
                state,
                NewHold {
                    reason: hold.reason,
                    reference: hold.reference,
                    scope: NewScope::Location {
                        location_id: hold.location.subject_id,
                        item_ids,
                    },
                    by: None,
                },
            )
            .await?;
            Ok(Some(placed.subject()))
        })
    }

    fn view<'a>(
        &'a self,
        state: &'a AppState,
        hold: &'a Subject,
    ) -> wl_kernel::inventory::StockFuture<'a, Option<HoldView>> {
        Box::pin(async move {
            if hold.subject_type != STOCK_HOLD_SUBJECT_TYPE {
                return Ok(None);
            }
            let Some(found) = StockHold::find(&state.db, hold.subject_id).await? else {
                return Ok(None);
            };
            let mut lines = Vec::new();
            for line in lines_of(&state.db, found.id).await? {
                let item = Item::find_or_404(&state.db, line.item_id).await?;
                lines.push(HoldLineView {
                    line: Subject::new(STOCK_HOLD_LINE_SUBJECT_TYPE, line.id),
                    item: Subject::new(ITEM_SUBJECT_TYPE, item.id),
                    item_code: item.code.clone(),
                    item_name: item.name_en.clone(),
                    location: Subject::new(LOCATION_SUBJECT_TYPE, line.location_id),
                    lot: line.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id)),
                    quantity_milli: line.quantity_milli,
                    value: value_of(line.quantity_milli, item.average_cost),
                    status: line.status.as_str().to_owned(),
                });
            }
            Ok(Some(HoldView {
                hold: found.subject(),
                number: found.number.clone().unwrap_or_default(),
                reason: found.reason.clone(),
                open: found.status == HoldStatus::Open,
                lines,
            }))
        })
    }

    fn locations<'a>(
        &'a self,
        state: &'a AppState,
    ) -> wl_kernel::inventory::StockFuture<'a, Vec<LocationInfo>> {
        Box::pin(async move {
            Ok(Location::query()
                .order_by(Location::CODE)
                .get(&state.db)
                .await?
                .into_iter()
                .map(|location| LocationInfo {
                    location: location.subject(),
                    code: location.code,
                    name: location.name_en,
                })
                .collect())
        })
    }

    fn decide<'a>(
        &'a self,
        state: &'a AppState,
        user: &'a AuthUser,
        line: &'a Subject,
        outcome: &'a str,
    ) -> wl_kernel::inventory::StockFuture<'a, Option<HoldDecision>> {
        Box::pin(async move {
            if line.subject_type != STOCK_HOLD_LINE_SUBJECT_TYPE {
                return Ok(None);
            }
            let decision = match outcome {
                "release" => Decision::Release,
                "discard" => Decision::Discard,
                "return_to_supplier" => Decision::ReturnToSupplier,
                _ => return Err(refused(said("wl_inventory.hold.bad_form", &[]))),
            };
            let done = decide(state, user, line.subject_id, decision)
                .await
                .map_err(TransitionError::into_error)?;
            Ok(Some(match done {
                DecideOutcome::Decided(_) => HoldDecision::Decided,
                DecideOutcome::Waiting(_) => HoldDecision::Waiting,
            }))
        })
    }
}
