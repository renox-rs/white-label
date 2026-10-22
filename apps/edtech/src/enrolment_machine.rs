//! The enrolment machine (10 §7.1 *Enrolment*; `EDU.ENR.01`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`ACTIVATE`] / [`ACTIVATE_SYSTEM`] | pending_payment | active | `enrolments.manage` / system (first payment) | `terms_accepted`, `discount_approved` |
//! | [`LAPSE`] | pending_payment | lapsed | system (`EDU.RUL.11`) | `no_payment` (slot, BL-EDU-01) |
//! | [`FREEZE`] / [`FREEZE_SYSTEM`] | active, frozen | frozen | `enrolments.manage` / system | `cause_given`, `within_allowance` |
//! | [`UNFREEZE`] / [`UNFREEZE_SYSTEM`] | frozen | active | `enrolments.manage` / system | `cause_given`, `no_cause_left` |
//! | [`COMPLETE`] | active | completed | system | — |
//! | [`WITHDRAW`] / [`WITHDRAW_SYSTEM`] | pending_payment, active, frozen | withdrawn | `enrolments.manage` / system | a reason, `refund_calculated` |
//! | [`TRANSFER`] / [`TRANSFER_SYSTEM`] | active, frozen | transferred | `enrolments.manage` / system | `target_batch_given`, `target_has_seat` (slot, BL-EDU-02) |
//!
//! Decisions:
//!
//! - A Renox transition has one kind of access, and the spec's "by
//!   `enrolments.manage`, or system" is two: every such transition is declared
//!   twice, the user's and its `_system` twin, with the same guards and
//!   effects (as wl-scheduling does for `reverse`). The service picks the
//!   twin by who acts ([`crate::enrolments`]).
//! - Guards see only the record and the run's input, not the database. The
//!   service gathers what they need (the freeze days used this year, the
//!   causes left, the refund the policy gives) and passes it in
//!   [`EnrolmentInput`]; a run without it is refused, so the transitions
//!   cannot be run around the service.
//! - `freeze` leaves `active` and also `frozen`: "a freeze on a `frozen`
//!   enrolment only adds its cause". Each cause is a row
//!   ([`EnrolmentFreeze`]); `unfreeze` runs only when the last cause is gone
//!   (the service ends the other causes without a transition).
//! - `lapse` and `transfer` are declared with the guard slots BL-EDU-01 and
//!   BL-EDU-02 fill; they release the seat. The package `extend` by the frozen
//!   days and the membership `pause`/`resume` (`EDU.RUL.14`) are BL-EDU-05's;
//!   the money documents of `withdraw` (credit note, cancelled instalment
//!   invoices, refund payment, approval) are BL-EDU-03's. Here `withdraw`
//!   stores the refund the policy gives, cancels the instalments not yet
//!   invoiced and releases the seat.

use renox::chrono::NaiveDate;
use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::db::set_enrolment;
use crate::enrolment::{
    Enrolment, EnrolmentFreeze, EnrolmentStatus, FreezeCause, Instalment, InstalmentStatus,
};
use crate::permissions;
use crate::refund::Refund;
use crate::seats;
use crate::texts::english;

/// `pending_payment → active`: paid, or activated by staff.
pub const ACTIVATE: &str = "activate";
/// `pending_payment → active`: the first payment arrived.
pub const ACTIVATE_SYSTEM: &str = "activate_system";
/// `pending_payment → lapsed`: the seat hold expired.
pub const LAPSE: &str = "lapse";
/// `active | frozen → frozen`: paused by staff.
pub const FREEZE: &str = "freeze";
/// `active | frozen → frozen`: paused by the system (a block, a membership pause).
pub const FREEZE_SYSTEM: &str = "freeze_system";
/// `frozen → active`: resumed by staff.
pub const UNFREEZE: &str = "unfreeze";
/// `frozen → active`: resumed by the system.
pub const UNFREEZE_SYSTEM: &str = "unfreeze_system";
/// `active → completed`: the course ended or the credits are used up.
pub const COMPLETE: &str = "complete";
/// `pending_payment | active | frozen → withdrawn`: the student left.
pub const WITHDRAW: &str = "withdraw";
/// `pending_payment | active | frozen → withdrawn`: a batch `terminate` or `cancel`.
pub const WITHDRAW_SYSTEM: &str = "withdraw_system";
/// `active | frozen → transferred`: moved to another batch.
pub const TRANSFER: &str = "transfer";
/// `active | frozen → transferred`: moved by a batch `terminate`.
pub const TRANSFER_SYSTEM: &str = "transfer_system";

