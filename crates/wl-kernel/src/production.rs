//! The production contract (07 Manufacturing §7.1, base for the work order):
//! what the manufacturing app asks of the other modules without depending on
//! their crates.
//!
//! - [`BillsOfMaterials`]: the BOM version a work order is made from (the BOM
//!   is wl-inventory's, `STK.BOM.01`; until it is built nobody provides it
//!   and a work order cannot be released);
//! - [`OrderLines`]: the sales order line a work order serves (wl-sales);
//! - [`ProductionStock`]: the stock movements of a work order that must post
//!   inside the transaction of the transition that causes them (a `return`
//!   of unused materials and `wastage` at an abort, the `produce` of a
//!   production report). The material issue itself is the consumption of
//!   [`crate::inventory::StockService::consume`].
//!
//! Every trait has a default that does nothing, and a `No...` type gives it
//! when the module is off. Quantities are thousandths of the base unit, money
//! is fils (AED minor units).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::parties::PartyId;
use crate::sales::SalesOrderId;
use crate::subject::Subject;

/// The subject type of a bill of materials version.
pub const BOM_SUBJECT_TYPE: &str = "inventory.bom";

/// The future the methods of this module return.
pub type ProductionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A component of a BOM version: what one unit of the output needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomComponent {
    /// The component item, `inventory.item:9`.
    pub item: Subject,
    /// The quantity for one unit of the output, in thousandths.
    pub quantity_milli: i64,
    /// The scrap allowance in basis points (500 = 5%).
    pub scrap_bps: i64,
}

/// A BOM version, as the work order reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BomVersion {
    /// The BOM version, `inventory.bom:3`.
    pub bom: Subject,
    /// The finished item it makes, `inventory.item:9`.
    pub item: Subject,
    /// The version number.
    pub version: i64,
    /// Whether it is `released` (a work order is made only from a released
    /// version, `MFG.ACL.06`).
    pub released: bool,
    /// The components of one unit.
    pub components: Vec<BomComponent>,
    /// The standard material cost of one unit at landed cost, in fils
    /// (`MFG.CST.01`).
    pub material_cost_per_unit: i64,
    /// The standard hours of one unit, in thousandths of an hour; they are
    /// priced at the hourly rate of the line.
    pub standard_milli_hours_per_unit: i64,
}

/// The bills of materials (wl-inventory §4 *BOM*). Nobody provides it yet:
/// [`boms`] then gives [`NoBoms`], which knows no version.
pub trait BillsOfMaterials: Send + Sync + 'static {
    /// The BOM version `bom` (`inventory.bom:3`); `None` for an unknown one.
    fn version<'a>(
        &'a self,
        _state: &'a AppState,
        _bom: &'a Subject,
    ) -> ProductionFuture<'a, Option<BomVersion>> {
        Box::pin(async { Ok(None) })
    }
}

/// The BOMs when nothing provides them.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoBoms;

impl BillsOfMaterials for NoBoms {}

/// Makes `boms` the app's [`BillsOfMaterials`].
pub fn provide_boms(app: &mut Registry, boms: impl BillsOfMaterials) {
    let boms: Arc<dyn BillsOfMaterials> = Arc::new(boms);
    app.provide(boms);
}

/// The app's [`BillsOfMaterials`], or [`NoBoms`] when none is provided.
pub fn boms(state: &AppState) -> Arc<dyn BillsOfMaterials> {
    match state.provided::<Arc<dyn BillsOfMaterials>>() {
        Some(boms) => Arc::clone(&boms),
        None => Arc::new(NoBoms),
    }
}

/// A sales order line, as a work order reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLineInfo {
    /// The order.
    pub order: SalesOrderId,
    /// The sale line (its id in `wl-sales`).
    pub line: i64,
    /// The number of the order, `SO-2026-0050`.
    pub number: Option<String>,
    /// The user who owns the order (the sales person), if it names one.
    pub owner: Option<i64>,
    /// The quantity of the line still open, in thousandths.
    pub open_milli: i64,
    /// Whether the line (or the whole order) was cancelled.
    pub cancelled: bool,
    /// The customer of the order (07 recall trace: a work order for an
    /// undelivered order belongs to that order's customer). `None` when the
    /// provider does not say.
    pub customer: Option<PartyId>,
}

