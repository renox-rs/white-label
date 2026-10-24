//! The SalesReturn state machine (wl-sales §4.5; base for BL-SAL-22).
//!
//! ```text
//! [*] requested --receive--> received --resolve--> resolved --reverse--> reversed
//!     requested --resolve--> resolved   (kind no goods)
//!     requested | received --reject--> rejected
//! ```
//!
//! The four transitions are by a holder of `returns.handle`. `resolve` and
//! `reverse` have system twins (BL-SAL-14): `resolve_approved` (the kernel
//! runs it at the last yes of a refund after completion, `SAL.ACL.06`),
//! `resolve_system` and `reverse_system` (the vertical's payout statement
//! `match` and `void`, only for a resolution *refunded by the channel*).
//!
//! Decisions:
//! - `receive` guards the return window; an approved late return and the
//!   always-accepted return of recalled goods (BL-SAL-15) are the `approved`
//!   flag of the input, which only those issues set.
//! - `resolve` of a goods return needs the inspection, one of a no goods
//!   return needs the complaint category and the responsible party; above the
//!   value of the settings a refund or credit of a no goods return waits for a
//!   manager who is not the recorder (`SAL.ACL.06`), decided by the service
//!   ([`crate::sales_returns`]) and enforced here by `needs_approval`. A
//!   *credit note* resolution names the credit note the caller issued.
//! - The order's *amount refunded* ([`SalesOrder::refunded_amount`]) rises
//!   with every resolution that gives money back (not a replacement) and falls
//!   when a channel refund is reversed; for a no goods return it may not
//!   exceed what the order was worth.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use wl_kernel::sales::SALES_ORDER_SUBJECT_TYPE;

use crate::permissions;
use crate::sales_order::SalesOrder;
use crate::sales_return::{ReturnKind, ReturnResolution, ReturnStatus, SalesReturn};
use crate::texts::english;

/// `requested → received`: the goods are back.
pub const RECEIVE: &str = "receive";
/// `received | requested → resolved`.
pub const RESOLVE: &str = "resolve";
/// `requested | received → rejected`.
pub const REJECT: &str = "reject";
/// `resolved → reversed`: a refund made by the channel was taken back.
pub const REVERSE: &str = "reverse";
/// `requested | received → resolved`, run by the kernel at the last approval.
pub const RESOLVE_APPROVED: &str = "resolve_approved";
/// `requested → resolved` by the system: the vertical's statement `match`
/// (resolution *refunded by the channel* only).
pub const RESOLVE_SYSTEM: &str = "resolve_system";
/// `resolved → reversed` by the system: the vertical's statement `void` or a
/// dispute the aggregator won (a channel refund only).
pub const REVERSE_SYSTEM: &str = "reverse_system";

/// What `receive` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveInput {
    /// The days between the sale and today.
    pub days_since_sale: i64,
    /// The return window in days (§7: 14).
    pub window_days: i64,
    /// Whether the return is accepted outside the window (approved, or goods
    /// of a recall).
    pub approved: bool,
}

/// What `resolve` takes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolveInput {
    /// How it is resolved.
    pub resolution: Option<ReturnResolution>,
    /// The credit note issued for it (a *credit note* resolution).
    pub credit_note: Option<i64>,
    /// Whether the inspection was recorded (goods).
    pub inspected: bool,
    /// The complaint category (no goods).
    pub complaint_category: Option<String>,
    /// The responsible party (no goods).
    pub responsible_party: Option<String>,
    /// The user who resolves it (the actor of the twin the kernel runs).
    pub by: Option<i64>,
    /// Whether a manager must approve before it resolves (`SAL.ACL.06`): the
    /// user's `resolve` refuses it, the twin runs after the yes.
    pub needs_approval: bool,
    /// Whether the sale is not an order of an aggregator channel (a *refunded
    /// by the channel* resolution is refused there).
    pub not_aggregator: bool,
    /// Why the service found the resolution not acceptable (a cap, a value
    /// not in the settings), `None` when nothing is wrong.
    pub problem: Option<String>,
    /// Whether the vertical resolves the *no goods* return as the system
    /// (06: a delivery reversed after review): a refund or a credit note, with
    /// no approval, and a credit note names no document (nothing was paid).
    pub by_vertical: bool,
}

