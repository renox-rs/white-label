//! Cancelling one line or the remaining lines of an order, with an outcome
//! for each line whose fulfilment started (wl-sales §4.2 rows `cancel_line`
//! and `cancel_remaining`, *Cancelling after fulfilment started*,
//! `SAL.ACL.05`; BL-SAL-08, BL-SAL-09). The outcome model
//! ([`WantedLine`], [`CancelledLine`], [`plan`]) is the one BL-SAL-07
//! (`cancel` of a whole order) reuses.
//!
//! **Line outcome model.** For each line cancelled the plan says how much is
//! cancelled, whether its fulfilment started (the kernel's
//! [`FulfilmentProgress`](wl_kernel::sales::FulfilmentProgress)), and for a
//! started line the outcome (*return to stock*, *finish to stock*, *scrap*,
//! limited to what the vertical offers), the cancellation fee and whether a
//! manager waived it. A started line needs an outcome; any started line needs
//! a manager's approval (`SAL.ACL.05`: the requester cannot approve, which also
//! covers *finish to stock*, *scrap* and a waived fee). The plan is pure, so
//! the service, the guards, the effect and the final re-check agree.
//!
//! **The whole order (BL-SAL-07).** `cancel` (`draft`, `pending_acceptance`,
//! `credit_hold`, `confirmed → cancelled`) takes the same plan with
//! [`CancelKind::Order`]: every open line is cancelled, nothing may be
//! fulfilled, and a started line needs an outcome. A manager approves when
//! a line started (`SAL.ACL.05`) or when the order was paid and its value is
//! above [`SalesConfig::cancel_approval_limit`] (`SAL.ACL.01`, AED 50 in 06).
//! One `SalesOrderCancelled` carries the outcome and fee of each started line.
//!
//! **Transitions.** `cancel_line` (`confirmed`, stays), `cancel_line_partly`
//! (`partially_fulfilled`, stays), `cancel_line_last` (`partially_fulfilled →
//! fulfilled` when nothing is open any more) and `cancel_remaining`
//! (`partially_fulfilled → fulfilled`), each with the `_approved` twin the
//! kernel runs at the last yes. Their effect cancels the quantity in one
//! guarded statement per line, stores the outcome and fee on the line and
//! writes the change record; the events (`SalesOrderLineCancelled`, one per
//! line) and the money (refund of the prepaid share less the fee, credit note
//! for an issued invoice, the fee document) follow the commit
//! ([`crate::order_changes`], [`crate::settle`]).
//!
//! Decisions:
//! - A cancellation that would leave nothing open on an order with nothing
//!   fulfilled is `cancel`'s (refused here); one that leaves only fulfilled
//!   quantities moves the order to `fulfilled` (`cancel_line_last`).
//! - The default fee is [`SalesConfig::cancellation_fee_bps`] of the prepaid
//!   share of the line, else the fixed
//!   [`SalesConfig::cancellation_fee_amount`] (never above the share); a user
//!   may give another amount, never above the share.
//! - The order's stored totals stay as ordered, as in the base: the money
//!   side of a cancelled line is the refund, credit note and fee document.
//! - One request per order and action waits at a time.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::sales::{LineOutcomeKind, fulfilment_progress};

use crate::amend::lines_in;
use crate::config::config_of;
use crate::order_change::{KIND_CANCEL, KIND_CANCEL_LINE, KIND_CANCEL_REMAINING, OrderChange};
use crate::permissions;
use crate::sale_line::{KIND_ITEM, SaleLine, ratio_half_up};
use crate::sales_order::{SalesOrder, SalesOrderStatus};
use crate::sales_order_machine::move_cancelled;
use crate::texts::english;

/// `confirmed → confirmed`: open quantity of one line cancelled.
pub const CANCEL_LINE: &str = "cancel_line";
/// `partially_fulfilled → partially_fulfilled`: open quantity of one line
/// cancelled, some quantity stays open.
pub const CANCEL_LINE_PARTLY: &str = "cancel_line_partly";
/// `partially_fulfilled → fulfilled`: the last open quantity of the order was
/// the line's.
pub const CANCEL_LINE_LAST: &str = "cancel_line_last";
/// `partially_fulfilled → fulfilled`: every open line cancelled.
pub const CANCEL_REMAINING: &str = "cancel_remaining";
/// `draft | pending_acceptance | credit_hold | confirmed → cancelled`: the
/// whole order (BL-SAL-07).
pub const CANCEL_ORDER: &str = "cancel";

/// The approval policies' action key of `cancel_line` after fulfilment of the
/// line started.
pub const ACTION_LINE: &str = "sales.order_cancel_line";
/// The approval policies' action key of `cancel_remaining` after fulfilment
/// of a line started.
pub const ACTION_REMAINING: &str = "sales.order_cancel_remaining";
/// The approval policies' action key of `cancel` of a whole order after
/// fulfilment of a line started, or of a paid order above the limit.
pub const ACTION_ORDER: &str = "sales.order_cancel";

/// What the user asks to cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelKind {
    /// The open quantity of one line (`cancel_line`).
    #[default]
    Line,
    /// Every open line (`cancel_remaining`).
    Remaining,
    /// The whole order (`cancel`).
    Order,
}

