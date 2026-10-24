//! Stock set aside for a sales order line (wl-sales §4.2 *Back-orders*,
//! wl-inventory §3 *Stock balance* "reserved"; BL-SAL-17, BL-SAL-11 point 2,
//! BL-MFG-02 point 5, BL-MFG-03). One table, [`StockReservation`], for both
//! ways stock is set aside:
//!
//! - `order` ([`SOURCE_ORDER`]): `wl-sales` asks through the kernel's
//!   [`StockReservations`]: [`reserve`] sets aside what is available of the
//!   quantity wanted when the order is confirmed (also when a held order is
//!   released), [`release`] gives it back (order cancelled, line cancelled or
//!   lowered, goods handed over). By item and location, not by lot.
//! - `work_order` ([`SOURCE_WORK_ORDER`]): the output lot of a work order
//!   serving the line ([`ProductionStock::reserve_output`]), by lot, kept
//!   equal to what the line needs by [`set`] and given back by
//!   [`release_output`] (the work order's `unlink` or abort) or when the line
//!   or the order goes ([`register`]).
//!
//! - Reserved stock is not available: [`crate::balance::reserved_at`] adds
//!   these rows to what draft returns to supplier and picked dispatches set
//!   aside, so no picking, transfer or consumption takes it. A row with a lot
//!   counts for that lot and for the item's total; a row without a lot
//!   counts for the item's total only.
//! - Available is on hand less reserved less on hold. For an item tracked by
//!   lot, a reservation without a lot lowers the item's total available at
//!   the location (the sum over its released lots), not a lot's balance.
//! - The location is the one the caller names, else the selling location: the
//!   first location of the type store, else the first warehouse, branch,
//!   central store or outlet.
//! - An item the catalogue does not know is not managed: everything asked is
//!   reserved. A client with no location has nothing in stock: nothing is
//!   reserved.
//! - A row given back keeps its history at quantity 0.
//!
//! A dispatch picked for the order that reserved the goods (BL-STK-26 beside
//! BL-SAL-17) is the same goods: `pick` sees the order's reservation of the
//! sale line as available (see [`reserved_for_line`]), and
//! [`crate::balance::reserved_at`] counts the picked line only for what the
//! order's reservation does not cover.
//!
//! Decision: a consumption posted for the order itself (a dispatch that
//! fulfils it) sees the reserved stock as unavailable, so the dispatch gives
//! the reservation back first; wl-sales does so when the order's goods are
//! handed over.

use renox::db::{Executor, Transaction};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::backorders::{BackOrderFuture, ReserveStock, Reserved, StockReservations};
use wl_kernel::sales::{
    SALES_ORDER_SUBJECT_TYPE, SalesOrderAmended, SalesOrderCancelled, SalesOrderId,
    SalesOrderLineCancelled, order_desk,
};

use crate::balance;
use crate::item::Item;
use crate::location::{Location, LocationType};

/// The source of a reservation `wl-sales` asked for (a confirmed order).
pub const SOURCE_ORDER: &str = "order";

/// The source of the reservation of a work order's output lot.
pub const SOURCE_WORK_ORDER: &str = "work_order";

/// Why the reservation of a work order's output is given back.
pub const WORK_ORDER_RELEASED: &str = "output no longer needed by the line";

/// Why the reservation of a cancelled or lowered line is given back.
pub const LINE_CHANGED: &str = "line cancelled or lowered";

/// Why the reservations of a cancelled order are given back.
pub const ORDER_CANCELLED: &str = "order cancelled";

/// Stock set aside for one line of a record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_reservations",
    index(item_id, location_id),
    index(owner_type, owner_id),
    index(lot_id)
)]
pub struct StockReservation {
    /// The primary key.
    pub id: i64,
    /// The item.
    pub item_id: i64,
    /// The location.
    pub location_id: i64,
    /// The type of the record the stock is set aside for.
    pub owner_type: String,
    /// The id of that record.
    pub owner_id: i64,
    /// The line of the record.
    pub line_id: i64,
    /// What is still set aside, in thousandths.
    pub quantity_milli: i64,
    /// The lot, for the output lot of a work order; none for stock set aside
    /// by item and location.
    pub lot_id: Option<i64>,
    /// [`SOURCE_ORDER`] or [`SOURCE_WORK_ORDER`].
    pub source: String,
    /// The record that asked, e.g. `manufacturing.work_order:5`.
    pub source_ref: Option<String>,
    /// Why it was given back (the quantity is then 0).
    pub released_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StockReservation {
    /// The reservations of line `line` of `order`, oldest first, given back
    /// ones (at 0) included.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_line<'c>(
        db: impl Executor<'c>,
        order: i64,
        line: i64,
    ) -> Result<Vec<StockReservation>> {
        StockReservation::query()
            .where_(StockReservation::OWNER_TYPE.eq(SALES_ORDER_SUBJECT_TYPE.to_owned()))
            .where_(StockReservation::OWNER_ID.eq(order))
            .where_(StockReservation::LINE_ID.eq(line))
            .order_by(StockReservation::ID)
            .get(db)
            .await
    }

