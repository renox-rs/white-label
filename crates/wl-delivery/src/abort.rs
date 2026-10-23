//! Aborting a run mid-route (wl-delivery §4.2 `abort`, §5.3 step 2, issue
//! #164).
//!
//! When a run cannot go on (breakdown, accident, driver unwell, weather, or a
//! reason the vertical adds: 03 *driver unreachable*) the dispatcher or the
//! driver aborts it, always with a reason (*other* needs a note). The goods
//! that can still be moved are handed over first ([`crate::hand_over`]); then
//! [`abort_run`]:
//!
//! - runs the run's `abort` (`runs.dispatch`) or `abort_by_driver`
//!   (`runs.drive`, the run's driver only) to `returned_to_depot`;
//! - fails every stop still `out_for_delivery` with the reason *run
//!   aborted* through the system transition `fail_aborted`, which does not
//!   count against the maximum attempts. All of it in one transaction;
//! - after the commit emits `DeliveryAttemptFailed` for each of them and
//!   `RunAborted`, tells each recipient of the delay and the dispatchers
//!   (the notices of [`crate::notices`]), opens a breakdown or accident
//!   ticket about the vehicle when the dispatcher ticked it, and ends the
//!   driver's cash period in the custody, so the cash handover of the run
//!   (opened by each `deliver`) is counted at the depot as usual
//!   (`FIN.CSH.01`, `FIN.RUL.22`);
//! - leaves `close` to the system step of [`crate::close`]: the run closes
//!   once the cash is counted and every failed stop is received back or
//!   reported missing.
//!
//! The perishable policy (`WRK.RUL.16`) and the reattempt of a non-perishable
//! stop (`WRK.RUL.08`) react to the failed attempts
//! ([`crate::attempted::after_failed_attempt`], BL-DLV-07); 03 has no
//! perishable deliveries.

use renox::db::{DateTime, Transaction};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};
use serde::{Deserialize, Serialize};
use wl_kernel::custody::custody;
use wl_kernel::delivery::{DeliveryAttemptFailed, RunAborted};
use wl_kernel::tickets::{self, OpenTicket, TASK_CATEGORY, TicketRef};

use crate::delivery::{Attempt, AttemptOutcome, Delivery, DeliveryStatus};
use crate::notices;
use crate::permissions;
use crate::proof::next_attempt_number;
use crate::run::{Run, RunStatus};
use crate::settings::settings;
use crate::texts::english;

/// `abort`: dispatched → returned_to_depot, by the dispatcher.
pub const ABORT: &str = "abort";
/// `abort`, by the run's driver from the phone.
pub const ABORT_BY_DRIVER: &str = "abort_by_driver";
/// A stop fails because its run was aborted: out_for_delivery → attempted,
/// by the system, not counted.
pub const FAIL_ABORTED: &str = "fail_aborted";

/// The failure reason of such a visit (§7: does not count).
pub const RUN_ABORTED_REASON: &str = "run_aborted";

/// The reason that needs a note.
pub const OTHER: &str = "other";

/// The tickets the dispatcher can open about the vehicle.
pub const TICKET_KINDS: [&str; 2] = ["breakdown", "accident"];

/// What the dispatcher or the driver gives.
#[derive(Debug, Clone, Default)]
pub struct AbortRequest {
    /// One of the abort reasons (`breakdown`, `accident`, `driver_unwell`,
    /// `weather`, `other`, and the vertical's).
    pub reason: String,
    /// A note; needed for *other*.
    pub note: String,
    /// `breakdown` or `accident` to open a ticket about the vehicle.
    pub ticket: Option<String>,
}

/// What the abort came to.
#[derive(Debug, Clone)]
pub struct Aborted {
    /// The stops that failed with *run aborted*.
    pub failed: Vec<Delivery>,
    /// The ticket opened about the vehicle, when asked and ticketing is on.
    pub ticket: Option<TicketRef>,
}

/// What `abort` runs on, worked out by the service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AbortInput {
    pub reason: String,
    pub reason_known: bool,
    pub note: String,
    pub driver_user_id: i64,
}

