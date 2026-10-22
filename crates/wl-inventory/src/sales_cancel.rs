//! The outcomes of sales cancellations applied to stock (wl-inventory §4
//! *Dispatch* row `cancel`, §5.2 step 3, §6 *Consumes*; BL-STK-26, issue
//! #245).
//!
//! When a sales order or one of its lines is cancelled after fulfilment
//! started, `wl-sales` says in its event what happens to the started goods
//! (`SalesOrderLineCancelled.outcome`, `SalesOrderCancelled.outcomes`).
//! Listeners here act on it, per line:
//!
//! - **return to stock** (and a line whose fulfilment had not started): the
//!   cancelled quantity is removed from the order's dispatches that are
//!   `draft` or `picked`, which releases the reservation (the picked goods
//!   never left the store, so no movement is needed). A dispatch left
//!   without lines is cancelled by the system (`cancel_by_system`); one that
//!   still has other lines keeps them. No wastage is posted.
//! - **scrap**: the same removal, and the goods are posted as `wastage`
//!   movements with the reason *cancelled order* and the order as reference:
//!   the picked dispatch lines first; for what is left (prepared meals) the
//!   vertical's [`SaleLineStockSource`](wl_kernel::inventory::SaleLineStockSource)
//!   gives the recipe: when the consumption was not yet posted the exploded
//!   components for the cancelled quantity are posted as wastage, when it
//!   was posted no second movement is made and the value of the line is
//!   reported as wastage cost. The cost of every scrap is posted as cost
//!   coded `wastage` on the order.
//! - **finish to stock**: the work order of Manufacturing (07) finishes; the
//!   dispatch lines are released, no movement and no wastage.
//! - a dispatch already `dispatched` (or beyond) is not cancelled: the goods
//!   follow the delivery (return or refusal). The event is logged and the
//!   dispatcher (a holder of `dispatch.manage` at the location) gets a task.
//!
//! An amendment that lowers a line (`SalesOrderAmended`) trims the same
//! quantity off the dispatch lines of that sale line.
//!
//! Idempotency is per event and line: every application leaves a
//! [`CancelEffect`] with a key made of the event kind, order, line, quantity,
//! outcome and (for an amendment) the revision; the same key is applied once.
//!
//! Decisions: the second approval that `STK.ACL.02` asks for wastage above
//! the value, by someone other than the recorder, is not built here: the
//! cancellation itself was approved by a manager (`SAL.ACL.05`) and the cost
//! is posted; see `DEFERRED-modules.md`. A scrapped quantity beyond the stock
//! at the location posts what is available and notes the rest in the effect.

use std::collections::BTreeSet;

use renox::auth::permissions::users_with_permission_in;
use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::sale_line_stock;
use wl_kernel::invoicing::{PostCost, post_cost};
use wl_kernel::sales::{
    LineOutcomeKind, SalesOrderAmended, SalesOrderCancelled, SalesOrderLineCancelled,
};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};

use crate::balance;
use crate::dispatch::{Dispatch, DispatchLine, DispatchStatus, cancel_by_system, lines_of};
use crate::item::{Item, value_of};
use crate::location::Location;
use crate::movement::{MovementType, StockMovement};
use crate::permissions;
use crate::stock_service::plan;
use crate::texts::said;

/// The reason on the wastage movements of a scrapped line.
pub const SCRAP_REASON: &str = "cancelled order";

/// The reason a dispatch is cancelled with.
pub const CANCEL_REASON: &str = "sales order line cancelled";

/// The cost code of a scrap.
pub const COST_CODE: &str = "wastage";

/// What was done for one cancelled line of an event.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_inventory_cancel_effects", unique(effect_key), index(order_id))]
pub struct CancelEffect {
    /// The primary key.
    pub id: i64,
    /// The key that makes the application once per event and line.
    pub effect_key: String,
    /// The sales order (its id in `wl-sales`).
    pub order_id: i64,
    /// The sale line.
    pub sale_line_id: i64,
    /// `return_to_stock`, `finish_to_stock`, `scrap`, or empty when the line
    /// had not started (or an amendment).
    pub outcome: Option<String>,
    /// The quantity cancelled, in thousandths (0 for a whole order, when the
    /// quantity is the dispatch's).
    pub quantity_milli: i64,
    /// `removed`, `wasted`, `reported`, `task`, `none`.
    pub action: String,
    /// The wastage value posted or reported, in fils.
    pub value: i64,
    /// Which dispatches were touched, or what was short.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl CancelEffect {
    /// The effects of the sales order `order_id`, oldest first.
    ///
    /// # Errors
    ///
    /// A database error.
    pub async fn of_order(db: &Db, order_id: i64) -> Result<Vec<CancelEffect>> {
        CancelEffect::query()
            .where_(CancelEffect::ORDER_ID.eq(order_id))
            .order_by(CancelEffect::ID)
            .get(db)
            .await
    }
}

