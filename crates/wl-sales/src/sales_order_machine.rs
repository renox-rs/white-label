//! The SalesOrder state machine (wl-sales §4.2; base for BL-SAL-22).
//!
//! ```text
//! [*] draft --submit--> confirmed
//!     draft --submit_online--> awaiting_payment
//!     draft --submit_for_acceptance--> pending_acceptance
//!     awaiting_payment --confirm_payment--> confirmed
//!     awaiting_payment --confirm_payment_for_acceptance--> pending_acceptance
//!     awaiting_payment --payment_failed--> cancelled
//!     pending_acceptance --accept--> confirmed
//!     pending_acceptance --reject--> rejected
//!     draft | pending_acceptance | credit_hold | confirmed --cancel--> cancelled
//!     confirmed | partially_fulfilled --fulfil--> partially_fulfilled
//!     confirmed | partially_fulfilled --fulfil_last--> fulfilled
//!     partially_fulfilled | fulfilled --reverse_fulfilment--> partially_fulfilled
//!     partially_fulfilled | fulfilled --reverse_fulfilment_all--> confirmed
//!     fulfilled --close--> closed
//!     confirmed --cancel_line--> confirmed
//!     partially_fulfilled --cancel_line_partly--> partially_fulfilled
//!     draft --submit_hold--> credit_hold
//!     pending_acceptance --accept_hold--> credit_hold
//!     confirmed | partially_fulfilled --hold--> credit_hold
//! ```
//!
//! `cancel_line` is the part of the spec's `cancel_line` that BL-SAL-23 needs
//! (open quantity of one line cancelled, a reason, a state that stays); the
//! outcome per line, the approval when fulfilment started, the refund and the
//! credit note, and the move to `fulfilled` when nothing is open (use
//! `cancel_remaining`) come with BL-SAL-06 and BL-SAL-09.
//! `submit_hold`, `accept_hold` and `hold` are the credit check's three ways
//! into `credit_hold` (BL-SAL-10, see [`crate::credit`]): the spec's `submit`,
//! `accept` and `hold` with the target `credit_hold`. `release`, `amend` and
//! `cancel_remaining` come with their issues (BL-SAL-05 to BL-SAL-11). `confirm_payment`, `payment_failed`,
//! `fulfil`, `fulfil_last`, `reverse_fulfilment`, `reverse_fulfilment_all` and
//! `close` are system transitions (the gateway, the hand-over, the invoicing
//! state); the rest are by users.
//!
//! The inputs carry what a guard needs and an effect cannot fetch (no
//! `AppState`): the line count and totals, the amount paid, and for a
//! hand-over the problem the service found with the quantities.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::sales::{CreditFailure, FulfilledLine, SALES_ORDER_SUBJECT_TYPE};

use crate::permissions;
use crate::sale_line::SaleLine;
use crate::sales_order::{FulfilmentRecord, SalesOrder, SalesOrderStatus};
use crate::texts::english;

/// `draft → confirmed`: a channel with no acceptance and no prepayment.
pub const SUBMIT: &str = "submit";
/// `draft → awaiting_payment`: paid online before it is placed.
pub const SUBMIT_ONLINE: &str = "submit_online";
/// `draft → pending_acceptance`: the seller must accept it.
pub const SUBMIT_FOR_ACCEPTANCE: &str = "submit_for_acceptance";
/// `awaiting_payment → confirmed`: the gateway confirmed the payment.
pub const CONFIRM_PAYMENT: &str = "confirm_payment";
/// `awaiting_payment → pending_acceptance`: paid, now the seller accepts.
pub const CONFIRM_PAYMENT_FOR_ACCEPTANCE: &str = "confirm_payment_for_acceptance";
/// `awaiting_payment → cancelled`: the online payment failed or timed out.
pub const PAYMENT_FAILED: &str = "payment_failed";
/// `pending_acceptance → confirmed`: the seller accepted.
pub const ACCEPT: &str = "accept";
/// `pending_acceptance → rejected`: the seller refused.
pub const REJECT: &str = "reject";
/// `draft | pending_acceptance | credit_hold | confirmed → cancelled`.
pub const CANCEL: &str = "cancel";
/// `confirmed | partially_fulfilled → partially_fulfilled`: quantities handed
/// over, some still open.
pub const FULFIL: &str = "fulfil";
/// `confirmed | partially_fulfilled → fulfilled`: the last open quantities
/// handed over.
pub const FULFIL_LAST: &str = "fulfil_last";
/// `partially_fulfilled | fulfilled → partially_fulfilled`: a hand-over is
/// reversed, some quantity stays fulfilled.
pub const REVERSE_FULFILMENT: &str = "reverse_fulfilment";
/// `partially_fulfilled | fulfilled → confirmed`: a hand-over is reversed and
/// nothing stays fulfilled.
pub const REVERSE_FULFILMENT_ALL: &str = "reverse_fulfilment_all";
/// `fulfilled → closed`: everything fulfilled is invoiced.
pub const CLOSE: &str = "close";
/// `confirmed → confirmed`: open quantity of one line cancelled.
pub const CANCEL_LINE: &str = "cancel_line";
/// `partially_fulfilled → partially_fulfilled`: open quantity of one line
/// cancelled.
pub const CANCEL_LINE_PARTLY: &str = "cancel_line_partly";

