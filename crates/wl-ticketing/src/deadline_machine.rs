//! The deadline Task state machine (wl-ticketing §4.4).
//!
//! ```text
//! [*] --> upcoming --start--> in_progress     overdue --start--> in_progress
//! in_progress --wait--> waiting --unblock | auto_unblock--> in_progress
//! in_progress --submit_review--> in_review --send_back--> in_progress
//! in_progress --complete--> completed     in_review --complete_reviewed--> completed
//! upcoming | in_progress | waiting | in_review --mark_overdue (system)--> overdue
//! open (any) --hand_over (system)--> completed   (the client files it itself)
//! overdue --close_missed--> missed
//! upcoming | in_progress | waiting --waive | auto_waive (system)--> not_required
//! ```
//!
//! Decision: the spec's one `complete` is two transitions, because a
//! transition has one permission: `complete` (from `in_progress`, by
//! `tasks.work`, only for a task without a reviewer role) and
//! `complete_reviewed` (from `in_review`, by `tasks.review`). The system
//! twins `auto_unblock` and `auto_waive` are the spec's "or system".
//!
//! Effects only write the task's own columns; the next occurrence, the
//! events and the notices come from the listeners in [`crate::deadlines`]
//! after the commit. `DeadlineTask` is `#[model(state_machine)]`, so `save`
//! refuses a changed status.

use std::future::Future;
use std::pin::Pin;

use renox::chrono::NaiveDate;
use renox::db::ToDbValue;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionCtx};
use serde::{Deserialize, Serialize};
use wl_kernel::deadlines::DeadlineTaskStatus;

use crate::deadline::DeadlineTask;
use crate::permissions;
use crate::texts::english;

/// `start`: work begins (also from `overdue`).
pub const START: &str = "start";
/// `wait`: blocked on someone else.
pub const WAIT: &str = "wait";
/// `unblock`: the wait is over.
pub const UNBLOCK: &str = "unblock";
/// `unblock`, run by the system (the linked items were answered).
pub const AUTO_UNBLOCK: &str = "auto_unblock";
/// `submit_review`: prepared, sent to the reviewer.
pub const SUBMIT_REVIEW: &str = "submit_review";
/// `send_back`: the reviewer returns it with a comment.
pub const SEND_BACK: &str = "send_back";
/// `complete`: done, by the worker of a task without a reviewer.
pub const COMPLETE: &str = "complete";
/// `complete`, by the reviewer of a task in review.
pub const COMPLETE_REVIEWED: &str = "complete_reviewed";
/// `mark_overdue`: the due date passed (system, `WRK.RUL.05`).
pub const MARK_OVERDUE: &str = "mark_overdue";
/// `close_missed`: an overdue task closed as not done.
pub const CLOSE_MISSED: &str = "close_missed";
/// `waive`: not applicable this period.
pub const WAIVE: &str = "waive";
/// `waive`, run by the system (the source agreement ended, a regeneration).
pub const AUTO_WAIVE: &str = "auto_waive";

/// `hand_over`: the firm no longer files it, the client does; completed by
/// the system with the evidence "handed over to client" (04 return `cancel`).
pub const HAND_OVER: &str = "hand_over";

/// What the services gather for the transitions that need it (effects and
/// guards get no `AppState`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskInput {
    /// The Asia/Dubai date of the run.
    pub today: NaiveDate,
    /// The client's *waive a started task* setting.
    pub waive_started: bool,
    /// The evidence given with `complete`.
    pub evidence: Option<String>,
}

/// The run's input, if it is a [`TaskInput`].
fn input(ctx: &TransitionCtx) -> Option<TaskInput> {
    ctx.input_as::<TaskInput>().ok()
}

/// Adds the guard that refuses a run without a [`TaskInput`].
fn with_input(transition: Transition<DeadlineTask>) -> Transition<DeadlineTask> {
    transition.guard("input", |_task: &DeadlineTask, ctx| {
        let present = input(ctx).is_some();
        Box::pin(async move {
            if present {
                Ok(())
            } else {
                Err(english("wl_ticketing.task.problem.input").to_owned())
            }
        })
    })
}

/// Writes `values` on task `id` inside the transition's transaction.
async fn write(
    tx: &mut renox::db::Transaction,
    id: i64,
    values: &[(&str, &(dyn ToDbValue + Sync))],
) -> Result {
    DeadlineTask::query()
        .where_(DeadlineTask::ID.eq(id))
        .update(&mut *tx, values)
        .await?;
    Ok(())
}

