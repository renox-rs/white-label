//! Changing a placed or accepted order when an item runs out (06 F&B §4.1 and
//! §4.2 rows *An item runs out after acceptance*, §7.1 `amend` and
//! `cancel_line`; BL-FNB-05).
//!
//! - **Amend** (`orders.amend`, [`amend`]): the cashier calls the customer and
//!   substitutes an item or changes a quantity or price on a `placed` or
//!   `accepted` direct order, with the "customer agreed" tick and the time of
//!   the call. It is the sales order's `amend`, a new revision: the price
//!   difference is refunded, or charged through a payment link (the kitchen
//!   does not wait for the link). A line the kitchen started cannot be changed.
//!   Aggregator orders are changed on the aggregator's tablet, never here.
//! - **Cancel line** (`orders.cancel`, [`cancel_line`]): an item ran out or the
//!   customer asked. The sales order's `cancel_line` refunds a paid line, posts
//!   the food of a started line as wastage (with an outcome and a manager,
//!   `SAL.ACL.05`) and refuses the last open line (that is `cancel`, BL-FNB-04).
//!   A bumped line cannot be cancelled. On an aggregator order the line is
//!   recorded with no refund and flagged for the payout reconciliation
//!   ([`ExpectedRefund`]).
//! - **The kitchen follows** ([`on_amended`], [`on_line_cancelled`]): on the
//!   sales order's events the tickets are brought in line at once: a changed
//!   quantity is changed on its queued ticket, a dropped or cancelled line's
//!   ticket is voided, an added line gets a queued ticket, and an order whose
//!   open tickets are all gone is `ready` ([`crate::kitchen::sync_tickets`]).
//!
//! A change that waits for a manager is recorded as `waiting` and changes
//! nothing until the yes; the events then do the same.

use renox::chrono::NaiveTime;
use renox::db::DateTime;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::sales::{
    AmendAdd, AmendChange, AmendOrder, CancelLine, ChangeOutcome, LineOutcomeKind, OrderView,
    SalesOrderAmended, SalesOrderLineCancelled, order_desk,
};

use crate::money::parse_dirhams;
use crate::order::FnbOrder;
use crate::order::FnbOrderStatus;
use crate::order_machine::{
    AMEND, AMEND_PLACED, CANCEL_LINE, CANCEL_LINE_PREPARING, LINE_REASONS, OrderInput,
};
use crate::permissions;
use crate::placing::refused;
use crate::progress;
use crate::texts::english;
use crate::ticket::{KitchenTicket, TicketStatus};

/// A change a person made to an order: the record of who, why and how it
/// stands.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_order_changes", index(order_id))]
pub struct ChangeNote {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "fnb_orders")]
    pub order_id: i64,
    /// `amend` or `cancel_line`.
    pub kind: String,
    /// The sale line cancelled, or the line an amendment replaced.
    pub sale_line_id: Option<i64>,
    /// The reason's code.
    pub reason_code: String,
    /// The reason in words.
    pub reason: String,
    /// The "customer agreed" tick of an amendment.
    pub customer_agreed: bool,
    /// The time of the call to the customer, `HH:MM`.
    pub call_time: Option<String>,
    /// The user who made the change.
    pub requested_by: Option<i64>,
    /// The approval request while a manager's yes is awaited.
    pub approval_request_id: Option<i64>,
    /// `done` or `waiting`.
    pub status: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The reasons a line is cancelled, as the sheet offers them.
pub const LINE_REASON_CODES: [&str; 2] = LINE_REASONS;

/// The change is made.
pub const DONE: &str = "done";
/// The change waits for a manager.
pub const WAITING: &str = "waiting";
/// The kind of an amendment.
pub const KIND_AMEND: &str = "amend";
/// The kind of a cancelled line.
pub const KIND_CANCEL_LINE: &str = "cancel_line";

