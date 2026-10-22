//! The batch machine (10 §7.1 *Batch*; `EDU.BAT.01`).
//!
//! | Transition | From | To | By | Guards |
//! |---|---|---|---|---|
//! | [`OPEN`] | planned | open | `batches.manage` | `plan_complete`, `sessions_planned` (effect: sessions generated) |
//! | [`CHANGE_TUTOR`] | open | open | `batches.manage` | `new_tutor_given` |
//! | [`CHANGE_TUTOR_RUNNING`] | running | running | `batches.manage` | `new_tutor_given` |
//! | [`START`] | open | running | system (first session date) | — |
//! | [`COMPLETE`] | running | completed | system (last session held; `batches.manage` through the service) | `attendance_submitted` |
//! | [`CANCEL`] | planned, open | cancelled | `batches.manage` | a reason, `enrolment_choices` |
//! | [`TERMINATE`] / [`TERMINATE_APPROVED`] | running | terminated | `batches.manage`, approved by the centre manager (`EDU.ACL.15`) / the kernel after the approval | a reason, `enrolment_choices`, `approved` |
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
//! - `cancel` (before the batch runs, BL-EDU-13) and `terminate` (while it runs,
//!   BL-EDU-12) end the batch early. The coordinator names, per enrolment,
//!   *transfer to batch X* or *withdraw* (default withdraw); the service checks
//!   the choices and carries them in [`BatchInput`], and the guard
//!   `enrolment_choices` refuses a run that does not carry them. The effects
//!   ([`crate::exit`]) cancel the sessions still `scheduled`, move or withdraw
//!   each enrolment and log what they did; the people are told and the money
//!   documents made after the commit. `terminate` is the action
//!   `edtech.terminate` of the kernel's approvals (`EDU.ACL.15`,
//!   [`terminate_policy`]): the coordinator's run is deferred as a request that
//!   a holder of `batches.approve` other than the requester decides, and the
//!   kernel runs [`TERMINATE_APPROVED`] (the same guards and effects) after the
//!   approval, with the input the request kept. The guard `approved` of the
//!   user's run asks that the service found the approval policy in force, so a
//!   policy taken away cannot let a termination through unapproved; the twin's
//!   asks that the request names who asked.
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
use wl_kernel::SubjectMachine;
use wl_kernel::approvals::{self, PolicyDefault};

use crate::batch::{BATCH_SUBJECT_TYPE, Batch, BatchStatus};
use crate::db::set_batch;
use crate::exit::{self, ExitChoice, ExitKind, ExitPaid};
use crate::permissions;
use crate::plan::PlannedSession;
use crate::sessions;
use crate::texts::english;
use crate::tutors::{self, Tutor};

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
/// `running → terminated`: the kernel runs it after the centre manager approved.
pub const TERMINATE_APPROVED: &str = "terminate_approved";
/// The approval action of `terminate` (`EDU.ACL.15`).
pub const TERMINATE_ACTION: &str = "edtech.terminate";

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
    /// Why the batch ends early ([`CANCEL`], [`TERMINATE`]); the kernel hands it
    /// to the approved run.
    #[serde(default)]
    pub reason: Option<String>,
    /// What to do with each enrolment ([`CANCEL`], [`TERMINATE`]).
    #[serde(default)]
    pub choices: Vec<ExitChoice>,
    /// What each had paid when the exit was asked.
    #[serde(default)]
    pub paid: Vec<ExitPaid>,
    /// The service has checked the choices.
    #[serde(default)]
    pub choices_checked: bool,
    /// The reason is regulatory (approval expired or revoked): the centre
    /// manager and the owner are told.
    #[serde(default)]
    pub regulatory: bool,
    /// The service found the approval policy of `terminate` in force.
    #[serde(default)]
    pub approval_ready: bool,
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
            .transition(terminate(TERMINATE, false))
            .transition(terminate(TERMINATE_APPROVED, true))
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
                .await?;
                if let (Some(resource), Some(from)) = (input.tutor_resource_id, input.from) {
                    let tutor = Tutor {
                        resource_id: resource,
                        employee_id: input.tutor_employee_id,
                    };
                    tutors::record_batch(tx, id, &tutor, from, input.by).await?;
                }
                Ok(())
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

