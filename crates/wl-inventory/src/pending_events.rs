//! `PendingIssueClosed` (the kernel's, wl-inventory §3 *Pending issue*): when
//! a pending issue leaves `open` (it posted against arriving stock, the
//! storekeeper cleared it, or its source was cancelled) the record that
//! consumed is told what became of its short part, after the commit, and how
//! many pending issues of the same source are still open. A work order uses
//! it to complete the row of the issue and to ask its supervisor to resume
//! (BL-MFG-05).
//!
//! It is announced from the `Transitioned<PendingIssue>` of every path
//! (`post`, `clear`, `clear_approved`, `source_cancelled`), so no path is
//! forgotten. The cost is the one of the `consume` movement that posted.

use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::inventory::{LOCATION_SUBJECT_TYPE, LOT_SUBJECT_TYPE, PendingIssueClosed};

use crate::item::Item;
use crate::movement::{MovementType, StockMovement};
use crate::pending_issue::{PendingIssue, PendingStatus};

/// The kernel event of pending issue `id`, which has just posted or been
/// cleared; `None` while it is open.
///
/// # Errors
///
/// A database error.
pub async fn closed_event(db: &Db, id: i64) -> Result<Option<PendingIssueClosed>> {
    let pending = PendingIssue::find_or_404(db, id).await?;
    let how = match (pending.status, pending.resolved_by.as_deref()) {
        (PendingStatus::Open, _) => return Ok(None),
        (PendingStatus::Posted, _) => "posted",
        (PendingStatus::Cleared, Some("source_cancelled")) => "source_cancelled",
        (PendingStatus::Cleared, _) => "cleared",
    };
    let unit_cost = if how == "source_cancelled" {
        0
    } else {
        StockMovement::query()
            .where_(StockMovement::MOVEMENT_TYPE.eq(MovementType::Consume))
            .where_(StockMovement::SOURCE_TYPE.eq(pending.source_type.as_str()))
            .where_(StockMovement::SOURCE_ID.eq(pending.source_id))
            .where_(StockMovement::ITEM_ID.eq(pending.item_id))
            .where_(StockMovement::QUANTITY_MILLI.eq(pending.quantity_milli))
            .order_by_desc(StockMovement::ID)
            .first(db)
            .await?
            .map_or_else(|| Ok(0), |movement| Ok::<i64, Error>(movement.unit_cost))?
    };
    let open_for_source = PendingIssue::query()
        .where_(PendingIssue::SOURCE_TYPE.eq(pending.source_type.as_str()))
        .where_(PendingIssue::SOURCE_ID.eq(pending.source_id))
        .where_(PendingIssue::STATUS.eq(PendingStatus::Open))
        .count(db)
        .await?;
    let original = pending.split_from_id.unwrap_or(pending.id);
    let item = Item::find_or_404(db, pending.item_id).await?;
    Ok(Some(PendingIssueClosed {
        pending_issue: pending.subject(),
        original: Subject::new(wl_kernel::inventory::PENDING_ISSUE_SUBJECT_TYPE, original),
        source: pending.source(),
        item: item.subject(),
        location: Subject::new(LOCATION_SUBJECT_TYPE, pending.location_id),
        how: how.to_owned(),
        lot: pending
            .lot_id
            .map(|lot| Subject::new(LOT_SUBJECT_TYPE, lot)),
        quantity_milli: pending.quantity_milli,
        unit_cost,
        open_for_source: i64::try_from(open_for_source).unwrap_or(i64::MAX),
    }))
}

/// Announces `PendingIssueClosed` after every transition that closes a
/// pending issue.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: Transitioned<PendingIssue>, state: AppState| async move {
            if let Some(closed) = closed_event(&state.db, event.key).await? {
                state.emit(closed).await?;
            }
            Ok(())
        },
    );
}
