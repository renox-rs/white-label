//! Approval when money or stock was posted (wl-kernel §5.12, `PLT.ACL.09`).
//!
//! There is no second mechanism: the void goes through the approval engine
//! like any other action.
//!
//! * The kernel's interceptor, for a user's `void` on a voidable type, first
//!   asks the entity's `pending` probes how many posted effects the record
//!   has ([`posted`]) and writes the count into the run's input under
//!   [`POSTED_KEY`], overwriting whatever the caller sent, before the policy
//!   is matched ([`with_posted`]).
//! * Each voidable type has a default policy ([`register_default_policy`],
//!   called when the entity is registered): the condition `amount_above(0)`
//!   on `posted`, one step decided by any holder of the void permission in
//!   the record's scope (`Step.permission`), SoD on, the default expiry. With
//!   nothing posted no policy matches and the void runs at once.
//! * On final approval the engine runs `void_approved` as the system; the
//!   Void record takes `voided_by` and the request from its history
//!   metadata, and [`announce_approved`] emits [`RecordVoided`] after the
//!   commit.

use renox::db::Transaction;
use renox::prelude::*;
use renox::serde_json::{Map, Value};

use super::{POSTED_KEY, RecordVoided, VOID, VoidRecord, Voidable, get};
use crate::approvals::{ApprovalRequest, PolicyDefault};
use crate::machines;
use crate::subject::Subject;

/// The approval policies' action key of the `void` of `subject_type`: the
/// machine's (see [`crate::SubjectMachine::action_key`]) once registered,
/// else `<module>.void`.
pub fn action_key(subject_type: &str) -> String {
    match machines::get(subject_type) {
        Some(machine) => machine.action_key(VOID),
        None => machines::default_action_key(subject_type, VOID),
    }
}

/// The default policy of a voidable type.
pub fn default_policy(voidable: &Voidable) -> PolicyDefault {
    PolicyDefault::new(action_key(voidable.subject_type()))
        .amount_above(0)
        .input_key(POSTED_KEY)
        .step_permission(voidable.permission())
}

/// Adds the default policy of `voidable` to the list `wl:install` stores.
/// Declare the type's `SubjectMachine` (with a custom `.action("void", ..)`,
/// if any) before the `Voidable`, so the policy takes its action key.
pub(crate) fn register_default_policy(voidable: &Voidable) {
    default_policy(voidable).register();
}

/// How many effects of record `id` are money or stock, by the entity's
/// `pending` probes; `None` when `subject_type` is not voidable.
pub async fn posted(tx: &mut Transaction, subject_type: &str, id: i64) -> Result<Option<i64>> {
    match get(subject_type) {
        Some(voidable) => voidable.posted_in(tx, id).await.map(Some),
        None => Ok(None),
    }
}

/// `input` with the posted count of the run written under [`POSTED_KEY`]
/// and the void `reason` under `reason` (so the request's payload snapshot
/// carries it to `void_approved`), when `transition` is the `void` of a
/// voidable type; `input` as it is for any other run. A caller's own
/// `posted` is never trusted.
pub async fn with_posted(
    tx: &mut Transaction,
    subject: &Subject,
    transition: &str,
    reason: Option<&str>,
    input: &Value,
) -> Result<Value> {
    if transition != VOID {
        return Ok(input.clone());
    }
    let Some(count) = posted(tx, &subject.subject_type, subject.subject_id).await? else {
        return Ok(input.clone());
    };
    let mut fields = match input {
        Value::Object(fields) => fields.clone(),
        _ => Map::new(),
    };
    fields.insert(POSTED_KEY.into(), Value::from(count));
    if let Some(reason) = reason {
        fields.insert("reason".into(), Value::from(reason));
    }
    Ok(Value::Object(fields))
}

/// After the commit of a final approval: emits [`RecordVoided`] when the
/// request was about a `void` that ran (`void_approved`), from the stored
/// Void record. Anything else announces nothing.
pub async fn announce_approved(state: &AppState, request: &ApprovalRequest) -> Result {
    if request.transition != VOID || get(&request.subject_type).is_none() {
        return Ok(());
    }
    if let Some(void) = VoidRecord::of_subject(&state.db, &request.subject()).await? {
        state.emit(RecordVoided::of(&void)).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_action_key_defaults_to_the_modules_void() {
        assert_eq!(action_key("nomachine.thing"), "nomachine.void");
    }
}
