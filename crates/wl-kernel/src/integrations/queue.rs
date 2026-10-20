//! The outbound queue of a connection (wl-kernel §5.14 step 2).
//!
//! Outbound work is never lost: a call that fails, or that arrives while the
//! connection is not `up`, is stored as a [`QueuedCall`]. Per connection the
//! calls go out first in, first out (by id); only the oldest queued call is
//! retried, so none overtakes another. A call leaves the queue only when it
//! is sent or its module reports it obsolete; there is no attempt limit, the
//! limit is time (`PLT.RUL.05`).

use renox::db::{Executor, Json, Transaction, now};
use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use super::def::{Outbound, retry_wait};
use crate::subject::Subject;

/// Where a queued call stands.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CallStatus {
    /// Waiting to be sent.
    #[default]
    Queued,
    /// The external system took it.
    Sent,
    /// Its module reported it obsolete (the record changed while it was
    /// queued): it was not sent.
    Obsolete,
}

/// One outbound call waiting for its connection (`wl_integration_queue`).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_integration_queue", index(connection_key, status, id))]
pub struct QueuedCall {
    /// The primary key; the order calls go out in.
    pub id: i64,
    /// The connection's key.
    pub connection_key: String,
    /// What the call is, in the module's words.
    pub kind: String,
    /// What the module needs to send it.
    pub payload: Json<Value>,
    /// The module's natural key, if any.
    pub natural_key: Option<String>,
    /// The type of the record the call is about, if any.
    pub subject_type: Option<String>,
    /// The id of the record the call is about.
    pub subject_id: Option<i64>,
    /// Queued, sent or obsolete.
    pub status: CallStatus,
    /// How many attempts failed.
    pub attempts: i64,
    /// When it may be tried again; `None` when it was not tried yet.
    pub next_attempt_at: Option<DateTime>,
    /// The error of the last failed attempt.
    pub last_error: Option<String>,
    /// When it was queued.
    pub enqueued_at: DateTime,
    /// When it was sent or reported obsolete.
    pub finished_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl QueuedCall {
    /// The record the call is about.
    pub fn subject(&self) -> Option<Subject> {
        match (&self.subject_type, self.subject_id) {
            (Some(subject_type), Some(id)) => Some(Subject::new(subject_type, id)),
            _ => None,
        }
    }

    /// The call as the module's handlers see it.
    pub fn outbound(&self) -> Outbound {
        Outbound {
            id: self.id,
            connection: self.connection_key.clone(),
            kind: self.kind.clone(),
            payload: self.payload.0.clone(),
            natural_key: self.natural_key.clone(),
            subject: self.subject(),
            attempts: self.attempts,
        }
    }

    /// The queued calls of `connection`, oldest first.
    pub async fn waiting<'c>(db: impl Executor<'c>, connection: &str) -> Result<Vec<QueuedCall>> {
        QueuedCall::query()
            .where_(QueuedCall::CONNECTION_KEY.eq(connection))
            .where_(QueuedCall::STATUS.eq(CallStatus::Queued))
            .order_by(QueuedCall::ID)
            .get(db)
            .await
    }

    /// The oldest queued call of `connection` that may be tried now: `None`
    /// when nothing is queued, or when the oldest waits for its backoff
    /// (the calls behind it wait too, so no call overtakes another).
    pub async fn next_due<'c>(
        db: impl Executor<'c>,
        connection: &str,
        at: DateTime,
    ) -> Result<Option<QueuedCall>> {
        let oldest = QueuedCall::query()
            .where_(QueuedCall::CONNECTION_KEY.eq(connection))
            .where_(QueuedCall::STATUS.eq(CallStatus::Queued))
            .order_by(QueuedCall::ID)
            .first(db)
            .await?;
        Ok(oldest.filter(|call| call.next_attempt_at.is_none_or(|next| next <= at)))
    }

    /// The oldest queued call of `connection`, whatever its wait: what a
    /// drain after a recovery or an `enable` sends first.
    pub async fn oldest<'c>(db: impl Executor<'c>, connection: &str) -> Result<Option<QueuedCall>> {
        QueuedCall::query()
            .where_(QueuedCall::CONNECTION_KEY.eq(connection))
            .where_(QueuedCall::STATUS.eq(CallStatus::Queued))
            .order_by(QueuedCall::ID)
            .first(db)
            .await
    }

    /// The call of `connection` with `natural_key` that is queued or was
    /// sent, if any: the module's natural key prevents a second one.
    pub async fn with_natural_key<'c>(
        db: impl Executor<'c>,
        connection: &str,
        natural_key: &str,
    ) -> Result<Option<QueuedCall>> {
        QueuedCall::query()
            .where_(QueuedCall::CONNECTION_KEY.eq(connection))
            .where_(QueuedCall::NATURAL_KEY.eq(natural_key))
            .where_(QueuedCall::STATUS.is_in([CallStatus::Queued, CallStatus::Sent]))
            .order_by(QueuedCall::ID)
            .first(db)
            .await
    }

    /// Whether `connection` has a queued call.
    pub async fn any_waiting<'c>(db: impl Executor<'c>, connection: &str) -> Result<bool> {
        QueuedCall::query()
            .where_(QueuedCall::CONNECTION_KEY.eq(connection))
            .where_(QueuedCall::STATUS.eq(CallStatus::Queued))
            .exists(db)
            .await
    }
}

