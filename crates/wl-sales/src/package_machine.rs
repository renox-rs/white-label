//! The Package state machine (wl-sales §4.4; base for BL-SAL-16).
//!
//! ```text
//! [*] pending_payment --activate--> active --consume--> active
//!                     active --consume_last--> completed
//!                     active | completed --reverse--> active
//!                     active --expire--> expired --extend--> active
//!                     active --extend--> active
//!                     pending_payment --cancel--> cancelled
//! ```
//!
//! `activate`, `consume`, `consume_last`, `reverse_system`, `extend_system`
//! and `expire` are system transitions: the payment event, the consuming
//! source, the cancelled source, the vertical's extension by the days an
//! agreement was frozen (10) and the daily run. `reverse` (`packages.adjust`),
//! `extend` (`packages.extend`) and `cancel` (`packages.sell`) are by users.
//!
//! Decisions:
//! - The spec's `consume` ends in `active` or `completed`; a transition has
//!   one target, so it is two transitions with guards that exclude each
//!   other: `consume` while credits stay, `consume_last` when the quantity is
//!   what is left.
//! - `completed` is not marked terminal in the machine, because `reverse`
//!   leaves it (the spec's diagram).
//! - `refund` is not built here.
//! - `consume` has the guard `paid_up` (BL-SAL-16): with the setting *suspend
//!   consumption while unpaid* on, a package with a reopened, unpaid balance
//!   takes no credits.
//!
//! The inputs carry what a guard needs and an effect cannot fetch (no
//! `AppState`): the day, the quantity, the settings.

use renox::chrono::{Months, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::package::{MovementKind, Package, PackageMovement, PackageStatus};
use crate::permissions;
use crate::texts::english;

/// `pending_payment → active`: the payment was recorded.
pub const ACTIVATE: &str = "activate";
/// `active → active`: credits used, some left.
pub const CONSUME: &str = "consume";
/// `active → completed`: the last credits used.
pub const CONSUME_LAST: &str = "consume_last";
/// `active | completed → active`: credits given back by a user.
pub const REVERSE: &str = "reverse";
/// `active | completed → active`: credits given back because the source was
/// cancelled.
pub const REVERSE_SYSTEM: &str = "reverse_system";
/// `active → expired`: the end date passed.
pub const EXPIRE: &str = "expire";
/// `active | expired → active`: a new end date.
pub const EXTEND: &str = "extend";
/// `active | expired → active`: a new end date moved by the vertical (10: the
/// days an enrolment was frozen); it does not count against the limit.
pub const EXTEND_SYSTEM: &str = "extend_system";
/// `pending_payment → cancelled`: nothing was paid.
pub const CANCEL: &str = "cancel";

/// What `activate` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivateInput {
    /// The day it activates (Asia/Dubai); the validity starts on it.
    pub on: NaiveDate,
    /// Whether the invoice is paid in full, or the first instalment is paid
    /// and instalments are allowed.
    pub paid: bool,
}

/// What `consume` and `consume_last` take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumeInput {
    /// The credits to use.
    pub quantity: i64,
    /// The record that consumes them.
    pub source: Subject,
    /// The day (Asia/Dubai), for the validity check.
    pub today: NaiveDate,
    /// The setting *suspend consumption while unpaid* (§7).
    pub suspend_while_unpaid: bool,
}

/// What `reverse` and `reverse_system` take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverseInput {
    /// The credits to give back.
    pub quantity: i64,
    /// The record whose consumption is undone, if any.
    pub source: Option<Subject>,
}

/// What `expire` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpireInput {
    /// The day (Asia/Dubai).
    pub today: NaiveDate,
}

/// What `extend` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendInput {
    /// The new last day of validity.
    pub new_end: NaiveDate,
    /// The day (Asia/Dubai).
    pub today: NaiveDate,
    /// How many extensions a package may have (§7: one).
    pub limit: i64,
}