    /// The reservations of `order`, oldest first, given back ones included.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_order<'c>(db: impl Executor<'c>, order: i64) -> Result<Vec<StockReservation>> {
        StockReservation::query()
            .where_(StockReservation::OWNER_TYPE.eq(SALES_ORDER_SUBJECT_TYPE.to_owned()))
            .where_(StockReservation::OWNER_ID.eq(order))
            .order_by(StockReservation::ID)
            .get(db)
            .await
    }

    /// Whether stock is set aside by this row.
    pub fn is_active(&self) -> bool {
        self.quantity_milli > 0
    }
}

/// What sales orders have set aside of `item_id` at `location_id` by item
/// (without a lot), in thousandths.
///
/// # Errors
///
/// A database error.
pub async fn sales_reserved<'c>(
    db: impl Executor<'c>,
    item_id: i64,
    location_id: i64,
) -> Result<i64> {
    StockReservation::query()
        .where_(StockReservation::ITEM_ID.eq(item_id))
        .where_(StockReservation::LOCATION_ID.eq(location_id))
        .where_(StockReservation::LOT_ID.is_null())
        .sum::<i64, _>(db, "quantity_milli")
        .await
}

/// What the reservations set aside of `item_id` at `location_id`, in
/// thousandths: all of them, or those of `lot_id` when given. `only` limits
/// it to one source.
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
    Ok(StockReservation::query()
        .where_(StockReservation::ITEM_ID.eq(item_id))
        .where_(StockReservation::LOCATION_ID.eq(location_id))
        .where_(StockReservation::QUANTITY_MILLI.gt(0i64))
        .get(db)
        .await?
        .iter()
        .filter(|row| lot_id.is_none() || row.lot_id == lot_id)
        .filter(|row| only.is_none_or(|source| row.source == source))
        .map(|row| row.quantity_milli)
        .sum())
}

/// What the record `owner` has set aside for `line` of `item_id` at
/// `location_id` (of `lot_id` when given), in thousandths: the part of a
/// picked dispatch that is already counted as reserved for its own order.
///
/// # Errors
///
/// A database error.
pub async fn reserved_for_line<'c>(
    db: impl Executor<'c>,
    owner: &Subject,
    line: i64,
    item_id: i64,
    location_id: i64,
    lot_id: Option<i64>,
) -> Result<i64> {
    Ok(StockReservation::query()
        .where_(StockReservation::OWNER_TYPE.eq(owner.subject_type.clone()))
        .where_(StockReservation::OWNER_ID.eq(owner.subject_id))
        .where_(StockReservation::LINE_ID.eq(line))
        .where_(StockReservation::ITEM_ID.eq(item_id))
        .where_(StockReservation::LOCATION_ID.eq(location_id))
        .where_(StockReservation::QUANTITY_MILLI.gt(0i64))
        .get(db)
        .await?
        .iter()
        .filter(|row| lot_id.is_none() || row.lot_id == lot_id)
        .map(|row| row.quantity_milli)
        .sum())
}

/// The location stock is sold from when the caller names none: the first
/// store, else the first warehouse, branch, central store or outlet.
///
/// # Errors
///
/// A database error.
pub async fn selling_location(db: &Db) -> Result<Option<Location>> {
    let found = Location::query().order_by(Location::ID).get(db).await?;
    let sells = |location: &&Location| {
        matches!(
            location.location_type,
            LocationType::Store
                | LocationType::Warehouse
                | LocationType::Branch
                | LocationType::CentralStore
                | LocationType::Outlet
        )
    };
    Ok(found
        .iter()
        .find(|location| location.location_type == LocationType::Store)
        .or_else(|| found.iter().find(sells))
        .cloned())
}

/// What can be set aside of `item` at `location_id` now, in thousandths.
async fn available(db: &Db, item: &Item, location_id: i64) -> Result<i64> {
    if item.lot_tracked {
        let free = balance::available_to_take(db, item, location_id).await?;
        let taken = sales_reserved(db, item.id, location_id).await?;
        Ok((free - taken).max(0))
    } else {
        Ok(balance::balance(db, item.id, location_id, None)
            .await?
            .available_milli())
    }
}