/// What a cancelled line asks of stock.
#[derive(Debug, Clone)]
struct LineRequest {
    key: String,
    order: Subject,
    line: i64,
    /// The quantity cancelled; `None` for the whole line of a cancelled order.
    quantity_milli: Option<i64>,
    outcome: Option<LineOutcomeKind>,
}

/// What taking quantity off the dispatch lines of a sale line did.
#[derive(Debug, Default)]
struct Taken {
    /// The quantity taken off draft or picked lines.
    quantity_milli: i64,
    /// The dispatches whose lines changed.
    touched: BTreeSet<i64>,
    /// The wastage value posted, in fils.
    value: i64,
    /// The dispatches already past `picked`.
    away: Vec<Dispatch>,
}

fn outcome_text(outcome: Option<LineOutcomeKind>) -> Option<String> {
    outcome.map(|kind| kind.as_str().to_owned())
}

/// Takes up to `limit` (all when `None`) thousandths of the sale line `line`
/// of `order` off the lines of its draft and picked dispatches, posting them
/// as wastage when `scrap`.
async fn take_off_lines(
    tx: &mut Transaction,
    order: &Subject,
    line: i64,
    limit: Option<i64>,
    scrap: bool,
) -> Result<Taken> {
    let rows = DispatchLine::query()
        .where_(DispatchLine::SALE_LINE_ID.eq(Some(line)))
        .order_by(DispatchLine::ID)
        .get(&mut *tx)
        .await?;
    let mut taken = Taken::default();
    let mut left = limit;
    for row in rows {
        let dispatch = Dispatch::find_or_404(&mut *tx, row.dispatch_id).await?;
        if dispatch.source() != *order {
            continue;
        }
        if !dispatch.status.in_store() {
            if matches!(
                dispatch.status,
                DispatchStatus::Dispatched | DispatchStatus::Delivered | DispatchStatus::Lost
            ) && !taken.away.iter().any(|d| d.id == dispatch.id)
            {
                taken.away.push(dispatch);
            }
            continue;
        }
        let want = left.unwrap_or(row.quantity_milli);
        if want <= 0 {
            break;
        }
        let take = want.min(row.quantity_milli);
        if scrap {
            let item = Item::find_or_404(&mut *tx, row.item_id).await?;
            let value = value_of(take, item.average_cost);
            let mut moved = StockMovement::leaving(
                MovementType::Wastage,
                item.id,
                row.lot_id,
                dispatch.location_id,
                take,
                item.average_cost,
                value,
                order,
                Some(SCRAP_REASON.to_owned()),
                None,
            );
            moved.insert(&mut *tx).await?;
            taken.value += value;
        }
        if take == row.quantity_milli {
            DispatchLine::query()
                .where_(DispatchLine::ID.eq(row.id))
                .delete(&mut *tx)
                .await?;
        } else {
            DispatchLine::query()
                .where_(DispatchLine::ID.eq(row.id))
                .update(
                    &mut *tx,
                    &[
                        ("quantity_milli", &(row.quantity_milli - take)),
                        ("updated_at", &renox::db::now()),
                    ],
                )
                .await?;
        }
        taken.quantity_milli += take;
        taken.touched.insert(dispatch.id);
        left = left.map(|l| l - take);
    }
    Ok(taken)
}

/// What posting the components of a prepared line as wastage did.
#[derive(Debug, Default)]
pub(crate) struct Wasted {
    /// The value posted, in fils.
    pub value: i64,
    /// What stock did not cover (item code, thousandths).
    pub short: Vec<(String, i64)>,
    /// The first wastage movement made.
    pub first_movement: Option<i64>,
}

/// Posts the exploded components of `quantity_milli` of a prepared line as
/// wastage at `location` with `reason` and `order` as reference, as far as
/// the stock covers them.
pub(crate) async fn waste_components(
    tx: &mut Transaction,
    order: &Subject,
    location: &Location,
    components: &[wl_kernel::inventory::LineComponent],
    quantity_milli: i64,
    reason: &str,
) -> Result<Wasted> {
    let mut wasted = Wasted::default();
    for component in components {
        let item = Item::find_or_404(&mut *tx, component.item.subject_id).await?;
        let need = i64::try_from(
            (i128::from(component.per_unit_milli) * i128::from(quantity_milli) + 500) / 1_000,
        )
        .unwrap_or(0);
        if need <= 0 {
            continue;
        }
        let candidates = balance::candidates(&mut *tx, &item, location.id).await?;
        let (parts, left) = plan(&candidates, need);
        for (lot_id, quantity) in parts {
            let part_value = value_of(quantity, item.average_cost);
            let mut moved = StockMovement::leaving(
                MovementType::Wastage,
                item.id,
                lot_id,
                location.id,
                quantity,
                item.average_cost,
                part_value,
                order,
                Some(reason.to_owned()),
                None,
            );
            moved.theoretical = true;
            moved.insert(&mut *tx).await?;
            wasted.value += part_value;
            wasted.first_movement.get_or_insert(moved.id);
        }
        if left > 0 {
            wasted.short.push((item.code.clone(), left));
        }
    }
    Ok(wasted)
}

