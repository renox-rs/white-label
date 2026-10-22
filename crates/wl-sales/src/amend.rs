//! Amending a placed order as a new revision (wl-sales §3 *revision*, §4.2
//! row `amend`, §5.7 steps 1-2, `SAL.ACL.02`, `SAL.ACL.05`; BL-SAL-05).
//!
//! A holder of `sales_orders.amend` changes the quantity, price or discount of
//! the lines of an order in `pending_acceptance`, `credit_hold`, `confirmed`
//! or `partially_fulfilled`, or adds lines, with a reason. The change is never
//! made in place: the revision in force is kept read-only in
//! [`OrderRevision`], the lines take the new figures and the revision goes up
//! by one.
//!
//! - **Guards.** A reason; the channel allows amendments (aggregator orders
//!   are changed on the aggregator); no line below what it has fulfilled or
//!   cancelled; no started line reduced (that is `cancel_line`, which asks for
//!   an outcome); something changed; the order is not left with nothing (that
//!   is `cancel`); the revision the user saw is still the one in force.
//! - **Approval.** After fulfilment of a line started (`SAL.ACL.05`), or with a
//!   manual discount beyond the limit (`SAL.ACL.02`), the amendment becomes an
//!   approval request of action [`ACTION`] and the revision in force stays so
//!   until the last yes; the requester cannot approve. The kernel then runs
//!   `<transition>_approved` with the request's payload. At the final approval
//!   [`recheck`] reads the order again (the revision, the fulfilled quantities).
//! - **Effect, after the commit** ([`crate::order_changes`]): the event
//!   `SalesOrderAmended` (the reservations and the vertical follow), the credit
//!   check when the value rose, the money (BL-SAL-06) and the revised
//!   confirmation.
//!
//! Decisions:
//! - A transition has one target, so the spec's `amend` from four states is a
//!   family, each with its `_approved` twin the kernel runs: [`AMEND_PENDING`],
//!   [`AMEND`] (`confirmed`), [`AMEND_PARTLY`], [`AMEND_ON_HOLD`]. The credit
//!   hold that follows a failed check is the system transition
//!   [`crate::credit::HOLD`], run after the amendment committed.
//! - "Fulfilment started" comes from the kernel's
//!   [`FulfilmentProgress`](wl_kernel::sales::FulfilmentProgress) (default:
//!   nothing started). It is read when the amendment is asked and kept in the
//!   request; an order whose line starts while the request waits is not
//!   re-read (the guards on fulfilled quantities are).
//! - Any amendment of an order with a started line needs a manager, not only
//!   one that touches the started line (`SAL.ACL.05`: "after its fulfilment
//!   started").
//! - A manual discount is compared on the lines the amendment changes, as a
//!   share of the line before the discount; there is no price list yet, so
//!   "below list" is not checked.
//! - One request waits at a time per order and action.
//! - A prepaid increase holds the added quantity back from fulfilment until
//!   the payment link is paid ([`SaleLine::held_milli`]); see
//!   [`crate::settle`].

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::ApprovalRequest;
use wl_kernel::sales::{AmendedLine, SALES_ORDER_SUBJECT_TYPE, fulfilment_progress};