/// A guard that fails with the English `key` unless `ok`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// The reason of the run, trimmed; `None` when blank.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

impl StateMachine for SalesReturn {
    type State = ReturnStatus;
    const COLUMN: renox::db::Col<SalesReturn, ReturnStatus> = SalesReturn::STATUS;

    fn machine() -> Machine<Self> {
        use ReturnStatus::{Received, Rejected, Requested, Resolved, Reversed};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .tone("neutral")
            .state(Received)
            .tone("info")
            .state(Resolved)
            .tone("success")
            .state(Reversed)
            .tone("neutral")
            .terminal()
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .transition(crate::search::reindex_return(receive()))
            .transition(crate::search::reindex_return(resolve()))
            .transition(crate::search::reindex_return(reject()))
            .transition(crate::search::reindex_return(reverse()))
            .transition(crate::search::reindex_return(resolve_approved()))
            .transition(crate::search::reindex_return(resolve_system()))
            .transition(crate::search::reindex_return(reverse_system()))
    }
}

/// `receive`: requested → received, inside the window.
fn receive() -> Transition<SalesReturn> {
    Transition::new(RECEIVE)
        .label("Receive the goods")
        .tone("primary")
        .permission(permissions::RETURNS_HANDLE)
        .from([ReturnStatus::Requested])
        .to(ReturnStatus::Received)
        .guard("goods_return", |r: &SalesReturn, _ctx| {
            let goods = r.kind == ReturnKind::Goods;
            Box::pin(async move { check(goods, "wl_sales.returns.problem.no_goods_to_receive") })
        })
        .guard("within_window", |_r: &SalesReturn, ctx| {
            let input = ctx.input_as::<ReceiveInput>().ok();
            Box::pin(async move {
                let ok = input.is_some_and(|i| i.approved || i.days_since_sale <= i.window_days);
                check(ok, "wl_sales.returns.problem.window")
            })
        })
        .effect(|r: &SalesReturn, _ctx, tx| {
            let id = r.id;
            Box::pin(async move {
                let stamp = Some(now());
                SalesReturn::query()
                    .where_(SalesReturn::ID.eq(id))
                    .update(&mut *tx, &[("received_at", &stamp)])
                    .await?;
                Ok(())
            })
        })
}

/// Adds `delta` (negative to take off) to the amount refunded of the sales
/// order `order`, in one statement. With `capped` it fails (422) when the
/// result would exceed what the order is worth, and it never goes below zero.
async fn move_refunded(
    tx: &mut renox::db::Transaction,
    order: i64,
    delta: i64,
    capped: bool,
) -> Result {
    let cap = if capped {
        " AND refunded_amount + ? <= gross_total"
    } else {
        ""
    };
    let query = renox::db::sql(format!(
        "UPDATE {} SET refunded_amount = refunded_amount + ?, updated_at = ? \
         WHERE id = ? AND refunded_amount + ? >= 0{cap}",
        SalesOrder::TABLE
    ))
    .bind(delta)
    .bind(now())
    .bind(order)
    .bind(delta);
    let query = if capped { query.bind(delta) } else { query };
    let changed = query.execute(&mut *tx).await?;
    if changed == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.returns.problem.cap_order", &[]),
        ));
    }
    Ok(())
}

/// Whether a resolution gives money back (and so counts as refunded).
fn gives_money_back(resolution: Option<ReturnResolution>) -> bool {
    resolution.is_some_and(|r| r != ReturnResolution::Replacement)
}