/// What the user chose for one line: the quantity (a line cancelled alone),
/// and for a started line its outcome and fee.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WantedLine {
    /// The sale line (its id).
    pub line: i64,
    /// The quantity to cancel, in thousandths; ignored for `cancel_remaining`,
    /// which cancels everything open.
    pub quantity_milli: i64,
    /// What happens to the started goods.
    pub outcome: Option<LineOutcomeKind>,
    /// The fee in fils; `None` takes the default of the settings.
    pub fee: Option<i64>,
    /// Waive the fee (the approval is the manager's yes).
    pub waive_fee: bool,
}

impl WantedLine {
    /// The open quantity of `line`, or `quantity_milli` of it.
    pub fn new(line: i64, quantity_milli: i64) -> Self {
        WantedLine {
            line,
            quantity_milli,
            ..WantedLine::default()
        }
    }

    /// With the outcome `kind` for the started goods.
    pub fn outcome(mut self, kind: LineOutcomeKind) -> Self {
        self.outcome = Some(kind);
        self
    }

    /// With a fee of `fils`.
    pub fn fee(mut self, fils: i64) -> Self {
        self.fee = Some(fils);
        self
    }

    /// With the fee waived.
    pub fn waived(mut self) -> Self {
        self.waive_fee = true;
        self
    }
}

/// What the user asks.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CancelAsk {
    /// One line or the remaining lines.
    pub kind: CancelKind,
    /// Why.
    pub reason: String,
    /// The lines and their choices.
    pub lines: Vec<WantedLine>,
    /// Keep what the customer paid: the cancellation requests no refund and
    /// drafts no fee document, the caller (06 `close_failed`) decides the
    /// refund itself.
    pub keep_payment: bool,
}

/// What the plan needs from outside the order's rows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanFacts {
    /// The sale lines whose fulfilment started.
    pub started_lines: Vec<i64>,
    /// The outcomes offered (stored names).
    pub offered: Vec<String>,
    /// The default fee, in basis points of the line's prepaid share.
    pub fee_bps: i64,
    /// The default fee as an amount in fils, when no percentage is set.
    pub fee_amount: i64,
    /// What the customer paid for the order, in fils (0 when nothing was
    /// paid in advance): read by the cancel of a whole order.
    pub paid: i64,
    /// A paid order above this value (fils) needs a manager to cancel it
    /// (`SAL.ACL.01`).
    pub approval_limit: i64,
}

/// One line of a cancellation, as planned.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CancelledLine {
    /// The sale line (its id).
    pub line: i64,
    /// The quantity cancelled, in thousandths.
    pub quantity_milli: i64,
    /// Whether the line's fulfilment had started.
    pub started: bool,
    /// What happens to its goods; `None` when it had not started.
    pub outcome: Option<LineOutcomeKind>,
    /// The fee on the line in fils (0 when none or waived).
    pub fee: i64,
    /// Whether the manager waived the fee.
    pub fee_waived: bool,
    /// The prepaid share of the cancelled quantity: the line with VAT times
    /// the quantity over the line's quantity, in fils.
    pub share: i64,
}

/// A cancellation checked against the order: what the guards, the effect, the
/// approval and the money read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CancelInput {
    /// What the user asked.
    pub ask: CancelAsk,
    /// What the plan was given.
    pub facts: PlanFacts,
    /// The user who cancels (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The lines as planned.
    pub lines: Vec<CancelledLine>,
    /// Whether nothing is open after it.
    pub complete: bool,
    /// Whether a manager must approve (a line started).
    pub approval: bool,
    /// The amount the approval policy reads: 1 when a manager must approve.
    pub amount: i64,
    /// The first thing wrong, `None` when nothing is.
    pub problem: Option<String>,
}

/// What the change record of a cancellation keeps (`kind` `cancel_line` or
/// `cancel_remaining`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CancelDetail {
    /// The lines cancelled.
    pub lines: Vec<CancelledLine>,
    /// Whether the order is `fulfilled` now.
    pub complete: bool,
    /// Whether the whole order was cancelled (`cancel`, BL-SAL-07).
    pub whole: bool,
    /// Whether the payment is kept: no refund is requested by the
    /// cancellation (see [`CancelAsk::keep_payment`]).
    pub keep_payment: bool,
}

impl CancelInput {
    /// The prepaid shares of all lines, in fils.
    pub fn share_total(&self) -> i64 {
        self.lines.iter().map(|l| l.share).sum()
    }

    /// The fees of all lines, in fils.
    pub fn fee_total(&self) -> i64 {
        self.lines.iter().map(|l| l.fee).sum()
    }
}

/// Keeps `text` as the problem unless one is already there.
fn first(problem: &mut Option<String>, text: String) {
    if problem.is_none() {
        *problem = Some(text);
    }
}

/// The default fee on `share` fils of a started line.
fn default_fee(facts: &PlanFacts, share: i64) -> i64 {
    if facts.fee_bps > 0 {
        ratio_half_up(share, facts.fee_bps, 10_000)
    } else {
        facts.fee_amount.min(share)
    }
}

