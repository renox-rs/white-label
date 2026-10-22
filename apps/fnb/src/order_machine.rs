//! The F&B order state machine (06 F&B §7.1 *Order*).
//!
//! ```text
//! [*] placed --accept (sales accepted it)--> accepted
//! [*] accepted                                        (aggregator orders start here)
//! placed --reject (sales rejected it)--> rejected
//! accepted --start (system, first ticket started)--> preparing
//! preparing --mark_ready (system, last ticket bumped)--> ready
//! ready --dispatch (orders.manage, or system: the run left)--> dispatched
//! dispatched --fail_delivery (system, DeliveryAttemptFailed)--> delivery_failed
//! delivery_failed --close_failed (system: DeliveryReturned or DeliveryDiscarded; or orders.manage)--> cancelled
//! ready | dispatched --complete (orders.manage, or system: DeliveryDelivered)--> completed
//! placed | accepted | preparing | ready --cancel_system (the sales order was cancelled)--> cancelled
//! placed --amend_placed (orders.amend)--> placed
//! accepted --amend (orders.amend)--> accepted
//! accepted --cancel_line (orders.cancel)--> accepted
//! preparing --cancel_line_preparing (orders.cancel)--> preparing
//! ```
//!
//! Every state of §7.1 is declared with its tone, so the diagram is whole.
//!
//! `cancel` is the sales order's: a person asks the sales order desk
//! (`orders.cancel`, a manager's yes under `SAL.ACL.01` and `SAL.ACL.05`) or
//! the customer does from the order page, and the order follows the sales
//! order's `SalesOrderCancelled` with the system's `cancel_system`, so the two
//! never disagree ([`crate::cancel`]). There is no `void` on an order.
//! `accept` and `reject` follow the sales order: the Accept button asks the
//! sales order desk (`sales_orders.accept`) and the order moves when the
//! sales order does, so the two never disagree ([`crate::placing`]).
//!
//! Renox guards get no `AppState`, so the checks that need the database run
//! in the services ([`crate::fulfilment`], [`crate::failed`],
//! [`crate::close`]) and the guards read what they worked out in an
//! [`OrderInput`].

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::order::{FnbOrder, FnbOrderStatus, decision, map_reason, party, side};
use crate::permissions;
use crate::texts::english;

/// `accept`: placed → accepted, by the system when the sales order is
/// accepted.
pub const ACCEPT: &str = "accept";
/// `reject`: placed → rejected, by the system when the sales order is
/// rejected.
pub const REJECT: &str = "reject";
/// `start`: accepted → preparing, by the system with the first ticket.
pub const START: &str = "start";
/// `mark_ready`: preparing → ready, by the system with the last ticket.
pub const MARK_READY: &str = "mark_ready";
/// `dispatch`: ready → dispatched, a manager's, with a rider assigned.
pub const DISPATCH: &str = "dispatch";
/// `dispatch`, run by the system when the run of the delivery left.
pub const DISPATCH_SYSTEM: &str = "dispatch_system";
/// `fail_delivery`: dispatched → delivery_failed, by the system only
/// (`DeliveryAttemptFailed`, or a run abort with no hand-over).
pub const FAIL_DELIVERY: &str = "fail_delivery";
/// `close_failed`: delivery_failed → cancelled, a manager's.
pub const CLOSE_FAILED: &str = "close_failed";
/// `close_failed`, run by the system on `DeliveryReturned` or
/// `DeliveryDiscarded`.
pub const CLOSE_FAILED_SYSTEM: &str = "close_failed_system";
/// `complete`: ready, dispatched → completed, a manager's.
pub const COMPLETE: &str = "complete";
/// `complete`, run by the system on `DeliveryDelivered`.
pub const COMPLETE_SYSTEM: &str = "complete_system";
/// `amend`: placed → placed, a new revision of the sales order (BL-FNB-05).
pub const AMEND_PLACED: &str = "amend_placed";
/// `amend`: accepted → accepted.
pub const AMEND: &str = "amend";
/// `cancel_line`: accepted → accepted.
pub const CANCEL_LINE: &str = "cancel_line";
/// `cancel_line`: preparing → preparing.
pub const CANCEL_LINE_PREPARING: &str = "cancel_line_preparing";
/// `cancel`: placed, accepted, preparing, ready → cancelled, run by the system
/// when the sales order was cancelled (BL-FNB-04).
pub const CANCEL_SYSTEM: &str = "cancel_system";