use crate::config::config_of;
use crate::order_change::{KIND_AMEND, OrderChange, OrderRevision, RevisionLine};
use crate::permissions;
use crate::sale_line::{KIND_ITEM, SaleLine, ratio_half_up, totals};
use crate::sales_order::{OrderPaymentStatus, PAYMENT_ONLINE, SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// `pending_acceptance → pending_acceptance`.
pub const AMEND_PENDING: &str = "amend_pending";
/// `confirmed → confirmed`.
pub const AMEND: &str = "amend";
/// `partially_fulfilled → partially_fulfilled`.
pub const AMEND_PARTLY: &str = "amend_partly";
/// `credit_hold → credit_hold`: stays on hold, the release is still needed.
pub const AMEND_ON_HOLD: &str = "amend_on_hold";

/// The four user transitions and the state each leaves and keeps.
pub const TRANSITIONS: [(&str, SalesOrderStatus); 4] = [
    (AMEND_PENDING, SalesOrderStatus::PendingAcceptance),
    (AMEND, SalesOrderStatus::Confirmed),
    (AMEND_PARTLY, SalesOrderStatus::PartiallyFulfilled),
    (AMEND_ON_HOLD, SalesOrderStatus::CreditHold),
];

/// The approval policies' action key of an amendment that needs a manager
/// (`SAL.ACL.02`, `SAL.ACL.05`).
pub const ACTION: &str = "sales.order_amend";

/// Why an amendment needs a manager: fulfilment of a line started.
pub const APPROVAL_STARTED: &str = "started";
/// Why an amendment needs a manager: a manual discount beyond the limit.
pub const APPROVAL_DISCOUNT: &str = "discount";

/// The new figures of one line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineChange {
    /// The sale line (its id).
    pub line: i64,
    /// The quantity it will have, in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The discount before VAT, in fils.
    pub discount: i64,
}

/// A line the amendment adds, standard-rated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedLine {
    /// The item code.
    pub item_code: String,
    /// The description.
    pub description: String,
    /// The quantity, in thousandths.
    pub quantity_milli: i64,
    /// The price of one unit before VAT, in fils.
    pub unit_price: i64,
    /// The discount before VAT, in fils.
    pub discount: i64,
}

/// What an amendment carries to its guards and effect: what the user asks
/// for, and what [`evaluate`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AmendInput {
    /// Why the order is amended.
    pub reason: String,
    /// The lines that change.
    pub changes: Vec<LineChange>,
    /// The lines that are added.
    pub added: Vec<AddedLine>,
    /// The revision the user saw; 0 when not given.
    pub base_revision: i64,
    /// Skip the money of the change (an amendment back after an unpaid
    /// payment link: the increase is removed, nothing is charged or refunded).
    pub no_money: bool,
    /// The user who amends (who asked, when it waited for approval).
    pub by: Option<i64>,
    /// The sale lines of the order whose fulfilment started.
    pub started_lines: Vec<i64>,
    /// The manual discount limit the run was checked against, in basis points.
    pub limit_bps: i64,
    /// Whether the channel allows amendments.
    pub channel_ok: bool,
    /// The total with VAT in force, in fils.
    pub old_gross: i64,
    /// The total with VAT of the new revision, in fils.
    pub new_gross: i64,
    /// Why a manager must approve: `started` or `discount`; `None` when no
    /// approval is needed.
    pub approval: Option<String>,
    /// The amount the approval policy reads: 1 when a manager must approve.
    pub amount: i64,
    /// The first thing wrong, `None` when nothing is.
    pub problem: Option<String>,
}

impl AmendInput {
    /// An amendment for `reason` with no change yet.
    pub fn new(reason: &str) -> Self {
        AmendInput {
            reason: reason.trim().to_owned(),
            ..AmendInput::default()
        }
    }

    /// Line `line` gets `quantity_milli`, `unit_price` and `discount`.
    pub fn line(mut self, line: i64, quantity_milli: i64, unit_price: i64, discount: i64) -> Self {
        self.changes.push(LineChange {
            line,
            quantity_milli,
            unit_price,
            discount,
        });
        self
    }

    /// Adds a line of `quantity_milli` of `item_code` at `unit_price`.
    pub fn add(mut self, item_code: &str, quantity_milli: i64, unit_price: i64) -> Self {
        self.added.push(AddedLine {
            item_code: item_code.to_owned(),
            description: item_code.to_owned(),
            quantity_milli,
            unit_price,
            discount: 0,
        });
        self
    }

    /// Seen at revision `revision`.
    pub fn seen_at(mut self, revision: i64) -> Self {
        self.base_revision = revision;
        self
    }
}

/// What the change record of an amendment keeps (`kind` `amend`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AmendDetail {
    /// The revision replaced.
    pub revision_before: i64,
    /// The revision now in force.
    pub revision_after: i64,
    /// The total with VAT before, in fils.
    pub value_before: i64,
    /// The total with VAT after, in fils.
    pub value_after: i64,
    /// The lines that changed.
    pub lines: Vec<AmendedLine>,
    /// Whether the money of the change is skipped.
    pub no_money: bool,
}

