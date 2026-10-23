//! What a vertical does with a lot that failed (07 Manufacturing §7.1 *NCR*,
//! `MFG.QCL.02`), without depending on `wl-inventory`: the facts of a lot,
//! the Return to supplier an NCR creates, the scrap and the release, the
//! lot's block while an NCR is open, and the lots behind a dispatch line.
//!
//! `wl-inventory` provides [`LotDisposal`] and [`DispatchLots`] (with
//! [`provide_lot_disposal`], [`provide_dispatch_lots`]); the vertical
//! provides [`LotReleaseGuard`]. Without a provider the accessors give a
//! `No...` implementation that knows no lot, creates nothing and blocks
//! nothing, so a vertical runs (and is tested) without the module.
//!
//! [`SupplierReturnCancelled`] is the event `wl-inventory` emits when a draft
//! Return to supplier is cancelled (the supplier refuses collection): an NCR
//! it came from is reopened.
//!
//! Quantities are thousandths of the item's base unit, money is fils.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::inventory::ReturnOutcome;
use crate::parties::PartyId;
use crate::subject::Subject;

/// The future a method of this module returns.
pub type LotFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// What an NCR needs to know about a lot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotFacts {
    /// The lot, `inventory.lot:4`.
    pub lot: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot number, as printed on the pack.
    pub number: String,
    /// The supplier's lot number captured at goods receipt; `None` when the
    /// lot does not come from a supplier's delivery.
    pub supplier_lot: Option<String>,
    /// `quarantine`, `released`, `on_hold`, `expired`, `rejected` or
    /// `depleted`.
    pub status: String,
    /// The supplier of the goods receipt the lot came from.
    pub supplier: Option<PartyId>,
    /// The goods receipt it came from, `inventory.goods_receipt:5`.
    pub receipt: Option<Subject>,
    /// The record that produced the lot (a work order) when it was made
    /// in-house.
    pub produced_by: Option<Subject>,
    /// What is left of the lot in all locations.
    pub quantity_milli: i64,
    /// The location most of it is at, `inventory.location:2`.
    pub location: Option<Subject>,
    /// What is left at that location.
    pub location_milli: i64,
    /// The cost of one base unit: the goods receipt's, else the item's
    /// average cost (a Return to supplier is valued at it).
    pub unit_cost: i64,
    /// The item's average cost of one base unit, which a write-off posts at.
    pub average_cost: i64,
    /// The purchase order line of the receipt.
    pub po_line: Option<i64>,
}

/// A Return to supplier an NCR asks for: a draft from the lot's location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnLot {
    /// The NCR, `manufacturing.ncr:5`; the source of the return.
    pub ncr: Subject,
    /// The lot.
    pub lot: Subject,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// Credit or replacement.
    pub outcome: ReturnOutcome,
    /// Why: the NCR's defect.
    pub reason: String,
    /// The user who dispositioned the NCR.
    pub user: Option<i64>,
}

/// A quantity of a lot an NCR scraps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrapLot {
    /// The NCR; the source of the movement.
    pub ncr: Subject,
    /// The lot.
    pub lot: Subject,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// Why.
    pub reason: String,
    /// The user who dispositioned the NCR; `None` for the system.
    pub user: Option<i64>,
}

/// What the scrap did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scrapped {
    /// The value written off, in fils.
    pub value: i64,
    /// Whether the lot itself was discarded (the whole remaining quantity of
    /// a held or expired lot).
    pub discarded: bool,
}

/// The lot operations an NCR uses. `wl-inventory` provides them.
pub trait LotDisposal: Send + Sync + 'static {
    /// The facts of `lot`; `None` for an unknown lot and by default.
    fn facts<'a>(
        &'a self,
        _state: &'a AppState,
        _lot: &'a Subject,
    ) -> LotFuture<'a, Option<LotFacts>> {
        Box::pin(async { Ok(None) })
    }

    /// Creates the draft Return to supplier of an NCR, from the location the
    /// lot is at, for the lot's supplier. Returns the return,
    /// `inventory.supplier_return:4`; `None` when the module is off. A 422
    /// when the lot has no supplier lot or the quantity is not at the
    /// location.
    fn create_return<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ReturnLot,
    ) -> LotFuture<'a, Option<Subject>> {
        Box::pin(async { Ok(None) })
    }

    /// Where a Return to supplier stands: `draft`, `dispatched`, `credited`
    /// or `cancelled`; `None` for an unknown one.
    fn return_status<'a>(
        &'a self,
        _state: &'a AppState,
        _supplier_return: &'a Subject,
    ) -> LotFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }

    /// Writes the quantity off: the lot `discard` when it is held or expired
    /// and the quantity is all that is left, else a `wastage` movement of the
    /// quantity from the lot (`STK.ACL.02` was approved by the caller).
    /// `None` when the module is off.
    fn scrap<'a>(
        &'a self,
        _state: &'a AppState,
        _request: ScrapLot,
    ) -> LotFuture<'a, Option<Scrapped>> {
        Box::pin(async { Ok(None) })
    }

    /// Releases the whole lot (an NCR *use as is*, approved). Returns
    /// whether the lot is released now.
    fn release<'a>(&'a self, _state: &'a AppState, _lot: &'a Subject) -> LotFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }

    /// A return took the last quantity of the lot: a lot in quarantine or on
    /// hold with nothing left is `reject`ed. Returns whether it was.
    fn close_out<'a>(&'a self, _state: &'a AppState, _lot: &'a Subject) -> LotFuture<'a, bool> {
        Box::pin(async { Ok(false) })
    }
}