/// `draft → credit_hold`: the credit check at `submit` failed.
pub const SUBMIT_HOLD: &str = "submit_hold";
/// `pending_acceptance → credit_hold`: the credit check at `accept` failed.
pub const ACCEPT_HOLD: &str = "accept_hold";
/// `confirmed | partially_fulfilled → credit_hold`: the re-check at dispatch
/// failed, or a bounced cheque (`SAL.RUL.05`).
pub const HOLD: &str = "hold";

/// What a hold takes: the failure and the figures the order keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldInput {
    /// Why the check failed.
    pub reason: CreditFailure,
    /// What the customer owes with this order counted, in fils.
    pub open_balance: i64,
    /// The overdue balance beyond the grace days, in fils.
    pub overdue: i64,
    /// The limit checked against, in fils.
    pub limit: Option<i64>,
}

/// What `accept` and `accept_hold` take: the result of the credit check the
/// service ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AcceptInput {
    /// The credit check was run and passed (`accept`).
    pub checked: bool,
    /// The hold the check calls for (`accept_hold`).
    pub hold: Option<HoldInput>,
}

/// What the `submit` family takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitInput {
    /// How many lines the order has.
    pub line_count: i64,
    /// The sum of the lines before VAT, in fils.
    pub net: i64,
    /// The VAT, in fils.
    pub vat: i64,
    /// Net plus VAT, in fils.
    pub gross: i64,
    /// Whether `accept` must run the credit check of the service (the order
    /// is on account and its customer has an account).
    #[serde(default)]
    pub credit_gate: bool,
    /// The hold the credit check calls for (`submit_hold`).
    #[serde(default)]
    pub hold: Option<HoldInput>,
}

/// What `confirm_payment` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayInput {
    /// The amount the gateway confirmed, in fils: it must equal the order
    /// total.
    pub amount: i64,
}

/// What `fulfil` and `fulfil_last` take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FulfilInput {
    /// The accepted quantities.
    pub lines: Vec<FulfilledLine>,
    /// The hand-over.
    pub source: Subject,
    /// Whether nothing is open after it (`fulfil_last`).
    pub complete: bool,
    /// Why the quantities are not acceptable, found by the service.
    pub problem: Option<String>,
}

/// What `reverse_fulfilment` and `reverse_fulfilment_all` take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverseInput {
    /// The quantities no longer fulfilled.
    pub lines: Vec<FulfilledLine>,
    /// The hand-over that was reversed or lost.
    pub source: Subject,
    /// What the hand-over still holds after the reversal (JSON).
    pub held_after: String,
    /// Whether nothing stays fulfilled (`reverse_fulfilment_all`).
    pub none_left: bool,
    /// Why the reversal is not acceptable, found by the service.
    pub problem: Option<String>,
}

/// What `close` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseInput {
    /// Whether everything fulfilled is invoiced.
    pub invoiced: bool,
}

/// What `cancel_line` takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelLineInput {
    /// The sale line.
    pub line: i64,
    /// The open quantity cancelled, in thousandths.
    pub quantity_milli: i64,
    /// Why the quantity is not acceptable, found by the service.
    pub problem: Option<String>,
}