fn start_transition() -> Transition<DeadlineTask> {
    use DeadlineTaskStatus::{InProgress, Overdue, Upcoming};
    Transition::new(START)
        .label("Start")
        .tone("primary")
        .permission(permissions::TASKS_WORK)
        .from([Upcoming, Overdue])
        .to(InProgress)
        .effect(|task: &DeadlineTask, _ctx, tx| {
            let id = task.id;
            let first = task.started_at.is_none();
            Box::pin(async move {
                if first {
                    write(tx, id, &[("started_at", &Some(renox::db::now()))]).await?;
                }
                Ok(())
            })
        })
}

fn wait_transition() -> Transition<DeadlineTask> {
    Transition::new(WAIT)
        .label("Wait")
        .tone("warning")
        .permission(permissions::TASKS_WORK)
        .from([DeadlineTaskStatus::InProgress])
        .to(DeadlineTaskStatus::Waiting)
        .guard("reason_or_item", |task: &DeadlineTask, ctx| {
            let given = ctx.reason().is_some_and(|r| !r.trim().is_empty());
            let linked = task.needs_client_input;
            Box::pin(async move {
                if given || linked {
                    Ok(())
                } else {
                    Err(english("wl_ticketing.task.problem.wait_reason").to_owned())
                }
            })
        })
        .effect(|task: &DeadlineTask, ctx, tx| {
            let id = task.id;
            let reason = ctx.reason().map(|r| r.trim().to_owned());
            Box::pin(async move { write(tx, id, &[("waiting_reason", &reason)]).await })
        })
}

/// `unblock` and `auto_unblock`.
fn unblocking(transition: Transition<DeadlineTask>) -> Transition<DeadlineTask> {
    transition
        .from([DeadlineTaskStatus::Waiting])
        .to(DeadlineTaskStatus::InProgress)
        .effect(|task: &DeadlineTask, _ctx, tx| {
            let id = task.id;
            Box::pin(async move { write(tx, id, &[("waiting_reason", &None::<String>)]).await })
        })
}

fn submit_review_transition() -> Transition<DeadlineTask> {
    Transition::new(SUBMIT_REVIEW)
        .label("Submit for review")
        .tone("primary")
        .permission(permissions::TASKS_WORK)
        .from([DeadlineTaskStatus::InProgress])
        .to(DeadlineTaskStatus::InReview)
        .guard("reviewer", |task: &DeadlineTask, _ctx| {
            let has = task.reviewer_role.is_some();
            Box::pin(async move {
                if has {
                    Ok(())
                } else {
                    Err(english("wl_ticketing.task.problem.no_reviewer").to_owned())
                }
            })
        })
}

fn send_back_transition() -> Transition<DeadlineTask> {
    Transition::new(SEND_BACK)
        .label("Send back")
        .tone("warning")
        .permission(permissions::TASKS_REVIEW)
        .reason_required()
        .from([DeadlineTaskStatus::InReview])
        .to(DeadlineTaskStatus::InProgress)
}

/// The evidence guard and the effect `complete` and `complete_reviewed`
/// share.
fn completing(transition: Transition<DeadlineTask>) -> Transition<DeadlineTask> {
    with_input(transition.to(DeadlineTaskStatus::Completed))
        .guard("evidence", |task: &DeadlineTask, ctx| {
            let given = input(ctx)
                .and_then(|i| i.evidence)
                .is_some_and(|e| !e.trim().is_empty())
                || task.evidence.is_some();
            let needed = task.evidence_required;
            Box::pin(async move {
                if given || !needed {
                    Ok(())
                } else {
                    Err(english("wl_ticketing.task.problem.evidence").to_owned())
                }
            })
        })
        .effect(|task: &DeadlineTask, ctx, tx| {
            let id = task.id;
            let evidence = input(ctx)
                .and_then(|i| i.evidence)
                .map(|e| e.trim().to_owned())
                .filter(|e| !e.is_empty())
                .or_else(|| task.evidence.clone());
            Box::pin(async move {
                write(
                    tx,
                    id,
                    &[
                        ("completed_at", &Some(renox::db::now())),
                        ("evidence", &evidence),
                    ],
                )
                .await
            })
        })
}

fn complete_transition() -> Transition<DeadlineTask> {
    completing(
        Transition::new(COMPLETE)
            .label("Complete")
            .tone("success")
            .permission(permissions::TASKS_WORK)
            .from([DeadlineTaskStatus::InProgress]),
    )
    .guard("no_reviewer", |task: &DeadlineTask, _ctx| {
        let reviewed = task.reviewer_role.is_some();
        Box::pin(async move {
            if reviewed {
                Err(english("wl_ticketing.task.problem.needs_review").to_owned())
            } else {
                Ok(())
            }
        })
    })
}