/// Sets aside what is available, up to the quantity wanted (see
/// [`StockReservations::reserve`]).
///
/// # Errors
///
/// A database error.
pub async fn reserve(state: &AppState, request: ReserveStock) -> Result<Reserved> {
    let Some(item) = Item::query()
        .where_(Item::CODE.eq(request.item_code.clone()))
        .first(&state.db)
        .await?
    else {
        // Not in the catalogue: stock is not managed for it.
        return Ok(Reserved {
            reserved_milli: request.quantity_milli,
            location: request.location,
        });
    };
    let location = match &request.location {
        Some(subject) => Location::find(&state.db, subject.subject_id).await?,
        None => selling_location(&state.db).await?,
    };
    let Some(location) = location else {
        return Ok(Reserved {
            reserved_milli: 0,
            location: None,
        });
    };
    let take = request
        .quantity_milli
        .min(available(&state.db, &item, location.id).await?)
        .max(0);
    if take > 0 {
        StockReservation::create(
            &state.db,
            StockReservation {
                item_id: item.id,
                location_id: location.id,
                owner_type: request.owner.subject_type.clone(),
                owner_id: request.owner.subject_id,
                line_id: request.line,
                quantity_milli: take,
                source: SOURCE_ORDER.to_owned(),
                ..StockReservation::default()
            },
        )
        .await?;
    }
    Ok(Reserved {
        reserved_milli: take,
        location: Some(location.subject()),
    })
}

/// Gives back up to `quantity_milli` (all when `None`) of what was set aside
/// for `line` (every line when `None`) of `owner`: what `wl-sales` reserved
/// first, newest first, then the output lots of work orders. Returns the
/// quantity given back.
///
/// # Errors
///
/// A database error.
pub async fn release(
    state: &AppState,
    owner: &Subject,
    line: Option<i64>,
    quantity_milli: Option<i64>,
) -> Result<i64> {
    let mut query = StockReservation::query()
        .where_(StockReservation::OWNER_TYPE.eq(owner.subject_type.clone()))
        .where_(StockReservation::OWNER_ID.eq(owner.subject_id))
        .where_(StockReservation::QUANTITY_MILLI.gt(0i64));
    if let Some(line) = line {
        query = query.where_(StockReservation::LINE_ID.eq(line));
    }
    let mut rows = query
        .order_by_desc(StockReservation::ID)
        .get(&state.db)
        .await?;
    rows.sort_by_key(|row| row.source == SOURCE_WORK_ORDER);
    let mut left = quantity_milli.unwrap_or(i64::MAX);
    let mut released = 0;
    for row in rows {
        if left <= 0 {
            break;
        }
        let take = row.quantity_milli.min(left);
        let remaining = row.quantity_milli - take;
        let now = renox::db::now();
        let reason = (remaining == 0).then(|| ORDER_CANCELLED.to_owned());
        StockReservation::query()
            .where_(StockReservation::ID.eq(row.id))
            .update(
                &state.db,
                &[
                    ("quantity_milli", &remaining),
                    ("released_reason", &reason),
                    ("updated_at", &now),
                ],
            )
            .await?;
        left -= take;
        released += take;
    }
    Ok(released)
}

/// The module's [`StockReservations`].
#[derive(Debug, Clone, Copy, Default)]
pub struct InventoryReservations;

impl StockReservations for InventoryReservations {
    fn reserve<'a>(
        &'a self,
        state: &'a AppState,
        request: ReserveStock,
    ) -> BackOrderFuture<'a, Reserved> {
        Box::pin(async move { reserve(state, request).await })
    }

    fn release<'a>(
        &'a self,
        state: &'a AppState,
        owner: &'a Subject,
        line: Option<i64>,
        quantity_milli: Option<i64>,
    ) -> BackOrderFuture<'a, i64> {
        Box::pin(async move { release(state, owner, line, quantity_milli).await })
    }
}