/// Why a line is cancelled (`cancel_line`): an item ran out, or the customer
/// asked.
pub const LINE_REASONS: [&str; 2] = ["item_unavailable", "customer_request"];

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderInput {
    /// Whether a rider is assigned to the order's delivery (`dispatch`).
    #[serde(default)]
    pub rider_assigned: bool,
    /// The failure reason wl-delivery gave (`fail_delivery`); the order's
    /// reason, side and responsible party are mapped from it.
    #[serde(default)]
    pub delivery_reason: Option<String>,
    /// The attempt that failed (`fail_delivery`).
    #[serde(default)]
    pub attempt: i64,
    /// What the customer paid, in fils, as the sales order says now
    /// (`close_failed`).
    #[serde(default)]
    pub paid_amount: i64,
    /// Whether the client's setting refunds a customer-side failure in full
    /// too (open item L-33; off by default) (`close_failed`).
    #[serde(default)]
    pub customer_side_refund: bool,
    /// Why the order is closed, the reason of the cancellation
    /// (`close_failed`), or of a rejection (`reject`).
    #[serde(default)]
    pub close_reason: Option<String>,
    /// The wastage record the delivery's `discard` made (`close_failed`).
    #[serde(default)]
    pub wastage_ref: Option<String>,
    /// Whether the delivery's `discard` already posted the food as wastage
    /// (`close_failed`), or the cancellation scrapped started food
    /// (`cancel_system`).
    #[serde(default)]
    pub wastage_posted: bool,
    /// The party responsible for the cancellation, one of
    /// [`party::CANCEL`](crate::order::party::CANCEL) (`cancel_system`).
    #[serde(default)]
    pub cancel_party: Option<String>,
    /// The reason of the cancellation, one of
    /// [`cancel_reason`](crate::order::cancel_reason) (`cancel_system`).
    #[serde(default)]
    pub cancel_reason_code: Option<String>,
    /// The user whose cancellation it is (`cancel_system`).
    #[serde(default)]
    pub cancelled_by: Option<i64>,
    /// Whether the customer cancelled it (`cancel_system`).
    #[serde(default)]
    pub by_customer: bool,
    /// Whether the order came through an aggregator's channel: it is changed
    /// on the aggregator's tablet, never amended here (`amend`).
    #[serde(default)]
    pub aggregator: bool,
    /// The "customer agreed" tick of the call (`amend`).
    #[serde(default)]
    pub customer_agreed: bool,
    /// Whether the line the amendment changes was started by the kitchen
    /// (`amend`).
    #[serde(default)]
    pub started_changed: bool,
    /// Why a line is cancelled: `item_unavailable` or `customer_request`
    /// (`cancel_line`).
    #[serde(default)]
    pub line_reason_code: Option<String>,
    /// Whether the line's ticket was bumped already (`cancel_line`).
    #[serde(default)]
    pub line_bumped: bool,
}

/// The input of a run; the default without one.
pub(crate) fn input_of(ctx: &TransitionCtx) -> OrderInput {
    ctx.input_as::<OrderInput>().unwrap_or_default()
}

/// The refund decision the rule proposes for the failed `order` and the
/// amount it comes to (`FNB.ACL.17`): *not applicable* when nothing was
/// paid in advance (cash on delivery collected nothing), *full refund* of
/// everything paid for a failure on the outlet's side, *no refund* for one on
/// the customer's, unless the client's setting refunds those too (open item
/// L-33: the lawyer has not answered, the default is no refund).
pub fn proposed_decision(
    order: &FnbOrder,
    paid_amount: i64,
    customer_side_refund: bool,
) -> (&'static str, i64) {
    if order.is_cash_on_delivery() || paid_amount <= 0 {
        (decision::NOT_APPLICABLE, 0)
    } else if order.failure_side.as_deref() == Some(side::OUTLET) || customer_side_refund {
        (decision::FULL_REFUND, paid_amount)
    } else {
        (decision::NO_REFUND, 0)
    }
}