/// The days of freeze a year allows, shared with the membership pause.
pub const FREEZE_ALLOWANCE_DAYS: i64 = 30;

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolmentInput {
    /// The day of the run (Asia/Dubai).
    #[serde(default)]
    pub on: Option<NaiveDate>,
    /// The user who acted.
    #[serde(default)]
    pub by: Option<i64>,
    /// Why it is frozen, or which freeze ends.
    #[serde(default)]
    pub cause: Option<FreezeCause>,
    /// The last day of the freeze, when it is planned.
    #[serde(default)]
    pub until: Option<NaiveDate>,
    /// The days this freeze counts against the allowance.
    #[serde(default)]
    pub counted_days: i64,
    /// The days already counted against the allowance this year.
    #[serde(default)]
    pub used_days: i64,
    /// The causes that still hold the enrolment frozen once this one ends.
    #[serde(default)]
    pub causes_left: Option<i64>,
    /// The refund the policy gives ([`WITHDRAW`]).
    #[serde(default)]
    pub refund: Option<Refund>,
    /// The batch the student moves to ([`TRANSFER`]).
    #[serde(default)]
    pub target_batch_id: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> EnrolmentInput {
    ctx.input_as::<EnrolmentInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

/// Gives the seat back when the enrolment holds one.
async fn release_seat(tx: &mut renox::db::Transaction, e: &Enrolment) -> Result {
    if e.seat_held
        && let Some(batch) = e.batch_id
    {
        seats::release(tx, batch, 1).await?;
        set_enrolment(tx, e.id, &[("seat_held", &false)]).await?;
    }
    Ok(())
}

/// Ends the open freeze rows of `cause` on `on`.
pub(crate) async fn end_cause(
    tx: &mut renox::db::Transaction,
    enrolment_id: i64,
    cause: FreezeCause,
    on: NaiveDate,
) -> Result {
    EnrolmentFreeze::query()
        .where_(EnrolmentFreeze::ENROLMENT_ID.eq(enrolment_id))
        .where_(EnrolmentFreeze::CAUSE.eq(cause))
        .where_(EnrolmentFreeze::ENDED_ON.is_null())
        .update(&mut *tx, &[("ended_on", &Some(on)), ("updated_at", &now())])
        .await?;
    Ok(())
}

impl StateMachine for Enrolment {
    type State = EnrolmentStatus;
    const COLUMN: renox::db::Col<Enrolment, EnrolmentStatus> = Enrolment::STATUS;

    fn machine() -> Machine<Self> {
        use EnrolmentStatus::{
            Active, Completed, Frozen, Lapsed, PendingPayment, Transferred, Withdrawn,
        };
        Machine::new()
            .initial([PendingPayment])
            .state(PendingPayment)
            .tone("warning")
            .state(Active)
            .tone("success")
            .state(Frozen)
            .tone("neutral")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Withdrawn)
            .tone("neutral")
            .terminal()
            .state(Transferred)
            .tone("neutral")
            .terminal()
            .state(Lapsed)
            .tone("neutral")
            .terminal()
            .transition(activate(ACTIVATE, false))
            .transition(activate(ACTIVATE_SYSTEM, true))
            .transition(lapse())
            .transition(freeze(FREEZE, false))
            .transition(freeze(FREEZE_SYSTEM, true))
            .transition(unfreeze(UNFREEZE, false))
            .transition(unfreeze(UNFREEZE_SYSTEM, true))
            .transition(complete())
            .transition(withdraw(WITHDRAW, false))
            .transition(withdraw(WITHDRAW_SYSTEM, true))
            .transition(transfer(TRANSFER, false))
            .transition(transfer(TRANSFER_SYSTEM, true))
    }
}

/// A user's transition needs `enrolments.manage`; its twin is the system's.
fn access(transition: Transition<Enrolment>, system: bool) -> Transition<Enrolment> {
    if system {
        transition.system()
    } else {
        transition.permission(permissions::ENROLMENTS_MANAGE)
    }
}

/// `activate`: the first payment came, or staff confirm. The terms are
/// accepted (`DOC.SIG.01`) and a discount above the limit is approved. The
/// seat hold ends: the seat is confirmed.
fn activate(name: &'static str, system: bool) -> Transition<Enrolment> {
    access(
        Transition::new(name)
            .label("Activate")
            .tone("primary")
            .from([EnrolmentStatus::PendingPayment])
            .to(EnrolmentStatus::Active),
        system,
    )
    .guard("terms_accepted", |e: &Enrolment, _ctx| {
        let ok = e.terms_accepted_at.is_some();
        Box::pin(async move { check(ok, "edtech.enrolments.problem.terms") })
    })
    .guard("discount_approved", |e: &Enrolment, _ctx| {
        let ok = e.discount_approved;
        Box::pin(async move { check(ok, "edtech.enrolments.problem.discount") })
    })
    .effect(|e: &Enrolment, _ctx, tx| {
        let id = e.id;
        Box::pin(async move {
            set_enrolment(tx, id, &[("seat_hold_until", &Option::<DateTime>::None)]).await
        })
    })
}

/// `lapse`: the seat hold expired without payment. The guard `no_payment` is
/// the slot BL-EDU-01 fills (nothing paid, no payment pending at the gateway).
fn lapse() -> Transition<Enrolment> {
    Transition::new(LAPSE)
        .label("Lapse")
        .system()
        .from([EnrolmentStatus::PendingPayment])
        .to(EnrolmentStatus::Lapsed)
        .guard("no_payment", |_e: &Enrolment, _ctx| {
            Box::pin(async { Ok::<(), String>(()) })
        })
        .effect(|e: &Enrolment, _ctx, tx| {
            let e = e.clone();
            Box::pin(async move { release_seat(tx, &e).await })
        })
}

/// `freeze`: the enrolment is paused for a cause; the cause is stored.
fn freeze(name: &'static str, system: bool) -> Transition<Enrolment> {
    access(
        Transition::new(name)
            .label("Freeze")
            .from([EnrolmentStatus::Active, EnrolmentStatus::Frozen])
            .to(EnrolmentStatus::Frozen),
        system,
    )
    .guard("cause_given", |_e: &Enrolment, ctx| {
        let ok = input_of(ctx).cause.is_some();
        Box::pin(async move { check(ok, "edtech.enrolments.problem.freeze_cause") })
    })
    .guard("within_allowance", |_e: &Enrolment, ctx| {
        let input = input_of(ctx);
        let ok = match input.cause {
            Some(cause) if cause.counts_against_allowance() => {
                input.used_days + input.counted_days <= FREEZE_ALLOWANCE_DAYS
            }
            _ => true,
        };
        Box::pin(async move { check(ok, "edtech.enrolments.problem.allowance") })
    })
    .effect(|e: &Enrolment, ctx, tx| {
        let id = e.id;
        let input = input_of(ctx);
        Box::pin(async move {
            let Some(cause) = input.cause else {
                return Ok(());
            };
            let counted = if cause.counts_against_allowance() {
                input.counted_days
            } else {
                0
            };
            let mut row = EnrolmentFreeze {
                enrolment_id: id,
                cause,
                started_on: input.on.unwrap_or_else(wl_kernel::documents::today),
                until_on: input.until,
                counted_days: counted,
                created_by: input.by,
                ..EnrolmentFreeze::default()
            };
            row.insert(&mut *tx).await
        })
    })
}

/// `unfreeze`: the last cause is gone; the enrolment is active again.
fn unfreeze(name: &'static str, system: bool) -> Transition<Enrolment> {
    access(
        Transition::new(name)
            .label("Unfreeze")
            .from([EnrolmentStatus::Frozen])
            .to(EnrolmentStatus::Active),
        system,
    )
    .guard("cause_given", |_e: &Enrolment, ctx| {
        let ok = input_of(ctx).cause.is_some();
        Box::pin(async move { check(ok, "edtech.enrolments.problem.unfreeze_cause") })
    })
    .guard("no_cause_left", |_e: &Enrolment, ctx| {
        let ok = input_of(ctx).causes_left == Some(0);
        Box::pin(async move { check(ok, "edtech.enrolments.problem.cause_left") })
    })
    .effect(|e: &Enrolment, ctx, tx| {
        let id = e.id;
        let input = input_of(ctx);
        Box::pin(async move {
            let Some(cause) = input.cause else {
                return Ok(());
            };
            let on = input.on.unwrap_or_else(wl_kernel::documents::today);
            end_cause(tx, id, cause, on).await
        })
    })
}

/// `complete`: the course ended or the credits are used up. The seat is
/// given back; the certificate is evaluated after the commit
/// ([`crate::certificates`]).
fn complete() -> Transition<Enrolment> {
    Transition::new(COMPLETE)
        .label("Complete")
        .system()
        .from([EnrolmentStatus::Active])
        .to(EnrolmentStatus::Completed)
        .effect(|e: &Enrolment, ctx, tx| {
            let e = e.clone();
            let on = input_of(ctx)
                .on
                .unwrap_or_else(wl_kernel::documents::today);
            Box::pin(async move {
                set_enrolment(tx, e.id, &[("completed_on", &Some(on))]).await?;
                release_seat(tx, &e).await
            })
        })
}

/// `withdraw`: the student leaves. The refund the policy gives is stored; the
/// instalments not yet invoiced are cancelled; the seat is released.
fn withdraw(name: &'static str, system: bool) -> Transition<Enrolment> {
    access(
        Transition::new(name)
            .label("Withdraw")
            .from([
                EnrolmentStatus::PendingPayment,
                EnrolmentStatus::Active,
                EnrolmentStatus::Frozen,
            ])
            .to(EnrolmentStatus::Withdrawn)
            .reason_required(),
        system,
    )
    .guard("refund_calculated", |_e: &Enrolment, ctx| {
        let ok = input_of(ctx).refund.is_some();
        Box::pin(async move { check(ok, "edtech.enrolments.problem.refund") })
    })
    .effect(|e: &Enrolment, ctx, tx| {
        let e = e.clone();
        let input = input_of(ctx);
        let reason = ctx.reason().map(str::to_owned);
        Box::pin(async move {
            let (refund, owed) = input.refund.as_ref().map_or((0, 0), |r| (r.refund, r.owed));
            let on = input.on.unwrap_or_else(wl_kernel::documents::today);
            set_enrolment(
                tx,
                e.id,
                &[
                    ("refund_amount", &refund),
                    ("amount_owed", &owed),
                    ("withdrawn_on", &Some(on)),
                    ("withdrawal_reason", &reason),
                ],
            )
            .await?;
            Instalment::query()
                .where_(Instalment::ENROLMENT_ID.eq(e.id))
                .where_(Instalment::STATUS.eq(InstalmentStatus::Scheduled))
                .update(
                    &mut *tx,
                    &[
                        ("status", &InstalmentStatus::Cancelled),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            release_seat(tx, &e).await
        })
    })
}

/// `transfer`: the student moves to another batch. The guard slot
/// `target_has_seat` is where BL-EDU-02 checks the target's seats.
fn transfer(name: &'static str, system: bool) -> Transition<Enrolment> {
    access(
        Transition::new(name)
            .label("Transfer")
            .from([EnrolmentStatus::Active, EnrolmentStatus::Frozen])
            .to(EnrolmentStatus::Transferred),
        system,
    )
    .guard("target_batch_given", |_e: &Enrolment, ctx| {
        let ok = input_of(ctx).target_batch_id.is_some();
        Box::pin(async move { check(ok, "edtech.enrolments.problem.target") })
    })
    .guard("target_has_seat", |_e: &Enrolment, _ctx| {
        Box::pin(async { Ok::<(), String>(()) })
    })
    .effect(|e: &Enrolment, _ctx, tx| {
        let e = e.clone();
        Box::pin(async move { release_seat(tx, &e).await })
    })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<Enrolment>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = Enrolment::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("pending_payment", Some("warning"), false),
                ("active", Some("success"), false),
                ("frozen", Some("neutral"), false),
                ("completed", Some("success"), true),
                ("withdrawn", Some("neutral"), true),
                ("transferred", Some("neutral"), true),
                ("lapsed", Some("neutral"), true),
            ]
        );
        let expected = [
            (ACTIVATE, vec!["pending_payment"], "active", false),
            (ACTIVATE_SYSTEM, vec!["pending_payment"], "active", true),
            (LAPSE, vec!["pending_payment"], "lapsed", true),
            (FREEZE, vec!["active", "frozen"], "frozen", false),
            (FREEZE_SYSTEM, vec!["active", "frozen"], "frozen", true),
            (UNFREEZE, vec!["frozen"], "active", false),
            (UNFREEZE_SYSTEM, vec!["frozen"], "active", true),
            (COMPLETE, vec!["active"], "completed", true),
            (
                WITHDRAW,
                vec!["pending_payment", "active", "frozen"],
                "withdrawn",
                false,
            ),
            (
                WITHDRAW_SYSTEM,
                vec!["pending_payment", "active", "frozen"],
                "withdrawn",
                true,
            ),
            (TRANSFER, vec!["active", "frozen"], "transferred", false),
            (TRANSFER_SYSTEM, vec!["active", "frozen"], "transferred", true),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slots_are_declared() {
        let machine = Enrolment::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        for name in [ACTIVATE, ACTIVATE_SYSTEM] {
            assert_eq!(guards(name), ["terms_accepted", "discount_approved"]);
        }
        assert_eq!(guards(LAPSE), ["no_payment"]);
        for name in [FREEZE, FREEZE_SYSTEM] {
            assert_eq!(guards(name), ["cause_given", "within_allowance"]);
        }
        for name in [UNFREEZE, UNFREEZE_SYSTEM] {
            assert_eq!(guards(name), ["cause_given", "no_cause_left"]);
        }
        assert!(guards(COMPLETE).is_empty());
        for name in [WITHDRAW, WITHDRAW_SYSTEM] {
            assert_eq!(guards(name), ["refund_calculated"]);
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
        for name in [TRANSFER, TRANSFER_SYSTEM] {
            assert_eq!(guards(name), ["target_batch_given", "target_has_seat"]);
        }
    }
}