fn abort_input(ctx: &TransitionCtx) -> Option<AbortInput> {
    ctx.input_as::<AbortInput>().ok()
}

/// What `fail_aborted` runs on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AbortedStop {
    pub at: DateTime,
    pub attempt_number: i64,
}

fn aborted_stop(ctx: &TransitionCtx) -> Option<AbortedStop> {
    ctx.input_as::<AbortedStop>().ok()
}

fn guard(problem: Option<String>) -> std::result::Result<(), String> {
    problem.map_or(Ok(()), Err)
}

/// The two run transitions, for the machine.
pub(crate) fn transitions() -> Vec<Transition<Run>> {
    vec![
        aborting(
            Transition::new(ABORT)
                .label("Abort")
                .tone("danger")
                .permission(permissions::RUNS_DISPATCH),
        ),
        aborting(
            Transition::new(ABORT_BY_DRIVER)
                .label("Abort (driver)")
                .tone("danger")
                .permission(permissions::RUNS_DRIVE)
                .guard("by_the_runs_driver", |_run: &Run, ctx| {
                    let driver = abort_input(ctx).map(|i| i.driver_user_id);
                    let allowed =
                        matches!((ctx.actor(), driver), (Some(user), Some(d)) if user.id == d);
                    Box::pin(async move {
                        guard(
                            (!allowed)
                                .then(|| english("wl_delivery.abort.only_the_driver").to_owned()),
                        )
                    })
                }),
        ),
    ]
}

/// What both aborts share: the states, the reason and the effect.
fn aborting(transition: Transition<Run>) -> Transition<Run> {
    transition
        .from([RunStatus::Dispatched])
        .to(RunStatus::ReturnedToDepot)
        .reason_required()
        .guard("reason_known", |_run: &Run, ctx| {
            let known = abort_input(ctx).is_some_and(|i| i.reason_known);
            Box::pin(async move {
                guard((!known).then(|| english("wl_delivery.abort.choose_reason").to_owned()))
            })
        })
        .guard("note_for_other", |_run: &Run, ctx| {
            let missing =
                abort_input(ctx).is_some_and(|i| i.reason == OTHER && i.note.trim().is_empty());
            Box::pin(async move {
                guard(missing.then(|| english("wl_delivery.abort.note_for_other").to_owned()))
            })
        })
        .effect(|run: &Run, ctx, tx| {
            let id = run.id;
            let input = abort_input(ctx);
            let actor = ctx.actor().map(|user| user.id);
            Box::pin(async move {
                let input = input.ok_or_else(|| {
                    Error::from(renox::anyhow::anyhow!("an abort without its input"))
                })?;
                let reason = Some(input.reason.clone());
                let note = Some(input.note.trim().to_owned());
                let now = Some(renox::db::now());
                Run::query()
                    .where_(Run::ID.eq(id))
                    .update(
                        &mut *tx,
                        &[
                            ("abort_reason", &reason),
                            ("abort_note", &note),
                            ("aborted_by", &actor),
                            ("aborted_at", &now),
                            ("returned_at", &now),
                        ],
                    )
                    .await?;
                Ok(())
            })
        })
}

/// The delivery transition `fail_aborted`, for the machine.
pub(crate) fn delivery_transitions() -> Vec<Transition<Delivery>> {
    vec![
        Transition::new(FAIL_ABORTED)
            .label("Fail (run aborted)")
            .tone("warning")
            .from([DeliveryStatus::OutForDelivery])
            .to(DeliveryStatus::Attempted)
            .system()
            .effect(|delivery: &Delivery, ctx, tx| {
                let delivery_id = delivery.id;
                let run_id = delivery.run_id;
                let actor = ctx.actor().map(|user| user.id);
                let stop = aborted_stop(ctx);
                Box::pin(async move {
                    let stop = stop.ok_or_else(|| {
                        Error::from(renox::anyhow::anyhow!("a failed stop without its input"))
                    })?;
                    save_attempt(&mut *tx, delivery_id, run_id, &stop, actor).await
                })
            }),
    ]
}