/// The cancellation `ask` checked against the order and its `lines`. Pure.
pub fn plan(
    order: &SalesOrder,
    lines: &[SaleLine],
    ask: &CancelAsk,
    facts: &PlanFacts,
) -> CancelInput {
    let mut input = CancelInput {
        ask: ask.clone(),
        facts: facts.clone(),
        ..CancelInput::default()
    };
    input.ask.reason = ask.reason.trim().to_owned();
    let mut problem: Option<String> = None;
    let state_ok = match ask.kind {
        CancelKind::Line => matches!(
            order.status,
            SalesOrderStatus::Confirmed | SalesOrderStatus::PartiallyFulfilled
        ),
        CancelKind::Remaining => order.status == SalesOrderStatus::PartiallyFulfilled,
        CancelKind::Order => matches!(
            order.status,
            SalesOrderStatus::Draft
                | SalesOrderStatus::PendingAcceptance
                | SalesOrderStatus::CreditHold
                | SalesOrderStatus::Confirmed
        ),
    };
    if !state_ok {
        first(
            &mut problem,
            english("wl_sales.cancelling.problem.state", &[]),
        );
    }
    if input.ask.reason.is_empty() {
        first(
            &mut problem,
            english("wl_sales.cancelling.problem.reason", &[]),
        );
    }
    let items: Vec<&SaleLine> = lines.iter().filter(|l| l.kind == KIND_ITEM).collect();
    // The quantity asked of each line.
    let mut chosen: Vec<(&SaleLine, &WantedLine, i64)> = Vec::new();
    match ask.kind {
        CancelKind::Line => {
            if let [wanted] = ask.lines.as_slice() {
                match items.iter().copied().find(|l| l.id == wanted.line) {
                    None => first(
                        &mut problem,
                        english("wl_sales.orders.problem.unknown_line", &[]),
                    ),
                    Some(line)
                        if wanted.quantity_milli <= 0
                            || wanted.quantity_milli > line.open_milli() =>
                    {
                        first(
                            &mut problem,
                            english("wl_sales.orders.problem.quantity", &[]),
                        );
                    }
                    Some(line) => chosen.push((line, wanted, wanted.quantity_milli)),
                }
            } else {
                first(&mut problem, english("wl_sales.orders.problem.input", &[]));
            }
        }
        CancelKind::Remaining => {
            for line in items.iter().copied().filter(|l| l.open_milli() > 0) {
                let wanted = ask.lines.iter().find(|w| w.line == line.id);
                chosen.push((line, wanted.unwrap_or(&BLANK), line.open_milli()));
            }
            if chosen.is_empty() {
                first(
                    &mut problem,
                    english("wl_sales.cancelling.problem.nothing_open", &[]),
                );
            }
        }
        CancelKind::Order => {
            // Nothing handed over: `cancel_remaining` is for the rest.
            if items.iter().any(|l| l.fulfilled_milli > 0) {
                first(
                    &mut problem,
                    english("wl_sales.cancelling.problem.fulfilled", &[]),
                );
            }
            for line in items.iter().copied().filter(|l| l.open_milli() > 0) {
                let wanted = ask.lines.iter().find(|w| w.line == line.id);
                chosen.push((line, wanted.unwrap_or(&BLANK), line.open_milli()));
            }
        }
    }
    let mut planned = Vec::new();
    for (line, wanted, quantity) in &chosen {
        let started = facts.started_lines.contains(&line.id);
        let share = line.share_of(*quantity);
        let mut cancelled = CancelledLine {
            line: line.id,
            quantity_milli: *quantity,
            started,
            share,
            ..CancelledLine::default()
        };
        if started {
            let position = line.line_no.to_string();
            match wanted.outcome {
                None => first(
                    &mut problem,
                    english(
                        "wl_sales.cancelling.problem.outcome",
                        &[("position", &position)],
                    ),
                ),
                Some(kind) if !facts.offered.iter().any(|o| o == kind.as_str()) => first(
                    &mut problem,
                    english(
                        "wl_sales.cancelling.problem.outcome_not_offered",
                        &[("position", &position)],
                    ),
                ),
                Some(kind) => cancelled.outcome = Some(kind),
            }
            if wanted.waive_fee {
                cancelled.fee_waived = true;
            } else {
                let fee = wanted.fee.unwrap_or_else(|| default_fee(facts, share));
                if fee < 0 || fee > share {
                    first(
                        &mut problem,
                        english(
                            "wl_sales.cancelling.problem.fee",
                            &[("position", &position)],
                        ),
                    );
                } else {
                    cancelled.fee = fee;
                }
            }
        }
        planned.push(cancelled);
    }
    let open_after: i64 = items
        .iter()
        .map(|l| {
            l.open_milli()
                - planned
                    .iter()
                    .find(|p| p.line == l.id)
                    .map_or(0, |p| p.quantity_milli)
        })
        .sum();
    input.complete = ask.kind != CancelKind::Order && open_after <= 0 && !planned.is_empty();
    if ask.kind == CancelKind::Line && input.complete && order.status == SalesOrderStatus::Confirmed
    {
        // Nothing fulfilled and nothing left open: that is `cancel`.
        first(
            &mut problem,
            english("wl_sales.orders.problem.cancel_line_last", &[]),
        );
    }
    input.approval = planned.iter().any(|p| p.started)
        || (ask.kind == CancelKind::Order && facts.paid > facts.approval_limit);
    input.amount = i64::from(input.approval);
    input.lines = planned;
    input.problem = problem;
    input
}