/// Applies the cancellation of one line (see the module docs).
async fn apply_line(state: &AppState, request: LineRequest) -> Result {
    if CancelEffect::query()
        .where_(CancelEffect::EFFECT_KEY.eq(request.key.clone()))
        .exists(&state.db)
        .await?
    {
        return Ok(());
    }
    let scrap = request.outcome == Some(LineOutcomeKind::Scrap);
    // What a prepared line is made of, asked before the transaction opens.
    let recipe = if scrap {
        sale_line_stock(state)
            .of_line(state, &request.order, request.line)
            .await?
    } else {
        None
    };
    let mut tx = state.db.begin().await?;
    let taken = take_off_lines(
        &mut tx,
        &request.order,
        request.line,
        request.quantity_milli,
        scrap,
    )
    .await?;
    let mut value = taken.value;
    let mut action = if taken.quantity_milli > 0 {
        if scrap { "wasted" } else { "removed" }
    } else {
        "none"
    };
    let mut notes: Vec<String> = taken
        .touched
        .iter()
        .map(|id| format!("dispatch {id}"))
        .collect();
    if scrap && let Some(recipe) = recipe {
        // Prepared goods: what is left of the cancelled quantity after the
        // picked lines.
        let wanted = request.quantity_milli.unwrap_or(recipe.quantity_milli);
        let left = if request.quantity_milli.is_some() {
            wanted - taken.quantity_milli
        } else if taken.quantity_milli > 0 {
            0
        } else {
            wanted
        };
        if left > 0 && recipe.consumed {
            // Already consumed: no second movement, the line is reported as
            // wastage value.
            let reported = if recipe.quantity_milli > 0 {
                i64::try_from(
                    i128::from(recipe.value) * i128::from(left)
                        / i128::from(recipe.quantity_milli),
                )
                .unwrap_or(recipe.value)
            } else {
                recipe.value
            };
            value += reported;
            action = "reported";
        } else if left > 0 {
            let location = Location::find_or_404(&mut tx, recipe.location.subject_id).await?;
            let wasted = waste_components(
                &mut tx,
                &request.order,
                &location,
                &recipe.components,
                left,
                SCRAP_REASON,
            )
            .await?;
            value += wasted.value;
            action = "wasted";
            for (code, quantity) in wasted.short {
                notes.push(format!("short {code} {quantity}"));
            }
        }
    }
    let tasks: Vec<Dispatch> = taken.away.clone();
    if !tasks.is_empty() && action == "none" {
        action = "task";
    }
    for dispatch in &tasks {
        notes.push(format!("dispatch {} already {}", dispatch.id, dispatch.status.as_str()));
    }
    let mut effect = CancelEffect {
        effect_key: request.key.clone(),
        order_id: request.order.subject_id,
        sale_line_id: request.line,
        outcome: outcome_text(request.outcome),
        quantity_milli: request.quantity_milli.unwrap_or(0),
        action: action.to_owned(),
        value,
        note: if notes.is_empty() {
            None
        } else {
            Some(notes.join("; "))
        },
        ..CancelEffect::default()
    };
    effect.insert(&mut tx).await?;
    tx.commit().await?;
    finish(state, &request, &taken, &tasks, value, scrap).await
}

/// After the commit: cancels the dispatches left without lines, posts the
/// cost of a scrap and opens the dispatcher's task for a dispatch that has
/// already left.
async fn finish(
    state: &AppState,
    request: &LineRequest,
    taken: &Taken,
    away: &[Dispatch],
    value: i64,
    scrap: bool,
) -> Result {
    for id in &taken.touched {
        let dispatch = Dispatch::find_or_404(&state.db, *id).await?;
        if dispatch.status.in_store() && lines_of(&state.db, *id).await?.is_empty() {
            cancel_by_system(state, *id, CANCEL_REASON)
                .await
                .map_err(renox::state_machine::TransitionError::into_error)?;
        }
    }
    if scrap && value > 0 {
        let cost = PostCost::new(
            request.order.clone(),
            format!("inventory.cancel_scrap:{}", request.key),
            request.order.clone(),
            value,
        )
        .cost_code(COST_CODE);
        post_cost(state, cost).await?;
    }
    for dispatch in away {
        dispatcher_task(state, dispatch, request).await?;
    }
    Ok(())
}

