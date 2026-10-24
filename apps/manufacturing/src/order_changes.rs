//! The work orders follow their sales order (07 Manufacturing §4.1 "When it
//! goes wrong" rows 1 and 2, §7.1 *Sales order* and *Work order*, §7.2
//! `MFG.RUL.09`; BL-MFG-03).
//!
//! The rule listens to the sales order events `SalesOrderAmended`,
//! `SalesOrderLineCancelled`, `SalesOrderCancelled` and
//! `SalesOrderLinesReleased` of wl-sales. wl-sales emits them **after** any
//! approval under `SAL.ACL.05`, so nothing happens to a work order while an
//! amendment or a cancellation waits. For each line that has linked work
//! orders:
//!
//! | Work order | Line's open quantity went down | went up |
//! |---|---|---|
//! | **not started** (`planned`, or `released` with nothing issued) | its quantity becomes the share of the new open quantity it carries (latest planned start gives way first); at 0 it is cancelled and its pick list withdrawn; a `released` one gets its pick list made again | the latest planned one takes the difference; none left: a new `planned` work order |
//! | **started** (`in_progress`, `on_hold`, or with material issued or output reported) | keeps its quantity; the output above the open quantity is flagged *for stock*; at 0 it is unlinked (outcome *finish to stock*); outcome *scrap*: nothing issued or reported: cancelled, else the production manager gets a task to request the abort, the rule never aborts | the *for stock* flag is taken off first, then a new `planned` work order for the rest, linked to the same line, item and BOM version |
//!
//! Outcome *return to stock* concerns picked finished goods (wl-inventory):
//! no work order changes. The planners of the plant are told what changed.
//!
//! Decisions:
//! - **Idempotent per event and work order.** Every change is a row of
//!   `manufacturing_order_adjustments` under a key that names the event and
//!   the line (`amend:{order}:{revision}:{line}`, `line_cancel:{order}:{line}:
//!   {quantity}:{open}`, `order_cancel:{order}`); a replay finds the row and
//!   does nothing. The decreases are also worked out from the line's state, so
//!   they converge whatever the order the events come in.
//! - **A decrease is worked out from what the line holds**: the quantities of
//!   the line's open work orders against its open quantity; a work order that
//!   covers only part of a line (the rest comes from stock) is left alone
//!   while the supply is not above the demand. An **increase** is worked out
//!   from the event's difference, for the same reason.
//! - **A prepaid increase** (`held_milli`) makes work orders for the part
//!   that is paid; the part held back waits for `SalesOrderLinesReleased` and
//!   then gets its work order, once.
//! - **Scrap with part of the line still open**: a started work order that
//!   made nothing yet keeps its quantity, flagged *for stock*; one with work in
//!   progress gets the abort task, as the spec says. The *for stock* output
//!   is flagged on the work order; wl-inventory reserves the output lot for
//!   the line only up to the line's open quantity less the part for stock
//!   ([`crate::output_reservation`]), so that part is free stock as soon as
//!   it is reported, and the reservation shrinks when the flag is set after
//!   the output was reported.
//! - The abort task is one per work order and closes when the abort is final.
//!   Its text carries the reason to type into the abort sheet, which the work
//!   order's page fills in.

use std::cmp::Reverse;

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::production::order_lines;
use wl_kernel::sales::{
    LineOutcome, LineOutcomeKind, SalesOrderAmended, SalesOrderCancelled, SalesOrderId,
    SalesOrderLineCancelled, SalesOrderLinesReleased,
};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, task_creator};

use crate::notices;
use crate::permissions;
use crate::screens::quantity_text;
use crate::texts::english;
use crate::unlink::unlink_by_system;
use crate::work_order::{WorkOrder, WorkOrderSource, WorkOrderStatus};
use crate::work_orders::{
    NewWorkOrder, QuantityChange, cancel_by_system, change_quantity_by_system, create_by_system,
    set_for_stock_by_system,
};

