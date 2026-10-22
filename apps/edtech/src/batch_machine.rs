//! The batch machine (10 §7.1 *Batch*; `EDU.BAT.01`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`OPEN`] | planned | open | `batches.manage` | `plan_complete`, `sessions_planned` (effect: sessions generated) |
//! | [`CHANGE_TUTOR`] | open | open | `batches.manage` | `new_tutor_given` |
//! | [`CHANGE_TUTOR_RUNNING`] | running | running | `batches.manage` | `new_tutor_given` |
//! | [`START`] | open | running | system (first session date) | — |
//! | [`COMPLETE`] | running | completed | system (last session held; `batches.manage` through the service) | `attendance_submitted` |
//! | [`CANCEL`] | planned, open | cancelled | `batches.manage` | `enrolment_choices` (slot, BL-EDU-13) |
//! | [`TERMINATE`] | running | terminated | `batches.manage` | `enrolment_choices`, `approved` (slots, BL-EDU-12) |
//!
//! Decisions:
//!
//! - Renox guards see only the record and the run's input, no database. The
//!   facts the spec's guards need (the tutor and the room free on every session
//!   date, the documents valid, the tutor eligible, the attendance of every
//!   session submitted) are gathered by [`crate::batches`] before the run and
//!   carried in [`BatchInput`]; the guards of `open` and `complete` refuse a
//!   run whose input does not carry them, so nothing but the service opens or
//!   completes a batch. The checks that need the document and qualification
//!   guards of the kernel (BL-EDU-07, BL-EDU-08) are made by the service in
//!   [`crate::batches::open`] before the run, as the other verticals do.
//! - `complete` is "system or `batches.manage`" in the spec and a transition
//!   has one kind of access: it is a system transition, and the service checks
//!   the permission of a user before it runs it (the same choice as
//!   wl-scheduling's booking machine). The user is kept in the history row.
//! - The spec's `change_tutor` keeps the state of `open` and of `running`; a
//!   transition has one end state, so there is one per state
//!   ([`CHANGE_TUTOR`], [`CHANGE_TUTOR_RUNNING`]) and the service picks the one
//!   of the batch's state (as wl-scheduling does for `reassign`).
//! - `cancel` and `terminate` are declared with their reason and the guard
//!   slots of BL-EDU-13 and BL-EDU-12 (the choice *transfer to batch X* or
//!   *withdraw* per enrolment; the centre manager's approval of a
//!   termination, `EDU.ACL.15`). The slots pass today; the issues put the
//!   check in their bodies. Their effects store the reason and nothing else:
//!   the enrolments, the refunds and the sessions are those issues'.
//! - `open` generates the sessions of the plan in its effect
//!   ([`crate::sessions::insert_planned`]), after checking once more, inside
//!   the transaction, that the tutor and the room are free.
//! - `change_tutor` sets the batch's tutor from the run's input and, in the
//!   same transaction, moves the `scheduled` sessions of the batch's old tutor
//!   from the given date to the new one; a session already moved to another
//!   tutor keeps its own. The service that checks that the new tutor is free
//!   and eligible on every remaining date, all or nothing, is BL-EDU-09's.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::batch::{Batch, BatchStatus};
use crate::db::set_batch;
use crate::permissions;
use crate::plan::PlannedSession;
use crate::sessions;
use crate::texts::english;

/// `planned → open`: open for enrolment; the sessions are generated.
pub const OPEN: &str = "open";
/// `open → open`: another tutor from a date.
pub const CHANGE_TUTOR: &str = "change_tutor";
/// `running → running`: another tutor from a date.
pub const CHANGE_TUTOR_RUNNING: &str = "change_tutor_running";
/// `open → running`: the first session date came.
pub const START: &str = "start";
/// `running → completed`: the last session was held.
pub const COMPLETE: &str = "complete";
/// `planned | open → cancelled`: it did not run.
pub const CANCEL: &str = "cancel";
/// `running → terminated`: stopped after it started.
pub const TERMINATE: &str = "terminate";

