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
//!     pending_acceptance | credit_hold | confirmed | partially_fulfilled --amend--> same state
//!     pending_acceptance | confirmed | partially_fulfilled --hold--> credit_hold
//!     confirmed | partially_fulfilled --cancel_line--> same state, or fulfilled
//!     partially_fulfilled --cancel_remaining--> fulfilled
//! ```
//!
//! `cancel` (with its approval twin, [`crate::cancel_lines`], BL-SAL-07: it
//! needs the plan of the cancellation as its input, so the generic transition
//! route cannot run it), `amend` (a family of four, [`crate::amend`]) and `hold` ([`crate::credit`])
//! come with BL-SAL-05, `cancel_line` and `cancel_remaining` (a family of
//! four, [`crate::cancel_lines`]) with BL-SAL-08 and BL-SAL-09, each with the
//! `_approved` twin the kernel runs at the last yes. `release` and the rest
//! of the credit hold come with BL-SAL-10, the other changes with their
//! issues (BL-SAL-07, BL-SAL-11). `confirm_payment`, `payment_failed`,
//! `fulfil`, `fulfil_last`, `reverse_fulfilment`, `reverse_fulfilment_all` and
//! `close` are system transitions (the gateway, the hand-over, the invoicing
//! state); the rest are by users.
//!
//! `submit_online` also stamps the payment window (`payment_due_at`) and
//! every submit takes one use of the order's promo code; `payment_failed`
//! gives that use back (BL-SAL-02, BL-SAL-03).
//!
//! The inputs carry what a guard needs and an effect cannot fetch (no
//! `AppState`): the line count and totals, the amount paid, and for a
//! hand-over the problem the service found with the quantities.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::sales::{FulfilledLine, SALES_ORDER_SUBJECT_TYPE};

use crate::permissions;
use crate::promo_code::PromoCode;
use crate::sale_line::SaleLine;
use crate::sales_order::{FulfilmentRecord, OrderPaymentStatus, SalesOrder, SalesOrderStatus};
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
/// `draft | pending_acceptance | credit_hold | confirmed → cancelled`; declared
/// with the other cancellations in [`crate::cancel_lines`].
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
    /// The minutes the gateway has to confirm an online payment
    /// (`submit_online` only); 0 for the other submits.
    #[serde(default)]
    pub window_minutes: i64,
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
pub(crate) async fn move_cancelled(
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
        let machine = Machine::new()
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
            .transition(fulfil(FULFIL, PartiallyFulfilled))
            .transition(fulfil(FULFIL_LAST, Fulfilled))
            .transition(reverse(REVERSE_FULFILMENT, PartiallyFulfilled))
            .transition(reverse(REVERSE_FULFILMENT_ALL, Confirmed))
            .transition(close());
        // The changes of a placed order: amend (BL-SAL-05), cancel a line or
        // the remaining lines (BL-SAL-08, BL-SAL-09) and the credit hold they
        // can lead to (BL-SAL-05).
        let machine = crate::amend::add_to(machine);
        let machine = crate::cancel_lines::add_to(machine);
        crate::credit::add_to(machine)
    }
}

/// `submit`, `submit_online`, `submit_for_acceptance`: draft → `to`, with
/// the totals stored.
fn submit(name: &str, label: &str, to: SalesOrderStatus) -> Transition<SalesOrder> {
    let confirms = to == SalesOrderStatus::Confirmed;
    let online = to == SalesOrderStatus::AwaitingPayment;
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
        .effect(move |order: &SalesOrder, ctx, tx| {
            let id = order.id;
            let promo = order.promo_code_id;
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
                        ],
                    )
                    .await?;
                if online {
                    // The gateway has the payment window from now (BL-SAL-03).
                    let due = Some(now() + renox::chrono::TimeDelta::minutes(input.window_minutes));
                    SalesOrder::query()
                        .where_(SalesOrder::ID.eq(id))
                        .update(
                            &mut *tx,
                            &[
                                (
                                    "payment_status",
                                    &OrderPaymentStatus::Pending.as_str().to_owned(),
                                ),
                                ("payment_due_at", &due),
                            ],
                        )
                        .await?;
                }
                if let Some(promo) = promo {
                    take_promo_use(tx, promo).await?;
                }
                Ok(())
            })
        })
}

/// Takes one use of the promo code `promo` in one statement; fails (422) when
/// its uses are used up (BL-SAL-02: usage is recorded at `submit`).
async fn take_promo_use(tx: &mut renox::db::Transaction, promo: i64) -> Result {
    let changed = renox::db::sql(format!(
        "UPDATE {} SET uses = uses + 1, updated_at = ? \
         WHERE id = ? AND (max_uses IS NULL OR uses < max_uses)",
        PromoCode::TABLE
    ))
    .bind(now())
    .bind(promo)
    .execute(&mut *tx)
    .await?;
    if changed == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.orders.problem.promo_used_up", &[]),
        ));
    }
    Ok(())
}

/// Gives the use of the promo code `promo` back, in one statement (BL-SAL-03:
/// the use is available again at once).
async fn release_promo_use(tx: &mut renox::db::Transaction, promo: i64) -> Result {
    renox::db::sql(format!(
        "UPDATE {} SET uses = uses - 1, updated_at = ? WHERE id = ? AND uses > 0",
        PromoCode::TABLE
    ))
    .bind(now())
    .bind(promo)
    .execute(&mut *tx)
    .await?;
    Ok(())
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
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[(
                            "payment_status",
                            &OrderPaymentStatus::Received.as_str().to_owned(),
                        )],
                    )
                    .await?;
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
            let promo = order.promo_code_id;
            Box::pin(async move {
                let reason = Some(english("wl_sales.orders.payment_failed_reason", &[]));
                SalesOrder::query()
                    .where_(SalesOrder::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("reason", &reason),
                            (
                                "payment_status",
                                &OrderPaymentStatus::Failed.as_str().to_owned(),
                            ),
                        ],
                    )
                    .await?;
                // The failed order does not consume the promo code.
                if let Some(promo) = promo {
                    release_promo_use(tx, promo).await?;
                }
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
        ];
        assert!(machine.transitions().len() >= table.len());
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
        assert!(
            machine
                .find(crate::cancel_lines::CANCEL_LINE)
                .unwrap()
                .needs_reason()
        );
        assert!(!machine.find(ACCEPT).unwrap().needs_reason());
    }
}
