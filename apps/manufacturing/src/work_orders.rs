//! The work order services: one function per action, each gathering what the
//! machine's guards need and running the transition (07 Manufacturing §7.1
//! *Work order*; Base 07). `abort` is in [`crate::abort`], `unlink` in
//! [`crate::unlink`].
//!
//! Who may do what (the plant or the line of the work order, `MFG.ACL.01`):
//! `create` needs `work_orders.create`; `release`, `cancel` and `unlink` need
//! `work_orders.release`; `start`, `hold`, `resume` and `complete` need
//! `work_orders.report`; `close` needs `work_orders.close`.
//!
//! Decisions where the spec is silent:
//!
//! - `release` lists the shortages of the pick list only as the list itself;
//!   the shortage check and its approval come with the schedule board.
//! - The standard cost of a unit is taken from the BOM version at release and
//!   kept on the work order: materials at landed cost plus the standard hours
//!   at the hourly rate of the line (`MFG.CST.01`).
//! - A pick line is the component quantity of one unit times the quantity,
//!   plus the scrap allowance of the component.
//! - A work order with no BOM cannot be released; the BOM is wl-inventory's
//!   (`STK.BOM.01`) and is reached through the kernel's `BillsOfMaterials`.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::approvals::announce;
use wl_kernel::documents::today;
use wl_kernel::inventory::{ITEM_SUBJECT_TYPE, LOT_SUBJECT_TYPE};
use wl_kernel::numbering::{Reset, Series};
use wl_kernel::production::{BOM_SUBJECT_TYPE, boms};
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE;

use crate::cost::value_of;
use crate::db::{answer, refused};
use crate::events::{WorkOrderCancelled, WorkOrderCompleted, WorkOrderReleased};
use crate::permissions;
use crate::plant::{Line, Plant};
use crate::texts::english;
use crate::work_order::{
    AUTO_CANCEL, AUTO_CLOSE, CANCEL, CLOSE, COMPLETE, HOLD, OrderInput, RELEASE, RESUME, START,
    WorkOrder, WorkOrderSource, WorkOrderStatus,
};

/// The numbers of work orders: `WO-2026-0001`.
pub const WORK_ORDER_SERIES: Series = Series {
    key: "work_order",
    pattern: "{prefix}-{yyyy}-{0000}",
    prefix: "WO",
    reset: Reset::Yearly,
    gapless: false,
    per_entity: false,
};

/// What a new work order is made of.
#[derive(Debug, Clone, Default)]
pub struct NewWorkOrder {
    /// The plant.
    pub plant_id: i64,
    /// The line it is scheduled on, if already known.
    pub line_id: Option<i64>,
    /// The finished item.
    pub item_id: i64,
    /// The BOM version.
    pub bom_id: Option<i64>,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The first planned day.
    pub planned_start: Option<NaiveDate>,
    /// The last planned day.
    pub planned_end: Option<NaiveDate>,
    /// The sales order and line it serves; none for stock.
    pub sales_order: Option<(i64, i64)>,
}

/// The work order `id`, or 404.
async fn load(state: &AppState, id: i64) -> Result<WorkOrder> {
    WorkOrder::find_or_404(&state.db, id).await
}