/// The choice for a line the user said nothing about.
static BLANK: WantedLine = WantedLine {
    line: 0,
    quantity_milli: 0,
    outcome: None,
    fee: None,
    waive_fee: false,
};

/// The transition that carries out `input` on an order in `status`.
pub fn transition_for(input: &CancelInput, status: SalesOrderStatus) -> &'static str {
    match input.ask.kind {
        CancelKind::Order => CANCEL_ORDER,
        CancelKind::Remaining => CANCEL_REMAINING,
        CancelKind::Line if input.complete => CANCEL_LINE_LAST,
        CancelKind::Line if status == SalesOrderStatus::PartiallyFulfilled => CANCEL_LINE_PARTLY,
        CancelKind::Line => CANCEL_LINE,
    }
}

/// The approval action key of a cancellation of `kind`.
pub fn action_of(kind: CancelKind) -> &'static str {
    match kind {
        CancelKind::Line => ACTION_LINE,
        CancelKind::Remaining => ACTION_REMAINING,
        CancelKind::Order => ACTION_ORDER,
    }
}

/// Whether two plans cancel the same quantities of the same lines: what a
/// manager approved must be what is cancelled.
fn same_quantities(a: &[CancelledLine], b: &[CancelledLine]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.line == y.line && x.quantity_milli == y.quantity_milli)
}