/// What a reservation is made of: the stock it holds.
#[derive(Debug, Clone)]
pub struct Place {
    /// The item.
    pub item_id: i64,
    /// The lot, for a lot-tracked item.
    pub lot_id: Option<i64>,
    /// Where the stock is.
    pub location_id: i64,
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

/// Sets the reservation of `for_line` on the stock `place` to exactly
/// `quantity_milli`: made when there is none, changed when it exists, given
/// back (with `reason`) at 0 or less. Returns whether stock is set aside
/// afterwards.
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
    let found = StockReservation::query()
        .where_(StockReservation::OWNER_TYPE.eq(SALES_ORDER_SUBJECT_TYPE.to_owned()))
        .where_(StockReservation::OWNER_ID.eq(order))
        .where_(StockReservation::LINE_ID.eq(line))
        .where_(StockReservation::SOURCE.eq(source.to_owned()))
        .where_(StockReservation::LOT_ID.eq(place.lot_id))
        .first(&mut *tx)
        .await?;
    let wanted = quantity_milli.max(0);
    let note = (wanted == 0).then(|| reason.to_owned());
    match found {
        Some(row) => {
            StockReservation::query()
                .where_(StockReservation::ID.eq(row.id))
                .update(
                    &mut *tx,
                    &[
                        ("location_id", &place.location_id),
                        ("quantity_milli", &wanted),
                        ("released_reason", &note),
                        ("source_ref", &source_ref),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        None => {
            let mut row = StockReservation {
                item_id: place.item_id,
                location_id: place.location_id,
                owner_type: SALES_ORDER_SUBJECT_TYPE.to_owned(),
                owner_id: order,
                line_id: line,
                quantity_milli: wanted,
                lot_id: place.lot_id,
                source: source.to_owned(),
                source_ref,
                released_reason: note,
                ..StockReservation::default()
            };
            row.insert(&mut *tx).await?;
        }
    }
    Ok(wanted > 0)
}

/// Gives back the output lots of work orders set aside for `order` (for its
/// line `line` when given; of `lot` when given) with `reason`. Returns
/// whether any was set aside.
///
/// # Errors
///
/// A database error.
pub async fn release_output(
    tx: &mut Transaction,
    order: i64,
    line: Option<i64>,
    lot: Option<i64>,
    reason: &str,
) -> Result<bool> {
    let mut released = false;
    for row in StockReservation::of_order(&mut *tx, order).await? {
        if !row.is_active()
            || row.source != SOURCE_WORK_ORDER
            || line.is_some_and(|line| row.line_id != line)
            || lot.is_some_and(|lot| row.lot_id != Some(lot))
        {
            continue;
        }
        StockReservation::query()
            .where_(StockReservation::ID.eq(row.id))
            .update(
                &mut *tx,
                &[
                    ("quantity_milli", &0i64),
                    ("released_reason", &Some(reason.to_owned())),
                    ("updated_at", &renox::db::now()),
                ],
            )
            .await?;
        released = true;
    }
    Ok(released)
}

/// Brings the output lots of work orders set aside for line `line` of
/// `order` down to `open_milli` (the line's open quantity), the latest
/// first. At 0 everything is given back.
///
/// # Errors
///
/// A database error.
pub async fn trim_output(tx: &mut Transaction, order: i64, line: i64, open_milli: i64) -> Result {
    let mut rows: Vec<StockReservation> = StockReservation::of_line(&mut *tx, order, line)
        .await?
        .into_iter()
        .filter(|row| row.is_active() && row.source == SOURCE_WORK_ORDER)
        .collect();
    let mut excess = rows.iter().map(|row| row.quantity_milli).sum::<i64>() - open_milli.max(0);
    rows.sort_by_key(|row| std::cmp::Reverse(row.id));
    for row in rows {
        if excess <= 0 {
            break;
        }
        let cut = excess.min(row.quantity_milli);
        let left = row.quantity_milli - cut;
        StockReservation::query()
            .where_(StockReservation::ID.eq(row.id))
            .update(
                &mut *tx,
                &[
                    ("quantity_milli", &left),
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

/// `SalesOrderCancelled`: the output lots set aside for the order are given
/// back (what `wl-sales` reserved it gives back itself).
///
/// # Errors
///
/// A database error.
pub async fn on_order_cancelled(state: &AppState, event: &SalesOrderCancelled) -> Result {
    let mut tx = state.db.begin().await?;
    release_output(&mut tx, event.order.0, None, None, ORDER_CANCELLED).await?;
    tx.commit().await?;
    Ok(())
}

/// `SalesOrderLineCancelled`: the output lots set aside for the line shrink
/// to what is still open of it (all given back when none is).
///
/// # Errors
///
/// A database error.
pub async fn on_line_cancelled(state: &AppState, event: &SalesOrderLineCancelled) -> Result {
    trim_line(state, event.order, event.line).await
}

/// `SalesOrderAmended`: a line lowered below what is set aside for it gives
/// the rest back.
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

/// Brings the output lots set aside for `line` down to the open quantity the
/// order says.
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
    trim_output(&mut tx, order.0, line, open).await?;
    tx.commit().await?;
    Ok(())
}

/// Registers the listeners (the model is registered by the module).
pub(crate) fn register(app: &mut Registry) {
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