/// A guard that fails with the English `key` unless `ok`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// The reason of the run, trimmed; `None` when blank.
fn reason_of(ctx: &renox::state_machine::TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// Adds `quantity` to the cancelled quantity of line `line` of order
/// `order`, in one statement. Fails when it would leave more than is open.
async fn move_cancelled(
    tx: &mut renox::db::Transaction,
    order: i64,
    line: i64,
    quantity: i64,
) -> Result {
    let changed = renox::db::sql(format!(
        "UPDATE {} SET cancelled_milli = cancelled_milli + ?, updated_at = ? \
         WHERE id = ? AND subject_type = '{}' AND subject_id = ? \
         AND fulfilled_milli + cancelled_milli + ? <= quantity_milli",
        SaleLine::TABLE,
        SALES_ORDER_SUBJECT_TYPE
    ))
    .bind(quantity)
    .bind(now())
    .bind(line)
    .bind(order)
    .bind(quantity)
    .execute(&mut *tx)
    .await?;
    if changed == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.orders.problem.quantity", &[]),
        ));
    }
    Ok(())
}

/// Adds `delta` (negative to take off) to the fulfilled quantity of line
/// `line` of order `order`, in one statement. Fails when the result would
/// leave `0..=quantity - cancelled`.
async fn move_fulfilled(
    tx: &mut renox::db::Transaction,
    order: i64,
    line: i64,
    delta: i64,
) -> Result {
    let changed = renox::db::sql(format!(
        "UPDATE {} SET fulfilled_milli = fulfilled_milli + ?, updated_at = ? \
         WHERE id = ? AND subject_type = '{}' AND subject_id = ? \
         AND fulfilled_milli + ? >= 0 AND fulfilled_milli + cancelled_milli + ? <= quantity_milli",
        SaleLine::TABLE,
        SALES_ORDER_SUBJECT_TYPE
    ))
    .bind(delta)
    .bind(now())
    .bind(line)
    .bind(order)
    .bind(delta)
    .bind(delta)
    .execute(&mut *tx)
    .await?;
    if changed == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.orders.problem.quantity", &[]),
        ));
    }
    Ok(())
}

impl StateMachine for SalesOrder {
    type State = SalesOrderStatus;
    const COLUMN: renox::db::Col<SalesOrder, SalesOrderStatus> = SalesOrder::STATUS;

    fn machine() -> Machine<Self> {
        use SalesOrderStatus::{
            AwaitingPayment, Cancelled, Closed, Confirmed, CreditHold, Draft, Fulfilled,
            PartiallyFulfilled, PendingAcceptance, Rejected,
        };
        Machine::new()
            .initial([Draft])
            .state(Draft)
            .tone("neutral")
            .state(AwaitingPayment)
            .tone("neutral")
            .state(PendingAcceptance)
            .tone("neutral")
            .state(CreditHold)
            .tone("warning")
            .state(Confirmed)
            .tone("info")
            .state(PartiallyFulfilled)
            .tone("info")
            .state(Fulfilled)
            .tone("success")
            .state(Closed)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(submit(SUBMIT, "Submit", Confirmed))
            .transition(submit(SUBMIT_ONLINE, "Submit for payment", AwaitingPayment))
            .transition(submit(
                SUBMIT_FOR_ACCEPTANCE,
                "Submit for acceptance",
                PendingAcceptance,
            ))
            .transition(confirm_payment(CONFIRM_PAYMENT, Confirmed))
            .transition(confirm_payment(
                CONFIRM_PAYMENT_FOR_ACCEPTANCE,
                PendingAcceptance,
            ))
            .transition(payment_failed())
            .transition(accept())
            .transition(reject())
            .transition(cancel())
            .transition(fulfil(FULFIL, PartiallyFulfilled))
            .transition(fulfil(FULFIL_LAST, Fulfilled))
            .transition(reverse(REVERSE_FULFILMENT, PartiallyFulfilled))
            .transition(reverse(REVERSE_FULFILMENT_ALL, Confirmed))
            .transition(close())
            .transition(cancel_line(CANCEL_LINE, Confirmed))
            .transition(cancel_line(CANCEL_LINE_PARTLY, PartiallyFulfilled))
            .transition(submit(SUBMIT_HOLD, "Submit", CreditHold))
            .transition(accept_hold())
            .transition(hold())
    }
}