impl ChangeNote {
    /// The changes of `order_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_order(db: &Db, order_id: i64) -> Result<Vec<ChangeNote>> {
        ChangeNote::query()
            .where_(ChangeNote::ORDER_ID.eq(order_id))
            .order_by(ChangeNote::ID)
            .get(db)
            .await
    }
}

/// The refund an aggregator is expected to make for a line cancelled on its
/// order: the payout reconciliation lists a flagged line with no matching
/// refund as an unmatched difference (`FNB.RUL.10`, BL-FNB-10).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "fnb_expected_refunds", index(order_id))]
pub struct ExpectedRefund {
    /// The primary key.
    pub id: i64,
    /// The order.
    #[model(references = "fnb_orders")]
    pub order_id: i64,
    /// The sale line cancelled.
    pub sale_line_id: Option<i64>,
    /// What the aggregator is expected to refund, with VAT, in fils.
    pub amount: i64,
    /// Why the line was cancelled.
    pub reason: String,
    /// When the payout statement matched it.
    pub matched_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ExpectedRefund {
    /// The expected refunds of `order_id`, oldest first.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub async fn of_order(db: &Db, order_id: i64) -> Result<Vec<ExpectedRefund>> {
        ExpectedRefund::query()
            .where_(ExpectedRefund::ORDER_ID.eq(order_id))
            .order_by(ExpectedRefund::ID)
            .get(db)
            .await
    }
}

/// What the cashier asks when amending an order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AmendAsk {
    /// The sale line that changes or is replaced.
    pub line: i64,
    /// The code of the item that takes the line's place; blank to change the
    /// line itself.
    pub new_item: String,
    /// The quantity the line, or the new item, has (units); blank keeps the
    /// line's open quantity.
    pub quantity: String,
    /// The price of one unit before VAT, in dirhams; blank keeps the line's
    /// price, or takes the menu's for a new item.
    pub price: String,
    /// Why, in words.
    pub reason: String,
    /// The "customer agreed" tick.
    pub customer_agreed: bool,
    /// The time of the call, `HH:MM`.
    pub call_time: String,
}

/// What the cashier asks when cancelling a line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineAsk {
    /// The sale line.
    pub line: i64,
    /// `item_unavailable` or `customer_request`.
    pub reason_code: String,
    /// A note.
    pub note: String,
    /// The outcome of the line when the kitchen started it:
    /// `return_to_stock` or `scrap`.
    pub outcome: String,
}

/// The line `id` of `view`, when it is an item with something open.
fn open_line(view: &OrderView, id: i64) -> Option<&wl_kernel::sales::OrderLineView> {
    view.lines
        .iter()
        .find(|line| line.id == id && line.is_item && line.open_milli() > 0)
}

/// Whole units asked for as thousandths; `None` when not a whole number above
/// zero (a plate is not cut in parts).
fn units_to_milli(text: &str) -> Option<i64> {
    let units: i64 = text.trim().parse().ok()?;
    if units <= 0 {
        return None;
    }
    units.checked_mul(1_000)
}

/// 422 unless `text` is a time of day, `HH:MM`.
fn ensure_call_time(text: &str) -> Result<String> {
    let time = text.trim();
    if NaiveTime::parse_from_str(time, "%H:%M").is_err() {
        return Err(refused(english("fnb.amend.call_time", &[])));
    }
    Ok(time.to_owned())
}

/// Stores the record of a change.
#[allow(clippy::too_many_arguments)]
async fn record(
    state: &AppState,
    order: &FnbOrder,
    kind: &str,
    line: i64,
    code: &str,
    reason: &str,
    agreed: bool,
    call_time: Option<String>,
    by: i64,
    outcome: &ChangeOutcome,
) -> Result {
    let (status, request) = match outcome {
        ChangeOutcome::Done => (DONE, None),
        ChangeOutcome::Waiting { request } => (WAITING, Some(*request)),
    };
    let mut note = ChangeNote {
        order_id: order.id,
        kind: kind.to_owned(),
        sale_line_id: Some(line),
        reason_code: code.to_owned(),
        reason: reason.to_owned(),
        customer_agreed: agreed,
        call_time,
        requested_by: Some(by),
        approval_request_id: request,
        status: status.to_owned(),
        ..ChangeNote::default()
    };
    note.insert(&state.db).await?;
    Ok(())
}