/// Keeps `text` as the problem unless one is already there.
fn first(problem: &mut Option<String>, text: String) {
    if problem.is_none() {
        *problem = Some(text);
    }
}

/// The discount of a line as a share of its value before the discount, in
/// basis points.
fn discount_bps(unit_price: i64, quantity_milli: i64, discount: i64) -> i64 {
    let before = ratio_half_up(unit_price, quantity_milli, 1000);
    if before <= 0 {
        return 0;
    }
    ratio_half_up(discount, 10_000, before)
}

/// The amendment `asked` checked against the order and its `lines`, with the
/// totals, the approval and the first problem filled in. Pure: the service
/// and the final re-check call it with the rows they read.
pub fn evaluate(order: &SalesOrder, lines: &[SaleLine], asked: &AmendInput) -> AmendInput {
    let mut input = asked.clone();
    input.reason = input.reason.trim().to_owned();
    input.problem = None;
    input.approval = None;
    input.amount = 0;
    let mut problem: Option<String> = None;
    let amendable = TRANSITIONS.iter().any(|(_, state)| *state == order.status);
    if !amendable {
        first(&mut problem, english("wl_sales.amend.problem.state", &[]));
    }
    if !input.channel_ok {
        first(&mut problem, english("wl_sales.amend.problem.channel", &[]));
    }
    if input.reason.is_empty() {
        first(&mut problem, english("wl_sales.amend.problem.reason", &[]));
    }
    if input.base_revision != 0 && input.base_revision != order.revision {
        first(&mut problem, english("wl_sales.amend.problem.moved", &[]));
    }
    let mut revised: Vec<SaleLine> = lines.to_vec();
    let mut changed = false;
    let mut top_bps = 0;
    for (n, change) in input.changes.iter().enumerate() {
        let Some(line) = revised
            .iter_mut()
            .find(|l| l.id == change.line && l.kind == KIND_ITEM)
        else {
            first(&mut problem, english("wl_sales.orders.problem.unknown_line", &[]));
            continue;
        };
        let position = line.line_no.to_string();
        if input.changes[..n].iter().any(|c| c.line == change.line) {
            first(&mut problem, english("wl_sales.orders.problem.quantity", &[]));
        } else if change.quantity_milli < 0 || change.unit_price < 0 || change.discount < 0 {
            first(
                &mut problem,
                english("wl_sales.amend.problem.negative", &[("position", &position)]),
            );
        } else if change.quantity_milli < line.fulfilled_milli + line.cancelled_milli {
            first(
                &mut problem,
                english(
                    "wl_sales.amend.problem.below_fulfilled",
                    &[("position", &position)],
                ),
            );
        } else if input.started_lines.contains(&line.id)
            && change.quantity_milli < line.quantity_milli
        {
            first(
                &mut problem,
                english(
                    "wl_sales.amend.problem.started_reduced",
                    &[("position", &position)],
                ),
            );
        }
        let figures_changed = line.unit_price != change.unit_price || line.discount != change.discount;
        changed |= figures_changed || line.quantity_milli != change.quantity_milli;
        if figures_changed {
            top_bps = top_bps.max(discount_bps(
                change.unit_price,
                change.quantity_milli,
                change.discount,
            ));
        }
        line.quantity_milli = change.quantity_milli;
        line.unit_price = change.unit_price;
        line.discount = change.discount;
    }
    for added in &input.added {
        if added.item_code.trim().is_empty()
            || added.quantity_milli <= 0
            || added.unit_price < 0
            || added.discount < 0
        {
            first(&mut problem, english("wl_sales.orders.problem.line", &[]));
            continue;
        }
        changed = true;
        top_bps = top_bps.max(discount_bps(
            added.unit_price,
            added.quantity_milli,
            added.discount,
        ));
        revised.push(SaleLine {
            quantity_milli: added.quantity_milli,
            unit_price: added.unit_price,
            discount: added.discount,
            kind: KIND_ITEM.to_owned(),
            ..SaleLine::default()
        });
    }
    if !changed {
        first(&mut problem, english("wl_sales.amend.problem.nothing", &[]));
    }
    let kept: i64 = revised
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| l.quantity_milli - l.cancelled_milli)
        .sum();
    if changed && kept <= 0 {
        first(&mut problem, english("wl_sales.amend.problem.empty", &[]));
    }
    input.old_gross = order.gross_total;
    input.new_gross = totals(&revised).gross;
    if !input.started_lines.is_empty() {
        input.approval = Some(APPROVAL_STARTED.to_owned());
    } else if top_bps > input.limit_bps {
        input.approval = Some(APPROVAL_DISCOUNT.to_owned());
    }
    input.amount = i64::from(input.approval.is_some());
    input.problem = problem;
    input
}