fn complete_reviewed_transition() -> Transition<DeadlineTask> {
    completing(
        Transition::new(COMPLETE_REVIEWED)
            .label("Complete")
            .tone("success")
            .permission(permissions::TASKS_REVIEW)
            .from([DeadlineTaskStatus::InReview]),
    )
}

/// `hand_over`: any open task is completed by the system, with the
/// evidence and closed reason `wl_kernel::deadlines::HANDED_OVER_EVIDENCE`.
/// No reviewer and no evidence are needed: the firm does not do the work.
fn hand_over_transition() -> Transition<DeadlineTask> {
    use DeadlineTaskStatus::{InProgress, InReview, Overdue, Upcoming, Waiting};
    Transition::new(HAND_OVER)
        .label("Hand over to client")
        .tone("neutral")
        .from([Upcoming, InProgress, Waiting, InReview, Overdue])
        .to(DeadlineTaskStatus::Completed)
        .system()
        .effect(|task: &DeadlineTask, _ctx, tx| {
            let id = task.id;
            Box::pin(async move {
                write(
                    tx,
                    id,
                    &[
                        ("completed_at", &Some(renox::db::now())),
                        (
                            "evidence",
                            &Some(wl_kernel::deadlines::HANDED_OVER_EVIDENCE.to_owned()),
                        ),
                        (
                            "closed_reason",
                            &Some(wl_kernel::deadlines::HANDED_OVER_EVIDENCE.to_owned()),
                        ),
                    ],
                )
                .await
            })
        })
}

fn mark_overdue_transition() -> Transition<DeadlineTask> {
    use DeadlineTaskStatus::{InProgress, InReview, Overdue, Upcoming, Waiting};
    with_input(
        Transition::new(MARK_OVERDUE)
            .label("Mark overdue")
            .tone("danger")
            .from([Upcoming, InProgress, Waiting, InReview])
            .to(Overdue)
            .system(),
    )
    .guard("due_passed", |task: &DeadlineTask, ctx| {
        let passed = input(ctx).is_some_and(|i| i.today > task.due_date);
        Box::pin(async move {
            if passed {
                Ok(())
            } else {
                Err(english("wl_ticketing.task.problem.not_due").to_owned())
            }
        })
    })
    .effect(|task: &DeadlineTask, ctx, tx| {
        let id = task.id;
        let stage = task.status.as_str().to_owned();
        let today = input(ctx).map(|i| i.today);
        Box::pin(async move {
            write(
                tx,
                id,
                &[
                    ("late", &true),
                    ("stage", &Some(stage)),
                    ("overdue_on", &today),
                ],
            )
            .await
        })
    })
}

fn close_missed_transition() -> Transition<DeadlineTask> {
    Transition::new(CLOSE_MISSED)
        .label("Close as missed")
        .tone("danger")
        .permission(permissions::TASKS_MANAGE)
        .reason_required()
        .from([DeadlineTaskStatus::Overdue])
        .to(DeadlineTaskStatus::Missed)
        .effect(|task: &DeadlineTask, ctx, tx| closing_reason(task, ctx, tx))
}

/// Stores the reason of `close_missed`, `waive` and `auto_waive`.
fn closing_reason<'a>(
    task: &'a DeadlineTask,
    ctx: &'a TransitionCtx,
    tx: &'a mut renox::db::Transaction,
) -> Pin<Box<dyn Future<Output = Result> + Send + 'a>> {
    let id = task.id;
    let reason = ctx.reason().map(|r| r.trim().to_owned());
    Box::pin(async move { write(tx, id, &[("closed_reason", &reason)]).await })
}

/// `waive` and `auto_waive`: from a started task only when the setting is
/// on.
fn waiving(transition: Transition<DeadlineTask>) -> Transition<DeadlineTask> {
    use DeadlineTaskStatus::{InProgress, NotRequired, Upcoming, Waiting};
    with_input(
        transition
            .reason_required()
            .from([Upcoming, InProgress, Waiting])
            .to(NotRequired),
    )
    .guard("started", |task: &DeadlineTask, ctx| {
        let allowed = task.status == Upcoming || input(ctx).is_some_and(|i| i.waive_started);
        Box::pin(async move {
            if allowed {
                Ok(())
            } else {
                Err(english("wl_ticketing.task.problem.started").to_owned())
            }
        })
    })
    .effect(|task: &DeadlineTask, ctx, tx| closing_reason(task, ctx, tx))
}

impl StateMachine for DeadlineTask {
    type State = DeadlineTaskStatus;
    const COLUMN: renox::db::Col<DeadlineTask, DeadlineTaskStatus> = DeadlineTask::STATUS;