/// The sales order lines a work order serves (wl-sales §6). Nobody provides
/// it until the sales side is reconciled: [`order_lines`] then gives
/// [`NoOrderLines`], which knows no line (a work order then treats its line
/// as open and has no owner to tell).
pub trait OrderLines: Send + Sync + 'static {
    /// Line `line` of `order`; `None` when unknown.
    fn line<'a>(
        &'a self,
        _state: &'a AppState,
        _order: SalesOrderId,
        _line: i64,
    ) -> ProductionFuture<'a, Option<OrderLineInfo>> {
        Box::pin(async { Ok(None) })
    }
}

/// The order lines when `sales` is off.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOrderLines;

impl OrderLines for NoOrderLines {}

/// Makes `lines` the app's [`OrderLines`].
pub fn provide_order_lines(app: &mut Registry, lines: impl OrderLines) {
    let lines: Arc<dyn OrderLines> = Arc::new(lines);
    app.provide(lines);
}

/// The app's [`OrderLines`], or [`NoOrderLines`] when none is provided.
pub fn order_lines(state: &AppState) -> Arc<dyn OrderLines> {
    match state.provided::<Arc<dyn OrderLines>>() {
        Some(lines) => Arc::clone(&lines),
        None => Arc::new(NoOrderLines),
    }
}

/// Unused material going back to the store of a work order (a `return_in`
/// movement).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnMaterial {
    /// The work order, `manufacturing.work_order:5`.
    pub source: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot, `inventory.lot:4`, for a lot-tracked item.
    pub lot: Option<Subject>,
    /// The store it goes back to, `inventory.location:2`.
    pub location: Subject,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The cost of one base unit it was issued at, in fils.
    pub unit_cost: i64,
    /// The user who caused it; `None` for the system.
    pub user: Option<i64>,
}

/// Material lost with a work order (a `wastage` movement out of the work
/// in progress).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WasteMaterial {
    /// The work order, `manufacturing.work_order:5`.
    pub source: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot, `inventory.lot:4`, for a lot-tracked item.
    pub lot: Option<Subject>,
    /// The quantity scrapped, in thousandths.
    pub quantity_milli: i64,
    /// The value written off, in fils.
    pub value: i64,
    /// Why (the reason of the abort).
    pub reason: String,
    /// The user who caused it; `None` for the system.
    pub user: Option<i64>,
}

/// Good output of a work order (a `produce` movement into its output lot).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProduceOutput {
    /// The work order, `manufacturing.work_order:5`.
    pub source: Subject,
    /// The finished item, `inventory.item:9`.
    pub item: Subject,
    /// Where the finished goods go, `inventory.location:2`.
    pub location: Subject,
    /// The output lot of the work order when it has one already.
    pub lot: Option<Subject>,
    /// The number of the lot when it has to be opened (in `quarantine`).
    pub lot_number: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The cost of one unit, in fils.
    pub unit_cost: i64,
    /// The user who reported it.
    pub user: Option<i64>,
}

/// A production report's good output taken out of the output lot again
/// (a void, BL-MFG-04): the reversing movement of its `produce`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoidOutput {
    /// The work order, `manufacturing.work_order:5`.
    pub source: Subject,
    /// The finished item, `inventory.item:9`.
    pub item: Subject,
    /// The output lot, `inventory.lot:4`, for a lot-tracked item.
    pub lot: Option<Subject>,
    /// Where the finished goods were put, `inventory.location:2`.
    pub location: Subject,
    /// The good quantity of the report, in thousandths.
    pub quantity_milli: i64,
    /// The cost of one unit the report posted it at, in fils.
    pub unit_cost: i64,
    /// Why, for the movement: the report and the reason of the void.
    pub reason: String,
    /// The user who voided; `None` for the system.
    pub user: Option<i64>,
}

/// The output lot of a work order set aside for a line of the sales order it
/// serves (`reserve_output`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReserveOutput {
    /// The work order, `manufacturing.work_order:5`.
    pub source: Subject,
    /// The finished item, `inventory.item:9`.
    pub item: Subject,
    /// The output lot, `inventory.lot:4`.
    pub lot: Subject,
    /// Where the finished goods are, `inventory.location:2`.
    pub location: Subject,
    /// The sales order the work order serves.
    pub order: SalesOrderId,
    /// The sale line (its id in `wl-sales`).
    pub line: i64,
    /// The quantity to hold for the line, in thousandths.
    pub quantity_milli: i64,
}