/// The input of the run, if it carries one.
fn input_of(ctx: &TransitionCtx) -> Option<AmendInput> {
    ctx.input_as::<AmendInput>().ok()
}

/// A 422 with `message`.
fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// The lines of order `order` read in `tx`, in order.
pub(crate) async fn lines_in(tx: &mut Transaction, order: i64) -> Result<Vec<SaleLine>> {
    SaleLine::query()
        .where_(SaleLine::SUBJECT_TYPE.eq(SALES_ORDER_SUBJECT_TYPE.to_owned()))
        .where_(SaleLine::SUBJECT_ID.eq(order))
        .order_by(SaleLine::ID)
        .get(&mut *tx)
        .await
}

/// The twin the kernel runs at the last approval: `amend_approved`...
pub fn approved(name: &str) -> String {
    wl_kernel::machines::approved(name)
}

/// The guards and the effect the user transition and its twin share, for an
/// order in `state`.
fn rules(transition: Transition<SalesOrder>, state: SalesOrderStatus) -> Transition<SalesOrder> {
    transition
        .from([state])
        .to(state)
        .guard("amendment_allowed", |_o: &SalesOrder, ctx| {
            let problem = input_of(ctx).map_or_else(
                || Some(english("wl_sales.orders.problem.input", &[])),
                |input| input.problem,
            );
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|order: &SalesOrder, ctx, tx| {
            let order = order.clone();
            let run = input_of(ctx).unwrap_or_default();
            Box::pin(async move { amend_in(tx, order, &run).await })
        })
}

/// The eight transitions: the four of the users and their twins.
pub(crate) fn transitions() -> Vec<Transition<SalesOrder>> {
    let mut all = Vec::new();
    for (name, state) in TRANSITIONS {
        all.push(rules(
            Transition::new(name)
                .label("Amend")
                .tone("primary")
                .permission(permissions::SALES_ORDERS_AMEND)
                .reason_required(),
            state,
        ));
        all.push(rules(
            Transition::new(approved(name))
                .label("Amend (approved)")
                .system(),
            state,
        ));
    }
    all
}

/// The machine of the order with the amend transitions added.
pub(crate) fn add_to(machine: Machine<SalesOrder>) -> Machine<SalesOrder> {
    transitions().into_iter().fold(machine, Machine::transition)
}

/// The sale line `line` as the revision keeps it.
fn kept(line: &SaleLine) -> RevisionLine {
    RevisionLine {
        line: line.id,
        line_no: line.line_no,
        item_code: line.item_code.clone(),
        description: line.description.clone(),
        quantity_milli: line.quantity_milli,
        unit_price: line.unit_price,
        discount: line.discount,
        gross: line.gross(),
    }
}

/// The effect of the eight transitions, in the run's transaction: keeps the
/// revision in force, applies the change, raises the revision and writes the
/// change record the events and the money are announced from.
async fn amend_in(tx: &mut Transaction, order: SalesOrder, run: &AmendInput) -> Result {
    let before = lines_in(tx, order.id).await?;
    // The lines may have moved since the guards looked (goods fulfilled).
    let found = evaluate(&order, &before, run);
    if let Some(problem) = found.problem {
        return Err(refused(problem));
    }
    let old = totals(&before);
    let kept_lines: Vec<RevisionLine> = before.iter().map(kept).collect();
    let mut earlier = OrderRevision {
        order_id: order.id,
        revision: order.revision,
        reason: run.reason.clone(),
        net_total: old.net,
        vat_total: old.vat,
        gross_total: order.gross_total,
        lines: renox::serde_json::to_string(&kept_lines)
            .map_err(|e| Error::Internal(renox::anyhow::Error::new(e)))?,
        amended_by: run.by,
        ..OrderRevision::default()
    };
    earlier.insert(&mut *tx).await?;
    // A prepaid increase holds the added quantity until the link is paid.
    let prepaid = order.payment_mode == PAYMENT_ONLINE
        && order.payment_status == OrderPaymentStatus::Received;
    let hold = prepaid && found.new_gross > found.old_gross && !run.no_money;
    let stamp = renox::db::now();
    let mut report = Vec::new();
    for change in &run.changes {
        let Some(line) = before.iter().find(|l| l.id == change.line) else {
            continue;
        };
        let increase = (change.quantity_milli - line.quantity_milli).max(0);
        let room = (change.quantity_milli - line.fulfilled_milli - line.cancelled_milli).max(0);
        let held = if hold {
            (line.held_milli + increase).min(room)
        } else {
            line.held_milli.min(room)
        };
        SaleLine::query()
            .where_(SaleLine::ID.eq(line.id))
            .update(
                &mut *tx,
                &[
                    ("quantity_milli", &change.quantity_milli),
                    ("unit_price", &change.unit_price),
                    ("discount", &change.discount),
                    ("held_milli", &held),
                    ("updated_at", &stamp),
                ],
            )
            .await?;
        if line.quantity_milli != change.quantity_milli || line.unit_price != change.unit_price {
            report.push(AmendedLine {
                line: line.id,
                quantity_before_milli: line.quantity_milli,
                quantity_after_milli: change.quantity_milli,
                unit_price_before: line.unit_price,
                unit_price_after: change.unit_price,
                held_milli: held,
            });
        }
    }
    let mut next = before.len() as i64;
    for added in &run.added {
        next += 1;
        let held = if hold { added.quantity_milli } else { 0 };
        let mut line = SaleLine {
            subject_type: SALES_ORDER_SUBJECT_TYPE.to_owned(),
            subject_id: order.id,
            line_no: next,
            item_code: added.item_code.trim().to_owned(),
            description: if added.description.trim().is_empty() {
                added.item_code.trim().to_owned()
            } else {
                added.description.trim().to_owned()
            },
            kind: KIND_ITEM.to_owned(),
            quantity_milli: added.quantity_milli,
            unit_price: added.unit_price,
            discount: added.discount,
            held_milli: held,
            ..SaleLine::default()
        };
        line.insert(&mut *tx).await?;
        report.push(AmendedLine {
            line: line.id,
            quantity_before_milli: 0,
            quantity_after_milli: added.quantity_milli,
            unit_price_before: 0,
            unit_price_after: added.unit_price,
            held_milli: held,
        });
    }
    let after = lines_in(tx, order.id).await?;
    let new = totals(&after);
    let revision = order.revision + 1;
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(order.id))
        .update(
            &mut *tx,
            &[
                ("net_total", &new.net),
                ("vat_total", &new.vat),
                ("gross_total", &new.gross),
                ("revision", &revision),
                ("updated_at", &stamp),
            ],
        )
        .await?;
    let detail = AmendDetail {
        revision_before: order.revision,
        revision_after: revision,
        value_before: order.gross_total,
        value_after: new.gross,
        lines: report,
        no_money: run.no_money,
    };
    OrderChange::record(tx, order.id, KIND_AMEND, &run.reason, run.by, &detail).await?;
    Ok(())
}