/// Amends the order `id` as `user`: `ask.line` is changed, or replaced by
/// `ask.new_item`. Returns whether it is made or waits for a manager (a line
/// the kitchen started elsewhere on the order, `SAL.ACL.05`).
///
/// # Errors
///
/// `Forbidden` without `orders.amend`; 422 for an order that is not `placed`
/// or `accepted`, an aggregator's order, a missing "customer agreed" tick or
/// call time, a reason left blank, a line that is unknown or already started,
/// a quantity or price that is not valid, and the sales module's refusals.
pub async fn amend(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    ask: AmendAsk,
) -> Result<ChangeOutcome> {
    if !user.has_permission(permissions::ORDERS_AMEND) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let name = match order.status {
        FnbOrderStatus::Placed => AMEND_PLACED,
        FnbOrderStatus::Accepted => AMEND,
        _ => return Err(refused(english("fnb.amend.state", &[]))),
    };
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Err(Error::NotFound);
    };
    if view.aggregator {
        return Err(refused(english("fnb.amend.aggregator", &[])));
    }
    if !ask.customer_agreed {
        return Err(refused(english("fnb.amend.agreed", &[])));
    }
    let call_time = ensure_call_time(&ask.call_time)?;
    let reason = ask.reason.trim().to_owned();
    if reason.is_empty() {
        return Err(refused(english("fnb.amend.reason", &[])));
    }
    let Some(line) = open_line(&view, ask.line) else {
        return Err(refused(english("fnb.amend.line", &[])));
    };
    let started = progress::started(&state.db, &order, &[line.id]).await?;
    if !started.is_empty() {
        return Err(refused(english("fnb.amend.started", &[])));
    }
    let quantity = if ask.quantity.trim().is_empty() {
        line.open_milli()
    } else {
        units_to_milli(&ask.quantity)
            .ok_or_else(|| refused(english("fnb.amend.quantity", &[])))?
    };
    let price = if ask.price.trim().is_empty() {
        None
    } else {
        Some(
            parse_dirhams(&ask.price)
                .filter(|fils| *fils >= 0)
                .ok_or_else(|| refused(english("fnb.amend.price", &[])))?,
        )
    };
    let new_item = ask.new_item.trim().to_owned();
    let mut request = AmendOrder {
        order: order.sales_order(),
        reason: reason.clone(),
        seen_revision: view.revision,
        changes: Vec::new(),
        added: Vec::new(),
    };
    if new_item.is_empty() {
        request.changes.push(AmendChange {
            line: line.id,
            quantity_milli: quantity,
            unit_price: price.unwrap_or(line.unit_price),
        });
    } else {
        // The line is dropped and the substitute takes its place.
        request.changes.push(AmendChange {
            line: line.id,
            quantity_milli: 0,
            unit_price: line.unit_price,
        });
        let menu = crate::catalogue::MenuItem::by_code(&state.db, &new_item).await?;
        let (description, listed) = match &menu {
            Some(item) => (item.name.clone(), item.price),
            None => (new_item.clone(), 0),
        };
        let unit_price = price.unwrap_or(listed);
        if unit_price <= 0 {
            return Err(refused(english("fnb.amend.price", &[])));
        }
        request.added.push(AmendAdd {
            item_code: new_item,
            description,
            quantity_milli: quantity,
            unit_price,
            // A substitute is taxed as the item it replaces.
            vat_like: Some(line.id),
        });
    }
    let outcome = order_desk(state).amend(state, user, request).await?;
    let text = english(
        "fnb.amend.history",
        &[
            ("reason", &reason),
            ("time", &call_time),
        ],
    );
    record(
        state,
        &order,
        KIND_AMEND,
        line.id,
        "amend",
        &reason,
        true,
        Some(call_time),
        user.id,
        &outcome,
    )
    .await?;
    if outcome == ChangeOutcome::Done {
        run_if_unmoved(state, &order, name, user, &text, OrderInput {
            aggregator: false,
            customer_agreed: true,
            started_changed: false,
            ..OrderInput::default()
        })
        .await?;
    }
    Ok(outcome)
}