/// The input of the run, if it carries one.
fn input_of(ctx: &TransitionCtx) -> Option<CancelInput> {
    ctx.input_as::<CancelInput>().ok()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The guard and the effect the four transitions and their twins share:
/// `to_complete` says whether the target is `fulfilled`.
fn rules(
    transition: Transition<SalesOrder>,
    from: &[SalesOrderStatus],
    to: SalesOrderStatus,
    kind: CancelKind,
) -> Transition<SalesOrder> {
    let to_fulfilled = to == SalesOrderStatus::Fulfilled;
    transition
        .from(from.iter().copied())
        .to(to)
        .guard("cancellation_allowed", move |_o: &SalesOrder, ctx| {
            let verdict = match input_of(ctx) {
                None => Err(english("wl_sales.orders.problem.input", &[])),
                Some(input) if input.ask.kind != kind => {
                    Err(english("wl_sales.orders.problem.input", &[]))
                }
                Some(input) if input.complete != to_fulfilled => {
                    Err(english("wl_sales.cancelling.problem.moved", &[]))
                }
                Some(input) => input.problem.map_or(Ok(()), Err),
            };
            Box::pin(async move { verdict })
        })
        .effect(move |order: &SalesOrder, ctx, tx| {
            let order = order.clone();
            let run = input_of(ctx).unwrap_or_default();
            Box::pin(async move { cancel_in(tx, order, &run, to_fulfilled).await })
        })
}

/// The ten transitions: the five of the users and their twins.
pub(crate) fn transitions() -> Vec<Transition<SalesOrder>> {
    use SalesOrderStatus::{Confirmed, Fulfilled, PartiallyFulfilled};
    let table = [
        (
            CANCEL_LINE,
            Confirmed,
            Confirmed,
            CancelKind::Line,
            "Cancel a line",
        ),
        (
            CANCEL_LINE_PARTLY,
            PartiallyFulfilled,
            PartiallyFulfilled,
            CancelKind::Line,
            "Cancel a line",
        ),
        (
            CANCEL_LINE_LAST,
            PartiallyFulfilled,
            Fulfilled,
            CancelKind::Line,
            "Cancel a line",
        ),
        (
            CANCEL_REMAINING,
            PartiallyFulfilled,
            Fulfilled,
            CancelKind::Remaining,
            "Cancel the remaining lines",
        ),
    ];
    let mut all = Vec::new();
    for (name, from, to, kind, label) in table {
        all.push(rules(
            Transition::new(name)
                .label(label)
                .tone("danger")
                .permission(permissions::SALES_ORDERS_CANCEL)
                .reason_required(),
            &[from],
            to,
            kind,
        ));
        all.push(rules(
            Transition::new(wl_kernel::machines::approved(name))
                .label(label)
                .tone("danger")
                .system(),
            &[from],
            to,
            kind,
        ));
    }
    // `cancel` of the whole order and its twin (BL-SAL-07).
    let leaves = [
        SalesOrderStatus::Draft,
        SalesOrderStatus::PendingAcceptance,
        SalesOrderStatus::CreditHold,
        Confirmed,
    ];
    all.push(rules(
        Transition::new(CANCEL_ORDER)
            .label("Cancel")
            .tone("danger")
            .permission(permissions::SALES_ORDERS_CANCEL)
            .reason_required(),
        &leaves,
        SalesOrderStatus::Cancelled,
        CancelKind::Order,
    ));
    all.push(rules(
        Transition::new(wl_kernel::machines::approved(CANCEL_ORDER))
            .label("Cancel")
            .tone("danger")
            .system(),
        &leaves,
        SalesOrderStatus::Cancelled,
        CancelKind::Order,
    ));
    all
}

/// The machine of the order with the cancel transitions added.
pub(crate) fn add_to(machine: Machine<SalesOrder>) -> Machine<SalesOrder> {
    transitions()
        .into_iter()
        // Each transition writes the order's search document (BL-AGT-11).
        .map(crate::search::reindex_order)
        .fold(machine, Machine::transition)
}

/// The effect: cancels the quantities, stores the outcome and fee on each
/// line and writes the change record the events and the money follow from.
async fn cancel_in(
    tx: &mut Transaction,
    order: SalesOrder,
    run: &CancelInput,
    to_fulfilled: bool,
) -> Result {
    let lines = lines_in(tx, order.id).await?;
    // The order may have moved since the guards looked (goods fulfilled).
    let found = plan(&order, &lines, &run.ask, &run.facts);
    if let Some(problem) = found.problem.clone() {
        return Err(refused(problem));
    }
    if found.complete != to_fulfilled || !same_quantities(&found.lines, &run.lines) {
        return Err(refused(english("wl_sales.cancelling.problem.moved", &[])));
    }
    let stamp = renox::db::now();
    for cancelled in &found.lines {
        move_cancelled(tx, order.id, cancelled.line, cancelled.quantity_milli).await?;
        let Some(line) = lines.iter().find(|l| l.id == cancelled.line) else {
            continue;
        };
        let room = (line.quantity_milli
            - line.fulfilled_milli
            - line.cancelled_milli
            - cancelled.quantity_milli)
            .max(0);
        let held = line.held_milli.min(room);
        let outcome = cancelled.outcome.map(|k| k.as_str().to_owned());
        let fee = line.cancel_fee + cancelled.fee;
        let waived = line.fee_waived || cancelled.fee_waived;
        let reason = Some(found.ask.reason.clone());
        SaleLine::query()
            .where_(SaleLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("held_milli", &held),
                    ("cancel_outcome", &outcome),
                    ("cancel_fee", &fee),
                    ("fee_waived", &waived),
                    ("cancel_reason", &reason),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
    }
    if found.complete {
        let done = Some(stamp);
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(order.id))
            .update(&mut *tx, &[("fulfilled_at", &done), ("updated_at", &stamp)])
            .await?;
    }
    let whole = found.ask.kind == CancelKind::Order;
    if whole {
        // The reason of the cancelled order, as `reject` keeps it.
        let reason = Some(found.ask.reason.clone());
        SalesOrder::query()
            .where_(SalesOrder::ID.eq(order.id))
            .update(&mut *tx, &[("reason", &reason), ("updated_at", &stamp)])
            .await?;
    }
    let kind = match found.ask.kind {
        CancelKind::Line => KIND_CANCEL_LINE,
        CancelKind::Remaining => KIND_CANCEL_REMAINING,
        CancelKind::Order => KIND_CANCEL,
    };
    let detail = CancelDetail {
        lines: found.lines.clone(),
        complete: found.complete,
        whole,
        keep_payment: found.ask.keep_payment,
    };
    OrderChange::record(tx, order.id, kind, &found.ask.reason, run.by, &detail).await?;
    Ok(())
}

/// What a cancellation did.
#[derive(Debug, Clone)]
pub enum CancelOutcome {
    /// The quantities are cancelled.
    Done(Box<SalesOrder>),
    /// A manager must approve first: nothing changed yet.
    Waiting(Box<ApprovalRequest>),
}

/// The facts the plan needs from the settings and the vertical.
async fn facts_of(state: &AppState, order: &SalesOrder, lines: &[SaleLine]) -> Result<PlanFacts> {
    let config = config_of(state);
    let ids: Vec<i64> = lines
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| l.id)
        .collect();
    let started_lines = fulfilment_progress(state)
        .started_lines(state, order.order_id(), &ids)
        .await?;
    Ok(PlanFacts {
        started_lines,
        offered: config.cancel_outcomes.clone(),
        fee_bps: config.cancellation_fee_bps,
        fee_amount: config.cancellation_fee_amount,
        paid: if crate::settle::is_prepaid(order) {
            order.gross_total
        } else {
            0
        },
        approval_limit: config.cancel_approval_limit,
    })
}

/// Plans and carries out (or asks approval for) the cancellation `ask`.
async fn run(
    state: &AppState,
    user: &AuthUser,
    order_id: i64,
    ask: CancelAsk,
) -> Result<CancelOutcome> {
    if !user.has_permission(permissions::SALES_ORDERS_CANCEL) {
        return Err(Error::Forbidden);
    }
    let order = SalesOrder::find_or_404(&state.db, order_id).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let facts = facts_of(state, &order, &lines).await?;
    let mut found = plan(&order, &lines, &ask, &facts);
    found.by = Some(user.id);
    if let Some(problem) = found.problem.clone() {
        return Err(refused(problem));
    }
    let name = transition_for(&found, order.status);
    let reason = found.ask.reason.clone();
    if let Some(request) = crate::order_changes::request_or_run(
        state,
        user,
        &order,
        action_of(found.ask.kind),
        name,
        &found,
        found.approval,
        &reason,
    )
    .await?
    {
        return Ok(CancelOutcome::Waiting(request));
    }
    Ok(CancelOutcome::Done(Box::new(
        SalesOrder::find_or_404(&state.db, order_id).await?,
    )))
}

