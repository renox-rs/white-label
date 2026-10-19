//! Rule runs (wl-kernel §4.2, §5.3): one firing of a rule for one record and
//! occurrence.
//!
//! ```text
//! [*] --> queued
//! queued | retrying --run--> succeeded
//! queued | retrying --skip--> skipped
//! queued | retrying --retry--> retrying      (attempts < 5, with backoff)
//! retrying --fail--> failed                  (attempts = 5; AutomationFailed)
//! queued --stop--> stopped                   (chain depth > 3; AutomationFailed)
//! failed --rerun--> queued                   (automations.retry; attempts reset)
//! ```
//!
//! [`run_rule`] runs an action once per occurrence key: a key that was
//! already used is skipped, a failing action is retried with [`backoff`]
//! (the next call with the same key tries again once the wait is over), the
//! fifth failure ends the run as `failed`, and a chain deeper than
//! [`MAX_CHAIN_DEPTH`] is `stopped` without running anything. All transitions
//! are `system`, except `rerun`.

use std::future::Future;

use renox::chrono::Duration;
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition, TransitionError};
use serde::{Deserialize, Serialize};

use super::AutomationFailed;
use crate::subject::Subject;

/// The attempts of one run before it is `failed`.
pub const MAX_ATTEMPTS: i64 = 5;

/// The deepest chain of rules triggering rules that still runs; a run
/// deeper than this is `stopped`.
pub const MAX_CHAIN_DEPTH: i64 = 3;

/// The transition names of a rule run.
pub mod transitions {
    /// The actions ran.
    pub const RUN: &str = "run";
    /// The conditions are false, or the occurrence key was already used.
    pub const SKIP: &str = "skip";
    /// An action failed; try again after the backoff.
    pub const RETRY: &str = "retry";
    /// The last attempt failed.
    pub const FAIL: &str = "fail";
    /// The chain is too deep.
    pub const STOP: &str = "stop";
    /// An admin queues a failed run again.
    pub const RERUN: &str = "rerun";
}

/// The state of a rule run.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RuleRunStatus {
    /// Trigger fired; waiting in the queue.
    #[default]
    Queued,
    /// All actions done.
    Succeeded,
    /// Conditions not met at run time, or already ran for this occurrence.
    Skipped,
    /// An action failed; will retry with backoff.
    Retrying,
    /// Retries exhausted.
    Failed,
    /// Chain depth above 3; reported, nothing done.
    Stopped,
}