    fn machine() -> Machine<Self> {
        use DeadlineTaskStatus::{
            Completed, InProgress, InReview, Missed, NotRequired, Overdue, Upcoming, Waiting,
        };
        Machine::new()
            .initial([Upcoming])
            .state(Upcoming)
            .tone("neutral")
            .state(InProgress)
            .tone("info")
            .state(Waiting)
            .tone("warning")
            .state(InReview)
            .tone("info")
            .state(Overdue)
            .tone("error")
            .state(Completed)
            .tone("success")
            .terminal()
            .state(Missed)
            .tone("error")
            .terminal()
            .state(NotRequired)
            .tone("neutral")
            .terminal()
            .transition(start_transition())
            .transition(wait_transition())
            .transition(unblocking(
                Transition::new(UNBLOCK)
                    .label("Unblock")
                    .tone("primary")
                    .permission(permissions::TASKS_WORK),
            ))
            .transition(unblocking(
                Transition::new(AUTO_UNBLOCK)
                    .label("Unblock (system)")
                    .system(),
            ))
            .transition(submit_review_transition())
            .transition(send_back_transition())
            .transition(complete_transition())
            .transition(complete_reviewed_transition())
            .transition(mark_overdue_transition())
            .transition(hand_over_transition())
            .transition(close_missed_transition())
            .transition(waiving(
                Transition::new(WAIVE)
                    .label("Waive")
                    .tone("neutral")
                    .permission(permissions::TASKS_MANAGE),
            ))
            .transition(waiving(
                Transition::new(AUTO_WAIVE).label("Waive (system)").system(),
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_is_declared_with_its_tone() {
        let machine = DeadlineTask::machine();
        let states: Vec<_> = machine
            .states()
            .iter()
            .map(|s| (s.name.as_str(), s.tone.as_deref(), s.terminal))
            .collect();
        assert_eq!(
            states,
            [
                ("upcoming", Some("neutral"), false),
                ("in_progress", Some("info"), false),
                ("waiting", Some("warning"), false),
                ("in_review", Some("info"), false),
                ("overdue", Some("error"), false),
                ("completed", Some("success"), true),
                ("missed", Some("error"), true),
                ("not_required", Some("neutral"), true),
            ]
        );
        assert_eq!(machine.initial_states(), ["upcoming"]);
    }

    #[test]
    fn every_transition_exists() {
        let machine = DeadlineTask::machine();
        let expected = [
            ("start", vec!["upcoming", "overdue"], "in_progress", false),
            ("wait", vec!["in_progress"], "waiting", false),
            ("unblock", vec!["waiting"], "in_progress", false),
            ("auto_unblock", vec!["waiting"], "in_progress", true),
            ("submit_review", vec!["in_progress"], "in_review", false),
            ("send_back", vec!["in_review"], "in_progress", false),
            ("complete", vec!["in_progress"], "completed", false),
            ("complete_reviewed", vec!["in_review"], "completed", false),
            (
                "mark_overdue",
                vec!["upcoming", "in_progress", "waiting", "in_review"],
                "overdue",
                true,
            ),
            (
                "hand_over",
                vec!["upcoming", "in_progress", "waiting", "in_review", "overdue"],
                "completed",
                true,
            ),
            ("close_missed", vec!["overdue"], "missed", false),
            (
                "waive",
                vec!["upcoming", "in_progress", "waiting"],
                "not_required",
                false,
            ),
            (
                "auto_waive",
                vec!["upcoming", "in_progress", "waiting"],
                "not_required",
                true,
            ),
        ];
        assert_eq!(machine.transitions().len(), expected.len());
        for (name, from, to, system) in expected {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert_eq!(transition.is_system(), system, "{name}");
        }
    }

    #[test]
    fn closing_needs_a_reason() {
        let machine = DeadlineTask::machine();
        for transition in machine.transitions() {
            let needs = matches!(
                transition.name(),
                "send_back" | "close_missed" | "waive" | "auto_waive"
            );
            assert_eq!(transition.needs_reason(), needs, "{}", transition.name());
        }
    }

    #[test]
    fn the_guards_are_those_of_the_spec() {
        let machine = DeadlineTask::machine();
        let guards = |name: &str| {
            machine
                .find(name)
                .expect("transition")
                .guard_names()
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(guards("wait"), ["reason_or_item"]);
        assert_eq!(guards("submit_review"), ["reviewer"]);
        assert_eq!(guards("complete"), ["input", "evidence", "no_reviewer"]);
        assert_eq!(guards("complete_reviewed"), ["input", "evidence"]);
        assert_eq!(guards("mark_overdue"), ["input", "due_passed"]);
        assert_eq!(guards("waive"), ["input", "started"]);
        assert_eq!(guards("auto_waive"), ["input", "started"]);
    }
}