/// The guards and the effect the three resolving transitions share. `user`
/// is the holder of `returns.handle` (the approval is checked for it only);
/// `channel_only` is the system twin of the payout statement.
fn resolving(
    transition: Transition<SalesReturn>,
    user: bool,
    channel_only: bool,
) -> Transition<SalesReturn> {
    transition
        .from([ReturnStatus::Received, ReturnStatus::Requested])
        .to(ReturnStatus::Resolved)
        .guard("resolution_chosen", |_r: &SalesReturn, ctx| {
            let chosen = ctx
                .input_as::<ResolveInput>()
                .is_ok_and(|i| i.resolution.is_some());
            Box::pin(async move { check(chosen, "wl_sales.returns.problem.resolution") })
        })
        .guard("goods_received_and_inspected", |r: &SalesReturn, ctx| {
            let goods = r.kind == ReturnKind::Goods;
            let received = r.status == ReturnStatus::Received;
            let inspected = ctx.input_as::<ResolveInput>().is_ok_and(|i| i.inspected);
            Box::pin(async move {
                check(
                    !goods || (received && inspected),
                    "wl_sales.returns.problem.inspection",
                )
            })
        })
        .guard("complaint_recorded", |r: &SalesReturn, ctx| {
            let no_goods = r.kind == ReturnKind::NoGoods;
            let input = ctx.input_as::<ResolveInput>().ok();
            Box::pin(async move {
                let filled =
                    |text: &Option<String>| text.as_deref().is_some_and(|t| !t.trim().is_empty());
                let ok = !no_goods
                    || input.is_some_and(|i| {
                        filled(&i.complaint_category) && filled(&i.responsible_party)
                    });
                check(ok, "wl_sales.returns.problem.complaint")
            })
        })
        .guard(
            "channel_refund_for_no_goods_only",
            |r: &SalesReturn, ctx| {
                let no_goods = r.kind == ReturnKind::NoGoods;
                let by_channel = ctx
                    .input_as::<ResolveInput>()
                    .is_ok_and(|i| i.resolution == Some(ReturnResolution::RefundedByChannel));
                Box::pin(async move {
                    check(!by_channel || no_goods, "wl_sales.returns.problem.channel")
                })
            },
        )
        .guard(
            "channel_refund_on_an_aggregator_order",
            |_r: &SalesReturn, ctx| {
                let wrong = ctx.input_as::<ResolveInput>().is_ok_and(|i| {
                    i.resolution == Some(ReturnResolution::RefundedByChannel) && i.not_aggregator
                });
                Box::pin(async move { check(!wrong, "wl_sales.returns.problem.channel_order") })
            },
        )
        .guard("credit_note_named", |_r: &SalesReturn, ctx| {
            let input = ctx.input_as::<ResolveInput>().ok();
            Box::pin(async move {
                let ok = input.is_none_or(|i| {
                    i.resolution != Some(ReturnResolution::CreditNote)
                        || i.credit_note.is_some()
                        || i.by_vertical
                });
                check(ok, "wl_sales.returns.problem.credit_note")
            })
        })
        .guard("service_found_nothing_wrong", |_r: &SalesReturn, ctx| {
            let problem = ctx.input_as::<ResolveInput>().ok().and_then(|i| i.problem);
            Box::pin(async move { problem.map_or(Ok(()), Err) })
        })
        .guard("approved_above_the_limit", move |_r: &SalesReturn, ctx| {
            let waits = user
                && ctx
                    .input_as::<ResolveInput>()
                    .is_ok_and(|i| i.needs_approval);
            Box::pin(async move { check(!waits, "wl_sales.returns.problem.needs_approval") })
        })
        .guard("channel_refund_only", move |_r: &SalesReturn, ctx| {
            let wrong = channel_only
                && ctx.input_as::<ResolveInput>().is_ok_and(|i| {
                    i.resolution != Some(ReturnResolution::RefundedByChannel) && !i.by_vertical
                });
            Box::pin(async move { check(!wrong, "wl_sales.returns.problem.channel_only") })
        })
        .effect(|r: &SalesReturn, ctx, tx| {
            let id = r.id;
            let (sale_type, sale_id) = (r.sale_type.clone(), r.sale_id);
            let (value, no_goods) = (r.gross_value, r.kind == ReturnKind::NoGoods);
            let input = ctx.input_as::<ResolveInput>().ok();
            let by = ctx.actor().map(|u| u.id);
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                let by = by.or(input.by);
                let stamp = Some(now());
                SalesReturn::query()
                    .where_(SalesReturn::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("resolution", &input.resolution),
                            ("credit_note_id", &input.credit_note),
                            ("complaint_category", &input.complaint_category),
                            ("responsible_party", &input.responsible_party),
                            ("handled_by", &by),
                            ("resolved_at", &stamp),
                        ],
                    )
                    .await?;
                if sale_type == SALES_ORDER_SUBJECT_TYPE && gives_money_back(input.resolution) {
                    move_refunded(tx, sale_id, value, no_goods).await?;
                }
                Ok(())
            })
        })
}