/// One firing of a rule for one record and occurrence.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_rule_runs",
    state_machine,
    unique(rule, subject_type, subject_id, occurrence_key),
    index(status)
)]
pub struct RuleRun {
    /// The primary key.
    pub id: i64,
    /// The rule id, e.g. `PLT.RUL.05`.
    pub rule: String,
    /// The subject's type, e.g. `kernel.integration_connection`.
    pub subject_type: String,
    /// The subject's id.
    pub subject_id: i64,
    /// Idempotency: at most one run per rule, subject and key.
    pub occurrence_key: String,
    /// How many rules triggered this one, from 0.
    pub chain_depth: i64,
    /// The state; changed only by a transition.
    pub status: RuleRunStatus,
    /// Attempts made so far.
    pub attempts: i64,
    /// The last error.
    pub error: Option<String>,
    /// When a retrying run may be tried again.
    pub next_attempt_at: Option<DateTime>,
    /// When the first attempt began.
    pub started_at: Option<DateTime>,
    /// When the run ended.
    pub finished_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl RuleRun {
    /// The record the run was about.
    pub fn subject(&self) -> Subject {
        Subject::new(&self.subject_type, self.subject_id)
    }

    /// Whether the run has ended in a state no transition leaves, except
    /// `rerun` of a failed one.
    pub fn is_over(&self) -> bool {
        !matches!(self.status, RuleRunStatus::Queued | RuleRunStatus::Retrying)
    }

    /// The run of `rule` about `subject` with `occurrence_key`, if any.
    pub async fn find_occurrence(
        db: &Db,
        rule: &str,
        subject: &Subject,
        occurrence_key: &str,
    ) -> Result<Option<RuleRun>> {
        RuleRun::query()
            .where_(RuleRun::RULE.eq(rule))
            .where_(RuleRun::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
            .where_(RuleRun::SUBJECT_ID.eq(subject.subject_id))
            .where_(RuleRun::OCCURRENCE_KEY.eq(occurrence_key))
            .first(db)
            .await
    }
}

/// The wait before the next attempt after failed attempt `attempt`
/// (1-based): 1 minute, 5 minutes, 15 minutes, then 1 hour.
pub fn backoff(attempt: i64) -> Duration {
    Duration::minutes(match attempt {
        ..=1 => 1,
        2 => 5,
        3 => 15,
        _ => 60,
    })
}

/// Sets columns of run `id` inside a transition's transaction.
async fn record(
    tx: &mut renox::db::Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    RuleRun::query()
        .where_(RuleRun::ID.eq(id))
        .update(&mut *tx, columns)
        .await?;
    Ok(())
}

/// The effect of a transition that ends the run: stamps `finished_at`.
async fn finish(tx: &mut renox::db::Transaction, id: i64) -> Result {
    let at = now();
    record(
        tx,
        id,
        &[
            ("finished_at", &Some(at)),
            ("next_attempt_at", &None::<DateTime>),
            ("updated_at", &Some(at)),
        ],
    )
    .await
}

impl StateMachine for RuleRun {
    type State = RuleRunStatus;
    const COLUMN: renox::db::Col<RuleRun, RuleRunStatus> = RuleRun::STATUS;

    fn machine() -> Machine<Self> {
        use RuleRunStatus::{Failed, Queued, Retrying, Skipped, Stopped, Succeeded};
        Machine::new()
            .initial([Queued])
            .state(Queued)
            .tone("neutral")
            .state(Succeeded)
            .tone("success")
            .terminal()
            .state(Skipped)
            .tone("neutral")
            .terminal()
            .state(Retrying)
            .tone("warning")
            .state(Failed)
            .tone("error")
            .state(Stopped)
            .tone("error")
            .terminal()
            .transition(
                Transition::new(transitions::RUN)
                    .label("Run")
                    .from([Queued, Retrying])
                    .to(Succeeded)
                    .system()
                    .effect(|run: &RuleRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move { finish(tx, id).await })
                    }),
            )
            .transition(
                Transition::new(transitions::SKIP)
                    .label("Skip")
                    .from([Queued, Retrying])
                    .to(Skipped)
                    .system()
                    .effect(|run: &RuleRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move { finish(tx, id).await })
                    }),
            )
            .transition(
                Transition::new(transitions::RETRY)
                    .label("Retry")
                    .from([Queued, Retrying])
                    .to(Retrying)
                    .system()
                    .guard("attempts_left", |run: &RuleRun, _ctx| {
                        let attempts = run.attempts;
                        Box::pin(async move {
                            if attempts < MAX_ATTEMPTS {
                                Ok(())
                            } else {
                                Err("All attempts are used.".into())
                            }
                        })
                    })
                    .effect(|run: &RuleRun, _ctx, tx| {
                        let (id, attempts) = (run.id, run.attempts);
                        Box::pin(async move {
                            let at = now();
                            record(
                                tx,
                                id,
                                &[
                                    ("next_attempt_at", &Some(at + backoff(attempts))),
                                    ("updated_at", &Some(at)),
                                ],
                            )
                            .await
                        })
                    }),
            )
            .transition(
                Transition::new(transitions::FAIL)
                    .label("Fail")
                    .from([Retrying])
                    .to(Failed)
                    .system()
                    .guard("attempts_used", |run: &RuleRun, _ctx| {
                        let attempts = run.attempts;
                        Box::pin(async move {
                            if attempts >= MAX_ATTEMPTS {
                                Ok(())
                            } else {
                                Err("Attempts are left.".into())
                            }
                        })
                    })
                    .effect(|run: &RuleRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move { finish(tx, id).await })
                    }),
            )
            .transition(
                Transition::new(transitions::STOP)
                    .label("Stop")
                    .from([Queued])
                    .to(Stopped)
                    .system()
                    .guard("too_deep", |run: &RuleRun, _ctx| {
                        let depth = run.chain_depth;
                        Box::pin(async move {
                            if depth > MAX_CHAIN_DEPTH {
                                Ok(())
                            } else {
                                Err("The chain is not too deep.".into())
                            }
                        })
                    })
                    .effect(|run: &RuleRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move { finish(tx, id).await })
                    }),
            )
            .transition(
                Transition::new(transitions::RERUN)
                    .label("Run again")
                    .from([Failed])
                    .to(Queued)
                    .permission(crate::permissions::AUTOMATIONS_RETRY)
                    .effect(|run: &RuleRun, _ctx, tx| {
                        let id = run.id;
                        Box::pin(async move {
                            record(
                                tx,
                                id,
                                &[
                                    ("attempts", &0_i64),
                                    ("error", &None::<String>),
                                    ("next_attempt_at", &None::<DateTime>),
                                    ("finished_at", &None::<DateTime>),
                                    ("updated_at", &Some(now())),
                                ],
                            )
                            .await
                        })
                    }),
            )
    }
}