/// What a run carries to the guards and effects.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BatchInput {
    /// The sessions of the plan ([`OPEN`]).
    #[serde(default)]
    pub sessions: Vec<PlannedSession>,
    /// The sessions still `scheduled` ([`COMPLETE`]).
    #[serde(default)]
    pub unsubmitted: Option<i64>,
    /// The first day the new tutor teaches ([`CHANGE_TUTOR`]).
    #[serde(default)]
    pub from: Option<NaiveDate>,
    /// The new tutor, a wl-scheduling resource ([`CHANGE_TUTOR`]).
    #[serde(default)]
    pub tutor_resource_id: Option<i64>,
    /// The employee behind the new tutor ([`CHANGE_TUTOR`]).
    #[serde(default)]
    pub tutor_employee_id: Option<i64>,
    /// The user who acted, when the run is the system's on a user's behalf.
    #[serde(default)]
    pub by: Option<i64>,
}

fn input_of(ctx: &TransitionCtx) -> BatchInput {
    ctx.input_as::<BatchInput>().unwrap_or_default()
}

/// `Ok` when `ok`, else the English text `key`.
fn check(ok: bool, key: &str) -> std::result::Result<(), String> {
    if ok { Ok(()) } else { Err(english(key, &[])) }
}

impl StateMachine for Batch {
    type State = BatchStatus;
    const COLUMN: renox::db::Col<Batch, BatchStatus> = Batch::STATUS;

    fn machine() -> Machine<Self> {
        use BatchStatus::{Cancelled, Completed, Open, Planned, Running, Terminated};
        Machine::new()
            .initial([Planned])
            .state(Planned)
            .tone("neutral")
            .state(Open)
            .tone("info")
            .state(Running)
            .tone("success")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .state(Terminated)
            .tone("error")
            .terminal()
            .transition(open())
            .transition(change_tutor(CHANGE_TUTOR, BatchStatus::Open))
            .transition(change_tutor(CHANGE_TUTOR_RUNNING, BatchStatus::Running))
            .transition(start())
            .transition(complete())
            .transition(cancel())
            .transition(terminate())
    }
}

/// `open`: the plan is complete and gives sessions.
fn open() -> Transition<Batch> {
    Transition::new(OPEN)
        .label("Open for enrolment")
        .tone("primary")
        .permission(permissions::BATCHES_MANAGE)
        .from([BatchStatus::Planned])
        .to(BatchStatus::Open)
        .guard("plan_complete", |b: &Batch, _ctx| {
            let ok = b.capacity >= 1
                && b.tutor_resource_id.is_some()
                && !b.weekday_list().is_empty()
                && b.start_date <= b.end_date
                && b.duration_minutes > 0;
            Box::pin(async move { check(ok, "edtech.batches.problem.plan_incomplete") })
        })
        .guard("sessions_planned", |_b: &Batch, ctx| {
            let ok = !input_of(ctx).sessions.is_empty();
            Box::pin(async move { check(ok, "edtech.batches.problem.no_sessions") })
        })
        .effect(|b: &Batch, ctx, tx| {
            let batch = b.clone();
            let input = input_of(ctx);
            Box::pin(async move { sessions::insert_planned(tx, &batch, &input.sessions).await })
        })
}

/// `change_tutor`: the batch's tutor is the one in the input, in the state
/// `state` (the transition keeps it).
fn change_tutor(name: &'static str, state: BatchStatus) -> Transition<Batch> {
    Transition::new(name)
        .label("Change tutor")
        .permission(permissions::BATCHES_MANAGE)
        .from([state])
        .to(state)
        .guard("new_tutor_given", |_b: &Batch, ctx| {
            let input = input_of(ctx);
            let ok = input.tutor_resource_id.is_some() && input.from.is_some();
            Box::pin(async move { check(ok, "edtech.batches.problem.new_tutor") })
        })
        .effect(|b: &Batch, ctx, tx| {
            let id = b.id;
            let old = b.tutor_resource_id;
            let input = input_of(ctx);
            Box::pin(async move {
                if let Some(from) = input.from {
                    sessions::reassign_remaining(
                        tx,
                        id,
                        from,
                        old,
                        input.tutor_resource_id,
                        input.tutor_employee_id,
                    )
                    .await?;
                }
                set_batch(
                    tx,
                    id,
                    &[
                        ("tutor_resource_id", &input.tutor_resource_id),
                        ("tutor_employee_id", &input.tutor_employee_id),
                    ],
                )
                .await
            })
        })
}