/// The stock movements of a work order, posted inside the caller's
/// transaction (07 §7.1 *Work order*: `abort` posts them in one transaction
/// at the final approval; a production report posts its `produce`).
/// `wl-inventory` provides it; without that module [`production_stock`]
/// gives [`NoProductionStock`], which posts nothing.
pub trait ProductionStock: Send + Sync + 'static {
    /// Posts the return of unused material to the store.
    fn return_material<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _request: ReturnMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Posts a `wastage` of material.
    fn waste_material<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _request: WasteMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Posts the good output into the output lot, opening the lot in
    /// `quarantine` at the first report. Returns the lot (`None` when the
    /// module is off).
    fn produce<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _request: ProduceOutput,
    ) -> ProductionFuture<'a, Option<Subject>> {
        Box::pin(async { Ok(None) })
    }

    /// The quantity of the output lot `lot` (of `item` when it has none) still
    /// at `location`, in thousandths, that nothing has reserved: what a void
    /// of a production report can take back. `None` when the module is off
    /// and nothing can be told.
    fn output_available<'a>(
        &'a self,
        _state: &'a AppState,
        _item: &'a Subject,
        _lot: Option<&'a Subject>,
        _location: &'a Subject,
    ) -> ProductionFuture<'a, Option<i64>> {
        Box::pin(async { Ok(None) })
    }

    /// Takes the good output of a voided production report out of the
    /// output lot with a reversing movement. A 422 ("output already used")
    /// when the lot no longer holds the quantity: part of it was moved,
    /// issued or picked.
    fn void_output<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _request: VoidOutput,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Reverses the `wastage` movement of the scrap of a voided production
    /// report; `request` is the one the `wastage` was posted with.
    fn void_waste<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _request: WasteMaterial,
    ) -> ProductionFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    /// Sets the output lot of a work order aside for `line` of the sales
    /// order, for exactly `request.quantity_milli` (a smaller or larger
    /// quantity changes the reservation made before; 0 gives it back).
    /// Returns whether a reservation is active afterwards: `false` when the
    /// module is off.
    fn reserve_output<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _request: ReserveOutput,
    ) -> ProductionFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// Frees the output lot `lot` from the reservation for `line` of the
    /// sales order (`unlink`, `MFG.RUL.09`, or the abort of the work order).
    /// Returns whether a reservation was released: `false` when there was
    /// none (or the module is off).
    fn release_reservation<'a>(
        &'a self,
        _tx: &'a mut Transaction,
        _lot: &'a Subject,
        _order: SalesOrderId,
        _line: i64,
    ) -> ProductionFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The production stock when `inventory` is off: nothing is posted.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoProductionStock;

impl ProductionStock for NoProductionStock {}

/// Makes `stock` the app's [`ProductionStock`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_production_stock(app: &mut Registry, stock: impl ProductionStock) {
    let stock: Arc<dyn ProductionStock> = Arc::new(stock);
    app.provide(stock);
}

/// The app's [`ProductionStock`], or [`NoProductionStock`] when none is
/// provided.
pub fn production_stock(state: &AppState) -> Arc<dyn ProductionStock> {
    match state.provided::<Arc<dyn ProductionStock>>() {
        Some(stock) => Arc::clone(&stock),
        None => Arc::new(NoProductionStock),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bom_version_round_trips_as_json() {
        let item = Subject::new("inventory.item", 9);
        let version = BomVersion {
            bom: Subject::new(BOM_SUBJECT_TYPE, 3),
            item,
            version: 1,
            released: true,
            components: Vec::new(),
            material_cost_per_unit: 1_800,
            standard_milli_hours_per_unit: 50,
        };
        let json = renox::serde_json::to_value(&version).unwrap();
        assert_eq!(json["material_cost_per_unit"], 1_800);
        assert_eq!(
            renox::serde_json::from_value::<BomVersion>(json).unwrap(),
            version
        );
    }
}