/// Refuses unless `by` holds `permission` for the work order.
fn allow(order: &WorkOrder, by: &AuthUser, permission: &str) -> Result {
    if order.may(by, permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Runs transition `name` as `by` and announces it.
async fn run(
    state: &AppState,
    by: &AuthUser,
    order: WorkOrder,
    name: &str,
    input: OrderInput,
    reason: Option<&str>,
) -> Result<WorkOrder> {
    let id = order.id;
    let mut run = order.transition(name).by(by).input(&input);
    if let Some(reason) = reason {
        run = run.reason(reason.trim());
    }
    let outcome = run.run(state).await.map_err(answer)?;
    announce(state, &outcome).await?;
    outcome.emit(state).await?;
    load(state, id).await
}

/// Creates a `planned` work order.
///
/// # Errors
///
/// `Forbidden` without `work_orders.create` for the plant; a 422 for an
/// unknown or inactive plant, a line of another plant, a quantity that is not
/// above zero or an end before the start.
pub async fn create(state: &AppState, by: &AuthUser, new: NewWorkOrder) -> Result<WorkOrder> {
    let plant = Plant::find(&state.db, new.plant_id)
        .await?
        .filter(|plant| plant.active)
        .ok_or_else(|| refused(english("manufacturing.create.plant", &[])))?;
    let allowed = by.has_permission_in(permissions::WORK_ORDERS_CREATE, &plant.scope())
        || new.line_id.is_some_and(|line| {
            by.has_permission_in(
                permissions::WORK_ORDERS_CREATE,
                &permissions::line_scope(line),
            )
        });
    if !allowed {
        return Err(Error::Forbidden);
    }
    insert(state, &plant, new, Some(by.id)).await
}

/// Creates a `planned` work order as the system, for the rework an NCR asks
/// for (07 §7.1 *NCR* `disposition`): the permission was checked on the NCR.
///
/// # Errors
///
/// A 422 for an unknown or inactive plant, a line of another plant, a
/// quantity that is not above zero or an end before the start.
pub async fn create_by_system(
    state: &AppState,
    new: NewWorkOrder,
    created_by: Option<i64>,
) -> Result<WorkOrder> {
    let plant = Plant::find(&state.db, new.plant_id)
        .await?
        .filter(|plant| plant.active)
        .ok_or_else(|| refused(english("manufacturing.create.plant", &[])))?;
    insert(state, &plant, new, created_by).await
}

/// The checks of the data and the insert of [`create`] and
/// [`create_by_system`].
async fn insert(
    state: &AppState,
    plant: &Plant,
    new: NewWorkOrder,
    created_by: Option<i64>,
) -> Result<WorkOrder> {
    if let Some(line) = new.line_id {
        let found = Line::find(&state.db, line).await?;
        if !found
            .as_ref()
            .is_some_and(|line| line.plant_id == plant.id && line.active)
        {
            return Err(refused(english("manufacturing.create.line", &[])));
        }
    }
    if new.quantity_milli <= 0 || new.item_id <= 0 {
        return Err(refused(english("manufacturing.create.quantity", &[])));
    }
    if new
        .planned_end
        .zip(new.planned_start)
        .is_some_and(|(end, start)| end < start)
    {
        return Err(refused(english("manufacturing.create.dates", &[])));
    }
    let number = WORK_ORDER_SERIES
        .next_allowing_gaps(&state.db, None, None, today())
        .await?;
    let (source, sales_order_id, sales_order_line_id) = match new.sales_order {
        Some((order, line)) => (WorkOrderSource::SalesOrder, Some(order), Some(line)),
        None => (WorkOrderSource::Stock, None, None),
    };
    WorkOrder::create(
        &state.db,
        WorkOrder {
            number,
            plant_id: plant.id,
            line_id: new.line_id,
            item_id: new.item_id,
            bom_id: new.bom_id,
            quantity_milli: new.quantity_milli,
            planned_start: new.planned_start,
            planned_end: new.planned_end,
            source,
            sales_order_id,
            sales_order_line_id,
            created_by,
            ..WorkOrder::default()
        },
    )
    .await
}

/// The checks that fail for releasing `order`, one English line each, and the
/// standard cost and pick list the release keeps.
async fn readiness(state: &AppState, order: &WorkOrder) -> Result<OrderInput> {
    let mut input = OrderInput::default();
    let line = match order.line_id {
        Some(line) => Line::find(&state.db, line).await?,
        None => None,
    };
    if line.is_none() {
        input
            .problems
            .push(english("manufacturing.release.no_line", &[]));
    }
    match order.bom_id {
        None => input
            .problems
            .push(english("manufacturing.release.no_bom", &[])),
        Some(bom) => {
            let found = boms(state)
                .version(state, &Subject::new(BOM_SUBJECT_TYPE, bom))
                .await?;
            match found {
                Some(version) if version.released => {
                    let rate = line.as_ref().map_or(0, |line| line.hourly_rate);
                    input.std_material_cost = Some(version.material_cost_per_unit);
                    input.std_unit_cost = Some(
                        version.material_cost_per_unit
                            + value_of(version.standard_milli_hours_per_unit, rate),
                    );
                    for component in &version.components {
                        let needed = i128::from(order.quantity_milli)
                            * i128::from(component.quantity_milli)
                            * i128::from(10_000 + component.scrap_bps)
                            / (1_000 * 10_000);
                        input.pick.push((
                            component.item.subject_id,
                            i64::try_from(needed).unwrap_or(i64::MAX),
                        ));
                    }
                }
                _ => input
                    .problems
                    .push(english("manufacturing.release.bom_not_released", &[])),
            }
        }
    }
    Ok(input)
}

/// Every check that fails for releasing `order` now, one line each; empty
/// when it can be released.
///
/// # Errors
///
/// A database error.
pub async fn release_problems(state: &AppState, order: &WorkOrder) -> Result<Vec<String>> {
    Ok(readiness(state, order).await?.problems)
}

/// `by` releases work order `id`: it goes on its line and the material pick
/// list is generated.
///
/// # Errors
///
/// `Forbidden` without `work_orders.release`; a 422 listing every failing
/// check (no line, a BOM version that is not released), or unless it is
/// `planned`.
pub async fn release(state: &AppState, by: &AuthUser, id: i64) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_RELEASE)?;
    let mut input = readiness(state, &order).await?;
    input.by = Some(by.id);
    let released = run(state, by, order, RELEASE, input, None).await?;
    if let Some(line) = released.line_id {
        state
            .emit(WorkOrderReleased {
                work_order: released.subject(),
                line: Subject::new(crate::plant::LINE_SUBJECT_TYPE, line),
            })
            .await?;
    }
    Ok(released)
}