/// The rule's identifier.
pub const RULE: &str = "MFG.RUL.09";

/// The actions the journal records.
pub mod action {
    /// The quantity of a not started work order changed.
    pub const QUANTITY: &str = "quantity";
    /// A not started work order was cancelled.
    pub const CANCEL: &str = "cancel";
    /// A started work order was unlinked.
    pub const UNLINK: &str = "unlink";
    /// The *for stock* part of a started work order changed.
    pub const FOR_STOCK: &str = "for_stock";
    /// A new planned work order was made for an increase.
    pub const EXTRA_ORDER: &str = "extra_order";
    /// The production manager was asked to request the abort.
    pub const ABORT_TASK: &str = "abort_task";
    /// Part of an increase was held back until it is paid.
    pub const HELD: &str = "held";
    /// A held part was released.
    pub const RELEASED: &str = "released";
}

/// One change made to a work order or a line, as the journal keeps it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_order_adjustments",
    unique(event_key, action, work_order_id),
    index(work_order_id),
    index(order_id, line_id)
)]
pub struct Adjustment {
    /// The primary key.
    pub id: i64,
    /// The event and line it answers.
    pub event_key: String,
    /// The sales order.
    pub order_id: i64,
    /// The sale line.
    pub line_id: i64,
    /// The work order changed, or 0 for the line.
    pub work_order_id: i64,
    /// What was done, one of [`action`].
    pub action: String,
    /// The quantity before, in thousandths.
    pub from_milli: i64,
    /// The quantity after, in thousandths.
    pub to_milli: i64,
    /// Words for the planner.
    pub note: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Adjustment {
    /// The changes made to work order `work_order_id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_work_order(db: &Db, work_order_id: i64) -> Result<Vec<Adjustment>> {
        Adjustment::query()
            .where_(Adjustment::WORK_ORDER_ID.eq(work_order_id))
            .order_by(Adjustment::ID)
            .get(db)
            .await
    }

    /// Whether this change was made already.
    async fn done(state: &AppState, key: &str, action: &str, work_order: i64) -> Result<bool> {
        Ok(Adjustment::query()
            .where_(Adjustment::EVENT_KEY.eq(key))
            .where_(Adjustment::ACTION.eq(action))
            .where_(Adjustment::WORK_ORDER_ID.eq(work_order))
            .first(&state.db)
            .await?
            .is_some())
    }
}

/// What one event asks of one line.
#[derive(Debug, Clone)]
struct Change {
    order: SalesOrderId,
    line: i64,
    /// The event and line, the key of its journal rows.
    key: String,
    /// The outcome sales chose for the started goods of the line.
    outcome: Option<LineOutcomeKind>,
    /// The quantity added, for an amendment that raised the line.
    increase_milli: i64,
    /// The part of `increase_milli` held back until it is paid.
    held_milli: i64,
    /// Why, for the history rows.
    reason: String,
}

/// The work orders linked to line `line` of `order` that are still open,
/// not started first, latest planned start first in each group.
async fn open_linked(
    state: &AppState,
    order: i64,
    line: i64,
) -> Result<(Vec<WorkOrder>, Vec<WorkOrder>)> {
    let mut linked = WorkOrder::query()
        .where_(WorkOrder::SALES_ORDER_ID.eq(order))
        .where_(WorkOrder::SALES_ORDER_LINE_ID.eq(line))
        .where_(WorkOrder::SOURCE.eq(WorkOrderSource::SalesOrder))
        .get(&state.db)
        .await?;
    linked.retain(|work| work.status.is_open());
    linked.sort_by_key(|work| (Reverse(work.planned_start), Reverse(work.id)));
    Ok(linked.into_iter().partition(|work| !work.started()))
}