/// What a rule run is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleRunSpec {
    /// The rule id, e.g. `PLT.RUL.05`.
    pub rule: String,
    /// The record it is about.
    pub subject: Subject,
    /// Idempotency key, e.g. `FIN.RUL.01:invoice:42:overdue+7`.
    pub occurrence_key: String,
    /// How many rules triggered this one; 0 for a trigger that is not a rule.
    pub chain_depth: i64,
    /// Whether the rule is compliance-critical (carried on
    /// [`AutomationFailed`]).
    pub compliance_critical: bool,
}

impl RuleRunSpec {
    /// A run of `rule` about `subject` for `occurrence_key`, at chain depth 0.
    pub fn new(rule: &str, subject: Subject, occurrence_key: impl Into<String>) -> Self {
        RuleRunSpec {
            rule: rule.to_owned(),
            subject,
            occurrence_key: occurrence_key.into(),
            chain_depth: 0,
            compliance_critical: false,
        }
    }

    /// Sets the chain depth.
    pub fn chain_depth(mut self, depth: i64) -> Self {
        self.chain_depth = depth;
        self
    }

    /// Marks the rule compliance-critical.
    pub fn compliance_critical(mut self) -> Self {
        self.compliance_critical = true;
        self
    }
}

/// What [`run_rule`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunOutcome {
    /// The action ran and the run `succeeded`.
    Succeeded,
    /// The occurrence key was already used (by a run that ended, or that
    /// someone else is running): nothing ran.
    Skipped,
    /// The action failed, or its wait is not over: it is `retrying`.
    Retrying,
    /// The last attempt failed (now or earlier); `AutomationFailed` was
    /// emitted when it happened. A failed run waits for `rerun`.
    Failed,
    /// The chain is too deep; nothing ran.
    Stopped,
}

