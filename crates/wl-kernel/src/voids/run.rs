//! The void run (wl-kernel §5.12, `PLT.ACL.09`).
//!
//! [`transitions`] gives a module the two transitions of its machine:
//! `void` (a user's, with a required reason) and `void_approved` (the
//! system's, run by the kernel once an approval is given). Both lead to
//! `voided`, carry the guard `not_given_to_third_party` and share one effect,
//! which in the transition's transaction, after the status update:
//!
//! 1. runs the reverse handlers of the entity in registration order
//!    ([`super::Voidable::reverse_in`]); an error from one fails the effect,
//!    so the status, the history row, the reversals and the Void record roll
//!    back together and the handler's message goes back to the user;
//! 2. writes the [`VoidRecord`]: the state before, the reason, who voided
//!    it, the approval request (from the history metadata) and what was
//!    reversed. The record keeps its number; the sequence is not touched.
//!
//! [`void`] runs the typed transition as a user and, after the commit, emits
//! `Transitioned` and [`RecordVoided`]. The helpers [`void_numbers`],
//! [`exclude_voided`], [`is_voided`] and [`ensure_editable`] serve the
//! modules' series views, totals, lists and update handlers.

use std::fmt::Display;

use renox::AuthUser;
use renox::db::{Executor, Query, now};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{StateMachine, Transition, TransitionCtx, TransitionError};

use super::record::{ReverseError, VoidRecord};
use super::{RecordVoided, VOID, VOID_APPROVED, VOIDED_STATE, get};
use crate::subject::Subject;
use crate::texts::en;

/// What [`void`] came to.
#[derive(Debug, Clone, PartialEq)]
pub enum VoidOutcome {
    /// The record is `voided`; the event was emitted.
    Voided(RecordVoided),
    /// An interceptor deferred the void (an approval request waits); the
    /// record did not change. Holds what the interceptor returned.
    Pending(Value),
}

/// The `void` and `void_approved` transitions of the voidable entity
/// `subject_type`, leaving `from_states`, for the module's machine. The
/// module declares the state `voided` itself (tone `neutral`, `.terminal()`).
///
/// `void` is for a user holding the entity's void permission in the record's
/// scope (checked by a guard, so `available()` leaves it out for anyone
/// else), and needs a reason. `void_approved` is `.system()`.
pub fn transitions<M, S>(
    subject_type: &str,
    from_states: impl IntoIterator<Item = S> + Clone,
) -> [Transition<M>; 2]
where
    M: StateMachine<Key = i64> + 'static,
    S: Display,
{
    [
        rules(
            Transition::new(VOID)
                .label("Void")
                .tone("danger")
                .reason_required(),
            subject_type,
            from_states.clone(),
        ),
        rules(
            Transition::new(VOID_APPROVED)
                .label("Void (approved)")
                .system(),
            subject_type,
            from_states,
        ),
    ]
}

/// What both transitions share: the states, the guards and the effect.
fn rules<M, S>(
    transition: Transition<M>,
    subject_type: &str,
    from_states: impl IntoIterator<Item = S>,
) -> Transition<M>
where
    M: StateMachine<Key = i64> + 'static,
    S: Display,
{
    let permission_type = subject_type.to_owned();
    let guard_type = subject_type.to_owned();
    let effect_type = subject_type.to_owned();
    transition
        .from(from_states)
        .to(VOIDED_STATE)
        .guard("permission_held", move |record: &M, ctx: &TransitionCtx| {
            let subject_type = permission_type.clone();
            Box::pin(async move { permission_held(&subject_type, record, ctx) })
        })
        .guard(
            "not_given_to_third_party",
            move |record: &M, _ctx: &TransitionCtx| {
                let subject_type = guard_type.clone();
                Box::pin(async move {
                    let voidable =
                        get(&subject_type).ok_or_else(|| en("wl_kernel.voids.not_voidable"))?;
                    if voidable.given_to_third_party_of(record) {
                        Err(en("wl_kernel.voids.given_to_third_party"))
                    } else {
                        Ok(())
                    }
                })
            },
        )
        .effect(move |record: &M, ctx: &TransitionCtx, tx| {
            let subject_type = effect_type.clone();
            Box::pin(async move { reverse_and_record(&subject_type, record, ctx, tx).await })
        })
}

/// A user must hold the entity's void permission in the record's scope; a
/// system run (no actor) passes.
fn permission_held<M: StateMachine<Key = i64> + 'static>(
    subject_type: &str,
    record: &M,
    ctx: &TransitionCtx,
) -> std::result::Result<(), String> {
    let voidable = get(subject_type).ok_or_else(|| en("wl_kernel.voids.not_voidable"))?;
    match ctx.actor() {
        Some(user)
            if !user.has_permission_in(voidable.permission(), &voidable.scope_of(record)) =>
        {
            Err(en("wl_kernel.voids.no_permission"))
        }
        _ => Ok(()),
    }
}

/// The record's status as the text the machine uses (the loaded one).
pub(crate) fn state_of<M: StateMachine>(record: &M) -> Option<String> {
    let column = M::COLUMN.name();
    let at = M::COLUMNS
        .iter()
        .filter(|c| **c != "id")
        .position(|c| *c == column)?;
    match record.values().into_iter().nth(at) {
        Some(renox::db::DbValue::Text(state)) => Some(state),
        _ => None,
    }
}