/// Whether any work order (open or done) is linked to the line.
async fn any_linked(state: &AppState, order: i64, line: i64) -> Result<Option<WorkOrder>> {
    let mut linked = WorkOrder::query()
        .where_(WorkOrder::SALES_ORDER_ID.eq(order))
        .where_(WorkOrder::SALES_ORDER_LINE_ID.eq(line))
        .where_(WorkOrder::SOURCE.eq(WorkOrderSource::SalesOrder))
        .order_by(WorkOrder::ID)
        .get(&state.db)
        .await?;
    linked.retain(|work| {
        !matches!(
            work.status,
            WorkOrderStatus::Cancelled | WorkOrderStatus::Aborted
        )
    });
    Ok(linked.into_iter().next())
}

/// Writes the journal row; answers whether it was new.
async fn record(
    state: &AppState,
    change: &Change,
    action: &str,
    work_order: i64,
    from: i64,
    to: i64,
    note: String,
) -> Result<bool> {
    if Adjustment::done(state, &change.key, action, work_order).await? {
        return Ok(false);
    }
    Adjustment::create(
        &state.db,
        Adjustment {
            event_key: change.key.clone(),
            order_id: change.order.0,
            line_id: change.line,
            work_order_id: work_order,
            action: action.to_owned(),
            from_milli: from,
            to_milli: to,
            note,
            ..Adjustment::default()
        },
    )
    .await?;
    Ok(true)
}

/// The source key of the abort task of work order `id`.
pub fn abort_task_key(id: i64) -> String {
    format!("manufacturing.abort_for_scrap:{id}")
}

/// What `MFG.RUL.09` did for one line, in words for the planners.
#[derive(Debug, Default)]
struct Told(Vec<String>);

impl Told {
    fn add(&mut self, line: String) {
        self.0.push(line);
    }
}

/// The holders of the production manager role of the plant, else whom the
/// kernel picks.
async fn production_managers(state: &AppState, plant: i64) -> Result<Vec<i64>> {
    notices::holders(state, permissions::PRODUCTION_MANAGER_ROLE, plant).await
}

/// Opens the task for the production manager to request the abort of `work`.
async fn ask_for_abort(
    state: &AppState,
    work: &WorkOrder,
    number: &str,
    why: &str,
) -> Result<bool> {
    let key = abort_task_key(work.id);
    let creator = task_creator(state);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(false);
    }
    let reason = abort_reason(number, why);
    let params: [(&str, &(dyn std::fmt::Display + Sync)); 3] = [
        ("order", &work.number),
        ("sales", &number),
        ("reason", &reason),
    ];
    let mut ticket = OpenTicket::new(
        work.subject(),
        TASK_CATEGORY,
        &english("manufacturing.order_change.abort_title", &params),
    )
    .description(&english("manufacturing.order_change.abort_body", &params))
    .source_key(&key);
    if let Some(user) = production_managers(state, work.plant_id).await?.first() {
        ticket = ticket.assign_to(Assignee::user(*user));
    }
    Ok(creator.open(state, ticket).await?.is_some())
}

/// The reason the abort sheet is filled with for a cancelled order.
pub fn abort_reason(number: &str, why: &str) -> String {
    english(
        "manufacturing.order_change.abort_reason",
        &[("sales", &number), ("why", &why)],
    )
}

/// Closes the abort task of work order `id`, if one is open.
///
/// # Errors
///
/// The ticketing's errors.
pub async fn close_abort_task(state: &AppState, id: i64, note: &str) -> Result<bool> {
    let creator = task_creator(state);
    match creator.find_open(state, &abort_task_key(id)).await? {
        Some(found) => creator.complete(state, found.id, note).await,
        None => Ok(false),
    }
}