/// `submit`, `submit_online`, `submit_for_acceptance`: draft → `to`, with
/// the totals stored.
fn submit(name: &str, label: &str, to: SalesOrderStatus) -> Transition<SalesOrder> {
    let confirms = to == SalesOrderStatus::Confirmed;
    let holds = to == SalesOrderStatus::CreditHold;
    Transition::new(name)
        .label(label)
        .tone("primary")
        .permission(permissions::SALES_ORDERS_CREATE)
        .from([SalesOrderStatus::Draft])
        .to(to)
        .guard("has_lines", |_o: &SalesOrder, ctx| {
            let lines = ctx.input_as::<SubmitInput>().map(|i| i.line_count);
            Box::pin(async move {
                check(
                    lines.is_ok_and(|n| n > 0),
                    "wl_sales.orders.problem.no_lines",
                )
            })
        })
        .guard("credit_result", move |_o: &SalesOrder, ctx| {
            // Only `submit_hold` carries a hold, and it must.
            let held = ctx
                .input_as::<SubmitInput>()
                .is_ok_and(|i| i.hold.is_some());
            Box::pin(async move { check(held == holds, "wl_sales.orders.problem.input") })
        })
        .effect(move |order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let input = ctx.input_as::<SubmitInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                let confirmed = confirms.then(now);
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("net_total", &input.net),
                            ("vat_total", &input.vat),
                            ("gross_total", &input.gross),
                            ("confirmed_at", &confirmed),
                            ("credit_gate", &input.credit_gate),
                        ],
                    )
                    .await?;
                if let Some(hold) = input.hold {
                    write_hold(tx, id, &hold, SalesOrderStatus::Confirmed).await?;
                }
                Ok(())
            })
        })
}

/// Stores the hold on order `id`: the state it returns to, why, and the
/// figures.
async fn write_hold(
    tx: &mut renox::db::Transaction,
    id: i64,
    hold: &HoldInput,
    before: SalesOrderStatus,
) -> Result {
    let before = Some(before.as_str().to_owned());
    let reason = Some(hold.reason.as_str().to_owned());
    let (balance, overdue, limit) = (Some(hold.open_balance), Some(hold.overdue), hold.limit);
    let held_at = Some(now());
    SalesOrder::query()
        .where_(SalesOrder::ID.eq(id))
        .update(
            &mut *tx,
            &[
                ("state_before_hold", &before),
                ("hold_reason", &reason),
                ("hold_open_balance", &balance),
                ("hold_overdue", &overdue),
                ("hold_limit", &limit),
                ("held_at", &held_at),
            ],
        )
        .await?;
    Ok(())
}

/// `accept_hold`: the credit check at `accept` failed; the order is held
/// without ever having been confirmed.
fn accept_hold() -> Transition<SalesOrder> {
    Transition::new(ACCEPT_HOLD)
        .label("Accept")
        .tone("warning")
        .permission(permissions::SALES_ORDERS_ACCEPT)
        .from([SalesOrderStatus::PendingAcceptance])
        .to(SalesOrderStatus::CreditHold)
        .guard("credit_result", |_o: &SalesOrder, ctx| {
            let held = ctx
                .input_as::<AcceptInput>()
                .is_ok_and(|i| i.hold.is_some());
            Box::pin(async move { check(held, "wl_sales.orders.problem.input") })
        })
        .effect(|order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let hold = ctx.input_as::<AcceptInput>().ok().and_then(|i| i.hold);
            Box::pin(async move {
                let Some(hold) = hold else {
                    return Ok(());
                };
                write_hold(tx, id, &hold, SalesOrderStatus::Confirmed).await
            })
        })
}