/// `by` starts work order `id`: the start time is recorded.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report` (`MFG.ACL.01`); a 422 unless it
/// is `released`.
pub async fn start(state: &AppState, by: &AuthUser, id: i64) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_REPORT)?;
    run(state, by, order, START, OrderInput::default(), None).await
}

/// `by` puts work order `id` on hold for `reason` (`material`, `machine` or
/// `quality`).
///
/// # Errors
///
/// `Forbidden` without `work_orders.report`; a 422 for another reason or
/// unless it is `in_progress`.
pub async fn hold(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_REPORT)?;
    let input = OrderInput {
        hold_reason: Some(reason.trim().to_owned()),
        by: Some(by.id),
        ..OrderInput::default()
    };
    run(state, by, order, HOLD, input, Some(reason)).await
}

/// `by` resumes work order `id`.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report`; a 422 unless it is `on_hold`.
pub async fn resume(state: &AppState, by: &AuthUser, id: i64) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_REPORT)?;
    run(state, by, order, RESUME, OrderInput::default(), None).await
}

/// `by` completes work order `id`: its output lot takes no more output and a
/// final inspection is requested of QC.
///
/// # Errors
///
/// `Forbidden` without `work_orders.report`; a 422 unless it is
/// `in_progress` with good output reported.
pub async fn complete(state: &AppState, by: &AuthUser, id: i64) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_REPORT)?;
    let done = run(state, by, order, COMPLETE, OrderInput::default(), None).await?;
    let lot = done
        .output_lot_id
        .map(|lot| Subject::new(LOT_SUBJECT_TYPE, lot));
    role_tasks::open(
        state,
        RoleTask::new(
            lot.clone().unwrap_or_else(|| done.subject()),
            permissions::QC_ROLE,
            &english(
                "manufacturing.complete.inspection",
                &[("order", &done.number)],
            ),
            &format!("manufacturing.final_inspection:{}", done.id),
        ),
    )
    .await?;
    state
        .emit(WorkOrderCompleted {
            work_order: done.subject(),
            output_lot: lot,
            good_milli: done.good_milli,
            scrap_milli: done.scrap_milli,
        })
        .await?;
    Ok(done)
}

