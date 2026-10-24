//! Stock set aside for a sales order line (wl-inventory §3 *Stock balance*:
//! reserved; BL-SAL-11 point 2, BL-MFG-02 point 5, BL-MFG-03).
//!
//! A [`Reservation`] names an item at a location (and a lot of it, for a
//! lot-tracked item), the sales order and the sale line it is for, and where
//! it came from:
//!
//! - `order`: the order was confirmed (`SalesOrderConfirmed`, also when a
//!   held order is released and confirmed). The stock of each item line that
//!   is available at the location with the most of it is set aside, first
//!   expiry first, for the open quantity less what dispatches already carry.
//!   One reservation per line, once: a replay of the event, or a line that
//!   had nothing to reserve, leaves a row behind that stops a second try.
//! - `work_order`: the output lot of a work order serving the line
//!   ([`ProductionStock::reserve_output`](wl_kernel::production::ProductionStock::reserve_output)).
//!
//! What a reservation does:
//!
//! - it counts in [`crate::balance::reserved_at`], so the available quantity
//!   falls and the pick of another order cannot take the lot;
//! - the dispatch of the same order can: its pick adds the order's own
//!   reservation back, and the pick turns the reservation into the
//!   dispatch's own (the reservation is `consumed`, the picked lines count as
//!   reserved from then on);
//! - it is given back (`released`) when the order is cancelled, when the line
//!   is cancelled or lowered below it (`SalesOrderLineCancelled`,
//!   `SalesOrderAmended`), and by the work order's `unlink` or abort.
//!
//! Decisions: the reservation of a confirmed order is made at one location
//! (the one with the most available stock of the item, not a quarantine,
//! scrap, transit or van location) and takes what is available, so it may be
//! less than the open quantity; the rest is not queued (back-orders come with
//! BL-SAL-17). An increase of a line by an amendment reserves nothing more.

use renox::db::{DateTime, Executor, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::sales::{
    SalesOrderAmended, SalesOrderCancelled, SalesOrderConfirmed, SalesOrderId,
    SalesOrderLineCancelled, order_desk,
};

use crate::balance;
use crate::dispatch::{Dispatch, DispatchLine, SALE_LINE_SUBJECT_TYPE};
use crate::item::Item;
use crate::location::{Location, LocationType};
use crate::stock_service::plan;

/// The source of the reservation a confirmed order makes.
pub const SOURCE_ORDER: &str = "order";

/// The source of the reservation of a work order's output lot.
pub const SOURCE_WORK_ORDER: &str = "work_order";

/// Why the reservations of a cancelled order are given back.
pub const ORDER_CANCELLED: &str = "order cancelled";

/// Why the reservations of a cancelled or lowered line are given back.
pub const LINE_CHANGED: &str = "line cancelled or lowered";

/// Why the reservation of a work order's output is given back.
pub const WORK_ORDER_RELEASED: &str = "output no longer needed by the line";

/// Why a line that found no stock keeps a row.
const NOTHING_AVAILABLE: &str = "no stock available";

/// Where a reservation stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReservationStatus {
    /// The stock is set aside.
    #[default]
    Active,
    /// Given back to free stock.
    Released,
    /// A dispatch of the order picked it: the dispatch's lines reserve it.
    Consumed,
}