/// Cancels `quantity_milli` of the open quantity of the item line `line` of
/// the order `id` as `user`, with `reason` (`cancel_line`). A line whose
/// fulfilment started needs `choice` with an outcome and a manager's approval
/// (`SAL.ACL.05`); the order becomes `fulfilled` when nothing is open.
///
/// # Errors
///
/// 403 without `sales_orders.cancel`; 422 when the order is not `confirmed`
/// or `partially_fulfilled`, the line is not an item of the order, the quantity
/// is not above zero or exceeds what is open, the reason is blank, nothing
/// would stay open on an order with nothing fulfilled (that is `cancel`), a
/// started line has no outcome, or a request for the order is already waiting.
pub async fn cancel_line(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    line: i64,
    quantity_milli: i64,
    reason: &str,
    choice: Option<WantedLine>,
) -> Result<CancelOutcome> {
    let mut wanted = choice.unwrap_or_default();
    wanted.line = line;
    wanted.quantity_milli = quantity_milli;
    run(
        state,
        user,
        id,
        CancelAsk {
            kind: CancelKind::Line,
            reason: reason.to_owned(),
            lines: vec![wanted],
            keep_payment: false,
        },
    )
    .await
}

/// Cancels every open line of the `partially_fulfilled` order `id` as `user`
/// with `reason` (`cancel_remaining`); `choices` give the outcome and fee of
/// the lines whose fulfilment started, which need a manager's approval.
///
/// # Errors
///
/// 403 without `sales_orders.cancel`; 422 when the order is not
/// `partially_fulfilled`, nothing is open, the reason is blank, a started line
/// has no outcome, or a request for the order is already waiting.
pub async fn cancel_remaining(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
    choices: Vec<WantedLine>,
) -> Result<CancelOutcome> {
    run(
        state,
        user,
        id,
        CancelAsk {
            kind: CancelKind::Remaining,
            reason: reason.to_owned(),
            lines: choices,
            keep_payment: false,
        },
    )
    .await
}

/// Cancels the whole order `id` as `user` with `reason` (`cancel`). Every
/// line whose fulfilment started needs an outcome in `choices` (and a
/// manager's approval, `SAL.ACL.05`); a paid order above the setting
/// [`SalesConfig::cancel_approval_limit`] needs the approval too
/// (`SAL.ACL.01`). Nothing may be fulfilled: `cancel_remaining` is for that.
///
/// # Errors
///
/// 403 without `sales_orders.cancel`; 422 when the order is not in a state
/// `cancel` leaves, something was fulfilled, the reason is blank, a started
/// line has no outcome, or a request for the order is already waiting.
pub async fn cancel_order(
    state: &AppState,
    user: &AuthUser,
    id: i64,
    reason: &str,
    choices: Vec<WantedLine>,
) -> Result<CancelOutcome> {
    run(
        state,
        user,
        id,
        CancelAsk {
            kind: CancelKind::Order,
            reason: reason.to_owned(),
            lines: choices,
            keep_payment: false,
        },
    )
    .await
}

/// Cancels the whole order `id` as the system, with no approval: the
/// vertical decided it (06 `close_failed` after a failed own-rider delivery,
/// BL-FNB-02). `choices` give the outcome of every line whose fulfilment
/// started; `keep_payment` leaves the refund to the caller. The `_approved`
/// twin of `cancel` runs, so the events, the stock outcome and the change
/// record are the ones of any cancellation.
///
/// # Errors
///
/// 422 when the order is not in a state `cancel` leaves, something was
/// fulfilled, a started line has no outcome, or the reason is blank.
pub async fn cancel_as_system(
    state: &AppState,
    id: i64,
    reason: &str,
    choices: Vec<WantedLine>,
    keep_payment: bool,
    by: Option<i64>,
) -> Result<SalesOrder> {
    let order = SalesOrder::find_or_404(&state.db, id).await?;
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let facts = facts_of(state, &order, &lines).await?;
    let ask = CancelAsk {
        kind: CancelKind::Order,
        reason: reason.to_owned(),
        lines: choices,
        keep_payment,
    };
    let mut found = plan(&order, &lines, &ask, &facts);
    found.by = by;
    if let Some(problem) = found.problem.clone() {
        return Err(refused(problem));
    }
    let name = wl_kernel::machines::approved(CANCEL_ORDER);
    let _ = order
        .transition(&name)
        .by_system()
        .reason(reason.to_owned())
        .input(&found)
        .run(state)
        .await
        .map_err(|e| e.into_error())?;
    SalesOrder::find_or_404(&state.db, id).await
}