/// The visit of a stop that failed because its run was aborted: it never
/// counts against the maximum attempts, so the delivery's count is left as it
/// is.
async fn save_attempt(
    tx: &mut Transaction,
    delivery_id: i64,
    run_id: Option<i64>,
    stop: &AbortedStop,
    actor: Option<i64>,
) -> Result<()> {
    let mut attempt = Attempt {
        delivery_id,
        run_id,
        number: stop.attempt_number,
        outcome: AttemptOutcome::Failed,
        failure_reason: Some(RUN_ABORTED_REASON.to_owned()),
        counts: false,
        occurred_at: stop.at,
        actor_user_id: actor,
        ..Attempt::default()
    };
    attempt.insert(&mut *tx).await
}

fn refused(message: impl Into<String>) -> TransitionError {
    TransitionError::Guard(message.into())
}

/// The reason's words.
pub fn reason_text(code: &str) -> String {
    english(&format!("wl_delivery.abort.reason.{code}")).to_owned()
}

/// Aborts `run` for `user`: the dispatcher (`runs.dispatch`), or the run's
/// driver (`runs.drive`). See the module text for what follows.
///
/// # Errors
///
/// `Forbidden` without either permission; `NotFrom` unless the run is
/// `dispatched`; `Guard` (422) without a known reason, for *other* without a
/// note, for a driver who is not the run's driver, or an unknown ticket kind.
pub async fn abort_run(
    state: &AppState,
    user: &AuthUser,
    run: &Run,
    request: &AbortRequest,
) -> std::result::Result<Aborted, TransitionError> {
    if let Some(kind) = &request.ticket
        && !TICKET_KINDS.contains(&kind.as_str())
    {
        return Err(refused(english("wl_delivery.abort.ticket_unknown")));
    }
    let known = settings(state).abort_reasons();
    let reason = request.reason.trim().to_owned();
    let input = AbortInput {
        reason_known: known.contains(&reason),
        reason: reason.clone(),
        note: request.note.trim().to_owned(),
        driver_user_id: run.driver_user_id,
    };
    let name = if user.has_permission(permissions::RUNS_DISPATCH) {
        ABORT
    } else {
        ABORT_BY_DRIVER
    };
    // The history reads "Breakdown: the van will not start".
    let history = if input.note.is_empty() {
        reason_text(&reason)
    } else {
        format!("{}: {}", reason_text(&reason), input.note)
    };
    let open: Vec<Delivery> = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .where_(Delivery::STATUS.eq(DeliveryStatus::OutForDelivery))
        .order_by(Delivery::RUN_SEQUENCE)
        .get(&state.db)
        .await
        .map_err(TransitionError::Failed)?;
    let at = renox::db::now();
    let mut numbers = Vec::new();
    for delivery in &open {
        numbers.push(
            next_attempt_number(&state.db, delivery)
                .await
                .map_err(TransitionError::Failed)?,
        );
    }
    let mut tx = state
        .db
        .begin()
        .await
        .map_err(|e| TransitionError::Failed(e.into()))?;
    let run_outcome = run
        .transition(name)
        .by(user)
        .reason(history.clone())
        .input(&input)
        .run_in(&mut tx)
        .await?;
    let mut outcomes = Vec::new();
    for (delivery, number) in open.iter().zip(&numbers) {
        outcomes.push(
            delivery
                .transition(FAIL_ABORTED)
                .by_system()
                .input(&AbortedStop {
                    at,
                    attempt_number: *number,
                })
                .run_in(&mut tx)
                .await?,
        );
    }
    tx.commit()
        .await
        .map_err(|e| TransitionError::Failed(e.into()))?;
    run_outcome
        .emit(state)
        .await
        .map_err(TransitionError::Failed)?;
    for outcome in outcomes {
        outcome.emit(state).await.map_err(TransitionError::Failed)?;
    }
    for (delivery, number) in open.iter().zip(&numbers) {
        state
            .emit(DeliveryAttemptFailed {
                delivery: delivery.delivery_id(),
                source: delivery.source(),
                reason: RUN_ABORTED_REASON.to_owned(),
                attempt: *number,
            })
            .await
            .map_err(TransitionError::Failed)?;
    }
    // A perishable stop follows `WRK.RUL.16` at once (return or discard per
    // the policy for an abort); the others `WRK.RUL.08` when it is on.
    for delivery in &open {
        crate::attempted::after_failed_attempt(state, delivery.id, true)
            .await
            .map_err(TransitionError::Failed)?;
    }
    // The driver's cash period ends now: the handover of the run is counted
    // at the depot as usual.
    custody(state)
        .end_period(state, run.driver_user_id, run.subject(), renox::db::now())
        .await
        .map_err(TransitionError::Failed)?;
    let ticket = match &request.ticket {
        Some(kind) => open_ticket(state, run, kind, &history)
            .await
            .map_err(TransitionError::Failed)?,
        None => None,
    };
    for delivery in &open {
        notices::run_delayed(state, delivery, run)
            .await
            .map_err(TransitionError::Failed)?;
    }
    notices::run_aborted(state, run, &history)
        .await
        .map_err(TransitionError::Failed)?;
    let cash_collected = crate::close::cash_expected(&state.db, run)
        .await
        .map_err(TransitionError::Failed)?;
    state
        .emit(RunAborted {
            run: run.run_id(),
            driver: run.driver_user_id,
            vehicle: run.vehicle_asset_id,
            reason,
            deliveries: open.iter().map(Delivery::delivery_id).collect(),
            cash_collected,
            aborted_by: user.id,
        })
        .await
        .map_err(TransitionError::Failed)?;
    // A run with nothing to account for closes at once, as on `return_to_depot`.
    crate::close::try_close(state, run.id).await?;
    let mut failed = Vec::new();
    for delivery in &open {
        failed.push(
            Delivery::find_or_404(&state.db, delivery.id)
                .await
                .map_err(TransitionError::Failed)?,
        );
    }
    Ok(Aborted { failed, ticket })
}