/// `hold`: a confirmed or partly fulfilled order on account goes on credit
/// hold. Its reservations stay.
fn hold() -> Transition<SalesOrder> {
    Transition::new(HOLD)
        .label("Hold")
        .tone("warning")
        .system()
        .from([
            SalesOrderStatus::Confirmed,
            SalesOrderStatus::PartiallyFulfilled,
        ])
        .to(SalesOrderStatus::CreditHold)
        .guard("on_account", |o: &SalesOrder, _ctx| {
            let on_account = o.is_on_account();
            Box::pin(async move { check(on_account, "wl_sales.orders.problem.not_on_account") })
        })
        .guard("failure_given", |_o: &SalesOrder, ctx| {
            let given = ctx.input_as::<HoldInput>().is_ok();
            Box::pin(async move { check(given, "wl_sales.orders.problem.input") })
        })
        .effect(|order: &SalesOrder, ctx, tx| {
            let (id, before) = (order.id, order.status);
            let hold = ctx.input_as::<HoldInput>().ok();
            Box::pin(async move {
                let Some(hold) = hold else {
                    return Ok(());
                };
                write_hold(tx, id, &hold, before).await
            })
        })
}

/// `confirm_payment`, `confirm_payment_for_acceptance`: the gateway
/// confirmed an amount equal to the order total.
fn confirm_payment(name: &str, to: SalesOrderStatus) -> Transition<SalesOrder> {
    let confirms = to == SalesOrderStatus::Confirmed;
    Transition::new(name)
        .label("Confirm payment")
        .tone("success")
        .system()
        .from([SalesOrderStatus::AwaitingPayment])
        .to(to)
        .guard("amount_is_total", |o: &SalesOrder, ctx| {
            let total = o.gross_total;
            let paid = ctx.input_as::<PayInput>().map(|i| i.amount);
            Box::pin(async move {
                check(
                    paid.is_ok_and(|a| a == total),
                    "wl_sales.orders.problem.amount",
                )
            })
        })
        .effect(move |order: &SalesOrder, _ctx, tx| {
            let id = order.id;
            Box::pin(async move {
                if confirms {
                    let stamp = Some(now());
                    SalesOrder::query()
                        .where_(SalesOrder::ID.eq(id))
                        .update(&mut *tx, &[("confirmed_at", &stamp)])
                        .await?;
                }
                Ok(())
            })
        })
}

/// `payment_failed`: awaiting_payment → cancelled, nothing reaches fulfilment.
fn payment_failed() -> Transition<SalesOrder> {
    Transition::new(PAYMENT_FAILED)
        .label("Payment failed")
        .tone("warning")
        .system()
        .from([SalesOrderStatus::AwaitingPayment])
        .to(SalesOrderStatus::Cancelled)
        .effect(|order: &SalesOrder, _ctx, tx| {
            let id = order.id;
            Box::pin(async move {
                let reason = Some(english("wl_sales.orders.payment_failed_reason", &[]));
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(&mut *tx, &[("reason", &reason)])
                    .await?;
                Ok(())
            })
        })
}

/// `accept`: pending_acceptance → confirmed.
fn accept() -> Transition<SalesOrder> {
    Transition::new(ACCEPT)
        .label("Accept")
        .tone("success")
        .permission(permissions::SALES_ORDERS_ACCEPT)
        .from([SalesOrderStatus::PendingAcceptance])
        .to(SalesOrderStatus::Confirmed)
        .guard("credit_checked", |o: &SalesOrder, ctx| {
            // An order whose customer has an account is accepted by the
            // service, which runs the credit check first.
            let ok = !o.credit_gate || ctx.input_as::<AcceptInput>().is_ok_and(|i| i.checked);
            Box::pin(async move { check(ok, "wl_sales.orders.problem.credit_not_checked") })
        })
        .effect(|order: &SalesOrder, _ctx, tx| {
            let id = order.id;
            Box::pin(async move {
                let stamp = Some(now());
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(&mut *tx, &[("confirmed_at", &stamp)])
                    .await?;
                Ok(())
            })
        })
}

/// A transition that ends an order with the reason given.
fn ending(
    name: &str,
    label: &str,
    permission: &str,
    to: SalesOrderStatus,
) -> Transition<SalesOrder> {
    Transition::new(name)
        .label(label)
        .tone("danger")
        .permission(permission)
        .reason_required()
        .to(to)
        .effect(|order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let reason = reason_of(ctx);
            Box::pin(async move {
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(&mut *tx, &[("reason", &reason)])
                    .await?;
                Ok(())
            })
        })
}

/// `reject`: pending_acceptance → rejected.
fn reject() -> Transition<SalesOrder> {
    ending(
        REJECT,
        "Reject",
        permissions::SALES_ORDERS_ACCEPT,
        SalesOrderStatus::Rejected,
    )
    .from([SalesOrderStatus::PendingAcceptance])
}