/// The task for the dispatcher: the cancellation arrived for a dispatch
/// that already left; the goods follow the delivery.
async fn dispatcher_task(state: &AppState, dispatch: &Dispatch, request: &LineRequest) -> Result {
    let key = format!("inventory.dispatch_cancelled:{}:{}", dispatch.id, request.line);
    let creator = tickets::task_creator(state);
    if creator.find_open(state, &key).await?.is_some() {
        return Ok(());
    }
    let number = dispatch.number();
    let line = request.line.to_string();
    let status = dispatch.status.as_str();
    let title = said("wl_inventory.dispatch.cancel_task_title", &[("dispatch", &number)]);
    let description = said(
        "wl_inventory.dispatch.cancel_task_body",
        &[("dispatch", &number), ("line", &line), ("status", status)],
    );
    let mut ticket = OpenTicket::new(dispatch.subject(), TASK_CATEGORY, &title)
        .description(&description)
        .source_key(&key);
    let dispatchers =
        users_with_permission_in(&state.db, permissions::DISPATCH_MANAGE, &dispatch.scope())
            .await?;
    if let Some(first) = dispatchers.first() {
        ticket = ticket.assign_to(Assignee::user(first.id));
    }
    creator.open(state, ticket).await?;
    Ok(())
}

/// `SalesOrderLineCancelled`: the line, its quantity and its outcome.
///
/// # Errors
///
/// A database error, or the cost poster's or the task creator's.
pub async fn on_line_cancelled(state: &AppState, event: &SalesOrderLineCancelled) -> Result {
    let outcome = event.outcome.map_or("none", LineOutcomeKind::as_str);
    apply_line(
        state,
        LineRequest {
            key: format!(
                "line_cancelled:{}:{}:{}:{}:{}",
                event.order, event.line, event.quantity_milli, outcome, event.reason
            ),
            order: event.order.subject(),
            line: event.line,
            quantity_milli: Some(event.quantity_milli),
            outcome: event.outcome,
        },
    )
    .await
}

/// `SalesOrderCancelled`: the outcome of each started line, then the drafts
/// and picked dispatches of the order that nothing named are cancelled.
///
/// # Errors
///
/// A database error, or the cost poster's or the task creator's.
pub async fn on_order_cancelled(state: &AppState, event: &SalesOrderCancelled) -> Result {
    let order = event.order.subject();
    for outcome in &event.outcomes {
        apply_line(
            state,
            LineRequest {
                key: format!(
                    "order_cancelled:{}:{}:{}",
                    event.order,
                    outcome.line,
                    outcome.kind.as_str()
                ),
                order: order.clone(),
                line: outcome.line,
                quantity_milli: None,
                outcome: Some(outcome.kind),
            },
        )
        .await?;
    }
    // What had not started: the order's dispatches still in the store are
    // not shipped.
    for dispatch in crate::dispatch::of_source(&state.db, &order).await? {
        if dispatch.status.in_store() {
            cancel_by_system(state, dispatch.id, CANCEL_REASON)
                .await
                .map_err(renox::state_machine::TransitionError::into_error)?;
        }
    }
    Ok(())
}

/// `SalesOrderAmended`: a line lowered by the amendment gives back the same
/// quantity from its dispatch lines (the reservation shrinks).
///
/// # Errors
///
/// A database error, or the task creator's.
pub async fn on_amended(state: &AppState, event: &SalesOrderAmended) -> Result {
    let order = event.order.subject();
    for line in &event.lines {
        let lowered = line.quantity_before_milli - line.quantity_after_milli;
        if lowered <= 0 {
            continue;
        }
        apply_line(
            state,
            LineRequest {
                key: format!(
                    "amended:{}:{}:{}",
                    event.order, event.revision, line.line
                ),
                order: order.clone(),
                line: line.line,
                quantity_milli: Some(lowered),
                outcome: None,
            },
        )
        .await?;
    }
    Ok(())
}

/// Registers the listeners.
pub(crate) fn register(app: &mut Registry) {
    app.listen(
        |event: SalesOrderLineCancelled, state: AppState| async move {
            on_line_cancelled(&state, &event).await
        },
    );
    app.listen(|event: SalesOrderCancelled, state: AppState| async move {
        on_order_cancelled(&state, &event).await
    });
    app.listen(|event: SalesOrderAmended, state: AppState| async move {
        on_amended(&state, &event).await
    });
}
