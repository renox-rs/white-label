//! Calling an external system through its connection (wl-kernel §5.14).
//!
//! A module hands its outbound calls to [`submit`]. The connection decides:
//!
//! - `up` with nothing queued: the call is sent at once through the module's
//!   handler. A failure runs `degrade` and queues the call for a retry;
//! - `degraded`, `down` or `disabled`, or `up` with calls already queued: the
//!   call is queued behind the others, so none overtakes another. A
//!   `disabled` connection is never called.
//!
//! [`run_due`] is the scheduled run (every minute): it retries the oldest
//! queued call of each connection when its wait is over, and runs `go_down`
//! (`PLT.RUL.05`) on every `degraded` connection that has had no success for
//! N minutes since `failed since`. A connection with no calls never goes
//! down: only a failed call starts the clock.

use renox::AuthUser;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use renox::state_machine::{Outcome, StateMachine, TransitionError};

use super::alerts;
use super::connection::{ConnectionStatus, IntegrationConnection, transitions};
use super::def::{Delivery, IntegrationDef, Outbound, get};
use super::events::IntegrationStatusChanged;
use super::queue::{self, QueuedCall};
use super::{SUBJECT_TYPE, create_row};
use crate::automation::{RuleRunSpec, RunOutcome, run_rule};
use crate::subject::Subject;
use crate::texts;

/// The scheduled task that runs [`run_due`] every minute.
pub const TASK: &str = "integrations:run";

/// The rule id of the health check (`PLT.RUL.05`).
pub const RULE: &str = "PLT.RUL.05";

/// What [`submit`] did with a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submitted {
    /// The external system took the call.
    Sent,
    /// The call is queued (the connection is not `up`, or the call failed)
    /// and will be sent in order.
    Queued {
        /// The queued call's id.
        id: i64,
    },
    /// The module's handler reported the call obsolete: nothing was sent.
    Obsolete,
}

/// What a scheduled run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Queued calls sent.
    pub sent: usize,
    /// Queued calls reported obsolete by their module.
    pub obsolete: usize,
    /// Connections that went `down`.
    pub went_down: usize,
}

/// The definition of `key`, or a 400 saying it is not registered.
fn definition(key: &str) -> Result<std::sync::Arc<IntegrationDef>> {
    get(key).ok_or_else(|| Error::BadRequest(texts::en("wl_kernel.integrations.unknown")))
}

/// The stored connection `key`, created from its definition when the
/// database lacks it.
pub async fn ensure(db: &Db, key: &str) -> Result<IntegrationConnection> {
    if let Some(found) = IntegrationConnection::by_key(db, key).await? {
        return Ok(found);
    }
    let def = definition(key)?;
    match create_row(db, &def).await {
        Ok(created) => Ok(created),
        // Another worker stored it first.
        Err(error) if error.is_unique_violation() => IntegrationConnection::by_key(db, key)
            .await?
            .ok_or(Error::NotFound),
        Err(error) => Err(error),
    }
}

/// Runs the system transition `name` on connection `key` in its own
/// transaction (the row locked), emits the engine's event and
/// [`IntegrationStatusChanged`] after the commit, and returns the connection
/// as it is then. `None` when the transition no longer applies: another
/// worker moved the connection first, or (for `go_down`) the limit is not
/// reached.
async fn run_system(
    state: &AppState,
    key: &str,
    name: &str,
    input: Value,
) -> Result<Option<IntegrationConnection>> {
    let mut tx = state.db.begin().await?;
    let connection = IntegrationConnection::query()
        .where_(IntegrationConnection::KEY.eq(key))
        .lock_for_update()
        .first(&mut tx)
        .await?
        .ok_or(Error::NotFound)?;
    let from = connection.status;
    let outcome = match connection
        .transition(name)
        .by_system()
        .input(&input)
        .run_in(&mut tx)
        .await
    {
        Ok(outcome) => outcome,
        Err(TransitionError::NotFrom(_) | TransitionError::Conflict | TransitionError::Guard(_)) => {
            return Ok(None);
        }
        Err(error) => return Err(error.into_error()),
    };
    let after = reload(&mut tx, connection.id).await?;
    tx.commit().await?;
    announce(state, outcome, key, from, &after).await?;
    Ok(Some(after))
}

async fn reload(tx: &mut Transaction, id: i64) -> Result<IntegrationConnection> {
    IntegrationConnection::find(&mut *tx, id)
        .await?
        .ok_or(Error::NotFound)
}