/// `resolve`: received (goods) or requested (no goods) → resolved.
fn resolve() -> Transition<SalesReturn> {
    resolving(
        Transition::new(RESOLVE)
            .label("Resolve")
            .tone("success")
            .permission(permissions::RETURNS_HANDLE),
        true,
        false,
    )
}

/// `resolve_approved`: the kernel's run at the last yes.
fn resolve_approved() -> Transition<SalesReturn> {
    resolving(
        Transition::new(wl_kernel::machines::approved(RESOLVE))
            .label("Resolve")
            .tone("success")
            .system(),
        false,
        false,
    )
}

/// `resolve_system`: the vertical's payout statement `match`.
fn resolve_system() -> Transition<SalesReturn> {
    resolving(
        Transition::new(RESOLVE_SYSTEM)
            .label("Resolve")
            .tone("success")
            .system(),
        false,
        true,
    )
}

/// `reject`: requested | received → rejected, with a reason.
fn reject() -> Transition<SalesReturn> {
    Transition::new(REJECT)
        .label("Reject")
        .tone("danger")
        .permission(permissions::RETURNS_HANDLE)
        .reason_required()
        .from([ReturnStatus::Requested, ReturnStatus::Received])
        .to(ReturnStatus::Rejected)
        .effect(|r: &SalesReturn, ctx, tx| {
            let id = r.id;
            let reason = reason_of(ctx);
            Box::pin(async move {
                SalesReturn::query()
                    .where_(SalesReturn::ID.eq(id))
                    .update(&mut *tx, &[("closing_reason", &reason)])
                    .await?;
                Ok(())
            })
        })
}

/// `reverse`, `reverse_system`: resolved → reversed, only a refund made by
/// the channel; the order's amount refunded goes down by the return's value.
fn reversing(transition: Transition<SalesReturn>) -> Transition<SalesReturn> {
    transition
        .reason_required()
        .from([ReturnStatus::Resolved])
        .to(ReturnStatus::Reversed)
        .guard("refunded_by_channel", |r: &SalesReturn, _ctx| {
            let by_channel = r.resolution == Some(ReturnResolution::RefundedByChannel);
            Box::pin(async move { check(by_channel, "wl_sales.returns.problem.not_by_channel") })
        })
        .effect(|r: &SalesReturn, ctx, tx| {
            let id = r.id;
            let (sale_type, sale_id, value) = (r.sale_type.clone(), r.sale_id, r.gross_value);
            let reason = reason_of(ctx);
            Box::pin(async move {
                SalesReturn::query()
                    .where_(SalesReturn::ID.eq(id))
                    .update(&mut *tx, &[("closing_reason", &reason)])
                    .await?;
                if sale_type == SALES_ORDER_SUBJECT_TYPE {
                    move_refunded(tx, sale_id, -value, false).await?;
                }
                Ok(())
            })
        })
}

/// `reverse`: by a holder of `returns.handle`.
fn reverse() -> Transition<SalesReturn> {
    reversing(
        Transition::new(REVERSE)
            .label("Reverse")
            .tone("warning")
            .permission(permissions::RETURNS_HANDLE),
    )
}

/// `reverse_system`: by the vertical.
fn reverse_system() -> Transition<SalesReturn> {
    reversing(
        Transition::new(REVERSE_SYSTEM)
            .label("Reverse")
            .tone("warning")
            .system(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = SalesReturn::machine();
        let table = [
            (RECEIVE, vec!["requested"], "received"),
            (RESOLVE, vec!["received", "requested"], "resolved"),
            (REJECT, vec!["requested", "received"], "rejected"),
            (REVERSE, vec!["resolved"], "reversed"),
        ];
        assert_eq!(machine.transitions().len(), table.len() + 3);
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(!t.is_system(), "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        for (name, from, to) in [
            (RESOLVE_APPROVED, vec!["received", "requested"], "resolved"),
            (RESOLVE_SYSTEM, vec!["received", "requested"], "resolved"),
            (REVERSE_SYSTEM, vec!["resolved"], "reversed"),
        ] {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert!(machine.find(REVERSE_SYSTEM).unwrap().needs_reason());
        assert_eq!(machine.initial_states(), ["requested"]);
        assert!(machine.find(REJECT).unwrap().needs_reason());
        assert!(machine.find(REVERSE).unwrap().needs_reason());
    }
}