/// Applies `change` to the work orders of its line; the planners are told.
async fn apply(state: &AppState, change: Change) -> Result<()> {
    let Some(info) = order_lines(state)
        .line(state, change.order, change.line)
        .await?
    else {
        return Ok(());
    };
    if change.outcome == Some(LineOutcomeKind::ReturnToStock) {
        // Picked finished goods go back to stock (wl-inventory): no work
        // order changes.
        return Ok(());
    }
    let Some(first) = any_linked(state, change.order.0, change.line).await? else {
        return Ok(());
    };
    let number = info
        .number
        .clone()
        .unwrap_or_else(|| format!("#{}", change.order.0));
    let open = if info.cancelled {
        0
    } else {
        info.open_milli.max(0)
    };
    let (not_started, started) = open_linked(state, change.order.0, change.line).await?;
    let mut told = Told::default();
    if change.increase_milli > 0 {
        increase(
            state,
            &change,
            &first,
            &not_started,
            &started,
            &number,
            &mut told,
        )
        .await?;
    } else {
        decrease(
            state,
            &change,
            open,
            &not_started,
            &started,
            &number,
            &mut told,
        )
        .await?;
    }
    if told.0.is_empty() {
        return Ok(());
    }
    let plant = first.plant_id;
    let planners = notices::holders(state, permissions::PLANNER_ROLE, plant).await?;
    notices::tell(
        state,
        first.subject(),
        &number,
        &planners,
        &english(
            "manufacturing.order_change.title",
            &[("sales", &number), ("line", &change.line)],
        ),
        &told.0.join("\n"),
    )
    .await?;
    Ok(())
}

/// A line went up by `change.increase_milli`.
async fn increase(
    state: &AppState,
    change: &Change,
    template: &WorkOrder,
    not_started: &[WorkOrder],
    started: &[WorkOrder],
    number: &str,
    told: &mut Told,
) -> Result<()> {
    let add = change.increase_milli - change.held_milli;
    if change.held_milli > 0 {
        record(
            state,
            change,
            action::HELD,
            0,
            0,
            change.held_milli,
            String::new(),
        )
        .await?;
    }
    supply_extra(
        state,
        change,
        template,
        not_started,
        started,
        number,
        add,
        told,
    )
    .await
}

/// Puts `add` thousandths of new demand on the work orders of the line:
/// first off the *for stock* flags of the started ones, then onto the latest
/// not started one, then into a new planned work order.
#[allow(clippy::too_many_arguments)]
async fn supply_extra(
    state: &AppState,
    change: &Change,
    template: &WorkOrder,
    not_started: &[WorkOrder],
    started: &[WorkOrder],
    number: &str,
    mut add: i64,
    told: &mut Told,
) -> Result {
    for work in started {
        if add <= 0 {
            break;
        }
        if work.quantity_for_stock_milli > 0 {
            let take = add.min(work.quantity_for_stock_milli);
            let to = work.quantity_for_stock_milli - take;
            if Adjustment::done(state, &change.key, action::FOR_STOCK, work.id).await? {
                add -= take;
                continue;
            }
            set_for_stock_by_system(state, work.id, to).await?;
            record(
                state,
                change,
                action::FOR_STOCK,
                work.id,
                work.quantity_for_stock_milli,
                to,
                String::new(),
            )
            .await?;
            told.add(english(
                "manufacturing.order_change.for_stock_down",
                &[("order", &work.number), ("quantity", &quantity_text(to))],
            ));
            add -= take;
        }
    }
    if add > 0
        && let Some(work) = not_started.first()
    {
        if Adjustment::done(state, &change.key, action::QUANTITY, work.id).await? {
            return Ok(());
        }
        let to = work.quantity_milli + add;
        if let QuantityChange::Changed { .. } =
            change_quantity_by_system(state, work.id, to).await?
        {
            record(
                state,
                change,
                action::QUANTITY,
                work.id,
                work.quantity_milli,
                to,
                String::new(),
            )
            .await?;
            told.add(english(
                "manufacturing.order_change.quantity",
                &[
                    ("order", &work.number),
                    ("from", &quantity_text(work.quantity_milli)),
                    ("to", &quantity_text(to)),
                    ("sales", &number),
                ],
            ));
        }
        return Ok(());
    }
    if add > 0 {
        if Adjustment::done(state, &change.key, action::EXTRA_ORDER, 0).await? {
            return Ok(());
        }
        let extra = create_by_system(
            state,
            NewWorkOrder {
                plant_id: template.plant_id,
                line_id: template.line_id,
                item_id: template.item_id,
                bom_id: template.bom_id,
                quantity_milli: add,
                planned_start: template.planned_start,
                planned_end: template.planned_end,
                sales_order: Some((change.order.0, change.line)),
            },
            None,
        )
        .await?;
        record(
            state,
            change,
            action::EXTRA_ORDER,
            0,
            0,
            add,
            extra.number.clone(),
        )
        .await?;
        // The new work order is the one the journal shows on its own page.
        Adjustment::create(
            &state.db,
            Adjustment {
                event_key: format!("{}:new", change.key),
                order_id: change.order.0,
                line_id: change.line,
                work_order_id: extra.id,
                action: action::EXTRA_ORDER.to_owned(),
                from_milli: 0,
                to_milli: add,
                note: template.number.clone(),
                ..Adjustment::default()
            },
        )
        .await?;
        told.add(english(
            "manufacturing.order_change.extra",
            &[
                ("order", &extra.number),
                ("quantity", &quantity_text(add)),
                ("sales", &number),
                ("started", &template.number),
            ],
        ));
    }
    Ok(())
}