/// The ticket about the vehicle (the run for an on-foot rider), once per run.
async fn open_ticket(
    state: &AppState,
    run: &Run,
    kind: &str,
    history: &str,
) -> Result<Option<TicketRef>> {
    let subject = run.vehicle_subject().unwrap_or_else(|| run.subject());
    let title =
        english(&format!("wl_delivery.abort.ticket_title.{kind}")).replace("{run}", &run.number);
    let ticket = OpenTicket::new(subject, TASK_CATEGORY, &title)
        .description(history)
        .source(run.subject())
        .source_key(&format!("delivery.run_abort:{}", run.id));
    tickets::task_creator(state).open(state, ticket).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_aborts_leave_a_dispatched_run_for_the_depot() {
        let machine = Run::machine();
        for name in [ABORT, ABORT_BY_DRIVER] {
            let abort = machine.find(name).expect(name);
            assert_eq!(abort.from_states(), ["dispatched"]);
            assert_eq!(abort.to_state(), Some("returned_to_depot"));
            assert!(!abort.is_system());
        }
    }

    #[test]
    fn a_failed_stop_is_the_systems_and_the_visit_does_not_count() {
        let machine = Delivery::machine();
        let fail = machine.find(FAIL_ABORTED).expect("fail_aborted");
        assert_eq!(fail.from_states(), ["out_for_delivery"]);
        assert_eq!(fail.to_state(), Some("attempted"));
        assert!(fail.is_system());
    }

    #[test]
    fn every_reason_and_ticket_kind_has_words() {
        for code in [
            "breakdown",
            "accident",
            "driver_unwell",
            "weather",
            "other",
            "driver_unreachable",
        ] {
            assert_ne!(
                reason_text(code),
                format!("wl_delivery.abort.reason.{code}")
            );
        }
        for kind in TICKET_KINDS {
            let key = format!("wl_delivery.abort.ticket_title.{kind}");
            assert_ne!(english(&key), key);
        }
    }
}
