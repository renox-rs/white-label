//! Stock set aside for a sales order line (wl-sales §4.2 *Back-orders*,
//! wl-inventory §3 *Stock balance* "reserved"; BL-SAL-17). `wl-sales` asks
//! through the kernel's [`StockReservations`]: [`reserve`] sets aside what is
//! available of the quantity wanted, [`release`] gives it back.
//!
//! - Reserved stock is not available: [`crate::balance::reserved_at`] adds
//!   these rows to what draft returns to supplier set aside, so no picking,
//!   transfer or consumption takes it.
//! - Available is on hand less reserved less on hold. A reservation is by
//!   item and location, not by lot: for an item tracked by lot it lowers the
//!   item's total available at the location (the sum over its released
//!   lots), not a lot's balance.
//! - The location is the one the caller names, else the selling location: the
//!   first location of the type store, else the first warehouse, branch,
//!   central store or outlet.
//! - An item the catalogue does not know is not managed: everything asked is
//!   reserved. A client with no location has nothing in stock: nothing is
//!   reserved.
//!
//! A dispatch picked for the order that reserved the goods (BL-STK-26 beside
//! BL-SAL-17) is the same goods: `pick` sees the order's reservation of the
//! sale line as available, and [`crate::balance::reserved_at`] counts the
//! picked line only for what the order's reservation does not cover.
//!
//! Decision: a consumption posted for the order itself (a dispatch that
//! fulfils it) sees the reserved stock as unavailable, so the dispatch gives
//! the reservation back first; wl-sales does so when the order's goods are
//! handed over.

use renox::db::Executor;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::backorders::{BackOrderFuture, ReserveStock, Reserved, StockReservations};

use crate::balance;
use crate::item::Item;
use crate::location::{Location, LocationType};

/// Stock set aside for one line of a record.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_reservations",
    index(item_id, location_id),
    index(owner_type, owner_id)
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
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What sales orders have set aside of `item_id` at `location_id`, in
/// thousandths.
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
        .sum::<i64, _>(db, "quantity_milli")
        .await
}

/// What the record `owner` has set aside for `line` of `item_id` at
/// `location_id`, in thousandths: the part of a picked dispatch that is
/// already counted as reserved for its own order.
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
) -> Result<i64> {
    StockReservation::query()
        .where_(StockReservation::OWNER_TYPE.eq(owner.subject_type.clone()))
        .where_(StockReservation::OWNER_ID.eq(owner.subject_id))
        .where_(StockReservation::LINE_ID.eq(line))
        .where_(StockReservation::ITEM_ID.eq(item_id))
        .where_(StockReservation::LOCATION_ID.eq(location_id))
        .sum::<i64, _>(db, "quantity_milli")
        .await
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
/// for `line` (every line when `None`) of `owner`, newest first. Returns the
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
    let rows = query
        .order_by_desc(StockReservation::ID)
        .get(&state.db)
        .await?;
    let mut left = quantity_milli.unwrap_or(i64::MAX);
    let mut released = 0;
    for row in rows {
        if left <= 0 {
            break;
        }
        let take = row.quantity_milli.min(left);
        let remaining = row.quantity_milli - take;
        let now = renox::db::now();
        StockReservation::query()
            .where_(StockReservation::ID.eq(row.id))
            .update(
                &state.db,
                &[("quantity_milli", &remaining), ("updated_at", &now)],
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