/// Runs `action` for the occurrence in `spec`, as a [`RuleRun`].
///
/// - No run for the key yet: one is queued and tried now.
/// - The key was used by a run that `succeeded`, was `skipped` or `stopped`:
///   [`RunOutcome::Skipped`], nothing runs and no row is added.
/// - A `retrying` run is tried again only when its wait is over; call again
///   (e.g. from a scheduled task) to retry. A `failed` run is left alone
///   until an admin runs `rerun`.
/// - An action that returns an error is `retry` after the 1st to 4th
///   attempt; after the 5th the run is `fail` and [`AutomationFailed`] is
///   emitted.
/// - A chain depth above [`MAX_CHAIN_DEPTH`] is `stop`: nothing runs and
///   [`AutomationFailed`] is emitted.
///
/// The action gets a clone of the state and runs as the system actor.
pub async fn run_rule<F, Fut>(state: &AppState, spec: RuleRunSpec, action: F) -> Result<RunOutcome>
where
    F: FnOnce(AppState) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let db = &state.db;
    let run = match RuleRun::find_occurrence(db, &spec.rule, &spec.subject, &spec.occurrence_key)
        .await?
    {
        Some(run) => run,
        None => {
            let created = RuleRun::create(
                db,
                RuleRun {
                    rule: spec.rule.clone(),
                    subject_type: spec.subject.subject_type.clone(),
                    subject_id: spec.subject.subject_id,
                    occurrence_key: spec.occurrence_key.clone(),
                    chain_depth: spec.chain_depth,
                    ..Default::default()
                },
            )
            .await;
            match created {
                Ok(run) => run,
                // Another worker queued the same occurrence first.
                Err(error) if error.is_unique_violation() => return Ok(RunOutcome::Skipped),
                Err(error) => return Err(error),
            }
        }
    };

    match run.status {
        RuleRunStatus::Succeeded | RuleRunStatus::Skipped | RuleRunStatus::Stopped => {
            return Ok(RunOutcome::Skipped);
        }
        RuleRunStatus::Failed => return Ok(RunOutcome::Failed),
        RuleRunStatus::Retrying if run.next_attempt_at.is_some_and(|at| at > now()) => {
            return Ok(RunOutcome::Retrying);
        }
        RuleRunStatus::Queued | RuleRunStatus::Retrying => {}
    }

    if run.status == RuleRunStatus::Queued && run.chain_depth > MAX_CHAIN_DEPTH {
        let error = format!(
            "The rule chain is {} deep; the limit is {MAX_CHAIN_DEPTH}.",
            run.chain_depth
        );
        RuleRun::where_eq("id", run.id)
            .update(db, &[("error", &Some(error.clone()))])
            .await?;
        transition(state, &run, transitions::STOP).await?;
        announce(state, &spec, error).await?;
        return Ok(RunOutcome::Stopped);
    }

    // Claim the attempt: of two workers holding the same row, one wins.
    let started = run.started_at.unwrap_or_else(now);
    let claimed = RuleRun::where_eq("id", run.id)
        .where_eq("attempts", run.attempts)
        .where_eq("status", run.status)
        .update(
            db,
            &[
                ("attempts", &(run.attempts + 1)),
                ("started_at", &Some(started)),
                ("updated_at", &Some(now())),
            ],
        )
        .await?;
    if claimed == 0 {
        return Ok(RunOutcome::Skipped);
    }
    let run = RuleRun::find_or_404(db, run.id).await?;

    match action(state.clone()).await {
        Ok(()) => {
            transition(state, &run, transitions::RUN).await?;
            Ok(RunOutcome::Succeeded)
        }
        Err(error) => {
            let message = error.to_string();
            RuleRun::where_eq("id", run.id)
                .update(db, &[("error", &Some(message.clone()))])
                .await?;
            if run.attempts < MAX_ATTEMPTS {
                transition(state, &run, transitions::RETRY).await?;
                Ok(RunOutcome::Retrying)
            } else {
                transition(state, &run, transitions::FAIL).await?;
                announce(state, &spec, message).await?;
                Ok(RunOutcome::Failed)
            }
        }
    }
}

/// Runs the system transition `name` on `run` (as loaded) and emits it.
async fn transition(state: &AppState, run: &RuleRun, name: &str) -> Result {
    run.transition(name)
        .by_system()
        .run(state)
        .await
        .map(|_| ())
        .map_err(TransitionError::into_error)
}

/// Emits `AutomationFailed` for a run that failed or was stopped.
async fn announce(state: &AppState, spec: &RuleRunSpec, error: String) -> Result {
    state
        .emit(AutomationFailed {
            rule: spec.rule.clone(),
            subject: spec.subject.clone(),
            error,
            compliance_critical: spec.compliance_critical,
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_grows_then_stays_an_hour() {
        let minutes: Vec<i64> = (1..=6).map(|n| backoff(n).num_minutes()).collect();
        assert_eq!(minutes, [1, 5, 15, 60, 60, 60]);
    }

    #[test]
    fn a_run_is_over_unless_it_waits_to_run() {
        let run = |status| RuleRun {
            status,
            ..Default::default()
        };
        assert!(!run(RuleRunStatus::Queued).is_over());
        assert!(!run(RuleRunStatus::Retrying).is_over());
        assert!(run(RuleRunStatus::Failed).is_over());
        assert!(run(RuleRunStatus::Succeeded).is_over());
    }

    #[test]
    fn the_machine_has_every_state_and_transition() {
        let machine = RuleRun::machine();
        for name in ["run", "skip", "retry", "fail", "stop", "rerun"] {
            assert!(machine.find(name).is_ok(), "{name}");
        }
    }
}