/// A line went down or was cancelled: the supply is brought down to the open
/// quantity.
async fn decrease(
    state: &AppState,
    change: &Change,
    open: i64,
    not_started: &[WorkOrder],
    started: &[WorkOrder],
    number: &str,
    told: &mut Told,
) -> Result<()> {
    let supply: i64 = not_started
        .iter()
        .chain(started)
        .map(|work| work.quantity_milli)
        .sum();
    let mut excess = supply - open;
    if excess <= 0 {
        return Ok(());
    }
    // 1. The not started work orders give way first.
    let mut kept_not_started = 0;
    for work in not_started {
        let reduce = excess.min(work.quantity_milli);
        let to = work.quantity_milli - reduce;
        excess -= reduce;
        kept_not_started += to;
        if reduce == 0 {
            continue;
        }
        if to == 0 {
            if Adjustment::done(state, &change.key, action::CANCEL, work.id).await? {
                continue;
            }
            cancel_by_system(
                state,
                work.id,
                &english(
                    "manufacturing.order_change.cancel_reason",
                    &[("sales", &number)],
                ),
            )
            .await?;
            record(
                state,
                change,
                action::CANCEL,
                work.id,
                work.quantity_milli,
                0,
                String::new(),
            )
            .await?;
            told.add(english(
                "manufacturing.order_change.cancelled",
                &[("order", &work.number), ("sales", &number)],
            ));
        } else {
            if Adjustment::done(state, &change.key, action::QUANTITY, work.id).await? {
                continue;
            }
            if let QuantityChange::Changed { pick_problem, .. } =
                change_quantity_by_system(state, work.id, to).await?
            {
                record(
                    state,
                    change,
                    action::QUANTITY,
                    work.id,
                    work.quantity_milli,
                    to,
                    pick_problem.clone().unwrap_or_default(),
                )
                .await?;
                told.add(english(
                    "manufacturing.order_change.quantity",
                    &[
                        ("order", &work.number),
                        ("from", &quantity_text(work.quantity_milli)),
                        ("to", &quantity_text(to)),
                        ("sales", &number),
                    ],
                ));
                if let Some(problem) = pick_problem {
                    told.add(problem);
                }
            }
        }
    }
    if excess <= 0 {
        return Ok(());
    }
    // 2. The started work orders keep their quantity: the earliest planned
    // ones carry the open quantity, the rest of the output is for stock.
    let mut carry = (open - kept_not_started).max(0);
    let mut by_start: Vec<&WorkOrder> = started.iter().collect();
    by_start.reverse();
    for work in by_start {
        let carried = carry.min(work.quantity_milli);
        carry -= carried;
        let surplus = work.quantity_milli - carried;
        if surplus == 0 {
            continue;
        }
        started_surplus(state, change, work, surplus, open, number, told).await?;
    }
    Ok(())
}

