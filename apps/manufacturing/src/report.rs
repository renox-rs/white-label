//! The production report (07 Manufacturing §6 *Production report*, §7.1): the
//! good quantity and the scrap a line reports for a work order that is
//! `in_progress` (`MFG.ACL.01`: the line's tablet and operators report only on
//! the work orders of their line). The good quantity is posted as a `produce`
//! movement into the work order's output lot, which the first report opens in
//! `quarantine` at the finished goods location of the line (the lot cannot be
//! picked or issued before QC releases it, `MFG.ACL.02`).
//!
//! The report machine declares both states, `posted` and `voided`; the `void`
//! with its reversing movements comes with BL-MFG-04, and so does the
//! `wastage` movement of a report's scrap (the scrap is kept on the report
//! and on the work order). While an abort waits, reports are refused (*abort
//! pending*).

use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine};
use serde::{Deserialize, Serialize};
use wl_kernel::production::{ProduceOutput, production_stock};

use crate::abort;
use crate::db::{lock_order, refused, set_order};
use crate::permissions;
use crate::plant::Line;
use crate::texts::english;
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// Where a production report stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ReportStatus {
    /// Counted in the totals of the work order.
    #[default]
    Posted,
    /// Entered on the wrong work order and taken back (terminal).
    Voided,
}

/// Output reported from a line.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_production_reports",
    state_machine,
    index(work_order_id)
)]
pub struct ProductionReport {
    /// The primary key.
    pub id: i64,
    /// The work order.
    pub work_order_id: i64,
    /// The good quantity, in thousandths.
    pub good_milli: i64,
    /// The scrap, in thousandths.
    pub scrap_milli: i64,
    /// The lifecycle state; changed only by a transition.
    pub status: ReportStatus,
    /// The user who reported.
    pub operator_id: Option<i64>,
    /// When the output was made (the business time).
    pub reported_at: DateTime,
    /// A note.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for ProductionReport {
    type State = ReportStatus;
    const COLUMN: renox::db::Col<ProductionReport, ReportStatus> = ProductionReport::STATUS;

    fn machine() -> Machine<Self> {
        Machine::new()
            .initial([ReportStatus::Posted])
            .state(ReportStatus::Posted)
            .tone("success")
            .state(ReportStatus::Voided)
            .tone("neutral")
            .terminal()
    }
}

/// Registers the machine with `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<ProductionReport>();
}

/// What the line reports.
#[derive(Debug, Clone, Default)]
pub struct ReportAsk {
    /// The good quantity, in thousandths.
    pub good_milli: i64,
    /// The scrap, in thousandths.
    pub scrap_milli: i64,
    /// A note.
    pub note: Option<String>,
}

/// `by` reports output on work order `id`.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report` in the plant or the line of the
/// work order (`MFG.ACL.01`); a 422 unless it is `in_progress`, while an
/// abort waits, for a quantity below zero or nothing reported, or a line
/// without a finished goods location.
pub async fn record(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: ReportAsk,
) -> Result<ProductionReport> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may(by, permissions::WORK_ORDERS_REPORT) {
        return Err(Error::Forbidden);
    }
    if order.status != WorkOrderStatus::InProgress {
        return Err(refused(english("manufacturing.report.state", &[])));
    }
    if abort::is_pending(state, &order).await? {
        return Err(refused(english("manufacturing.abort.pending", &[])));
    }
    if ask.good_milli < 0 || ask.scrap_milli < 0 || ask.good_milli + ask.scrap_milli == 0 {
        return Err(refused(english("manufacturing.report.quantity", &[])));
    }
    let finished = match order.line_id {
        Some(line) => Line::find(&state.db, line)
            .await?
            .and_then(|line| line.finished_goods()),
        None => None,
    };
    if ask.good_milli > 0 && finished.is_none() {
        return Err(refused(english("manufacturing.report.no_location", &[])));
    }
    let stock = production_stock(state);
    let mut tx = state.db.begin().await?;
    let order = lock_order(&mut tx, id).await?;
    if order.status != WorkOrderStatus::InProgress {
        return Err(refused(english("manufacturing.report.state", &[])));
    }
    let mut lot = order
        .output_lot_id
        .map(|lot| wl_kernel::Subject::new(wl_kernel::inventory::LOT_SUBJECT_TYPE, lot));
    if ask.good_milli > 0
        && let Some(location) = finished
    {
        let opened = stock
            .produce(
                &mut tx,
                ProduceOutput {
                    source: order.subject(),
                    item: order.item(),
                    location,
                    lot: lot.clone(),
                    lot_number: format!("F-{}", order.number),
                    quantity_milli: ask.good_milli,
                    unit_cost: order.std_unit_cost,
                    user: Some(by.id),
                },
            )
            .await?;
        if opened.is_some() {
            lot = opened;
        }
    }
    let mut report = ProductionReport {
        work_order_id: id,
        good_milli: ask.good_milli,
        scrap_milli: ask.scrap_milli,
        operator_id: Some(by.id),
        reported_at: renox::db::now(),
        note: ask.note.filter(|note| !note.trim().is_empty()),
        ..ProductionReport::default()
    };
    report.insert(&mut tx).await?;
    set_order(
        &mut tx,
        id,
        &[
            ("good_milli", &(order.good_milli + ask.good_milli)),
            ("scrap_milli", &(order.scrap_milli + ask.scrap_milli)),
            ("output_lot_id", &lot.map(|lot| lot.subject_id)),
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(report)
}