/// A system transition `name` from `from` to `to`.
fn system(
    name: &'static str,
    label: &'static str,
    from: &[FnbOrderStatus],
    to: FnbOrderStatus,
) -> Transition<FnbOrder> {
    Transition::new(name)
        .label(label)
        .system()
        .from(from.iter().copied())
        .to(to)
}

/// Stamps `column` of the order with now.
async fn stamp(tx: &mut Transaction, id: i64, column: &'static str) -> Result {
    let at = Some(renox::db::now());
    FnbOrder::query()
        .where_(FnbOrder::ID.eq(id))
        .update(&mut *tx, &[(column, &at)])
        .await?;
    Ok(())
}

fn accept() -> Transition<FnbOrder> {
    system(
        ACCEPT,
        "Accept",
        &[FnbOrderStatus::Placed],
        FnbOrderStatus::Accepted,
    )
    .effect(|order: &FnbOrder, _ctx, tx| {
        let id = order.id;
        Box::pin(async move { stamp(&mut *tx, id, "accepted_at").await })
    })
}

fn reject() -> Transition<FnbOrder> {
    system(
        REJECT,
        "Reject",
        &[FnbOrderStatus::Placed],
        FnbOrderStatus::Rejected,
    )
    .effect(|order: &FnbOrder, ctx, tx| {
        let id = order.id;
        let reason = input_of(ctx).close_reason;
        Box::pin(async move {
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(id))
                .update(&mut *tx, &[("close_reason", &reason)])
                .await?;
            Ok(())
        })
    })
}

fn dispatch(name: &'static str, by_user: bool) -> Transition<FnbOrder> {
    if !by_user {
        return system(
            name,
            "Dispatch",
            &[FnbOrderStatus::Ready],
            FnbOrderStatus::Dispatched,
        );
    }
    Transition::new(name)
        .label("Dispatch")
        .tone("primary")
        .permission(permissions::ORDERS_MANAGE)
        .from([FnbOrderStatus::Ready])
        .to(FnbOrderStatus::Dispatched)
        .guard("delivery_order_with_a_rider", |order: &FnbOrder, ctx| {
            let delivery = order.is_delivery();
            let assigned = input_of(ctx).rider_assigned;
            Box::pin(async move {
                if !delivery {
                    Err(english("fnb.dispatch.not_delivery", &[]))
                } else if !assigned {
                    Err(english("fnb.dispatch.no_rider", &[]))
                } else {
                    Ok(())
                }
            })
        })
}

fn fail_delivery() -> Transition<FnbOrder> {
    system(
        FAIL_DELIVERY,
        "Delivery failed",
        &[FnbOrderStatus::Dispatched],
        FnbOrderStatus::DeliveryFailed,
    )
    .guard("failure_reason_recorded", |_order: &FnbOrder, ctx| {
        let recorded = input_of(ctx)
            .delivery_reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty());
        Box::pin(async move {
            if recorded {
                Ok(())
            } else {
                Err(english("fnb.failed.reason_needed", &[]))
            }
        })
    })
    .effect(|order: &FnbOrder, ctx, tx| {
        let id = order.id;
        let input = input_of(ctx);
        Box::pin(async move {
            // The wl-delivery reason is kept; the order's reason, its side
            // and the proposed responsible party are the table's (§7.1).
            let delivery_reason = input.delivery_reason.clone();
            let mapped = map_reason(delivery_reason.as_deref().unwrap_or(""));
            let reason = Some(mapped.reason.to_owned());
            let side = Some(mapped.side.to_owned());
            let party = Some(mapped.party.to_owned());
            let at = Some(renox::db::now());
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(id))
                .update(
                    &mut *tx,
                    &[
                        ("failure_reason", &reason),
                        ("delivery_reason", &delivery_reason),
                        ("failure_side", &side),
                        ("failure_attempt", &input.attempt),
                        ("failed_at", &at),
                        ("responsible_party", &party),
                    ],
                )
                .await?;
            Ok(())
        })
    })
}