/// What [`amend`] did.
#[derive(Debug, Clone)]
pub enum AmendOutcome {
    /// The new revision is in force.
    Amended(Box<SalesOrder>),
    /// A manager must approve first: the revision in force stays until then.
    Waiting(Box<ApprovalRequest>),
}

/// The transition that amends an order in `status`.
fn transition_for(status: SalesOrderStatus) -> Option<&'static str> {
    TRANSITIONS
        .iter()
        .find(|(_, state)| *state == status)
        .map(|(name, _)| *name)
}

/// `user` amends the order `order_id` as `asked`. Refused (422) with the first
/// problem before anything is written; after fulfilment of a line started, or
/// with a manual discount beyond the limit, it becomes an approval request
/// instead (see the module docs).
///
/// # Errors
///
/// 403 without `sales_orders.amend`; 422 for a problem of the amendment, or
/// when a request for the order is already waiting, or when no approval policy
/// exists for a change that needs one.
pub async fn amend(
    state: &AppState,
    user: &AuthUser,
    order_id: i64,
    asked: AmendInput,
) -> Result<AmendOutcome> {
    if !user.has_permission(permissions::SALES_ORDERS_AMEND) {
        return Err(Error::Forbidden);
    }
    let order = SalesOrder::find_or_404(&state.db, order_id).await?;
    let Some(name) = transition_for(order.status) else {
        return Err(refused(english("wl_sales.amend.problem.state", &[])));
    };
    let config = config_of(state);
    let lines = SaleLine::of(&state.db, &order.subject()).await?;
    let item_ids: Vec<i64> = lines
        .iter()
        .filter(|l| l.kind == KIND_ITEM)
        .map(|l| l.id)
        .collect();
    let mut asked = asked;
    asked.by = Some(user.id);
    asked.limit_bps = config.manual_discount_limit_bps;
    asked.channel_ok = config.amendments_allowed(&order.channel);
    asked.started_lines = fulfilment_progress(state)
        .started_lines(state, order.order_id(), &item_ids)
        .await?;
    let found = evaluate(&order, &lines, &asked);
    if let Some(problem) = found.problem.clone() {
        return Err(refused(problem));
    }
    let reason = found.reason.clone();
    if let Some(request) = crate::order_changes::request_or_run(
        state,
        user,
        &order,
        ACTION,
        name,
        &found,
        found.approval.is_some(),
        &reason,
    )
    .await?
    {
        return Ok(AmendOutcome::Waiting(request));
    }
    Ok(AmendOutcome::Amended(Box::new(
        SalesOrder::find_or_404(&state.db, order_id).await?,
    )))
}