/// After the commit: the engine's event, then [`IntegrationStatusChanged`].
async fn announce(
    state: &AppState,
    outcome: Outcome<IntegrationConnection>,
    key: &str,
    from: ConnectionStatus,
    after: &IntegrationConnection,
) -> Result {
    if let Outcome::Done(done) = outcome {
        done.emit(state).await?;
    }
    state
        .emit(IntegrationStatusChanged {
            connection: key.to_owned(),
            from: from.as_text().to_owned(),
            to: after.status.as_text().to_owned(),
            failed_since: after.failed_since,
        })
        .await
}

/// A call of `key` failed with `error`: an `up` connection `degrade`s (its
/// failures begin now); one that is already `degraded` or `down` keeps its
/// `failed since` and notes the error; a `disabled` one is left alone.
pub async fn report_failure(state: &AppState, key: &str, error: &str) -> Result {
    ensure(&state.db, key).await?;
    let degraded = run_system(state, key, transitions::DEGRADE, json!({ "error": error })).await?;
    if degraded.is_none() {
        IntegrationConnection::where_eq("key", key)
            .where_op("status", "!=", ConnectionStatus::Disabled)
            .update(
                &state.db,
                &[
                    ("last_error", &Some(error.to_owned())),
                    ("updated_at", &Some(now())),
                ],
            )
            .await?;
    }
    Ok(())
}

/// A call of `key` succeeded: a `degraded` or `down` connection `recover`s
/// to `up` (the banner goes); an `up` one notes the time; a `disabled` one
/// is left alone. Returns whether the connection recovered.
pub async fn report_success(state: &AppState, key: &str) -> Result<bool> {
    let connection = ensure(&state.db, key).await?;
    match connection.status {
        ConnectionStatus::Degraded | ConnectionStatus::Down => {
            Ok(run_system(state, key, transitions::RECOVER, json!({}))
                .await?
                .is_some())
        }
        ConnectionStatus::Up => {
            IntegrationConnection::where_eq("key", key)
                .where_op("status", "!=", ConnectionStatus::Disabled)
                .update(
                    &state.db,
                    &[
                        ("last_success_at", &Some(now())),
                        ("updated_at", &Some(now())),
                    ],
                )
                .await?;
            Ok(false)
        }
        ConnectionStatus::Disabled => Ok(false),
    }
}

/// Hands `call` to its connection (see the module's docs).
///
/// # Errors
///
/// 400 when the connection is not registered; the database's errors. A
/// failed call is not an error: it is queued.
pub async fn submit(state: &AppState, call: Outbound) -> Result<Submitted> {
    let def = definition(&call.connection)?;
    let connection = ensure(&state.db, &call.connection).await?;
    let waiting = QueuedCall::any_waiting(&state.db, &call.connection).await?;
    if connection.status != ConnectionStatus::Up || waiting {
        return Ok(Submitted::Queued {
            id: enqueue(&state.db, &call, None).await?,
        });
    }
    match def.send(state, &call).await {
        Ok(Delivery::Sent) => {
            report_success(state, &call.connection).await?;
            Ok(Submitted::Sent)
        }
        Ok(Delivery::Obsolete) => Ok(Submitted::Obsolete),
        Err(error) => {
            let message = error.to_string();
            report_failure(state, &call.connection, &message).await?;
            Ok(Submitted::Queued {
                id: enqueue(&state.db, &call, Some(&message)).await?,
            })
        }
    }
}

async fn enqueue(db: &Db, call: &Outbound, error: Option<&str>) -> Result<i64> {
    let mut tx = db.begin().await?;
    let queued = queue::enqueue(&mut tx, call, error).await?;
    tx.commit().await?;
    Ok(queued.id)
}

/// Sends the queued calls of `key` in order, as far as they are due: for
/// each, the module is asked whether it is still due (an obsolete one is
/// closed, not sent), then the handler sends it. The first failure records
/// the error and the wait, runs `degrade` when the connection is `up`, and
/// stops: the calls behind it wait. The first success `recover`s a
/// `degraded` or `down` connection. Nothing is called while the connection
/// is `disabled`.
pub async fn run_queue(state: &AppState, key: &str) -> Result<Report> {
    let mut report = Report::default();
    let def = definition(key)?;
    loop {
        let connection = ensure(&state.db, key).await?;
        if connection.status == ConnectionStatus::Disabled {
            return Ok(report);
        }
        let Some(item) = QueuedCall::next_due(&state.db, key, now()).await? else {
            return Ok(report);
        };
        let call = item.outbound();
        if !def.is_due(state, &call).await? {
            queue::mark_obsolete(&state.db, &item).await?;
            report.obsolete += 1;
            continue;
        }
        match def.send(state, &call).await {
            Ok(Delivery::Sent) => {
                queue::mark_sent(&state.db, &item).await?;
                report_success(state, key).await?;
                report.sent += 1;
            }
            Ok(Delivery::Obsolete) => {
                queue::mark_obsolete(&state.db, &item).await?;
                report.obsolete += 1;
            }
            Err(error) => {
                let message = error.to_string();
                queue::mark_failed(&state.db, &item, &message).await?;
                report_failure(state, key, &message).await?;
                return Ok(report);
            }
        }
    }
}

