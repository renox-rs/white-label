//! Holding a work order for material when an issue is pending (07
//! Manufacturing §4.2 "When it goes wrong" last row, §7.1 *Work order*
//! `hold` and `resume`; BL-MFG-05).
//!
//! In this vertical a store never goes below zero: the short part of a
//! material issue waits as a pending issue of wl-inventory. The system never
//! holds or resumes a work order by itself; it asks the line's supervisor
//! with a task, one per work order:
//!
//! - **Hold task.** When a pending issue is recorded for an `in_progress`
//!   work order ([`StockNegative`]), the supervisor of its line gets the task
//!   *Hold WO for material* (the reason *material* is the one the hold sheet
//!   offers first). A `released` work order is not held: the pending issue
//!   shows on its page and on the schedule board. The task closes when the
//!   work order goes `on_hold` (any reason) or stops, or when its last
//!   pending issue posts or is cleared while it is still `in_progress`.
//! - **Resume task.** When no pending issue of the work order is `open` any
//!   more (each `posted` or `cleared`, [`PendingIssueClosed`]) and the work
//!   order is `on_hold` with the reason *material*, the supervisor gets the
//!   task *Resume WO*; a work order on hold for another reason (machine,
//!   quality) gets none. It closes when the work order resumes or stops.
//! - **The issue rows.** The short part of an issue is a row with its
//!   pending issue and no lot or cost. When the pending issue posts (or the
//!   storekeeper clears it) the row takes the lot and the cost it posted
//!   with, so the cost of the work order and a recall trace see it; a
//!   partly covered one is split in two rows. A pending issue that was
//!   cleared because the work order was cancelled never became an issue:
//!   its row goes.
//!
//! Decisions: the supervisor is found in the scope of the line, else the
//! kernel's pick for the supervisor role; a work order started after the
//! pending issue was recorded gets no hold task (the pending issue shows on
//! its page).

use renox::auth::permissions::users_with_role_in;
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::inventory::{PendingIssueClosed, StockNegative};
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};

use crate::permissions;
use crate::texts::english;
use crate::work_order::{HOLD, RESUME, SUBJECT_TYPE, WorkOrder, WorkOrderStatus};

/// The reason a work order is held for material.
pub const MATERIAL: &str = "material";

/// The source key of the hold task of work order `id`.
pub fn hold_key(id: i64) -> String {
    format!("manufacturing.hold_for_material:{id}")
}

/// The source key of the resume task of work order `id`.
pub fn resume_key(id: i64) -> String {
    format!("manufacturing.resume_for_material:{id}")
}

/// The supervisor of the line of `order`: a holder of the role in the line's
/// scope, else whom the kernel picks for the role.
async fn supervisor(state: &AppState, order: &WorkOrder) -> Result<Option<i64>> {
    if let Some(line) = order.line_id
        && let Some(user) = users_with_role_in(
            &state.db,
            permissions::SUPERVISOR_ROLE,
            &permissions::line_scope(line),
        )
        .await?
        .first()
    {
        return Ok(Some(user.id));
    }
    role_tasks::assignee(&state.db, permissions::SUPERVISOR_ROLE).await
}

/// Opens the task `key` for the supervisor of `order` unless it is open
/// already (one task per work order).
async fn open_task(
    state: &AppState,
    order: &WorkOrder,
    key: String,
    title: String,
    description: String,
) -> Result<bool> {
    let creator = task_creator(state);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(false);
    }
    let mut ticket = OpenTicket::new(order.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&key);
    if let Some(user) = supervisor(state, order).await? {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    Ok(creator.open(state, ticket).await?.is_some())
}

/// Closes the open task `key`, if there is one.
async fn close_task(state: &AppState, key: &str, note: &str) -> Result<bool> {
    let creator = task_creator(state);
    match creator.find_open(state, key).await? {
        Some(found) => creator.complete(state, found.id, note).await,
        None => Ok(false),
    }
}

/// A pending issue was recorded for `event.source`: an `in_progress` work
/// order gets its hold task.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_short(state: &AppState, event: &StockNegative) -> Result {
    let Some(source) = event.source.as_ref() else {
        return Ok(());
    };
    if source.subject_type != SUBJECT_TYPE || event.pending_issue.is_none() {
        return Ok(());
    }
    let Some(order) = WorkOrder::find(&state.db, source.subject_id).await? else {
        return Ok(());
    };
    if order.status != WorkOrderStatus::InProgress {
        return Ok(());
    }
    open_task(
        state,
        &order,
        hold_key(order.id),
        english(
            "manufacturing.material.hold_title",
            &[("order", &order.number)],
        ),
        english(
            "manufacturing.material.hold_body",
            &[("order", &order.number), ("reason", &MATERIAL)],
        ),
    )
    .await?;
    Ok(())
}

/// A pending issue of `event.source` posted or was cleared: its row takes
/// what posted, and when it was the last one open the tasks follow.
///
/// # Errors
///
/// The database's and the ticketing's errors.
pub async fn on_closed(state: &AppState, event: &PendingIssueClosed) -> Result {
    if event.source.subject_type != SUBJECT_TYPE {
        return Ok(());
    }
    let order_id = event.source.subject_id;
    crate::issue::settle_pending(state, order_id, event).await?;
    if event.open_for_source > 0 {
        return Ok(());
    }
    let Some(order) = WorkOrder::find(&state.db, order_id).await? else {
        return Ok(());
    };
    match order.status {
        WorkOrderStatus::OnHold if order.hold_reason.as_deref() == Some(MATERIAL) => {
            open_task(
                state,
                &order,
                resume_key(order.id),
                english(
                    "manufacturing.material.resume_title",
                    &[("order", &order.number)],
                ),
                english(
                    "manufacturing.material.resume_body",
                    &[("order", &order.number)],
                ),
            )
            .await?;
        }
        WorkOrderStatus::InProgress | WorkOrderStatus::Released => {
            close_task(
                state,
                &hold_key(order.id),
                &english("manufacturing.material.arrived", &[]),
            )
            .await?;
        }
        _ => {}
    }
    Ok(())
}

/// The work order changed: going `on_hold` closes the hold task, resuming
/// the resume task, and a work order that stopped closes both.
///
/// # Errors
///
/// The ticketing's errors.
pub async fn on_transition(state: &AppState, id: i64, transition: &str, to: &str) -> Result {
    let note = english("manufacturing.material.done", &[]);
    if transition == HOLD || to == WorkOrderStatus::OnHold.as_str() {
        close_task(state, &hold_key(id), &note).await?;
    }
    if transition == RESUME {
        close_task(state, &resume_key(id), &note).await?;
    }
    let stopped = [
        WorkOrderStatus::Completed,
        WorkOrderStatus::Closed,
        WorkOrderStatus::Cancelled,
        WorkOrderStatus::Aborted,
    ];
    if stopped.iter().any(|status| status.as_str() == to) {
        close_task(state, &hold_key(id), &note).await?;
        close_task(state, &resume_key(id), &note).await?;
    }
    Ok(())
}

/// The listeners.
pub(crate) fn listen(app: &mut Registry) {
    app.listen(
        |event: StockNegative, state: AppState| async move { on_short(&state, &event).await },
    );
    app.listen(|event: PendingIssueClosed, state: AppState| async move {
        on_closed(&state, &event).await
    });
    app.listen(
        |event: Transitioned<WorkOrder>, state: AppState| async move {
            on_transition(&state, event.key, &event.transition, &event.to).await
        },
    );
}