/// Cancels the line `ask.line` of the order `id` as `user`.
///
/// # Errors
///
/// `Forbidden` without `orders.cancel`; 422 for an order that is not
/// `accepted` or `preparing`, a reason other than *item unavailable* or
/// *customer request*, an unknown line, a bumped line, the last open line
/// (cancel the order), a started line with no outcome, and the sales module's
/// refusals.
pub async fn cancel_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    ask: LineAsk,
) -> Result<ChangeOutcome> {
    if !user.has_permission(permissions::ORDERS_CANCEL) {
        return Err(Error::Forbidden);
    }
    let order = FnbOrder::find_or_404(&state.db, id).await?;
    let name = match order.status {
        FnbOrderStatus::Accepted => CANCEL_LINE,
        FnbOrderStatus::Preparing => CANCEL_LINE_PREPARING,
        _ => return Err(refused(english("fnb.line.state", &[]))),
    };
    let code = ask.reason_code.trim().to_owned();
    if !LINE_REASONS.contains(&code.as_str()) {
        return Err(refused(english("fnb.line.reason", &[])));
    }
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Err(Error::NotFound);
    };
    let Some(line) = open_line(&view, ask.line) else {
        return Err(refused(english("fnb.amend.line", &[])));
    };
    let tickets: Vec<KitchenTicket> = KitchenTicket::of_order(&state.db, order.id)
        .await?
        .into_iter()
        .filter(|ticket| ticket.sale_line_id == line.id)
        .collect();
    let bumped = tickets.iter().any(|ticket| ticket.status == TicketStatus::Done);
    if bumped {
        return Err(refused(english("fnb.line.bumped", &[])));
    }
    let others = view
        .lines
        .iter()
        .filter(|other| other.is_item && other.id != line.id && other.open_milli() > 0)
        .count();
    if others == 0 {
        return Err(refused(english("fnb.line.last", &[])));
    }
    let started = tickets.iter().any(|ticket| ticket.status.is_started());
    let outcome_kind = if started {
        match LineOutcomeKind::parse(&ask.outcome).filter(|kind| {
            matches!(kind, LineOutcomeKind::ReturnToStock | LineOutcomeKind::Scrap)
        }) {
            Some(kind) => Some(kind),
            None => {
                return Err(refused(english(
                    "fnb.cancel.outcome_needed",
                    &[("item", &line.description)],
                )));
            }
        }
    } else {
        None
    };
    let label = english(&format!("fnb.line.reason.{code}"), &[]);
    let text = if ask.note.trim().is_empty() {
        label
    } else {
        format!("{label}: {}", ask.note.trim())
    };
    let outcome = order_desk(state)
        .cancel_line(
            state,
            user,
            CancelLine {
                order: order.sales_order(),
                line: line.id,
                quantity_milli: 0,
                reason: text.clone(),
                outcome: outcome_kind,
            },
        )
        .await?;
    record(
        state,
        &order,
        KIND_CANCEL_LINE,
        line.id,
        &code,
        &text,
        false,
        None,
        user.id,
        &outcome,
    )
    .await?;
    if outcome == ChangeOutcome::Done {
        run_if_unmoved(state, &order, name, user, &text, OrderInput {
            line_reason_code: Some(code),
            line_bumped: false,
            ..OrderInput::default()
        })
        .await?;
    }
    Ok(outcome)
}