fn close_failed(name: &'static str, by_user: bool) -> Transition<FnbOrder> {
    let base = if by_user {
        Transition::new(name)
            .label("Close the failed delivery")
            .tone("danger")
            .permission(permissions::ORDERS_MANAGE)
            .from([FnbOrderStatus::DeliveryFailed])
            .to(FnbOrderStatus::Cancelled)
    } else {
        system(
            name,
            "Close the failed delivery",
            &[FnbOrderStatus::DeliveryFailed],
            FnbOrderStatus::Cancelled,
        )
    };
    // No `SAL.ACL.05` approval: the refund rule is `FNB.ACL.17`.
    base.effect(|order: &FnbOrder, ctx, tx| {
        let original = order.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            let (chosen, amount) = proposed_decision(&original, input.paid_amount, input.customer_side_refund);
            let decision = Some(chosen.to_owned());
            let at = Some(renox::db::now());
            let reason = input.close_reason.clone();
            // Food is posted as wastage once: by the discard, or by the scrap
            // of the cancellation that follows.
            let posted = original.wastage_posted || input.wastage_posted;
            let wastage = input.wastage_ref.clone().or_else(|| original.wastage_ref.clone());
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(original.id))
                .update(
                    &mut *tx,
                    &[
                        ("refund_decision", &decision),
                        ("refund_amount", &amount),
                        ("paid_amount", &input.paid_amount),
                        ("closed_at", &at),
                        ("close_reason", &reason),
                        ("wastage_posted", &posted),
                        ("wastage_ref", &wastage),
                    ],
                )
                .await?;
            Ok(())
        })
    })
}

fn complete(name: &'static str, by_user: bool) -> Transition<FnbOrder> {
    let from = [FnbOrderStatus::Ready, FnbOrderStatus::Dispatched];
    let base = if by_user {
        Transition::new(name)
            .label("Complete")
            .tone("success")
            .permission(permissions::ORDERS_MANAGE)
            .from(from)
            .to(FnbOrderStatus::Completed)
    } else {
        system(name, "Complete", &from, FnbOrderStatus::Completed)
    };
    base.effect(|order: &FnbOrder, _ctx, tx| {
        let id = order.id;
        Box::pin(async move { stamp(&mut *tx, id, "completed_at").await })
    })
}

/// `cancel_system`: the sales order was cancelled. The reason, the
/// responsible party and who asked were kept on the order when it was asked,
/// and the listener passes them in the input (`SAL.ACL.01`/`SAL.ACL.05`
/// approvals are the sales order's).
fn cancel_system() -> Transition<FnbOrder> {
    system(
        CANCEL_SYSTEM,
        "Cancel",
        &[
            FnbOrderStatus::Placed,
            FnbOrderStatus::Accepted,
            FnbOrderStatus::Preparing,
            FnbOrderStatus::Ready,
        ],
        FnbOrderStatus::Cancelled,
    )
    .guard("reason_and_party_recorded", |_order: &FnbOrder, ctx| {
        let input = input_of(ctx);
        let party_ok = input
            .cancel_party
            .as_deref()
            .is_some_and(|chosen| party::CANCEL.contains(&chosen));
        let reason_ok = input
            .cancel_reason_code
            .as_deref()
            .is_some_and(|code| !code.trim().is_empty());
        Box::pin(async move {
            if party_ok && reason_ok {
                Ok(())
            } else {
                Err(english("fnb.cancel.party_and_reason_needed", &[]))
            }
        })
    })
    .effect(|order: &FnbOrder, ctx, tx| {
        let original = order.clone();
        let input = input_of(ctx);
        Box::pin(async move {
            let compensation = input.cancel_party.as_deref() == Some(party::AGGREGATOR);
            let at = Some(renox::db::now());
            // A prepaid order is refunded in full by the sales order's
            // cancellation; an aggregator's or a cash order paid nothing.
            let paid = if input.paid_amount > 0 {
                input.paid_amount
            } else {
                original.paid_amount
            };
            let (chosen, amount) = if paid > 0 && !compensation {
                (decision::FULL_REFUND, paid)
            } else {
                (decision::NOT_APPLICABLE, 0)
            };
            let decided = Some(chosen.to_owned());
            let posted = original.wastage_posted || input.wastage_posted;
            let waiting: Option<i64> = None;
            FnbOrder::query()
                .where_(FnbOrder::ID.eq(original.id))
                .update(
                    &mut *tx,
                    &[
                        ("cancel_party", &input.cancel_party),
                        ("cancel_reason_code", &input.cancel_reason_code),
                        ("close_reason", &input.close_reason),
                        ("cancelled_by", &input.cancelled_by),
                        ("cancelled_by_customer", &input.by_customer),
                        ("cancelled_at", &at),
                        ("compensation_expected", &compensation),
                        ("refund_decision", &decided),
                        ("refund_amount", &amount),
                        ("wastage_posted", &posted),
                        ("cancel_request_id", &waiting),
                    ],
                )
                .await?;
            Ok(())
        })
    })
}

