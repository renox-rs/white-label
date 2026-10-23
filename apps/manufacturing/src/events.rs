//! The events the vertical emits (07 Manufacturing §7.1). Other modules and
//! the issues that build on the base listen to them; each is emitted once,
//! after the commit of the transition it names.

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{PartyId, Subject};

/// A work order was `release`d.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOrderReleased {
    /// The work order, `manufacturing.work_order:4`.
    pub work_order: Subject,
    /// The line it is scheduled on, `manufacturing.line:2`.
    pub line: Subject,
}

impl Event for WorkOrderReleased {}

/// A work order was `complete`d: its output lot takes no more output and a
/// final inspection is due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOrderCompleted {
    /// The work order.
    pub work_order: Subject,
    /// The output lot, `inventory.lot:4`, when the work order has one.
    pub output_lot: Option<Subject>,
    /// The good quantity, in thousandths.
    pub good_milli: i64,
    /// The scrap, in thousandths.
    pub scrap_milli: i64,
}

impl Event for WorkOrderCompleted {}

/// A work order was `cancel`led.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOrderCancelled {
    /// The work order.
    pub work_order: Subject,
    /// The reason, if one was given.
    pub reason: Option<String>,
}

impl Event for WorkOrderCancelled {}

/// A work order was `unlink`ed from its sales order: its output goes to free
/// stock (`MFG.RUL.09`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOrderUnlinked {
    /// The work order.
    pub work_order: Subject,
    /// The sales order it served, `sales.sales_order:50`.
    pub sales_order: Subject,
    /// The sales order line it served.
    pub sales_order_line: i64,
    /// Why.
    pub reason: String,
}

impl Event for WorkOrderUnlinked {}

/// A work order was `abort`ed after the approval of the production manager
/// (`MFG.ACL.12`); the return and wastage movements are posted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkOrderAborted {
    /// The work order.
    pub work_order: Subject,
    /// The reason given.
    pub reason: String,
    /// The value of the materials that went back to the store, in fils.
    pub returned: i64,
    /// The value scrapped, in fils.
    pub scrapped: i64,
    /// The output lot kept in quarantine, `inventory.lot:4`.
    pub output_lot: Option<Subject>,
    /// The loss, in fils; negative is a gain.
    pub loss: i64,
    /// The user who approved last.
    pub approver: Option<i64>,
}

impl Event for WorkOrderAborted {}

/// An NCR was raised (`MFG.QCL.02`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NcrRaised {
    /// The NCR, `manufacturing.ncr:5`.
    pub ncr: Subject,
    /// The lot, `inventory.lot:4`, if it is on one.
    pub lot: Option<Subject>,
    /// The item.
    pub item: Subject,
    /// The supplier of the lot at the time.
    pub supplier: Option<PartyId>,
    /// `supplier`, `internal_process` or `unknown`.
    pub cause: String,
    /// `incoming`, `dispute`, `production` or `other`.
    pub origin: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
}

impl Event for NcrRaised {}

/// QC decided what becomes of an NCR and the action was set going: the
/// rework work order and the draft Return to supplier exist, or the scrap
/// posted, or the lot was released.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NcrDispositioned {
    /// The NCR.
    pub ncr: Subject,
    /// `rework`, `scrap`, `use_as_is` or `return_to_supplier`.
    pub disposition: String,
    /// For a return to supplier: `credit` or `replace`.
    pub outcome: Option<String>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The draft Return to supplier, `inventory.supplier_return:4`.
    pub return_to_supplier: Option<Subject>,
    /// The rework work order.
    pub rework_work_order: Option<Subject>,
}

impl Event for NcrDispositioned {}

/// An NCR was closed by QC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NcrClosed {
    /// The NCR.
    pub ncr: Subject,
    /// The disposition it had.
    pub disposition: Option<String>,
}

impl Event for NcrClosed {}

/// An NCR was cancelled: raised in error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NcrCancelled {
    /// The NCR.
    pub ncr: Subject,
    /// The supplier it was raised against, if any.
    pub supplier: Option<PartyId>,
    /// The reason.
    pub reason: String,
}

impl Event for NcrCancelled {}

/// An NCR was reopened for a new disposition: the action of the earlier one
/// did not happen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NcrReopened {
    /// The NCR.
    pub ncr: Subject,
    /// The reason.
    pub reason: String,
}

impl Event for NcrReopened {}