/// What `cancel` takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelInput {
    /// Whether nothing was paid on the package's invoice.
    pub nothing_paid: bool,
}

/// The reason of the run, trimmed; `None` when blank.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// A guard that fails with the English `key` unless `ok`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// Sets non-status columns of package `id` outside the machine's guard and
/// stamps `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    Package::query()
        .where_(Package::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// Adds `delta` (negative to give back) to the credits used of package `id`,
/// in one statement, so two consumptions cannot lose one another. Fails when
/// the result would leave `0..=total`.
async fn move_credits(tx: &mut renox::db::Transaction, id: i64, delta: i64) -> Result {
    let changed = renox::db::sql(format!(
        "UPDATE {} SET credits_used = credits_used + ?, updated_at = ? \
         WHERE id = ? AND credits_used + ? >= 0 AND credits_used + ? <= credits_total",
        Package::TABLE
    ))
    .bind(delta)
    .bind(now())
    .bind(id)
    .bind(delta)
    .bind(delta)
    .execute(&mut *tx)
    .await?;
    if changed == 0 {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            english("wl_sales.packages.problem.credits", &[]),
        ));
    }
    Ok(())
}

/// Records a movement.
async fn movement(
    tx: &mut renox::db::Transaction,
    package: i64,
    kind: MovementKind,
    quantity: i64,
    source: Option<&Subject>,
    user: Option<i64>,
    note: String,
) -> Result {
    PackageMovement {
        package_id: package,
        kind,
        quantity,
        source_type: source.map(|s| s.subject_type.clone()),
        source_id: source.map(|s| s.subject_id),
        user_id: user,
        note,
        ..PackageMovement::default()
    }
    .insert(&mut *tx)
    .await?;
    Ok(())
}

impl StateMachine for Package {
    type State = PackageStatus;
    const COLUMN: renox::db::Col<Package, PackageStatus> = Package::STATUS;

    fn machine() -> Machine<Self> {
        use PackageStatus::{Active, Cancelled, Completed, Expired, PendingPayment, Refunded};
        Machine::new()
            .initial([PendingPayment])
            .state(PendingPayment)
            .tone("neutral")
            .state(Active)
            .tone("info")
            .state(Completed)
            .tone("success")
            .state(Expired)
            .tone("warning")
            .state(Refunded)
            .tone("neutral")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(activate())
            .transition(consume(CONSUME, "Consume", Active))
            .transition(consume(CONSUME_LAST, "Consume the last credits", Completed))
            .transition(reverse(REVERSE, true))
            .transition(reverse(REVERSE_SYSTEM, false))
            .transition(expire())
            .transition(extend(EXTEND, false))
            .transition(extend(EXTEND_SYSTEM, true))
            .transition(cancel())
    }
}

/// `activate`: pending_payment → active, validity dates set.
fn activate() -> Transition<Package> {
    Transition::new(ACTIVATE)
        .label("Activate")
        .tone("success")
        .system()
        .from([PackageStatus::PendingPayment])
        .to(PackageStatus::Active)
        .guard("paid", |_p: &Package, ctx| {
            let paid = ctx.input_as::<ActivateInput>().is_ok_and(|i| i.paid);
            Box::pin(async move { check(paid, "wl_sales.packages.problem.not_paid") })
        })
        .effect(|package: &Package, ctx, tx| {
            let (id, months) = (package.id, package.validity_months);
            let on = ctx.input_as::<ActivateInput>().ok().map(|i| i.on);
            Box::pin(async move {
                let Some(on) = on else {
                    return Ok(());
                };
                let end = on
                    .checked_add_months(Months::new(u32::try_from(months).unwrap_or(6)))
                    .unwrap_or(on);
                let stamp = Some(now());
                set(
                    tx,
                    id,
                    &[
                        ("valid_from", &Some(on)),
                        ("valid_to", &Some(end)),
                        ("activated_at", &stamp),
                    ],
                )
                .await
            })
        })
}