/// `cancel`: before anything was fulfilled.
fn cancel() -> Transition<SalesOrder> {
    ending(
        CANCEL,
        "Cancel",
        permissions::SALES_ORDERS_CANCEL,
        SalesOrderStatus::Cancelled,
    )
    .from([
        SalesOrderStatus::Draft,
        SalesOrderStatus::PendingAcceptance,
        SalesOrderStatus::CreditHold,
        SalesOrderStatus::Confirmed,
    ])
}

/// `fulfil` and `fulfil_last`: the accepted quantities are handed over.
fn fulfil(name: &str, to: SalesOrderStatus) -> Transition<SalesOrder> {
    let last = to == SalesOrderStatus::Fulfilled;
    Transition::new(name)
        .label("Fulfil")
        .tone("info")
        .system()
        .from([
            SalesOrderStatus::Confirmed,
            SalesOrderStatus::PartiallyFulfilled,
        ])
        .to(to)
        .guard("quantities_open", |_o: &SalesOrder, ctx| {
            let problem = match ctx.input_as::<FulfilInput>() {
                Ok(input) => input.problem,
                Err(_) => Some(english("wl_sales.orders.problem.input", &[])),
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .guard(
            "completes_only_when_nothing_is_open",
            move |_o: &SalesOrder, ctx| {
                let complete = ctx.input_as::<FulfilInput>().map(|i| i.complete);
                Box::pin(async move {
                    check(
                        complete.is_ok_and(|c| c == last),
                        "wl_sales.orders.problem.completeness",
                    )
                })
            },
        )
        .effect(move |order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let input = ctx.input_as::<FulfilInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                for line in &input.lines {
                    move_fulfilled(tx, id, line.line, line.quantity_milli).await?;
                }
                FulfilmentRecord {
                    order_id: id,
                    source_type: input.source.subject_type.clone(),
                    source_id: input.source.subject_id,
                    lines: renox::serde_json::to_string(&input.lines).unwrap_or_default(),
                    ..FulfilmentRecord::default()
                }
                .insert(&mut *tx)
                .await?;
                if last {
                    let stamp = Some(now());
                    SalesOrder::query()
                        .where_(SalesOrder::ID.eq(id))
                        .update(&mut *tx, &[("fulfilled_at", &stamp)])
                        .await?;
                }
                Ok(())
            })
        })
}

/// `reverse_fulfilment` and `reverse_fulfilment_all`: quantities of a
/// hand-over are taken off again.
fn reverse(name: &str, to: SalesOrderStatus) -> Transition<SalesOrder> {
    let none_left = to == SalesOrderStatus::Confirmed;
    Transition::new(name)
        .label("Reverse fulfilment")
        .tone("warning")
        .system()
        .from([
            SalesOrderStatus::PartiallyFulfilled,
            SalesOrderStatus::Fulfilled,
        ])
        .to(to)
        .guard("quantities_held", |_o: &SalesOrder, ctx| {
            let problem = match ctx.input_as::<ReverseInput>() {
                Ok(input) => input.problem,
                Err(_) => Some(english("wl_sales.orders.problem.input", &[])),
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .guard(
            "confirmed_only_when_nothing_stays",
            move |_o: &SalesOrder, ctx| {
                let left = ctx.input_as::<ReverseInput>().map(|i| i.none_left);
                Box::pin(async move {
                    check(
                        left.is_ok_and(|l| l == none_left),
                        "wl_sales.orders.problem.completeness",
                    )
                })
            },
        )
        .effect(|order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let input = ctx.input_as::<ReverseInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                for line in &input.lines {
                    move_fulfilled(tx, id, line.line, -line.quantity_milli).await?;
                }
                FulfilmentRecord::query()
                    .where_(FulfilmentRecord::ORDER_ID.eq(id))
                    .where_(FulfilmentRecord::SOURCE_TYPE.eq(input.source.subject_type.clone()))
                    .where_(FulfilmentRecord::SOURCE_ID.eq(input.source.subject_id))
                    .update(&mut *tx, &[("lines", &input.held_after)])
                    .await?;
                let cleared: Option<renox::db::DateTime> = None;
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(&mut *tx, &[("fulfilled_at", &cleared)])
                    .await?;
                Ok(())
            })
        })
}