/// `amend`: a direct order's lines change after the cashier called the
/// customer (`orders.amend`). The sales order's amendment did it (revision,
/// money, kitchen tickets); this is the order's record of who and why.
fn amend(name: &'static str, state: FnbOrderStatus) -> Transition<FnbOrder> {
    Transition::new(name)
        .label("Amend")
        .tone("primary")
        .permission(permissions::ORDERS_AMEND)
        .reason_required()
        .from([state])
        .to(state)
        .guard("direct_order", |_order: &FnbOrder, ctx| {
            let aggregator = input_of(ctx).aggregator;
            Box::pin(async move {
                if aggregator {
                    Err(english("fnb.amend.aggregator", &[]))
                } else {
                    Ok(())
                }
            })
        })
        .guard("customer_agreed", |_order: &FnbOrder, ctx| {
            let agreed = input_of(ctx).customer_agreed;
            Box::pin(async move {
                if agreed {
                    Ok(())
                } else {
                    Err(english("fnb.amend.agreed", &[]))
                }
            })
        })
        .guard("changed_lines_not_started", |_order: &FnbOrder, ctx| {
            let started = input_of(ctx).started_changed;
            Box::pin(async move {
                if started {
                    Err(english("fnb.amend.started", &[]))
                } else {
                    Ok(())
                }
            })
        })
}

/// `cancel_line`: a line is dropped because an item ran out or the customer
/// asked (`orders.cancel`); the sales order's `cancel_line` did it (refund,
/// wastage, tickets), and a started line needed a manager (`SAL.ACL.05`).
fn cancel_line(name: &'static str, state: FnbOrderStatus) -> Transition<FnbOrder> {
    Transition::new(name)
        .label("Cancel line")
        .tone("danger")
        .permission(permissions::ORDERS_CANCEL)
        .reason_required()
        .from([state])
        .to(state)
        .guard("item_unavailable_or_customer_request", |_order: &FnbOrder, ctx| {
            let ok = input_of(ctx)
                .line_reason_code
                .as_deref()
                .is_some_and(|code| LINE_REASONS.contains(&code));
            Box::pin(async move {
                if ok {
                    Ok(())
                } else {
                    Err(english("fnb.line.reason", &[]))
                }
            })
        })
        .guard("line_not_bumped", |_order: &FnbOrder, ctx| {
            let bumped = input_of(ctx).line_bumped;
            Box::pin(async move {
                if bumped {
                    Err(english("fnb.line.bumped", &[]))
                } else {
                    Ok(())
                }
            })
        })
}

/// The transitions of the base and of the failed delivery.
pub(crate) fn transitions() -> Vec<Transition<FnbOrder>> {
    vec![
        accept(),
        reject(),
        system(
            START,
            "Start",
            &[FnbOrderStatus::Accepted],
            FnbOrderStatus::Preparing,
        ),
        system(
            MARK_READY,
            "Ready",
            &[FnbOrderStatus::Preparing],
            FnbOrderStatus::Ready,
        )
        .effect(|order: &FnbOrder, _ctx, tx| {
            let id = order.id;
            Box::pin(async move { stamp(&mut *tx, id, "ready_at").await })
        }),
        dispatch(DISPATCH, true),
        dispatch(DISPATCH_SYSTEM, false),
        fail_delivery(),
        close_failed(CLOSE_FAILED, true),
        close_failed(CLOSE_FAILED_SYSTEM, false),
        complete(COMPLETE, true),
        complete(COMPLETE_SYSTEM, false),
        cancel_system(),
        amend(AMEND_PLACED, FnbOrderStatus::Placed),
        amend(AMEND, FnbOrderStatus::Accepted),
        cancel_line(CANCEL_LINE, FnbOrderStatus::Accepted),
        cancel_line(CANCEL_LINE_PREPARING, FnbOrderStatus::Preparing),
    ]
}