/// The history metadata's approval request, if any.
fn approval_of(meta: &Value) -> Option<i64> {
    meta.get("approval_request").and_then(Value::as_i64)
}

/// Who voided it: the user who ran it, or the requester of the approval.
fn voided_by(ctx: &TransitionCtx) -> Option<i64> {
    ctx.actor()
        .map(|user| user.id)
        .or_else(|| ctx.meta().get("requester_id").and_then(Value::as_i64))
}

/// How a refusal of a reverse handler goes back to the user: a 422 with the
/// handler's message unchanged (a text of the kernel's when it gave none).
fn refusal(error: &ReverseError) -> Error {
    let message = error.message().trim();
    let message = if !message.is_empty() {
        message.to_owned()
    } else if matches!(error, ReverseError::LockedPeriod(_)) {
        en("wl_kernel.voids.locked_period")
    } else {
        en("wl_kernel.voids.reverse_failed")
    };
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The shared effect: reverse, then write the Void record.
async fn reverse_and_record<M: StateMachine<Key = i64>>(
    subject_type: &str,
    record: &M,
    ctx: &TransitionCtx,
    tx: &mut renox::db::Transaction,
) -> Result {
    let voidable =
        get(subject_type).ok_or_else(|| Error::BadRequest(en("wl_kernel.voids.not_voidable")))?;
    let id = record.id();
    let internal =
        |what: &str| Error::Internal(renox::anyhow::anyhow!("{subject_type} {id}: {what}"));
    let state_before = state_of(record).ok_or_else(|| internal("it has no readable status"))?;
    let voided_by = voided_by(ctx).ok_or_else(|| internal("no one is voiding it"))?;
    let number = voidable.number_in(tx, id).await?;
    let effects = voidable.reverse_in(tx, id).await.map_err(|e| refusal(&e))?;
    let reason = ctx
        .reason()
        .or_else(|| ctx.input().get("reason").and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .to_owned();
    VoidRecord {
        subject_type: subject_type.to_owned(),
        subject_id: id,
        number,
        series_key: voidable.series_key().map(str::to_owned),
        state_before,
        reason,
        voided_by,
        approval_request_id: approval_of(ctx.meta()),
        effects: renox::db::Json(effects),
        at: now(),
        ..Default::default()
    }
    .insert(&mut *tx)
    .await?;
    // The status is already `voided` here: the document keeps the number
    // and shows the record as void (wl-kernel §5.8).
    crate::search::index_in(tx, &Subject::new(subject_type, id)).await?;
    Ok(())
}

/// `user` voids `subject` with `reason`, in its own transaction. After the
/// commit the engine's `Transitioned` and then [`RecordVoided`] are emitted.
///
/// # Errors
///
/// `Unknown` when the subject type is not voidable; `ReasonRequired` for a
/// blank reason; `NotFrom` in a state the module does not allow; `Guard`
/// (422) without the void permission, for a record given to a third party,
/// or when a reverse handler refuses (its message unchanged, also for a
/// locked period). Nothing is written then.
pub async fn void(
    state: &AppState,
    user: &AuthUser,
    subject: &Subject,
    reason: &str,
) -> std::result::Result<VoidOutcome, TransitionError> {
    let voidable =
        get(&subject.subject_type).ok_or_else(|| TransitionError::Unknown(VOID.into()))?;
    voidable
        .run_as(state, user, subject.subject_id, reason)
        .await
}

/// The numbers shown as void in the series `series_key`, oldest void first,
/// so the series view can mark them.
pub async fn void_numbers<'c>(db: impl Executor<'c>, series_key: &str) -> Result<Vec<String>> {
    let rows = VoidRecord::query()
        .where_(VoidRecord::SERIES_KEY.eq(series_key))
        .order_by(VoidRecord::ID)
        .get(db)
        .await?;
    Ok(rows.into_iter().filter_map(|row| row.number).collect())
}

/// `query` without voided records (status other than `voided`), for totals
/// and aggregates.
pub fn exclude_voided<M: StateMachine>(query: Query<M>) -> Query<M> {
    query.where_op(M::COLUMN.name(), "!=", VOIDED_STATE)
}

/// Whether `record` is voided: the list shows its row struck through (CSS
/// class `wl-voided`).
pub fn is_voided<M: StateMachine>(record: &M) -> bool {
    state_of(record).as_deref() == Some(VOIDED_STATE)
}

/// For a module's update handlers: a voided record is never changed (403).
pub fn ensure_editable<M: StateMachine>(record: &M) -> Result {
    if is_voided(record) {
        Err(Error::Status(
            StatusCode::FORBIDDEN,
            en("wl_kernel.voids.cannot_change"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_keeps_the_handlers_message() {
        let failed = refusal(&ReverseError::Failed("stock already issued onward".into()));
        assert!(matches!(&failed, Error::Status(s, m)
            if *s == StatusCode::UNPROCESSABLE_ENTITY && m == "stock already issued onward"));
        let locked = refusal(&ReverseError::LockedPeriod(String::new()));
        assert!(
            matches!(&locked, Error::Status(_, m) if m == &en("wl_kernel.voids.locked_period"))
        );
    }

    #[test]
    fn the_approval_comes_from_the_history_metadata() {
        assert_eq!(
            approval_of(&renox::serde_json::json!({ "approval_request": 9 })),
            Some(9)
        );
        assert_eq!(approval_of(&Value::Null), None);
    }
}