/// The final check of a request for a cancellation: before the approved
/// transition runs, the plan is made again on the order as it is; the
/// request fails when the quantities are no longer open or the order's target
/// state changed (`approval failed: ...`).
///
/// # Errors
///
/// A database error.
pub async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    let asked: CancelInput =
        renox::serde_json::from_value(request.payload.0.clone()).unwrap_or_default();
    let order = SalesOrder::find_or_404(&mut *tx, request.subject_id).await?;
    let lines = lines_in(tx, order.id).await?;
    let found = plan(&order, &lines, &asked.ask, &asked.facts);
    if found.problem.is_some() {
        return Ok(found.problem);
    }
    if found.complete != asked.complete || !same_quantities(&found.lines, &asked.lines) {
        return Ok(Some(english("wl_sales.cancelling.problem.moved", &[])));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(status: SalesOrderStatus) -> SalesOrder {
        SalesOrder {
            id: 1,
            status,
            ..SalesOrder::default()
        }
    }

    fn line(id: i64, quantity: i64, price: i64) -> SaleLine {
        SaleLine {
            id,
            line_no: id,
            kind: KIND_ITEM.to_owned(),
            quantity_milli: quantity,
            unit_price: price,
            vat_category: wl_kernel::invoicing::VatCategory::ZeroRated,
            ..SaleLine::default()
        }
    }

    fn facts(started: &[i64]) -> PlanFacts {
        PlanFacts {
            started_lines: started.to_vec(),
            offered: vec!["return_to_stock".into(), "scrap".into()],
            fee_bps: 0,
            fee_amount: 0,
            ..PlanFacts::default()
        }
    }

    fn ask(kind: CancelKind, lines: Vec<WantedLine>) -> CancelAsk {
        CancelAsk {
            kind,
            reason: "out of stock".into(),
            lines,
            keep_payment: false,
        }
    }

    #[test]
    fn two_units_of_an_unstarted_line_are_cancelled_with_their_prepaid_share() {
        let found = plan(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 5_000, 2_000), line(2, 1_000, 500)],
            &ask(CancelKind::Line, vec![WantedLine::new(1, 2_000)]),
            &facts(&[]),
        );
        assert_eq!(found.problem, None);
        assert!(!found.approval && !found.complete);
        assert_eq!(found.lines[0].share, 4_000);
        assert_eq!(found.lines[0].outcome, None);
        assert_eq!(
            transition_for(&found, SalesOrderStatus::Confirmed),
            CANCEL_LINE
        );
    }

    #[test]
    fn a_started_line_needs_an_outcome_and_a_manager() {
        let lines = [line(1, 5_000, 2_000)];
        let no_outcome = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &[line(1, 5_000, 2_000), line(2, 1_000, 500)],
            &ask(CancelKind::Line, vec![WantedLine::new(1, 2_000)]),
            &facts(&[1]),
        );
        assert!(no_outcome.problem.is_some());
        let with = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &[line(1, 5_000, 2_000), line(2, 1_000, 500)],
            &ask(
                CancelKind::Line,
                vec![WantedLine::new(1, 2_000).outcome(LineOutcomeKind::Scrap)],
            ),
            &facts(&[1]),
        );
        assert_eq!(with.problem, None);
        assert!(with.approval);
        assert_eq!(with.lines[0].outcome, Some(LineOutcomeKind::Scrap));
        let not_offered = plan(
            &order(SalesOrderStatus::Confirmed),
            &lines,
            &ask(
                CancelKind::Line,
                vec![WantedLine::new(1, 2_000).outcome(LineOutcomeKind::FinishToStock)],
            ),
            &facts(&[1]),
        );
        assert!(not_offered.problem.is_some());
    }

    #[test]
    fn the_fee_defaults_to_the_settings_and_never_exceeds_the_share() {
        let mut f = facts(&[1]);
        f.fee_bps = 1000;
        let found = plan(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 1_000, 8_000), line(2, 1_000, 4_000)],
            &ask(
                CancelKind::Line,
                vec![WantedLine::new(1, 1_000).outcome(LineOutcomeKind::Scrap)],
            ),
            &f,
        );
        assert_eq!(found.lines[0].fee, 800);
        let too_high = plan(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 1_000, 8_000), line(2, 1_000, 4_000)],
            &ask(
                CancelKind::Line,
                vec![
                    WantedLine::new(1, 1_000)
                        .outcome(LineOutcomeKind::Scrap)
                        .fee(9_000),
                ],
            ),
            &f,
        );
        assert!(too_high.problem.is_some());
        let waived = plan(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 1_000, 8_000), line(2, 1_000, 4_000)],
            &ask(
                CancelKind::Line,
                vec![
                    WantedLine::new(1, 1_000)
                        .outcome(LineOutcomeKind::Scrap)
                        .waived(),
                ],
            ),
            &f,
        );
        assert_eq!((waived.lines[0].fee, waived.lines[0].fee_waived), (0, true));
    }

    #[test]
    fn the_last_open_line_of_a_partly_fulfilled_order_completes_it() {
        let mut fulfilled = line(1, 3_000, 1_000);
        fulfilled.fulfilled_milli = 3_000;
        let found = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &[fulfilled, line(2, 2_000, 1_000)],
            &ask(CancelKind::Line, vec![WantedLine::new(2, 2_000)]),
            &facts(&[]),
        );
        assert_eq!(found.problem, None);
        assert!(found.complete);
        assert_eq!(
            transition_for(&found, SalesOrderStatus::PartiallyFulfilled),
            CANCEL_LINE_LAST
        );
    }

    #[test]
    fn all_the_quantity_of_the_only_line_of_a_confirmed_order_is_refused() {
        let found = plan(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 3_000, 1_000)],
            &ask(CancelKind::Line, vec![WantedLine::new(1, 3_000)]),
            &facts(&[]),
        );
        assert!(found.problem.is_some());
    }

    #[test]
    fn the_remaining_lines_are_cancelled_whole_and_only_started_ones_need_an_outcome() {
        let mut done = line(1, 2_000, 1_000);
        done.fulfilled_milli = 2_000;
        let lines = [done, line(2, 3_000, 1_000), line(3, 1_000, 1_000)];
        let missing = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &lines,
            &ask(CancelKind::Remaining, Vec::new()),
            &facts(&[2]),
        );
        assert!(missing.problem.is_some());
        let found = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &lines,
            &ask(
                CancelKind::Remaining,
                vec![WantedLine::new(2, 0).outcome(LineOutcomeKind::ReturnToStock)],
            ),
            &facts(&[2]),
        );
        assert_eq!(found.problem, None);
        assert!(found.complete && found.approval);
        let quantities: Vec<(i64, i64)> = found
            .lines
            .iter()
            .map(|l| (l.line, l.quantity_milli))
            .collect();
        assert_eq!(quantities, [(2, 3_000), (3, 1_000)]);
        let plain = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &lines,
            &ask(CancelKind::Remaining, Vec::new()),
            &facts(&[]),
        );
        assert_eq!(plain.problem, None);
        assert!(!plain.approval);
        let wrong_state = plan(
            &order(SalesOrderStatus::Confirmed),
            &lines,
            &ask(CancelKind::Remaining, Vec::new()),
            &facts(&[]),
        );
        assert!(wrong_state.problem.is_some());
    }

    #[test]
    fn a_paid_order_above_the_limit_or_with_a_started_line_needs_a_manager() {
        let lines = [line(1, 2_000, 4_000), line(2, 1_000, 4_000)];
        let mut f = facts(&[]);
        f.approval_limit = 5_000;
        f.paid = 12_000;
        let paid = plan(
            &order(SalesOrderStatus::Confirmed),
            &lines,
            &ask(CancelKind::Order, Vec::new()),
            &f,
        );
        assert_eq!(paid.problem, None);
        assert!(paid.approval && !paid.complete);
        assert_eq!(paid.lines.len(), 2);
        assert_eq!(
            transition_for(&paid, SalesOrderStatus::Confirmed),
            CANCEL_ORDER
        );
        f.paid = 3_000;
        let small = plan(
            &order(SalesOrderStatus::Confirmed),
            &lines,
            &ask(CancelKind::Order, Vec::new()),
            &f,
        );
        assert!(!small.approval);
        f.paid = 0;
        f.started_lines = vec![1];
        let started = plan(
            &order(SalesOrderStatus::Confirmed),
            &lines,
            &ask(CancelKind::Order, Vec::new()),
            &f,
        );
        assert!(started.problem.is_some(), "a started line needs an outcome");
        let with = plan(
            &order(SalesOrderStatus::Confirmed),
            &lines,
            &ask(
                CancelKind::Order,
                vec![WantedLine::new(1, 0).outcome(LineOutcomeKind::Scrap)],
            ),
            &f,
        );
        assert_eq!(with.problem, None);
        assert!(with.approval);
    }

    #[test]
    fn an_order_with_anything_fulfilled_is_not_cancelled_as_a_whole() {
        let mut done = line(1, 2_000, 1_000);
        done.fulfilled_milli = 500;
        let found = plan(
            &order(SalesOrderStatus::Confirmed),
            &[done],
            &ask(CancelKind::Order, Vec::new()),
            &facts(&[]),
        );
        assert!(found.problem.is_some());
        let partly = plan(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &[line(1, 1_000, 1_000)],
            &ask(CancelKind::Order, Vec::new()),
            &facts(&[]),
        );
        assert!(partly.problem.is_some());
    }

    #[test]
    fn the_ten_transitions_are_declared_with_their_twins() {
        let machine = SalesOrder::machine();
        for (name, from, to) in [
            (CANCEL_LINE, "confirmed", "confirmed"),
            (
                CANCEL_LINE_PARTLY,
                "partially_fulfilled",
                "partially_fulfilled",
            ),
            (CANCEL_LINE_LAST, "partially_fulfilled", "fulfilled"),
            (CANCEL_REMAINING, "partially_fulfilled", "fulfilled"),
        ] {
            let user = machine.find(name).expect(name);
            assert!(!user.is_system() && user.needs_reason(), "{name}");
            assert_eq!(user.from_states(), [from], "{name}");
            assert_eq!(user.to_state(), Some(to), "{name}");
            let twin = machine
                .find(&wl_kernel::machines::approved(name))
                .expect(name);
            assert!(twin.is_system(), "{name}");
            assert_eq!(twin.to_state(), Some(to), "{name}");
        }
    }
}