/// `start`: the first session date came.
fn start() -> Transition<Batch> {
    Transition::new(START)
        .label("Start")
        .system()
        .from([BatchStatus::Open])
        .to(BatchStatus::Running)
}

/// `complete`: every session is held or cancelled.
fn complete() -> Transition<Batch> {
    Transition::new(COMPLETE)
        .label("Complete")
        .system()
        .from([BatchStatus::Running])
        .to(BatchStatus::Completed)
        .guard("attendance_submitted", |_b: &Batch, ctx| {
            let ok = input_of(ctx).unsubmitted == Some(0);
            Box::pin(async move { check(ok, "edtech.batches.problem.attendance_open") })
        })
}

/// `cancel`: the batch did not run. The slot `enrolment_choices` is where
/// BL-EDU-13 checks the choice per enrolment.
fn cancel() -> Transition<Batch> {
    Transition::new(CANCEL)
        .label("Cancel")
        .permission(permissions::BATCHES_MANAGE)
        .from([BatchStatus::Planned, BatchStatus::Open])
        .to(BatchStatus::Cancelled)
        .reason_required()
        .guard("enrolment_choices", |_b: &Batch, _ctx| {
            Box::pin(async { Ok::<(), String>(()) })
        })
        .effect(|b: &Batch, ctx, tx| {
            let id = b.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move { set_batch(tx, id, &[("cancel_reason", &reason)]).await })
        })
}

/// `terminate`: the batch stopped after it started. The slots
/// `enrolment_choices` and `approved` are where BL-EDU-12 checks the choice per
/// enrolment and the centre manager's approval (`EDU.ACL.15`).
fn terminate() -> Transition<Batch> {
    Transition::new(TERMINATE)
        .label("Terminate")
        .permission(permissions::BATCHES_MANAGE)
        .from([BatchStatus::Running])
        .to(BatchStatus::Terminated)
        .reason_required()
        .guard("enrolment_choices", |_b: &Batch, _ctx| {
            Box::pin(async { Ok::<(), String>(()) })
        })
        .guard("approved", |_b: &Batch, _ctx| {
            Box::pin(async { Ok::<(), String>(()) })
        })
        .effect(|b: &Batch, ctx, tx| {
            let id = b.id;
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move { set_batch(tx, id, &[("termination_reason", &reason)]).await })
        })
}

/// Registers the machine for `rnx state:diagram`.
pub(crate) fn register() {
    renox::state_machine::register::<Batch>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_state_and_transition_of_the_spec() {
        let machine = Batch::machine();
        let states: Vec<(&str, Option<&str>, bool)> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("planned", Some("neutral"), false),
                ("open", Some("info"), false),
                ("running", Some("success"), false),
                ("completed", Some("success"), true),
                ("cancelled", Some("neutral"), true),
                ("terminated", Some("error"), true),
            ]
        );
        let expected = [
            (OPEN, vec!["planned"], "open"),
            (CHANGE_TUTOR, vec!["open"], "open"),
            (CHANGE_TUTOR_RUNNING, vec!["running"], "running"),
            (START, vec!["open"], "running"),
            (COMPLETE, vec!["running"], "completed"),
            (CANCEL, vec!["planned", "open"], "cancelled"),
            (TERMINATE, vec!["running"], "terminated"),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to) in expected {
            let transition = machine.find(name).unwrap();
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
        }
    }

    #[test]
    fn the_guards_and_the_guard_slots_are_declared() {
        let machine = Batch::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .unwrap()
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards(OPEN), ["plan_complete", "sessions_planned"]);
        assert_eq!(guards(CHANGE_TUTOR), ["new_tutor_given"]);
        assert_eq!(guards(CHANGE_TUTOR_RUNNING), ["new_tutor_given"]);
        assert_eq!(guards(COMPLETE), ["attendance_submitted"]);
        assert_eq!(guards(CANCEL), ["enrolment_choices"]);
        assert_eq!(guards(TERMINATE), ["enrolment_choices", "approved"]);
        assert!(guards(START).is_empty());
        for name in [CANCEL, TERMINATE] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
        for name in [START, COMPLETE] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
    }
}
