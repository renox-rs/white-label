//! The Onboarding and OnboardingStep state machines (wl-hr §4, issue #216).
//!
//! ```text
//! onboarding: [*] in_progress --complete--> completed
//!                  in_progress --cancel--> cancelled
//! step:       [*] open --mark_done--> done --reopen--> open
//!                  open --waive--> waived --reopen--> open
//!                  open --cancel--> cancelled
//! ```
//!
//! Every transition is `.system()`: the services in [`crate::onboarding`]
//! and [`crate::steps`] do what a user or an event may do (who is an
//! assignee, `HRM.ACL.10`, the evidence a step needs) and run exactly the
//! transition that fits, giving the machine what its guards check as the
//! run's input, so the guards agree with the service. The engine keeps every
//! run in the history, with the user in the run's metadata (`ran_by`), so a
//! done or waived record survives a `reopen`.
//!
//! - `complete` needs every required step `done` or `waived`
//!   ([`CompleteInput::open_required`] is 0).
//! - `cancel` (onboarding) needs a reason; `cancel` of a step takes an open
//!   step of the cancelled onboarding.
//! - `mark_done` records who and when; `waive` needs a reason; `reopen`
//!   needs a reason, and clears the record of the step.

use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};

use crate::onboarding::{Onboarding, OnboardingStatus, OnboardingStep, StepStatus};
use crate::texts::english;

/// The onboarding completes.
pub const COMPLETE: &str = "complete";
/// The onboarding is cancelled.
pub const CANCEL: &str = "cancel";
/// A step is done.
pub const MARK_DONE: &str = "mark_done";
/// A step is waived.
pub const WAIVE: &str = "waive";
/// A done or waived step is opened again.
pub const REOPEN: &str = "reopen";
/// An open step is cancelled with its onboarding.
pub const CANCEL_STEP: &str = "cancel_step";

/// What `complete` is given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteInput {
    /// How many required steps are neither `done` nor `waived`.
    pub open_required: usize,
}

/// What a step run is given: the columns its effect sets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRun {
    /// The user who acts; `None` for the system.
    pub by: Option<i64>,
    /// An evidence note.
    pub note: Option<String>,
    /// An evidence document.
    pub document_id: Option<i64>,
    /// The checklist run that passed.
    pub checklist_run_id: Option<i64>,
    /// The staff user who recorded an employee step in person.
    pub witness_id: Option<i64>,
    /// The drawn signature of an in-person record.
    pub signature: Option<String>,
}

/// The reason of the run, trimmed; `None` when blank.
fn reason_of(ctx: &TransitionCtx) -> Option<String> {
    ctx.reason()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

fn run_of(ctx: &TransitionCtx) -> StepRun {
    ctx.input_as::<StepRun>().unwrap_or_default()
}

impl StateMachine for Onboarding {
    type State = OnboardingStatus;
    const COLUMN: renox::db::Col<Onboarding, OnboardingStatus> = Onboarding::STATUS;

    fn machine() -> Machine<Self> {
        use OnboardingStatus::{Cancelled, Completed, InProgress};
        Machine::new()
            .initial([InProgress])
            .state(InProgress)
            .tone("info")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(COMPLETE)
                    .label("Complete")
                    .tone("success")
                    .system()
                    .from([InProgress])
                    .to(Completed)
                    .guard("required_steps_settled", |_o: &Onboarding, ctx| {
                        let open = ctx
                            .input_as::<CompleteInput>()
                            .map_or(usize::MAX, |i| i.open_required);
                        Box::pin(async move {
                            if open == 0 {
                                Ok(())
                            } else {
                                Err(english("wl_hr.onboarding.problem.open_steps", &[]))
                            }
                        })
                    }),
            )
            .transition(
                Transition::new(CANCEL)
                    .label("Cancel")
                    .tone("neutral")
                    .system()
                    .from([InProgress])
                    .to(Cancelled)
                    .guard("reason_given", |_o: &Onboarding, ctx| {
                        let given = reason_of(ctx).is_some();
                        Box::pin(async move {
                            if given {
                                Ok(())
                            } else {
                                Err(english("wl_hr.onboarding.problem.reason", &[]))
                            }
                        })
                    }),
            )
    }
}

