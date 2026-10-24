//! Back-orders (wl-sales §4.2 *Back-orders*, wl-inventory §6 `STK.RUL.04`;
//! BL-SAL-17): what `wl-sales` and `wl-inventory` tell each other so that a
//! sales order line without stock is back-ordered instead of refused.
//!
//! - [`StockReservations`] (wl-inventory provides it): set stock aside for a
//!   line of an order, up to what is available at a location, and give it
//!   back. Reserved stock is not available to any other picking or
//!   consumption.
//! - [`BackOrderDemand`] (wl-sales provides it): the open back-ordered
//!   quantity per item and location, which the daily reorder suggestion adds
//!   to what the item's reorder policy asks for.
//! - [`StockArrived`] and [`StockArrivalReversed`] (wl-inventory emits them):
//!   a receipt or a transfer brought stock to a location, or a goods receipt
//!   was reversed; `wl-sales` serves the oldest back-orders from the arrival
//!   and takes the reservations back when the receipt is reversed.
//!
//! Quantities are thousandths of the item's base unit. Without the inventory
//! module [`NoStockReservations`] reserves everything asked (stock is not
//! managed, so nothing is short); without the sales module [`NoBackOrders`]
//! reports no demand.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;

/// The future of the contracts in this module.
pub type BackOrderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A request to set stock aside for a line of an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReserveStock {
    /// The code of the item.
    pub item_code: String,
    /// The location to reserve at, `inventory.location:2`; `None`: the
    /// inventory module's selling location (the first store).
    pub location: Option<Subject>,
    /// The quantity wanted, in thousandths.
    pub quantity_milli: i64,
    /// The record the stock is set aside for, `sales.sales_order:5`.
    pub owner: Subject,
    /// The line of the owner (its id).
    pub line: i64,
}

impl ReserveStock {
    /// `quantity_milli` of `item_code` for line `line` of `owner`, at the
    /// default location.
    pub fn new(item_code: &str, quantity_milli: i64, owner: Subject, line: i64) -> Self {
        ReserveStock {
            item_code: item_code.trim().to_owned(),
            location: None,
            quantity_milli,
            owner,
            line,
        }
    }

    /// At `location`.
    pub fn at(mut self, location: Subject) -> Self {
        self.location = Some(location);
        self
    }
}

/// What a [`ReserveStock`] set aside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reserved {
    /// The quantity reserved: what was wanted, or what was available.
    pub reserved_milli: i64,
    /// The location it was reserved at; `None` when the stock is not managed.
    pub location: Option<Subject>,
}

/// Reservations of stock for sales order lines (wl-inventory §3 *Stock
/// balance*, *reserved*). Without the inventory module
/// [`stock_reservations`] gives [`NoStockReservations`].
pub trait StockReservations: Send + Sync + 'static {
    /// Reserves the quantity wanted, or what is available of it, at the
    /// location (available: on hand less reserved less on hold). Returns what
    /// was reserved, which may be nothing. The default reserves all asked.
    fn reserve<'a>(
        &'a self,
        _state: &'a AppState,
        request: ReserveStock,
    ) -> BackOrderFuture<'a, Reserved> {
        Box::pin(async move {
            Ok(Reserved {
                reserved_milli: request.quantity_milli,
                location: request.location,
            })
        })
    }

    /// Gives back up to `quantity_milli` (all when `None`) of what was
    /// reserved for line `line` of `owner` (every line when `None`). Returns
    /// the quantity given back. The default releases nothing.
    fn release<'a>(
        &'a self,
        _state: &'a AppState,
        _owner: &'a Subject,
        _line: Option<i64>,
        _quantity_milli: Option<i64>,
    ) -> BackOrderFuture<'a, i64> {
        Box::pin(async { Ok(0) })
    }
}

/// The reservations when `inventory` is off: stock is not managed, every
/// reservation succeeds in full.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoStockReservations;

impl StockReservations for NoStockReservations {}

/// Makes `reservations` the app's [`StockReservations`]; `wl-inventory` calls
/// it in `register`.
pub fn provide_stock_reservations(app: &mut Registry, reservations: impl StockReservations) {
    let reservations: Arc<dyn StockReservations> = Arc::new(reservations);
    app.provide(reservations);
}