/// `consume` (credits stay) and `consume_last` (credits reach zero): active →
/// `to`, with the movement.
fn consume(name: &str, label: &str, to: PackageStatus) -> Transition<Package> {
    let last = to == PackageStatus::Completed;
    Transition::new(name)
        .label(label)
        .tone("info")
        .system()
        .from([PackageStatus::Active])
        .to(to)
        .guard("credits_remaining", move |p: &Package, ctx| {
            let remaining = p.remaining();
            let quantity = ctx.input_as::<ConsumeInput>().map(|i| i.quantity);
            Box::pin(async move {
                match quantity {
                    Ok(q) if q >= 1 && q <= remaining && (q == remaining) == last => Ok(()),
                    Ok(q) if q < 1 || q > remaining => {
                        Err(english("wl_sales.packages.problem.credits", &[]))
                    }
                    Ok(_) if last => Err(english("wl_sales.packages.problem.not_last", &[])),
                    Ok(_) => Err(english("wl_sales.packages.problem.is_last", &[])),
                    Err(_) => Err(english("wl_sales.packages.problem.credits", &[])),
                }
            })
        })
        .guard("paid_up", |p: &Package, ctx| {
            // With the setting on, a reopened balance that is unpaid
            // suspends consumption (§4.4).
            let suspended = p.has_unpaid_balance()
                && ctx
                    .input_as::<ConsumeInput>()
                    .is_ok_and(|i| i.suspend_while_unpaid);
            Box::pin(async move { check(!suspended, "wl_sales.packages.problem.unpaid") })
        })
        .guard("not_expired", |p: &Package, ctx| {
            let expired = ctx
                .input_as::<ConsumeInput>()
                .is_ok_and(|i| p.is_past_validity(i.today));
            Box::pin(async move { check(!expired, "wl_sales.packages.problem.expired") })
        })
        .effect(|package: &Package, ctx, tx| {
            let id = package.id;
            let input = ctx.input_as::<ConsumeInput>().ok();
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                move_credits(tx, id, input.quantity).await?;
                movement(
                    tx,
                    id,
                    MovementKind::Consume,
                    input.quantity,
                    Some(&input.source),
                    None,
                    String::new(),
                )
                .await
            })
        })
}

/// `reverse` / `reverse_system`: active | completed → active, the credits
/// given back with the reason.
fn reverse(name: &str, by_user: bool) -> Transition<Package> {
    let transition = Transition::new(name)
        .label("Give credits back")
        .tone("warning")
        .from([PackageStatus::Active, PackageStatus::Completed])
        .to(PackageStatus::Active)
        .guard("credits_used", |p: &Package, ctx| {
            let used = p.credits_used;
            let quantity = ctx.input_as::<ReverseInput>().map(|i| i.quantity);
            Box::pin(async move {
                check(
                    quantity.is_ok_and(|q| q >= 1 && q <= used),
                    "wl_sales.packages.problem.reverse",
                )
            })
        })
        .effect(|package: &Package, ctx, tx| {
            let id = package.id;
            let input = ctx.input_as::<ReverseInput>().ok();
            let (user, note) = (ctx.actor().map(|u| u.id), reason_of(ctx));
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                move_credits(tx, id, -input.quantity).await?;
                movement(
                    tx,
                    id,
                    MovementKind::Reverse,
                    input.quantity,
                    input.source.as_ref(),
                    user,
                    note.unwrap_or_default(),
                )
                .await
            })
        });
    if by_user {
        transition
            .permission(permissions::PACKAGES_ADJUST)
            .reason_required()
    } else {
        transition.system()
    }
}

/// `expire`: active → expired once the end date passed.
fn expire() -> Transition<Package> {
    Transition::new(EXPIRE)
        .label("Expire")
        .tone("warning")
        .system()
        .from([PackageStatus::Active])
        .to(PackageStatus::Expired)
        .guard("end_date_passed", |p: &Package, ctx| {
            let passed = ctx
                .input_as::<ExpireInput>()
                .is_ok_and(|i| p.is_past_validity(i.today));
            Box::pin(async move { check(passed, "wl_sales.packages.problem.not_ended") })
        })
}

