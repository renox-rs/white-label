//! The SalesReturn state machine (wl-sales §4.5; base for BL-SAL-22).
//!
//! ```text
//! [*] requested --receive--> received --resolve--> resolved --reverse--> reversed
//!     requested --resolve--> resolved   (kind no goods)
//!     requested | received --reject--> rejected
//! ```
//!
//! All four transitions are by a holder of `returns.handle`. The system
//! twins the spec allows for `resolve` and `reverse` (06's payout statement)
//! come with the vertical that needs them.
//!
//! Decisions:
//! - `receive` guards the return window; an approved late return and the
//!   always-accepted return of recalled goods (BL-SAL-15) are the `approved`
//!   flag of the input, which only those issues set.
//! - `resolve` of a goods return needs the inspection, one of a no goods
//!   return needs the complaint category and the responsible party; the
//!   approval above a value (`SAL.ACL.06`, BL-SAL-14) is not built here.
//!   A *credit note* resolution names the credit note the caller issued.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::permissions;
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
            .transition(receive())
            .transition(resolve())
            .transition(reject())
            .transition(reverse())
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

/// `resolve`: received (goods) or requested (no goods) → resolved.
fn resolve() -> Transition<SalesReturn> {
    Transition::new(RESOLVE)
        .label("Resolve")
        .tone("success")
        .permission(permissions::RETURNS_HANDLE)
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
        .guard("credit_note_named", |_r: &SalesReturn, ctx| {
            let input = ctx.input_as::<ResolveInput>().ok();
            Box::pin(async move {
                let ok = input.is_none_or(|i| {
                    i.resolution != Some(ReturnResolution::CreditNote) || i.credit_note.is_some()
                });
                check(ok, "wl_sales.returns.problem.credit_note")
            })
        })
        .effect(|r: &SalesReturn, ctx, tx| {
            let id = r.id;
            let input = ctx.input_as::<ResolveInput>().ok();
            let by = ctx.actor().map(|u| u.id);
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
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
                Ok(())
            })
        })
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

/// `reverse`: resolved → reversed, only a refund made by the channel.
fn reverse() -> Transition<SalesReturn> {
    Transition::new(REVERSE)
        .label("Reverse")
        .tone("warning")
        .permission(permissions::RETURNS_HANDLE)
        .reason_required()
        .from([ReturnStatus::Resolved])
        .to(ReturnStatus::Reversed)
        .guard("refunded_by_channel", |r: &SalesReturn, _ctx| {
            let by_channel = r.resolution == Some(ReturnResolution::RefundedByChannel);
            Box::pin(async move { check(by_channel, "wl_sales.returns.problem.not_by_channel") })
        })
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
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert!(!t.is_system(), "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["requested"]);
        assert!(machine.find(REJECT).unwrap().needs_reason());
        assert!(machine.find(REVERSE).unwrap().needs_reason());
    }
}