/// The final check of a request for an amendment: before the approved
/// transition runs, the order must still be on the revision the request was
/// made on and no line may be below what it has fulfilled by now. `Some`
/// message fails the request (`approval failed: ...`).
///
/// # Errors
///
/// A database error.
pub async fn recheck(tx: &mut Transaction, request: &ApprovalRequest) -> Result<Option<String>> {
    let asked: AmendInput =
        renox::serde_json::from_value(request.payload.0.clone()).unwrap_or_default();
    let order = SalesOrder::find_or_404(&mut *tx, request.subject_id).await?;
    if asked.base_revision != 0 && asked.base_revision != order.revision {
        return Ok(Some(english("wl_sales.amend.problem.moved", &[])));
    }
    let lines = lines_in(tx, order.id).await?;
    Ok(evaluate(&order, &lines, &asked).problem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use renox::state_machine::StateMachine;

    fn order(status: SalesOrderStatus) -> SalesOrder {
        SalesOrder {
            id: 1,
            revision: 1,
            status,
            gross_total: 42_000,
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
            ..SaleLine::default()
        }
    }

    fn ask(reason: &str) -> AmendInput {
        AmendInput {
            channel_ok: true,
            limit_bps: 1000,
            ..AmendInput::new(reason)
        }
    }

    #[test]
    fn a_quantity_increase_is_evaluated_with_both_totals_and_no_approval() {
        let found = evaluate(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 10_000, 4_000)],
            &ask("two more").line(1, 12_000, 4_000, 0),
        );
        assert_eq!(found.problem, None);
        assert_eq!((found.old_gross, found.new_gross), (42_000, 50_400));
        assert_eq!(found.approval, None);
        assert_eq!(found.amount, 0);
    }

    #[test]
    fn a_line_cannot_go_below_what_it_has_fulfilled() {
        let mut fulfilled = line(1, 10_000, 4_000);
        fulfilled.fulfilled_milli = 5_000;
        let found = evaluate(
            &order(SalesOrderStatus::PartiallyFulfilled),
            &[fulfilled],
            &ask("less").line(1, 4_000, 4_000, 0),
        );
        assert!(found.problem.is_some());
    }

    #[test]
    fn a_started_line_is_not_reduced_and_any_change_then_needs_a_manager() {
        let mut asked = ask("less");
        asked.started_lines = vec![1];
        let reduced = evaluate(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 10_000, 4_000)],
            &AmendInput {
                changes: vec![LineChange {
                    line: 1,
                    quantity_milli: 8_000,
                    unit_price: 4_000,
                    discount: 0,
                }],
                ..asked.clone()
            },
        );
        assert!(reduced.problem.is_some());
        let raised = evaluate(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 10_000, 4_000)],
            &AmendInput {
                changes: vec![LineChange {
                    line: 1,
                    quantity_milli: 12_000,
                    unit_price: 4_000,
                    discount: 0,
                }],
                ..asked
            },
        );
        assert_eq!(raised.problem, None);
        assert_eq!(raised.approval.as_deref(), Some(APPROVAL_STARTED));
        assert_eq!(raised.amount, 1);
    }

    #[test]
    fn a_discount_beyond_the_limit_needs_a_manager() {
        let found = evaluate(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 10_000, 4_000)],
            &ask("deal").line(1, 10_000, 4_000, 4_800),
        );
        assert_eq!(found.problem, None);
        assert_eq!(found.approval.as_deref(), Some(APPROVAL_DISCOUNT));
        let within = evaluate(
            &order(SalesOrderStatus::Confirmed),
            &[line(1, 10_000, 4_000)],
            &ask("deal").line(1, 10_000, 4_000, 4_000),
        );
        assert_eq!(within.approval, None);
    }

    #[test]
    fn an_aggregator_channel_a_blank_reason_and_a_stale_revision_are_refused() {
        let base = [line(1, 10_000, 4_000)];
        let change = |input: AmendInput| input.line(1, 12_000, 4_000, 0);
        let mut off = ask("x");
        off.channel_ok = false;
        assert!(evaluate(&order(SalesOrderStatus::Confirmed), &base, &change(off)).problem.is_some());
        assert!(
            evaluate(&order(SalesOrderStatus::Confirmed), &base, &change(ask("  ")))
                .problem
                .is_some()
        );
        assert!(
            evaluate(
                &order(SalesOrderStatus::Confirmed),
                &base,
                &change(ask("x").seen_at(3))
            )
            .problem
            .is_some()
        );
        assert!(
            evaluate(&order(SalesOrderStatus::Fulfilled), &base, &change(ask("x")))
                .problem
                .is_some()
        );
    }

    #[test]
    fn nothing_changed_and_nothing_left_are_refused() {
        let base = [line(1, 10_000, 4_000)];
        assert!(
            evaluate(
                &order(SalesOrderStatus::Confirmed),
                &base,
                &ask("same").line(1, 10_000, 4_000, 0)
            )
            .problem
            .is_some()
        );
        assert!(
            evaluate(
                &order(SalesOrderStatus::Confirmed),
                &base,
                &ask("none").line(1, 0, 4_000, 0)
            )
            .problem
            .is_some()
        );
    }

    #[test]
    fn the_eight_transitions_stay_in_their_state() {
        let machine = SalesOrder::machine();
        for (name, state) in TRANSITIONS {
            let user = machine.find(name).expect(name);
            assert!(!user.is_system(), "{name}");
            assert!(user.needs_reason(), "{name}");
            assert_eq!(user.from_states(), [state.as_str()], "{name}");
            assert_eq!(user.to_state(), Some(state.as_str()), "{name}");
            let twin = machine.find(&approved(name)).expect(name);
            assert!(twin.is_system(), "{name}");
            assert_eq!(twin.guard_names(), ["amendment_allowed"], "{name}");
        }
    }
}