/// `cancel`: the batch did not run. The choices per enrolment are checked
/// (`enrolment_choices`); the effect cancels the sessions, moves or withdraws
/// each enrolment with a full refund and logs it ([`crate::exit`]).
fn cancel() -> Transition<Batch> {
    Transition::new(CANCEL)
        .label("Cancel")
        .permission(permissions::BATCHES_MANAGE)
        .from([BatchStatus::Planned, BatchStatus::Open])
        .to(BatchStatus::Cancelled)
        .reason_required()
        .guard("enrolment_choices", |_b: &Batch, ctx| {
            let ok = input_of(ctx).choices_checked;
            Box::pin(async move { check(ok, "edtech.exit.problem.choices") })
        })
        .effect(|b: &Batch, ctx, tx| {
            let batch = b.clone();
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                exit::run_in(tx, &batch, &input, reason, ExitKind::Cancel).await
            })
        })
}

/// `terminate`: the batch stopped after it started. A coordinator's run is
/// deferred for the centre manager's approval (`EDU.ACL.15`); the kernel runs
/// the system's twin after it.
fn terminate(name: &'static str, system: bool) -> Transition<Batch> {
    let transition = Transition::new(name)
        .label(if system { "Terminate (approved)" } else { "Terminate" })
        .from([BatchStatus::Running])
        .to(BatchStatus::Terminated)
        .reason_required();
    let transition = if system {
        transition.system()
    } else {
        transition.permission(permissions::BATCHES_MANAGE)
    };
    transition
        .guard("enrolment_choices", |_b: &Batch, ctx| {
            let ok = input_of(ctx).choices_checked;
            Box::pin(async move { check(ok, "edtech.exit.problem.choices") })
        })
        .guard("approved", move |_b: &Batch, ctx| {
            let input = input_of(ctx);
            let ok = if system {
                input.by.is_some()
            } else {
                input.approval_ready
            };
            Box::pin(async move { check(ok, "edtech.exit.problem.approval") })
        })
        .effect(|b: &Batch, ctx, tx| {
            let batch = b.clone();
            let input = input_of(ctx);
            let reason = ctx.reason().map(str::to_owned);
            Box::pin(async move {
                exit::run_in(tx, &batch, &input, reason, ExitKind::Terminate).await
            })
        })
}

/// The default approval policy of `terminate` (`EDU.ACL.15`): every request
/// needs one step by a holder of `batches.approve` (the centre manager), never
/// the user who asked.
pub fn terminate_policy() -> PolicyDefault {
    PolicyDefault::new(TERMINATE_ACTION).step_permission(permissions::BATCHES_APPROVE)
}

/// The batch in the kernel's registry, so the approvals can run
/// [`TERMINATE_APPROVED`] after the approval.
pub fn subject_machine() -> SubjectMachine {
    SubjectMachine::of::<Batch>(BATCH_SUBJECT_TYPE).build()
}

/// Registers the machine for `rnx state:diagram` and with the kernel, the
/// approval policy of `terminate` and the page the approvals link to.
pub(crate) fn register() {
    subject_machine().register();
    renox::state_machine::register::<Batch>();
    terminate_policy().register();
    approvals::register_subject_link(BATCH_SUBJECT_TYPE, |id| format!("/edtech/batches/{id}"));
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
            (TERMINATE_APPROVED, vec!["running"], "terminated"),
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
        for name in [TERMINATE, TERMINATE_APPROVED] {
            assert_eq!(guards(name), ["enrolment_choices", "approved"]);
        }
        assert!(guards(START).is_empty());
        for name in [CANCEL, TERMINATE, TERMINATE_APPROVED] {
            assert!(machine.find(name).unwrap().needs_reason(), "{name}");
        }
        for name in [START, COMPLETE, TERMINATE_APPROVED] {
            assert!(machine.find(name).unwrap().is_system(), "{name}");
        }
    }
}