/// `cancel_line` and `cancel_line_partly`: open quantity of one line is
/// cancelled and the order stays where it is.
fn cancel_line(name: &str, state: SalesOrderStatus) -> Transition<SalesOrder> {
    Transition::new(name)
        .label("Cancel a line")
        .tone("danger")
        .permission(permissions::SALES_ORDERS_CANCEL)
        .reason_required()
        .from([state])
        .to(state)
        .guard("quantity_open", |_o: &SalesOrder, ctx| {
            let problem = match ctx.input_as::<CancelLineInput>() {
                Ok(input) => input.problem,
                Err(_) => Some(english("wl_sales.orders.problem.input", &[])),
            };
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .effect(|order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let input = ctx.input_as::<CancelLineInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                move_cancelled(tx, id, input.line, input.quantity_milli).await
            })
        })
}

/// `close`: fulfilled → closed once everything fulfilled is invoiced.
fn close() -> Transition<SalesOrder> {
    Transition::new(CLOSE)
        .label("Close")
        .tone("success")
        .system()
        .from([SalesOrderStatus::Fulfilled])
        .to(SalesOrderStatus::Closed)
        .guard("invoiced", |_o: &SalesOrder, ctx| {
            let invoiced = ctx.input_as::<CloseInput>().is_ok_and(|i| i.invoiced);
            Box::pin(async move { check(invoiced, "wl_sales.orders.problem.not_invoiced") })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_base_is_declared() {
        let machine = SalesOrder::machine();
        let table = [
            (SUBMIT, vec!["draft"], "confirmed", false),
            (SUBMIT_ONLINE, vec!["draft"], "awaiting_payment", false),
            (
                SUBMIT_FOR_ACCEPTANCE,
                vec!["draft"],
                "pending_acceptance",
                false,
            ),
            (CONFIRM_PAYMENT, vec!["awaiting_payment"], "confirmed", true),
            (
                CONFIRM_PAYMENT_FOR_ACCEPTANCE,
                vec!["awaiting_payment"],
                "pending_acceptance",
                true,
            ),
            (PAYMENT_FAILED, vec!["awaiting_payment"], "cancelled", true),
            (ACCEPT, vec!["pending_acceptance"], "confirmed", false),
            (REJECT, vec!["pending_acceptance"], "rejected", false),
            (
                CANCEL,
                vec!["draft", "pending_acceptance", "credit_hold", "confirmed"],
                "cancelled",
                false,
            ),
            (
                FULFIL,
                vec!["confirmed", "partially_fulfilled"],
                "partially_fulfilled",
                true,
            ),
            (
                FULFIL_LAST,
                vec!["confirmed", "partially_fulfilled"],
                "fulfilled",
                true,
            ),
            (
                REVERSE_FULFILMENT,
                vec!["partially_fulfilled", "fulfilled"],
                "partially_fulfilled",
                true,
            ),
            (
                REVERSE_FULFILMENT_ALL,
                vec!["partially_fulfilled", "fulfilled"],
                "confirmed",
                true,
            ),
            (CLOSE, vec!["fulfilled"], "closed", true),
            (CANCEL_LINE, vec!["confirmed"], "confirmed", false),
            (
                CANCEL_LINE_PARTLY,
                vec!["partially_fulfilled"],
                "partially_fulfilled",
                false,
            ),
            (SUBMIT_HOLD, vec!["draft"], "credit_hold", false),
            (
                ACCEPT_HOLD,
                vec!["pending_acceptance"],
                "credit_hold",
                false,
            ),
            (
                HOLD,
                vec!["confirmed", "partially_fulfilled"],
                "credit_hold",
                true,
            ),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, system) in table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.is_system(), system, "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["draft"]);
    }

    #[test]
    fn ending_an_order_needs_a_reason() {
        let machine = SalesOrder::machine();
        assert!(machine.find(CANCEL).unwrap().needs_reason());
        assert!(machine.find(REJECT).unwrap().needs_reason());
        assert!(machine.find(CANCEL_LINE).unwrap().needs_reason());
        assert!(!machine.find(ACCEPT).unwrap().needs_reason());
    }
}