/// What happens to the part `surplus` of started work order `work` that the
/// line no longer needs.
async fn started_surplus(
    state: &AppState,
    change: &Change,
    work: &WorkOrder,
    surplus: i64,
    open: i64,
    number: &str,
    told: &mut Told,
) -> Result<()> {
    let scrap = change.outcome == Some(LineOutcomeKind::Scrap);
    if scrap && !work.has_work_in_progress() && open == 0 {
        if Adjustment::done(state, &change.key, action::CANCEL, work.id).await? {
            return Ok(());
        }
        cancel_by_system(
            state,
            work.id,
            &english(
                "manufacturing.order_change.cancel_reason",
                &[("sales", &number)],
            ),
        )
        .await?;
        record(
            state,
            change,
            action::CANCEL,
            work.id,
            work.quantity_milli,
            0,
            String::new(),
        )
        .await?;
        told.add(english(
            "manufacturing.order_change.cancelled",
            &[("order", &work.number), ("sales", &number)],
        ));
        return Ok(());
    }
    if scrap && work.has_work_in_progress() {
        if Adjustment::done(state, &change.key, action::ABORT_TASK, work.id).await? {
            return Ok(());
        }
        if ask_for_abort(state, work, number, &change.reason).await? {
            record(
                state,
                change,
                action::ABORT_TASK,
                work.id,
                work.quantity_milli,
                surplus,
                change.reason.clone(),
            )
            .await?;
            told.add(english(
                "manufacturing.order_change.abort_asked",
                &[("order", &work.number), ("sales", &number)],
            ));
        }
        return Ok(());
    }
    if open == 0 {
        if Adjustment::done(state, &change.key, action::UNLINK, work.id).await? {
            return Ok(());
        }
        let reason = english(
            "manufacturing.order_change.unlink_reason",
            &[("sales", &number)],
        );
        if unlink_by_system(state, work.id, &reason).await?.is_some() {
            record(
                state,
                change,
                action::UNLINK,
                work.id,
                work.quantity_milli,
                0,
                String::new(),
            )
            .await?;
            told.add(english(
                "manufacturing.order_change.unlinked",
                &[("order", &work.number), ("sales", &number)],
            ));
        }
        return Ok(());
    }
    // Part of the line is still open: the output above it is for stock.
    if Adjustment::done(state, &change.key, action::FOR_STOCK, work.id).await? {
        return Ok(());
    }
    if set_for_stock_by_system(state, work.id, surplus).await? {
        record(
            state,
            change,
            action::FOR_STOCK,
            work.id,
            work.quantity_for_stock_milli,
            surplus,
            String::new(),
        )
        .await?;
        told.add(english(
            "manufacturing.order_change.for_stock",
            &[
                ("order", &work.number),
                ("quantity", &quantity_text(surplus)),
                ("sales", &number),
            ],
        ));
    }
    Ok(())
}

/// The lines of an amendment that changed the quantity.
async fn on_amended(state: &AppState, event: &SalesOrderAmended) -> Result<()> {
    for line in &event.lines {
        let delta = line.quantity_after_milli - line.quantity_before_milli;
        if delta == 0 {
            continue;
        }
        let change = Change {
            order: event.order,
            line: line.line,
            key: format!("amend:{}:{}:{}", event.order.0, event.revision, line.line),
            outcome: None,
            increase_milli: delta.max(0),
            held_milli: if delta > 0 {
                line.held_milli.min(delta)
            } else {
                0
            },
            reason: event.reason.clone(),
        };
        apply(state, change).await?;
    }
    Ok(())
}