/// Why `order` cannot be closed now, if it cannot: material still waits as a
/// pending issue.
fn close_problems(issues: &[crate::issue::MaterialIssue]) -> Vec<String> {
    if issues.iter().any(|issue| issue.pending_issue_id.is_some()) {
        vec![english("manufacturing.close.pending_issue", &[])]
    } else {
        Vec::new()
    }
}

/// `by` closes completed work order `id`: the actual cost is calculated.
///
/// # Errors
///
/// `Forbidden` without `work_orders.close`; a 422 while an issue waits as a
/// pending issue, or unless it is `completed`.
pub async fn close(state: &AppState, by: &AuthUser, id: i64) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_CLOSE)?;
    let issues = crate::issue::MaterialIssue::of_order(&state.db, id).await?;
    let input = OrderInput {
        problems: close_problems(&issues),
        by: Some(by.id),
        ..OrderInput::default()
    };
    run(state, by, order, CLOSE, input, None).await
}

/// The system closes completed work order `id` (all issues and output
/// posted).
///
/// # Errors
///
/// As [`close`], without the permission.
pub async fn close_by_system(state: &AppState, id: i64) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    let issues = crate::issue::MaterialIssue::of_order(&state.db, id).await?;
    let input = OrderInput {
        problems: close_problems(&issues),
        ..OrderInput::default()
    };
    let outcome = order
        .transition(AUTO_CLOSE)
        .by_system()
        .input(&input)
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    load(state, id).await
}

/// `by` cancels work order `id` with `reason` (needed from `in_progress` and
/// `on_hold`). The pick list is withdrawn and the sales order link removed.
///
/// # Errors
///
/// `Forbidden` without `work_orders.release`; a 422 once a material was
/// issued or output reported (use `abort`), without the reason it needs, or
/// unless it is `planned`, `released`, `in_progress` or `on_hold`.
pub async fn cancel(state: &AppState, by: &AuthUser, id: i64, reason: &str) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    allow(&order, by, permissions::WORK_ORDERS_RELEASE)?;
    let cancelled = run(
        state,
        by,
        order,
        CANCEL,
        OrderInput::default(),
        Some(reason),
    )
    .await?;
    state
        .emit(WorkOrderCancelled {
            work_order: cancelled.subject(),
            reason: cancelled.cancel_reason.clone(),
        })
        .await?;
    Ok(cancelled)
}

/// The system cancels work order `id` with `reason` (`MFG.RUL.09`).
///
/// # Errors
///
/// As [`cancel`], without the permission.
pub async fn cancel_by_system(state: &AppState, id: i64, reason: &str) -> Result<WorkOrder> {
    let order = load(state, id).await?;
    let outcome = order
        .transition(AUTO_CANCEL)
        .by_system()
        .reason(reason.trim())
        .run(state)
        .await
        .map_err(answer)?;
    outcome.emit(state).await?;
    let cancelled = load(state, id).await?;
    state
        .emit(WorkOrderCancelled {
            work_order: cancelled.subject(),
            reason: cancelled.cancel_reason.clone(),
        })
        .await?;
    Ok(cancelled)
}

/// The subject of the sales order a work order serves, if it is linked.
pub fn sales_subject(order: &WorkOrder) -> Option<Subject> {
    order
        .sales_order_id
        .map(|id| Subject::new(SALES_ORDER_SUBJECT_TYPE, id))
}

/// The item of a work order as a subject (re-exported for the screens).
pub fn item_subject(order: &WorkOrder) -> Subject {
    Subject::new(ITEM_SUBJECT_TYPE, order.item_id)
}

/// Whether `status` is one a work order's materials can be issued in.
pub fn takes_material(status: WorkOrderStatus) -> bool {
    matches!(
        status,
        WorkOrderStatus::Released | WorkOrderStatus::InProgress | WorkOrderStatus::OnHold
    )
}