/// Queues `call` at the end of its connection's queue. A call that was
/// tried and failed (`error`) has one failed attempt and waits
/// [`retry_wait`] for the next; one that was never tried (the connection is
/// not `up`) waits for the scheduled run.
pub async fn enqueue(
    tx: &mut Transaction,
    call: &Outbound,
    error: Option<&str>,
) -> Result<QueuedCall> {
    let at = now();
    let failed = i64::from(error.is_some());
    QueuedCall::create(
        &mut *tx,
        QueuedCall {
            connection_key: call.connection.clone(),
            kind: call.kind.clone(),
            payload: Json(call.payload.clone()),
            natural_key: call.natural_key.clone(),
            subject_type: call.subject.as_ref().map(|s| s.subject_type.clone()),
            subject_id: call.subject.as_ref().map(|s| s.subject_id),
            status: CallStatus::Queued,
            attempts: failed,
            next_attempt_at: error.map(|_| at + retry_wait(failed)),
            last_error: error.map(str::to_owned),
            enqueued_at: at,
            ..Default::default()
        },
    )
    .await
}

/// Marks `call` sent.
pub async fn mark_sent(db: &Db, call: &QueuedCall) -> Result {
    finish(db, call, CallStatus::Sent).await
}

/// Marks `call` obsolete: not sent, listed on the reconciliation task.
pub async fn mark_obsolete(db: &Db, call: &QueuedCall) -> Result {
    finish(db, call, CallStatus::Obsolete).await
}

async fn finish(db: &Db, call: &QueuedCall, status: CallStatus) -> Result {
    let at = now();
    QueuedCall::where_eq("id", call.id)
        .where_eq("status", CallStatus::Queued)
        .update(
            db,
            &[
                ("status", &status),
                ("finished_at", &Some(at)),
                ("next_attempt_at", &None::<DateTime>),
                ("updated_at", &Some(at)),
            ],
        )
        .await?;
    Ok(())
}

/// Records a failed attempt of `call`: one more failed attempt, the error,
/// and the wait before the next ([`retry_wait`]).
pub async fn mark_failed(db: &Db, call: &QueuedCall, error: &str) -> Result {
    let at = now();
    let failed = call.attempts + 1;
    QueuedCall::where_eq("id", call.id)
        .where_eq("status", CallStatus::Queued)
        .update(
            db,
            &[
                ("attempts", &failed),
                ("last_error", &Some(error.to_owned())),
                ("next_attempt_at", &Some(at + retry_wait(failed))),
                ("updated_at", &Some(at)),
            ],
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_queued_call_hands_its_module_the_call() {
        let queued = QueuedCall {
            id: 7,
            connection_key: "gateway.acme".into(),
            kind: "refund".into(),
            payload: Json(renox::serde_json::json!({ "amount": 25_000 })),
            natural_key: Some("TX-889".into()),
            subject_type: Some("payments.refund".into()),
            subject_id: Some(3),
            attempts: 2,
            ..Default::default()
        };
        let call = queued.outbound();
        assert_eq!(call.id, 7);
        assert_eq!(call.connection, "gateway.acme");
        assert_eq!(call.kind, "refund");
        assert_eq!(call.payload["amount"], 25_000);
        assert_eq!(call.natural_key.as_deref(), Some("TX-889"));
        assert_eq!(call.subject, Some(Subject::new("payments.refund", 3)));
        assert_eq!(call.attempts, 2);
        assert_eq!(QueuedCall::default().subject(), None);
    }
}