/// Runs `go_down` on `connection` (`PLT.RUL.05`), once per failure run: the
/// occurrence key is the time of the first failure. The admin and the owner
/// role are told once, after the commit.
pub async fn go_down(state: &AppState, connection: &IntegrationConnection) -> Result<bool> {
    let Some(since) = connection.failed_since else {
        return Ok(false);
    };
    let spec = RuleRunSpec::new(
        RULE,
        Subject::new(SUBJECT_TYPE, connection.id),
        down_key(since),
    );
    let key = connection.key.clone();
    let outcome = run_rule(state, spec, move |state| async move {
        if let Some(down) = run_system(&state, &key, transitions::GO_DOWN, json!({})).await? {
            alerts::connection_down(&state, &down).await?;
        }
        Ok(())
    })
    .await?;
    Ok(outcome == RunOutcome::Succeeded)
}

/// The occurrence key of the `go_down` of the failure run that began at
/// `since`.
pub fn down_key(since: DateTime) -> String {
    format!("down:{}", since.timestamp())
}

/// The scheduled run: retries what is due, then runs `go_down` where N
/// minutes have passed.
pub async fn run_due(state: &AppState) -> Result<Report> {
    let mut report = Report::default();
    let connections = IntegrationConnection::query()
        .where_op("status", "!=", ConnectionStatus::Disabled)
        .order_by(IntegrationConnection::ID)
        .get(&state.db)
        .await?;
    for connection in &connections {
        if get(&connection.key).is_none()
            || !QueuedCall::any_waiting(&state.db, &connection.key).await?
        {
            continue;
        }
        let done = run_queue(state, &connection.key).await?;
        report.sent += done.sent;
        report.obsolete += done.obsolete;
    }
    let degraded = IntegrationConnection::query()
        .where_(IntegrationConnection::STATUS.eq(ConnectionStatus::Degraded))
        .order_by(IntegrationConnection::ID)
        .get(&state.db)
        .await?;
    for connection in degraded {
        if connection.goes_down_at().is_some_and(|at| at <= now())
            && go_down(state, &connection).await?
        {
            report.went_down += 1;
        }
    }
    Ok(report)
}

/// Runs the user's transition `name` (`disable` or `enable`) on connection
/// `key`, in its own transaction, then emits the engine's event and
/// [`IntegrationStatusChanged`].
async fn run_user(
    state: &AppState,
    user: &AuthUser,
    key: &str,
    name: &str,
    reason: Option<&str>,
) -> std::result::Result<IntegrationConnection, TransitionError> {
    let connection = ensure(&state.db, key).await?;
    let from = connection.status;
    let mut run = connection.transition(name).by(user);
    if let Some(reason) = reason {
        run = run.reason(reason.trim());
    }
    let outcome = run.run(state).await?;
    let after = IntegrationConnection::find(&state.db, connection.id)
        .await?
        .ok_or(Error::NotFound)?;
    announce(state, outcome, key, from, &after).await?;
    Ok(after)
}

/// An admin switches `key` off, with a `reason`. The queue is kept and
/// nothing is sent while it is `disabled`; modules use their manual path.
///
/// # Errors
///
/// `ReasonRequired` for a blank reason; `Forbidden` without
/// `integrations.manage`; `NotFrom` for a connection that is already
/// disabled.
pub async fn disable(
    state: &AppState,
    user: &AuthUser,
    key: &str,
    reason: &str,
) -> std::result::Result<IntegrationConnection, TransitionError> {
    run_user(state, user, key, transitions::DISABLE, Some(reason)).await
}

/// An admin switches `key` on again; it is `up`.
///
/// # Errors
///
/// `Forbidden` without `integrations.manage`; `NotFrom` for a connection
/// that is not disabled.
pub async fn enable(
    state: &AppState,
    user: &AuthUser,
    key: &str,
) -> std::result::Result<IntegrationConnection, TransitionError> {
    run_user(state, user, key, transitions::ENABLE, None).await
}

/// Registers the scheduled task [`TASK`] (every minute) and the command of
/// the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule().every_minute(TASK, |state: AppState| async move {
        run_due(&state).await.map(|_| ())
    });
    app.command(
        TASK,
        "Retry the queued calls that are due and take down the connections without a success \
         for N minutes",
        |_args, state: AppState| async move {
            let report = run_due(&state).await?;
            println!(
                "{} calls sent, {} obsolete, {} connections down.",
                report.sent, report.obsolete, report.went_down
            );
            Ok(())
        },
    );
}