/// Stock set aside for one sale line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_reservations",
    unique(order_id, sale_line_id, source, lot_key),
    index(item_id, location_id),
    index(order_id),
    index(lot_id)
)]
pub struct Reservation {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// The lot, or 0 (makes the key unique).
    pub lot_key: i64,
    /// Where the stock is.
    pub location_id: i64,
    /// The quantity set aside, in thousandths.
    pub quantity_milli: i64,
    /// The sales order (its id in `wl-sales`).
    pub order_id: i64,
    /// The sale line (its id in `wl-sales`).
    pub sale_line_id: i64,
    /// [`SOURCE_ORDER`] or [`SOURCE_WORK_ORDER`].
    pub source: String,
    /// The record that asked, e.g. `manufacturing.work_order:5`.
    pub source_ref: Option<String>,
    /// Where it stands.
    pub status: ReservationStatus,
    /// Why it was given back.
    pub released_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Reservation {
    /// The reservations of sale line `line` of `order`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_line<'c>(
        db: impl Executor<'c>,
        order: i64,
        line: i64,
    ) -> Result<Vec<Reservation>> {
        Reservation::query()
            .where_(Reservation::ORDER_ID.eq(order))
            .where_(Reservation::SALE_LINE_ID.eq(line))
            .order_by(Reservation::ID)
            .get(db)
            .await
    }

    /// The reservations of `order`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_order<'c>(db: impl Executor<'c>, order: i64) -> Result<Vec<Reservation>> {
        Reservation::query()
            .where_(Reservation::ORDER_ID.eq(order))
            .order_by(Reservation::ID)
            .get(db)
            .await
    }

    /// The active reservations that hold `item` at `location`.
    async fn active_at<'c>(
        db: impl Executor<'c>,
        item_id: i64,
        location_id: i64,
    ) -> Result<Vec<Reservation>> {
        Reservation::query()
            .where_(Reservation::ITEM_ID.eq(item_id))
            .where_(Reservation::LOCATION_ID.eq(location_id))
            .where_(Reservation::STATUS.eq(ReservationStatus::Active))
            .get(db)
            .await
    }
}

/// What a reservation is made of: the stock and the line it is for.
#[derive(Debug, Clone)]
pub struct Place {
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// Where the stock is.
    pub location_id: i64,
}

/// What of `item_id` at `location_id` (of `lot_id` when given) the active
/// reservations set aside, in thousandths. `only` limits it to one source.
///
/// # Errors
///
/// A database error.
pub async fn reserved_in<'c>(
    db: impl Executor<'c>,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
    only: Option<&str>,
) -> Result<i64> {
    Ok(Reservation::active_at(db, item_id, location_id)
        .await?
        .iter()
        .filter(|row| lot_id.is_none() || row.lot_id == lot_id)
        .filter(|row| only.is_none_or(|source| row.source == source))
        .map(|row| row.quantity_milli)
        .sum())
}

/// What the active reservations of sale line `line` of `order` set aside at
/// `location_id` of `item_id` (of `lot_id` when given), in thousandths: the
/// part of the reserved quantity that the order's own dispatch may take.
///
/// # Errors
///
/// A database error.
pub async fn own_in<'c>(
    db: impl Executor<'c>,
    order: i64,
    line: i64,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    Ok(Reservation::query()
        .where_(Reservation::ORDER_ID.eq(order))
        .where_(Reservation::SALE_LINE_ID.eq(line))
        .where_(Reservation::ITEM_ID.eq(item_id))
        .where_(Reservation::LOCATION_ID.eq(location_id))
        .where_(Reservation::STATUS.eq(ReservationStatus::Active))
        .get(db)
        .await?
        .iter()
        .filter(|row| lot_id.is_none() || row.lot_id.is_none() || row.lot_id == lot_id)
        .map(|row| row.quantity_milli)
        .sum())
}

/// The sale line a reservation is for, and who asked.
#[derive(Debug, Clone)]
pub struct ForLine {
    /// The sales order.
    pub order: i64,
    /// The sale line.
    pub line: i64,
    /// [`SOURCE_ORDER`] or [`SOURCE_WORK_ORDER`].
    pub source: &'static str,
    /// The record that asked.
    pub source_ref: Option<String>,
}