/// Runs the same-state transition `name` as `user`, for the history of the
/// order, unless the kitchen moved the order meanwhile (the last open ticket
/// of a cancelled line was voided and the order became `ready`): the change
/// record keeps who and why then.
async fn run_if_unmoved(
    state: &AppState,
    before: &FnbOrder,
    name: &str,
    user: &AuthUser,
    reason: &str,
    input: OrderInput,
) -> Result {
    let now = FnbOrder::find_or_404(&state.db, before.id).await?;
    if now.status != before.status {
        return Ok(());
    }
    let _outcome = now
        .transition(name)
        .by(user)
        .reason(reason.to_owned())
        .input(&input)
        .run(state)
        .await
        .map_err(TransitionError::into_error)?;
    Ok(())
}

/// Marks the waiting note of `kind` for `order_id` done: the manager said yes.
async fn settle_waiting(state: &AppState, order_id: i64, kind: &str) -> Result {
    let done = DONE.to_owned();
    ChangeNote::query()
        .where_(ChangeNote::ORDER_ID.eq(order_id))
        .where_(ChangeNote::KIND.eq(kind.to_owned()))
        .where_(ChangeNote::STATUS.eq(WAITING.to_owned()))
        .update(&state.db, &[("status", &done)])
        .await?;
    Ok(())
}

/// What is still to be made, paid and delivered: the open quantity of each
/// item line with VAT, in fils. A cancelled line leaves the sales order's
/// stored totals as ordered (the refund is its money), so the delivery of a
/// cash order collects this, not the ordered total.
pub(crate) fn open_total(view: &OrderView) -> i64 {
    view.lines
        .iter()
        .filter(|line| line.is_item && line.quantity_milli > 0)
        .map(|line| {
            let share = i128::from(line.gross) * i128::from(line.open_milli())
                / i128::from(line.quantity_milli);
            i64::try_from(share).unwrap_or(line.gross)
        })
        .sum()
}

/// The order's total follows its lines after a change, and what was paid in
/// advance never exceeds it (the difference was refunded).
async fn refresh_money(state: &AppState, order: &FnbOrder) -> Result {
    let Some(view) = order_desk(state).find(state, order.sales_order()).await? else {
        return Ok(());
    };
    let total = open_total(&view);
    let paid = order.paid_amount.min(total);
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(order.id))
        .update(&state.db, &[("total", &total), ("paid_amount", &paid)])
        .await?;
    Ok(())
}

/// `SalesOrderAmended`: the kitchen's tickets follow the new revision.
pub(crate) async fn on_amended(state: &AppState, event: SalesOrderAmended) -> Result {
    let Some(order) = FnbOrder::of_sales_order(&state.db, event.order.0).await? else {
        return Ok(());
    };
    settle_waiting(state, order.id, KIND_AMEND).await?;
    refresh_money(state, &order).await?;
    crate::kitchen::sync_tickets(state, order.id).await
}

/// `SalesOrderLineCancelled`: the line's ticket is voided (a bumped one stays
/// `done`), the quantity of a partly cancelled one shrinks, and a line
/// cancelled on an aggregator's order is flagged for the payout
/// reconciliation.
pub(crate) async fn on_line_cancelled(state: &AppState, event: SalesOrderLineCancelled) -> Result {
    let Some(order) = FnbOrder::of_sales_order(&state.db, event.order.0).await? else {
        return Ok(());
    };
    settle_waiting(state, order.id, KIND_CANCEL_LINE).await?;
    if let Some(view) = order_desk(state).find(state, order.sales_order()).await?
        && view.aggregator
        && let Some(line) = view.lines.iter().find(|line| line.id == event.line)
        && line.quantity_milli > 0
    {
        let share = i64::try_from(
            i128::from(line.gross) * i128::from(event.quantity_milli)
                / i128::from(line.quantity_milli),
        )
        .unwrap_or(line.gross);
        let mut expected = ExpectedRefund {
            order_id: order.id,
            sale_line_id: Some(event.line),
            amount: share,
            reason: event.reason.clone(),
            ..ExpectedRefund::default()
        };
        expected.insert(&state.db).await?;
    }
    refresh_money(state, &order).await?;
    crate::kitchen::sync_tickets(state, order.id).await
}