/// `extend`: active | expired → active with a later end date, within the
/// limit.
fn extend(name: &'static str, system: bool) -> Transition<Package> {
    let transition = Transition::new(name)
        .label("Extend")
        .tone("info")
        .from([PackageStatus::Active, PackageStatus::Expired])
        .to(PackageStatus::Active);
    // A user needs `packages.extend`; the twin is the vertical's, for the days
    // an enrolment was frozen.
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::PACKAGES_EXTEND)
    };
    transition
        .guard("later_end_date", |p: &Package, ctx| {
            let current = p.valid_to;
            let input = ctx.input_as::<ExtendInput>().ok();
            Box::pin(async move {
                let ok = input.is_some_and(|i| {
                    i.new_end >= i.today && current.is_none_or(|end| i.new_end > end)
                });
                check(ok, "wl_sales.packages.problem.new_end")
            })
        })
        .guard("within_limit", |p: &Package, ctx| {
            let used = p.extensions;
            let limit = ctx.input_as::<ExtendInput>().map(|i| i.limit);
            Box::pin(async move {
                check(
                    limit.is_ok_and(|l| used < l),
                    "wl_sales.packages.problem.extension_limit",
                )
            })
        })
        .effect(move |package: &Package, ctx, tx| {
            let (id, extensions) = (package.id, package.extensions);
            let input = ctx.input_as::<ExtendInput>().ok();
            let (user, note) = (ctx.actor().map(|u| u.id), reason_of(ctx));
            Box::pin(async move {
                let Some(input) = input else {
                    return Ok(());
                };
                // The days a frozen enrolment is given back are not an
                // extension the limit counts.
                let counted = if system { extensions } else { extensions + 1 };
                set(
                    tx,
                    id,
                    &[("valid_to", &Some(input.new_end)), ("extensions", &counted)],
                )
                .await?;
                movement(
                    tx,
                    id,
                    MovementKind::Extend,
                    0,
                    None,
                    user,
                    note.unwrap_or_default(),
                )
                .await
            })
        })
}

/// `cancel`: pending_payment → cancelled when nothing was paid.
fn cancel() -> Transition<Package> {
    Transition::new(CANCEL)
        .label("Cancel")
        .tone("danger")
        .permission(permissions::PACKAGES_SELL)
        .from([PackageStatus::PendingPayment])
        .to(PackageStatus::Cancelled)
        .guard("nothing_paid", |_p: &Package, ctx| {
            let nothing = ctx.input_as::<CancelInput>().is_ok_and(|i| i.nothing_paid);
            Box::pin(async move { check(nothing, "wl_sales.packages.problem.paid") })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transition_of_the_spec_is_declared() {
        let machine = Package::machine();
        let table = [
            (ACTIVATE, vec!["pending_payment"], "active", true),
            (CONSUME, vec!["active"], "active", true),
            (CONSUME_LAST, vec!["active"], "completed", true),
            (REVERSE, vec!["active", "completed"], "active", false),
            (REVERSE_SYSTEM, vec!["active", "completed"], "active", true),
            (EXPIRE, vec!["active"], "expired", true),
            (EXTEND, vec!["active", "expired"], "active", false),
            (EXTEND_SYSTEM, vec!["active", "expired"], "active", true),
            (CANCEL, vec!["pending_payment"], "cancelled", false),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to, system) in table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.is_system(), system, "{name}");
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["pending_payment"]);
        assert_eq!(machine.states().len(), PackageStatus::ALL.len());
    }

    #[test]
    fn refunded_and_cancelled_are_terminal_and_completed_is_not() {
        let machine = Package::machine();
        let terminal: Vec<_> = machine
            .states()
            .iter()
            .filter(|s| s.terminal)
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(terminal, ["refunded", "cancelled"]);
    }
}