/// Sets the reservation of `for_line` on the stock `place` to exactly `quantity_milli`: made when there is none, changed
/// when it exists, given back (with `reason`) at 0 or less. Returns whether a
/// reservation is active afterwards.
///
/// # Errors
///
/// A database error.
pub async fn set(
    tx: &mut Transaction,
    place: &Place,
    for_line: ForLine,
    quantity_milli: i64,
    reason: &str,
) -> Result<bool> {
    let ForLine {
        order,
        line,
        source,
        source_ref,
    } = for_line;
    let lot_key = place.lot_id.unwrap_or(0);
    let found = Reservation::query()
        .where_(Reservation::ORDER_ID.eq(order))
        .where_(Reservation::SALE_LINE_ID.eq(line))
        .where_(Reservation::SOURCE.eq(source.to_owned()))
        .where_(Reservation::LOT_KEY.eq(lot_key))
        .first(&mut *tx)
        .await?;
    let wanted = quantity_milli.max(0);
    let status = if wanted > 0 {
        ReservationStatus::Active
    } else {
        ReservationStatus::Released
    };
    let note = (wanted == 0).then(|| reason.to_owned());
    match found {
        Some(row) => {
            Reservation::query()
                .where_(Reservation::ID.eq(row.id))
                .update(
                    &mut *tx,
                    &[
                        ("location_id", &place.location_id),
                        ("quantity_milli", &wanted),
                        ("status", &status),
                        ("released_reason", &note),
                        ("source_ref", &source_ref),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        None => {
            let mut row = Reservation {
                item_id: place.item_id,
                lot_id: place.lot_id,
                lot_key,
                location_id: place.location_id,
                quantity_milli: wanted,
                order_id: order,
                sale_line_id: line,
                source: source.to_owned(),
                source_ref,
                status,
                released_reason: note,
                ..Reservation::default()
            };
            row.insert(&mut *tx).await?;
        }
    }
    Ok(wanted > 0)
}

/// Gives back the active reservations of `order` (of its line `line` when
/// given; of `lot` when given) with `reason`. Returns whether any was active.
///
/// # Errors
///
/// A database error.
pub async fn release(
    tx: &mut Transaction,
    order: i64,
    line: Option<i64>,
    lot: Option<i64>,
    reason: &str,
) -> Result<bool> {
    let mut released = false;
    for row in Reservation::of_order(&mut *tx, order).await? {
        if row.status != ReservationStatus::Active
            || line.is_some_and(|line| row.sale_line_id != line)
            || lot.is_some_and(|lot| row.lot_id != Some(lot))
        {
            continue;
        }
        Reservation::query()
            .where_(Reservation::ID.eq(row.id))
            .update(
                &mut *tx,
                &[
                    ("status", &ReservationStatus::Released),
                    ("released_reason", &Some(reason.to_owned())),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        released = true;
    }
    Ok(released)
}

/// Brings the active reservations of line `line` of `order` down to `open_milli`
/// (the line's open quantity): the confirmed order's own reservation gives
/// way first, then the work orders'. At 0 everything is given back.
///
/// # Errors
///
/// A database error.
pub async fn trim_to(tx: &mut Transaction, order: i64, line: i64, open_milli: i64) -> Result {
    let mut rows: Vec<Reservation> = Reservation::of_line(&mut *tx, order, line)
        .await?
        .into_iter()
        .filter(|row| row.status == ReservationStatus::Active)
        .collect();
    let mut excess = rows.iter().map(|row| row.quantity_milli).sum::<i64>() - open_milli.max(0);
    // The order's own rows first, the latest first.
    rows.sort_by_key(|row| (row.source != SOURCE_ORDER, std::cmp::Reverse(row.id)));
    for row in rows {
        if excess <= 0 {
            break;
        }
        let cut = excess.min(row.quantity_milli);
        let left = row.quantity_milli - cut;
        Reservation::query()
            .where_(Reservation::ID.eq(row.id))
            .update(
                &mut *tx,
                &[
                    ("quantity_milli", &left),
                    (
                        "status",
                        &if left > 0 {
                            ReservationStatus::Active
                        } else {
                            ReservationStatus::Released
                        },
                    ),
                    (
                        "released_reason",
                        &(left == 0).then(|| LINE_CHANGED.to_owned()),
                    ),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        excess -= cut;
    }
    Ok(())
}

/// A dispatch of `order` picked `quantity_milli` of `item_id` for `line` at
/// `location_id`: the reservation of the line gives way to the dispatch's own
/// lines (which count as reserved while it is `picked`).
///
/// # Errors
///
/// A database error.
pub async fn consume(
    tx: &mut Transaction,
    order: i64,
    line: i64,
    place: &Place,
    quantity_milli: i64,
) -> Result {
    let mut left = quantity_milli;
    for row in Reservation::of_line(&mut *tx, order, line).await? {
        if left <= 0 {
            break;
        }
        if row.status != ReservationStatus::Active
            || row.item_id != place.item_id
            || row.location_id != place.location_id
            || !(place.lot_id.is_none() || row.lot_id.is_none() || row.lot_id == place.lot_id)
        {
            continue;
        }
        let take = left.min(row.quantity_milli);
        let rest = row.quantity_milli - take;
        Reservation::query()
            .where_(Reservation::ID.eq(row.id))
            .update(
                &mut *tx,
                &[
                    ("quantity_milli", &rest),
                    (
                        "status",
                        &if rest > 0 {
                            ReservationStatus::Active
                        } else {
                            ReservationStatus::Consumed
                        },
                    ),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        left -= take;
    }
    Ok(())
}

/// The quantity of sale line `line` that dispatches of `order` already carry
/// (their lines in any state but cancelled), in thousandths.
async fn carried(tx: &mut Transaction, order: i64, line: i64) -> Result<i64> {
    let mut total = 0;
    for row in DispatchLine::query()
        .where_(DispatchLine::SOURCE_LINE_TYPE.eq(Some(SALE_LINE_SUBJECT_TYPE.to_owned())))
        .where_(DispatchLine::SOURCE_LINE_ID.eq(Some(line)))
        .get(&mut *tx)
        .await?
    {
        let dispatch = Dispatch::find_or_404(&mut *tx, row.dispatch_id).await?;
        if dispatch.source_id == order
            && dispatch.status != crate::dispatch::DispatchStatus::Cancelled
        {
            total += row.quantity_milli;
        }
    }
    Ok(total)
}

/// The location with the most available stock of `item`, with the stock to
/// take in the order to pick it: `(location, [(lot, quantity)])`.
async fn best_place(
    tx: &mut Transaction,
    item: &Item,
    want: i64,
) -> Result<Option<(i64, Vec<(Option<i64>, i64)>)>> {
    let mut best: Option<(i64, i64)> = None;
    for location in Location::query()
        .order_by(Location::ID)
        .get(&mut *tx)
        .await?
    {
        if matches!(
            location.location_type,
            LocationType::Quarantine
                | LocationType::Scrap
                | LocationType::InTransit
                | LocationType::Van
        ) {
            continue;
        }
        let free = balance::available_to_take(&mut *tx, item, location.id).await?;
        if free > 0 && best.is_none_or(|(_, most)| free > most) {
            best = Some((location.id, free));
        }
    }
    let Some((location, _)) = best else {
        return Ok(None);
    };
    let candidates = balance::candidates(&mut *tx, item, location).await?;
    let (parts, _) = plan(&candidates, want);
    Ok(Some((location, parts)))
}

/// `SalesOrderConfirmed`: reserves the stock of each item line of the order
/// (see the module). Once per line.
///
/// # Errors
///
/// A database error.
pub async fn on_confirmed(state: &AppState, event: &SalesOrderConfirmed) -> Result {
    let Some(view) = order_desk(state).find(state, event.order).await? else {
        return Ok(());
    };
    let order = event.order.0;
    for line in view.lines.iter().filter(|line| line.is_item) {
        let Some(item) = Item::query()
            .where_(Item::CODE.eq(line.item_code.clone()))
            .first(&state.db)
            .await?
        else {
            continue;
        };
        let mut tx = state.db.begin().await?;
        let done = Reservation::of_line(&mut tx, order, line.id)
            .await?
            .iter()
            .any(|row| row.source == SOURCE_ORDER);
        if done {
            continue;
        }
        let want = line.open_milli() - carried(&mut tx, order, line.id).await?;
        if want <= 0 {
            continue;
        }
        match best_place(&mut tx, &item, want).await? {
            Some((location_id, parts)) => {
                for (lot_id, quantity) in parts {
                    let place = Place {
                        item_id: item.id,
                        lot_id,
                        location_id,
                    };
                    set(&mut tx, &place, for_order(order, line.id), quantity, "").await?;
                }
            }
            None => {
                // A mark that this line was looked at, so a replay of the
                // event does not reserve later arrivals (there is nothing to
                // mark when the module has no location at all).
                if let Some(location_id) = first_location(&mut tx).await? {
                    let place = Place {
                        item_id: item.id,
                        lot_id: None,
                        location_id,
                    };
                    set(
                        &mut tx,
                        &place,
                        for_order(order, line.id),
                        0,
                        NOTHING_AVAILABLE,
                    )
                    .await?;
                }
            }
        }
        tx.commit().await?;
    }
    Ok(())
}

/// The reservation a confirmed order makes for its line.
fn for_order(order: i64, line: i64) -> ForLine {
    ForLine {
        order,
        line,
        source: SOURCE_ORDER,
        source_ref: None,
    }
}

/// Any location (the mark of a line with nothing to reserve needs one).
async fn first_location(tx: &mut Transaction) -> Result<Option<i64>> {
    Ok(Location::query()
        .order_by(Location::ID)
        .first(&mut *tx)
        .await?
        .map(|location| location.id))
}

/// `SalesOrderCancelled`: every reservation of the order is given back.
///
/// # Errors
///
/// A database error.
pub async fn on_order_cancelled(state: &AppState, event: &SalesOrderCancelled) -> Result {
    let mut tx = state.db.begin().await?;
    release(&mut tx, event.order.0, None, None, ORDER_CANCELLED).await?;
    tx.commit().await?;
    Ok(())
}

/// `SalesOrderLineCancelled`: the reservations of the line shrink to what is
/// still open of it (all given back when none is).
///
/// # Errors
///
/// A database error.
pub async fn on_line_cancelled(state: &AppState, event: &SalesOrderLineCancelled) -> Result {
    trim_line(state, event.order, event.line).await
}

/// `SalesOrderAmended`: a line lowered below its reservation gives the rest
/// back.
///
/// # Errors
///
/// A database error.
pub async fn on_amended(state: &AppState, event: &SalesOrderAmended) -> Result {
    for line in &event.lines {
        if line.quantity_after_milli < line.quantity_before_milli {
            trim_line(state, event.order, line.line).await?;
        }
    }
    Ok(())
}

/// Brings the reservations of `line` down to the open quantity the order says.
async fn trim_line(state: &AppState, order: SalesOrderId, line: i64) -> Result {
    let open = match order_desk(state).find(state, order).await? {
        Some(view) => view
            .lines
            .iter()
            .find(|candidate| candidate.id == line)
            .map_or(0, |found| found.open_milli()),
        None => 0,
    };
    let mut tx = state.db.begin().await?;
    trim_to(&mut tx, order.0, line, open).await?;
    tx.commit().await?;
    Ok(())
}

/// Registers the model and the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.model::<Reservation>();
    app.listen(|event: SalesOrderConfirmed, state: AppState| async move {
        on_confirmed(&state, &event).await
    });
    app.listen(|event: SalesOrderCancelled, state: AppState| async move {
        on_order_cancelled(&state, &event).await
    });
    app.listen(
        |event: SalesOrderLineCancelled, state: AppState| async move {
            on_line_cancelled(&state, &event).await
        },
    );
    app.listen(|event: SalesOrderAmended, state: AppState| async move {
        on_amended(&state, &event).await
    });
}
