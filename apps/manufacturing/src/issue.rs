//! Material issued to a work order (07 Manufacturing §4.2 step 5, §7.1): the
//! storekeeper issues a material to a `released`, `in_progress` or `on_hold`
//! work order; the work order posts it as a consumption through the kernel's
//! [`StockService`](wl_kernel::inventory::StockService), so a short quantity
//! waits as a pending issue and the business event is never blocked.
//!
//! Every lot that posted becomes a [`MaterialIssue`] with the unit cost it was
//! posted with (an abort values what goes back or is scrapped from them); the
//! short part is a row with its pending issue and no cost, which the cost and
//! the abort leave out until it posts (it comes with BL-MFG-05).
//!
//! Decisions: with no stock module (nothing posts, nothing is short) the
//! quantity is recorded as issued at no cost. The lot of a lot-tracked
//! material is required by BL-MFG-09, not here. While an abort waits, new
//! issues are refused (*abort pending*).

use renox::db::{DateTime, Executor, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::{ConsumeRequest, ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE, stock_service};

use crate::abort;
use crate::db::{lock_order, refused, set_order};
use crate::permissions;
use crate::plant::Line;
use crate::texts::english;
use crate::work_order::{WorkOrder, WorkOrderStatus};

/// A material issued to a work order, one row per lot.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_material_issues", index(work_order_id))]
pub struct MaterialIssue {
    /// The primary key.
    pub id: i64,
    /// The work order.
    pub work_order_id: i64,
    /// The material, a wl-inventory item.
    pub item_id: i64,
    /// The lot, for a lot-tracked material.
    pub lot_id: Option<i64>,
    /// The store it came from, a wl-inventory location.
    pub location_id: i64,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The cost of one base unit it was posted at, in fils.
    pub unit_cost: i64,
    /// What went back to the store at an abort, in thousandths.
    pub returned_milli: i64,
    /// The pending issue that holds the short part.
    pub pending_issue_id: Option<i64>,
    /// The user who issued it.
    pub issued_by: Option<i64>,
    /// When it was issued.
    pub issued_at: DateTime,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl MaterialIssue {
    /// The issues of work order `id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_order<'c>(db: impl Executor<'c>, id: i64) -> Result<Vec<MaterialIssue>> {
        MaterialIssue::query()
            .where_(MaterialIssue::WORK_ORDER_ID.eq(id))
            .order_by(MaterialIssue::ID)
            .get(db)
            .await
    }

    /// The item as a subject.
    pub fn item(&self) -> Subject {
        Subject::new(ITEM_SUBJECT_TYPE, self.item_id)
    }

    /// The lot as a subject.
    pub fn lot(&self) -> Option<Subject> {
        self.lot_id.map(|id| Subject::new(LOT_SUBJECT_TYPE, id))
    }
}

/// What the storekeeper issues.
#[derive(Debug, Clone)]
pub struct IssueAsk {
    /// The material.
    pub item_id: i64,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The lot to take it from first.
    pub lot_id: Option<i64>,
}

/// `by` issues a material to work order `id` from the store of its line.
///
/// # Errors
///
/// `Forbidden` without `work_orders.issue` for the work order; a 422 unless
/// the work order is `released`, `in_progress` or `on_hold`, while an abort
/// waits, for a quantity that is not above zero or a line without a store.
pub async fn issue(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    ask: IssueAsk,
) -> Result<Vec<MaterialIssue>> {
    let order = WorkOrder::find_or_404(&state.db, id).await?;
    if !order.may(by, permissions::WORK_ORDERS_ISSUE) {
        return Err(Error::Forbidden);
    }
    if !matches!(
        order.status,
        WorkOrderStatus::Released | WorkOrderStatus::InProgress | WorkOrderStatus::OnHold
    ) {
        return Err(refused(english("manufacturing.issue.state", &[])));
    }
    if abort::is_pending(state, &order).await? {
        return Err(refused(english("manufacturing.abort.pending", &[])));
    }
    if ask.quantity_milli <= 0 {
        return Err(refused(english("manufacturing.issue.quantity", &[])));
    }
    let line = match order.line_id {
        Some(line) => Line::find(&state.db, line).await?,
        None => None,
    };
    let store = line
        .and_then(|line| line.store())
        .ok_or_else(|| refused(english("manufacturing.issue.no_store", &[])))?;
    let mut request = ConsumeRequest::new(
        Subject::new(ITEM_SUBJECT_TYPE, ask.item_id),
        store.clone(),
        ask.quantity_milli,
        order.subject(),
    )
    .cost_object(order.subject());
    if let Some(lot) = ask.lot_id {
        request = request.lot(Subject::new(LOT_SUBJECT_TYPE, lot));
    }
    let consumed = stock_service(state).consume(state, request).await?;
    let mut tx = state.db.begin().await?;
    // The state read in the transaction: an abort asked meanwhile wins.
    lock_order(&mut tx, id).await?;
    let at = now();
    let mut rows = Vec::new();
    for lot in &consumed.lots {
        rows.push(MaterialIssue {
            work_order_id: id,
            item_id: ask.item_id,
            lot_id: lot.lot.as_ref().map(|l| l.subject_id),
            location_id: store.subject_id,
            quantity_milli: lot.quantity_milli,
            unit_cost: lot.unit_cost,
            issued_by: Some(by.id),
            issued_at: at,
            ..MaterialIssue::default()
        });
    }
    if consumed.lots.is_empty() && consumed.posted_milli == 0 && consumed.short_milli == 0 {
        rows.push(MaterialIssue {
            work_order_id: id,
            item_id: ask.item_id,
            location_id: store.subject_id,
            quantity_milli: ask.quantity_milli,
            issued_by: Some(by.id),
            issued_at: at,
            ..MaterialIssue::default()
        });
    }
    if consumed.short_milli > 0 {
        rows.push(MaterialIssue {
            work_order_id: id,
            item_id: ask.item_id,
            location_id: store.subject_id,
            quantity_milli: consumed.short_milli,
            pending_issue_id: consumed.pending_issue.as_ref().map(|p| p.subject_id),
            issued_by: Some(by.id),
            issued_at: at,
            ..MaterialIssue::default()
        });
    }
    let mut saved = Vec::new();
    for row in rows {
        saved.push(MaterialIssue::create(&mut tx, row).await?);
    }
    if saved.iter().any(|row| row.pending_issue_id.is_none()) {
        set_order(&mut tx, id, &[("has_issue", &true)]).await?;
    }
    tx.commit().await?;
    Ok(saved)
}