/// A line was cancelled in part or in full.
async fn on_line_cancelled(state: &AppState, event: &SalesOrderLineCancelled) -> Result<()> {
    let open = order_lines(state)
        .line(state, event.order, event.line)
        .await?
        .map_or(0, |info| if info.cancelled { 0 } else { info.open_milli });
    let change = Change {
        order: event.order,
        line: event.line,
        key: format!(
            "line_cancel:{}:{}:{}:{}",
            event.order.0, event.line, event.quantity_milli, open
        ),
        outcome: event.outcome,
        increase_milli: 0,
        held_milli: 0,
        reason: event.reason.clone(),
    };
    apply(state, change).await
}

/// The whole order was cancelled: every line with a linked work order.
async fn on_order_cancelled(state: &AppState, event: &SalesOrderCancelled) -> Result<()> {
    let linked = WorkOrder::query()
        .where_(WorkOrder::SALES_ORDER_ID.eq(event.order.0))
        .where_(WorkOrder::SOURCE.eq(WorkOrderSource::SalesOrder))
        .get(&state.db)
        .await?;
    let mut lines: Vec<i64> = linked
        .iter()
        .filter_map(|work| work.sales_order_line_id)
        .collect();
    lines.sort_unstable();
    lines.dedup();
    for line in lines {
        let outcome = event
            .outcomes
            .iter()
            .find(|outcome: &&LineOutcome| outcome.line == line)
            .map(|outcome| outcome.kind);
        let change = Change {
            order: event.order,
            line,
            key: format!("order_cancel:{}:{}", event.order.0, line),
            outcome,
            increase_milli: 0,
            held_milli: 0,
            reason: event.reason.clone(),
        };
        apply(state, change).await?;
    }
    Ok(())
}

/// A held part of an increase was paid: its work order is made, once.
async fn on_released(state: &AppState, event: &SalesOrderLinesReleased) -> Result<()> {
    for released in &event.lines {
        let held: i64 = Adjustment::query()
            .where_(Adjustment::ORDER_ID.eq(event.order.0))
            .where_(Adjustment::LINE_ID.eq(released.line))
            .where_(Adjustment::ACTION.eq(action::HELD))
            .get(&state.db)
            .await?
            .iter()
            .map(|row| row.to_milli)
            .sum();
        let already: i64 = Adjustment::query()
            .where_(Adjustment::ORDER_ID.eq(event.order.0))
            .where_(Adjustment::LINE_ID.eq(released.line))
            .where_(Adjustment::ACTION.eq(action::RELEASED))
            .get(&state.db)
            .await?
            .iter()
            .map(|row| row.to_milli)
            .sum();
        let quantity = released.quantity_milli.min(held - already);
        if quantity <= 0 {
            continue;
        }
        let change = Change {
            order: event.order,
            line: released.line,
            key: format!("released:{}:{}:{}", event.order.0, released.line, already),
            outcome: None,
            increase_milli: quantity,
            held_milli: 0,
            reason: String::new(),
        };
        record(
            state,
            &change,
            action::RELEASED,
            0,
            already,
            quantity,
            String::new(),
        )
        .await?;
        apply(state, change).await?;
    }
    Ok(())
}

/// Registers the listeners of `MFG.RUL.09`.
pub fn listen(app: &mut Registry) {
    app.listen(|event: SalesOrderAmended, state: AppState| async move {
        on_amended(&state, &event).await
    });
    app.listen(
        |event: SalesOrderLineCancelled, state: AppState| async move {
            on_line_cancelled(&state, &event).await
        },
    );
    app.listen(|event: SalesOrderCancelled, state: AppState| async move {
        on_order_cancelled(&state, &event).await
    });
    app.listen(
        |event: SalesOrderLinesReleased, state: AppState| async move {
            on_released(&state, &event).await
        },
    );
}