/// The lot operations when `inventory` is off: no lot is known.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLotDisposal;

impl LotDisposal for NoLotDisposal {}

/// Makes `service` the app's [`LotDisposal`]; `wl-inventory` calls it in
/// `register`.
pub fn provide_lot_disposal(app: &mut Registry, service: impl LotDisposal) {
    let service: Arc<dyn LotDisposal> = Arc::new(service);
    app.provide(service);
}

/// The app's [`LotDisposal`], or [`NoLotDisposal`] when none is provided.
pub fn lot_disposal(state: &AppState) -> Arc<dyn LotDisposal> {
    match state.provided::<Arc<dyn LotDisposal>>() {
        Some(service) => Arc::clone(&service),
        None => Arc::new(NoLotDisposal),
    }
}

/// Whether something outside stock keeps a lot from being released: 07 keeps a
/// lot with an NCR `open` or `dispositioned` (§7.1 *NCR*). A vertical provides
/// it with [`provide_lot_release_guard`]; `wl-inventory` asks before it
/// releases a lot ([`lot_release_blocked`]).
pub trait LotReleaseGuard: Send + Sync + 'static {
    /// Why `lot` may not be released now, in words; `None` when it may.
    fn blocked<'a>(&'a self, _db: &'a Db, _lot: &'a Subject) -> LotFuture<'a, Option<String>> {
        Box::pin(async { Ok(None) })
    }
}

/// The guard when no vertical provides one: nothing blocks a release.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLotReleaseGuard;

impl LotReleaseGuard for NoLotReleaseGuard {}

/// Makes `guard` the app's [`LotReleaseGuard`].
pub fn provide_lot_release_guard(app: &mut Registry, guard: impl LotReleaseGuard) {
    let guard: Arc<dyn LotReleaseGuard> = Arc::new(guard);
    app.provide(guard);
}

/// Why `lot` may not be released now, if the app has a guard that says so.
///
/// # Errors
///
/// The guard's errors.
pub async fn lot_release_blocked(state: &AppState, lot: &Subject) -> Result<Option<String>> {
    match state.provided::<Arc<dyn LotReleaseGuard>>() {
        Some(guard) => guard.blocked(&state.db, lot).await,
        None => Ok(None),
    }
}

/// A lot (or an item with no lot) behind a dispatch line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchedLot {
    /// The dispatch line, `inventory.dispatch_line:4`.
    pub line: Subject,
    /// The item, `inventory.item:9`.
    pub item: Subject,
    /// The lot, `inventory.lot:4`; `None` for an item that is not
    /// lot-tracked.
    pub lot: Option<Subject>,
    /// The quantity sent, in thousandths.
    pub quantity_milli: i64,
}

/// The lots a dispatch sent (`MFG.RUL.04`: an invoice line is drafted from a
/// dispatch line, and the dispatch line records its lots). `wl-inventory`
/// provides it.
pub trait DispatchLots: Send + Sync + 'static {
    /// The lots behind `origin`: a dispatch line (its own lot) or a dispatch
    /// (the lots of all its lines). Empty for any other record.
    fn lots_of<'a>(
        &'a self,
        _state: &'a AppState,
        _origin: &'a Subject,
    ) -> LotFuture<'a, Vec<DispatchedLot>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

/// The dispatch lots when `inventory` is off: none.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDispatchLots;

impl DispatchLots for NoDispatchLots {}

/// Makes `lots` the app's [`DispatchLots`].
pub fn provide_dispatch_lots(app: &mut Registry, lots: impl DispatchLots) {
    let lots: Arc<dyn DispatchLots> = Arc::new(lots);
    app.provide(lots);
}

/// The app's [`DispatchLots`], or [`NoDispatchLots`] when none is provided.
pub fn dispatch_lots(state: &AppState) -> Arc<dyn DispatchLots> {
    match state.provided::<Arc<dyn DispatchLots>>() {
        Some(lots) => Arc::clone(&lots),
        None => Arc::new(NoDispatchLots),
    }
}

/// A draft Return to supplier was cancelled (wl-inventory §4 *Return to
/// supplier* `cancel`): the supplier refused collection, or the user changed
/// their mind. The record it came from (an NCR) learns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplierReturnCancelled {
    /// The return, `inventory.supplier_return:4`.
    pub supplier_return: Subject,
    /// The supplier.
    pub supplier: PartyId,
    /// The record the goods came from: the goods receipt, an NCR...
    pub source: Subject,
    /// Why it was cancelled.
    pub reason: Option<String>,
}

impl Event for SupplierReturnCancelled {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cancelled_event_round_trips_through_json() {
        let event = SupplierReturnCancelled {
            supplier_return: Subject::new("inventory.supplier_return", 4),
            supplier: PartyId(7),
            source: Subject::new("manufacturing.ncr", 5),
            reason: Some("refused collection".into()),
        };
        let json = renox::serde_json::to_string(&event).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<SupplierReturnCancelled>(&json).unwrap(),
            event
        );
    }
}