/// The app's [`StockReservations`], or [`NoStockReservations`] when none is
/// provided.
pub fn stock_reservations(state: &AppState) -> Arc<dyn StockReservations> {
    match state.provided::<Arc<dyn StockReservations>>() {
        Some(reservations) => Arc::clone(&reservations),
        None => Arc::new(NoStockReservations),
    }
}

/// The open back-ordered quantity of one item at one location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackOrdered {
    /// The code of the item.
    pub item_code: String,
    /// The location of the orders, `inventory.location:2`.
    pub location: Option<Subject>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

/// The back-ordered demand for the reorder suggestions (`STK.RUL.04`).
/// Without the sales module [`back_orders`] gives [`NoBackOrders`].
pub trait BackOrderDemand: Send + Sync + 'static {
    /// The open back-ordered quantity per item and location, over the orders
    /// that are `confirmed`, `partially_fulfilled` or on `credit_hold`. The
    /// default is none.
    fn open<'a>(&'a self, _state: &'a AppState) -> BackOrderFuture<'a, Vec<BackOrdered>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The demand when `sales` is off: none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBackOrders;

impl BackOrderDemand for NoBackOrders {}

/// Makes `demand` the app's [`BackOrderDemand`]; `wl-sales` calls it in
/// `register`.
pub fn provide_back_orders(app: &mut Registry, demand: impl BackOrderDemand) {
    let demand: Arc<dyn BackOrderDemand> = Arc::new(demand);
    app.provide(demand);
}

/// The app's [`BackOrderDemand`], or [`NoBackOrders`] when none is provided.
pub fn back_orders(state: &AppState) -> Arc<dyn BackOrderDemand> {
    match state.provided::<Arc<dyn BackOrderDemand>>() {
        Some(demand) => Arc::clone(&demand),
        None => Arc::new(NoBackOrders),
    }
}

/// One item that arrived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrivedLine {
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// Its code.
    pub item_code: String,
    /// The quantity that entered stock, in thousandths.
    pub quantity_milli: i64,
}

/// Stock arrived at a location (wl-inventory `StockMoved` for a `receive` or
/// a `transfer_in` movement): a goods receipt was posted or a transfer was
/// received. `wl-sales` reserves it for the oldest back-orders of the
/// location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockArrived {
    /// What brought it: `inventory.goods_receipt:5` or
    /// `inventory.transfer:3`.
    pub source: Subject,
    /// `receive` or `transfer_in`.
    pub kind: String,
    /// The location it arrived at, `inventory.location:2`.
    pub location: Subject,
    /// The items.
    pub lines: Vec<ArrivedLine>,
}

impl Event for StockArrived {}

/// A goods receipt was reversed: its stock left again. `wl-sales` gives back
/// the reservations it made from the receipt and back-orders the lines again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StockArrivalReversed {
    /// The receipt, as in [`StockArrived::source`].
    pub source: Subject,
    /// The location it had arrived at.
    pub location: Subject,
    /// The items.
    pub lines: Vec<ArrivedLine>,
}

impl Event for StockArrivalReversed {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_names_the_line_and_the_default_location() {
        let owner = Subject::new("sales.sales_order", 5);
        let request = ReserveStock::new(" BRK-PAD ", 3_000, owner.clone(), 9);
        assert_eq!(request.item_code, "BRK-PAD");
        assert_eq!(request.location, None);
        let at = request.at(Subject::new("inventory.location", 2));
        assert_eq!(at.location, Some(Subject::new("inventory.location", 2)));
        assert_eq!((at.owner, at.line), (owner, 9));
    }

    #[test]
    fn the_arrival_events_round_trip_through_json() {
        let event = StockArrived {
            source: Subject::new("inventory.goods_receipt", 5),
            kind: "receive".to_owned(),
            location: Subject::new("inventory.location", 2),
            lines: vec![ArrivedLine {
                item: Subject::new("inventory.item", 9),
                item_code: "BRK-PAD".to_owned(),
                quantity_milli: 4_000,
            }],
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        let back: StockArrived = renox::serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