/// Sets the columns of step `id` outside the machine's guard and stamps
/// `updated_at`.
async fn set(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let stamp = now();
    all.push(("updated_at", &stamp));
    OnboardingStep::query()
        .where_(OnboardingStep::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

impl StateMachine for OnboardingStep {
    type State = StepStatus;
    const COLUMN: renox::db::Col<OnboardingStep, StepStatus> = OnboardingStep::STATUS;

    fn machine() -> Machine<Self> {
        use StepStatus::{Cancelled, Done, Open, Waived};
        Machine::new()
            .initial([Open])
            .state(Open)
            .tone("neutral")
            .state(Done)
            .tone("success")
            .state(Waived)
            .tone("neutral")
            .state(Cancelled)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(MARK_DONE)
                    .label("Mark done")
                    .tone("success")
                    .system()
                    .from([Open])
                    .to(Done)
                    .effect(|step: &OnboardingStep, ctx, tx| {
                        let id = step.id;
                        let run = run_of(ctx);
                        Box::pin(async move {
                            let at = Some(now());
                            set(
                                tx,
                                id,
                                &[
                                    ("done_by", &run.by),
                                    ("done_at", &at),
                                    ("evidence_note", &run.note),
                                    ("evidence_document_id", &run.document_id),
                                    ("checklist_run_id", &run.checklist_run_id),
                                    ("witness_id", &run.witness_id),
                                    ("signature", &run.signature),
                                    ("failed_items", &None::<String>),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(WAIVE)
                    .label("Waive")
                    .tone("neutral")
                    .system()
                    .from([Open])
                    .to(Waived)
                    .guard("reason_given", reason_guard)
                    .effect(|step: &OnboardingStep, ctx, tx| {
                        let (id, by, reason) = (step.id, run_of(ctx).by, reason_of(ctx));
                        Box::pin(async move {
                            set(tx, id, &[("waived_by", &by), ("waive_reason", &reason)]).await
                        })
                    }),
            )
            .transition(
                Transition::new(REOPEN)
                    .label("Reopen")
                    .tone("warning")
                    .system()
                    .from([Done, Waived])
                    .to(Open)
                    .guard("reason_given", reason_guard)
                    .effect(|step: &OnboardingStep, _ctx, tx| {
                        let id = step.id;
                        Box::pin(async move {
                            set(
                                tx,
                                id,
                                &[
                                    ("done_by", &None::<i64>),
                                    ("done_at", &None::<renox::db::DateTime>),
                                    ("evidence_note", &None::<String>),
                                    ("evidence_document_id", &None::<i64>),
                                    ("witness_id", &None::<i64>),
                                    ("signature", &None::<String>),
                                    ("waived_by", &None::<i64>),
                                    ("waive_reason", &None::<String>),
                                    ("note", &None::<String>),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(CANCEL_STEP)
                    .label("Cancel")
                    .tone("neutral")
                    .system()
                    .from([Open])
                    .to(Cancelled),
            )
    }
}

fn reason_guard<'a>(
    _step: &'a OnboardingStep,
    ctx: &'a TransitionCtx,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = std::result::Result<(), String>> + Send + 'a>>
{
    let given = reason_of(ctx).is_some();
    Box::pin(async move {
        if given {
            Ok(())
        } else {
            Err(english("wl_hr.steps.problem.reason", &[]))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_onboarding_machine_has_complete_and_cancel() {
        let machine = Onboarding::machine();
        assert_eq!(machine.transitions().len(), 2);
        for (name, to) in [(COMPLETE, "completed"), (CANCEL, "cancelled")] {
            let t = machine.find(name).expect(name);
            assert!(t.is_system(), "{name}");
            assert_eq!(t.from_states(), ["in_progress"], "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["in_progress"]);
    }

    #[test]
    fn the_step_machine_has_the_four_transitions_of_the_spec() {
        let machine = OnboardingStep::machine();
        let table = [
            (MARK_DONE, vec!["open"], "done"),
            (WAIVE, vec!["open"], "waived"),
            (REOPEN, vec!["done", "waived"], "open"),
            (CANCEL_STEP, vec!["open"], "cancelled"),
        ];
        assert_eq!(machine.transitions().len(), table.len());
        for (name, from, to) in table {
            let t = machine.find(name).expect(name);
            assert_eq!(t.from_states(), from, "{name}");
            assert_eq!(t.to_state(), Some(to), "{name}");
        }
        assert_eq!(machine.initial_states(), ["open"]);
    }

    #[renox::test]
    async fn complete_needs_every_required_step_settled_and_cancel_a_reason() {
        let machine = Onboarding::machine();
        let onboarding = Onboarding::default();
        let complete = machine.find(COMPLETE).unwrap();
        let ctx = |open: usize| {
            TransitionCtx::new(None).with_input(
                renox::serde_json::to_value(CompleteInput {
                    open_required: open,
                })
                .unwrap(),
            )
        };
        assert!(complete.check_guards(&onboarding, &ctx(1)).await.is_err());
        assert!(complete.check_guards(&onboarding, &ctx(0)).await.is_ok());
        assert!(
            complete
                .check_guards(&onboarding, &TransitionCtx::new(None))
                .await
                .is_err()
        );
        let cancel = machine.find(CANCEL).unwrap();
        assert!(
            cancel
                .check_guards(&onboarding, &TransitionCtx::new(None))
                .await
                .is_err()
        );
        let with_reason = TransitionCtx::new(None).with_reason("exited");
        assert!(cancel.check_guards(&onboarding, &with_reason).await.is_ok());
    }

    #[renox::test]
    async fn waive_and_reopen_need_a_reason() {
        let machine = OnboardingStep::machine();
        let step = OnboardingStep::default();
        for name in [WAIVE, REOPEN] {
            let t = machine.find(name).unwrap();
            assert!(
                t.check_guards(&step, &TransitionCtx::new(None))
                    .await
                    .is_err(),
                "{name}"
            );
            assert!(
                t.check_guards(&step, &TransitionCtx::new(None).with_reason("not needed"))
                    .await
                    .is_ok(),
                "{name}"
            );
        }
    }
}