impl StateMachine for FnbOrder {
    type State = FnbOrderStatus;
    const COLUMN: renox::db::Col<FnbOrder, FnbOrderStatus> = FnbOrder::STATUS;

    fn machine() -> Machine<Self> {
        use FnbOrderStatus::{
            Accepted, Cancelled, Completed, DeliveryFailed, Dispatched, Placed, Preparing, Ready,
            Rejected,
        };
        let machine = Machine::new()
            .initial([Placed, Accepted])
            .state(Placed)
            .tone("neutral")
            .state(Accepted)
            .tone("info")
            .state(Preparing)
            .tone("info")
            .state(Ready)
            .tone("success")
            .state(Dispatched)
            .tone("info")
            .state(DeliveryFailed)
            .tone("warning")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("error")
            .terminal();
        transitions().into_iter().fold(machine, Machine::transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(side: &str, mode: &str) -> FnbOrder {
        FnbOrder {
            payment_mode: mode.to_owned(),
            failure_side: Some(side.to_owned()),
            status: FnbOrderStatus::DeliveryFailed,
            ..FnbOrder::default()
        }
    }

    #[test]
    fn the_machine_declares_the_nine_states_and_the_transitions_of_the_base() {
        let machine = FnbOrder::machine();
        assert_eq!(machine.states().len(), 9);
        for name in [
            ACCEPT,
            REJECT,
            START,
            MARK_READY,
            DISPATCH,
            DISPATCH_SYSTEM,
            FAIL_DELIVERY,
            CLOSE_FAILED,
            CLOSE_FAILED_SYSTEM,
            COMPLETE,
            COMPLETE_SYSTEM,
            CANCEL_SYSTEM,
            AMEND_PLACED,
            AMEND,
            CANCEL_LINE,
            CANCEL_LINE_PREPARING,
        ] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
        // Orders are never voided.
        assert!(machine.find("void").is_err());
        // No user runs these.
        for name in [
            ACCEPT,
            REJECT,
            START,
            MARK_READY,
            DISPATCH_SYSTEM,
            FAIL_DELIVERY,
            CLOSE_FAILED_SYSTEM,
            COMPLETE_SYSTEM,
            CANCEL_SYSTEM,
        ] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
        for name in [
            DISPATCH,
            CLOSE_FAILED,
            COMPLETE,
            AMEND_PLACED,
            AMEND,
            CANCEL_LINE,
            CANCEL_LINE_PREPARING,
        ] {
            assert!(!machine.find(name).unwrap().is_system(), "{name}");
        }
    }

    #[test]
    fn the_rule_refunds_an_outlet_side_failure_in_full_and_a_customer_side_one_not_at_all() {
        let outlet = failed(side::OUTLET, "online");
        assert_eq!(
            proposed_decision(&outlet, 8_500, false),
            (decision::FULL_REFUND, 8_500)
        );
        let customer = failed(side::CUSTOMER, "online");
        assert_eq!(proposed_decision(&customer, 6_000, false),
            (decision::NO_REFUND, 0)
        );
        // The open item L-33 as a setting: the client may refund these too.
        assert_eq!(
            proposed_decision(&customer, 6_000, true),
            (decision::FULL_REFUND, 6_000)
        );
    }

    #[test]
    fn cash_on_delivery_and_an_unpaid_order_have_nothing_to_refund() {
        let cash = failed(side::OUTLET, "cash_on_delivery");
        assert_eq!(
            proposed_decision(&cash, 8_500, false),
            (decision::NOT_APPLICABLE, 0)
        );
        let unpaid = failed(side::OUTLET, "online");
        assert_eq!(
            proposed_decision(&unpaid, 0, false),
            (decision::NOT_APPLICABLE, 0)
        );
    }
}
