//! The late entry record and its machine (wl-kernel §5.13).
//!
//! A late entry is the trace of a record entered after the fact, and, while
//! it needs an approval, the subject of that approval (`kernel.late_entry`):
//! the record itself is created only once the approval is given, so a
//! rejected late entry creates nothing.
//!
//! ```text
//! [*] --> requested
//! requested --record--> recorded                         (flag only)
//! requested --request_approval--> awaiting_approval      (beyond N hours, or a closed period)
//! awaiting_approval --request_approval_approved--> recorded
//! awaiting_approval --back_to_requested--> discarded     (rejected, withdrawn, expired, failed)
//! ```
//!
//! All transitions are `system`: the kernel's [`enter`](super::enter) runs
//! them for the user who entered the record. `record` and
//! `request_approval_approved` share one effect: they call the module's
//! `create` handler, store the record's id and the rule runs to review.

use renox::db::{Json, Transaction, now};
use renox::prelude::*;
use renox::serde_json::json;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};

use super::events::FiredRun;
use super::register::{self, NewRecord};
use super::{ACTION, SUBJECT_TYPE, fired};
use crate::approvals::{self, Require, Required};
use crate::subject::Subject;
use crate::texts;

/// The transition names of a late entry.
pub mod transitions {
    /// A flagged entry within N hours: the record is created at once.
    pub const RECORD: &str = "record";
    /// An entry that needs approval: the request is created.
    pub const REQUEST_APPROVAL: &str = "request_approval";
    /// The approval is given: the record is created.
    pub const REQUEST_APPROVAL_APPROVED: &str = "request_approval_approved";
    /// The request ended without a yes: nothing was created.
    pub const BACK_TO_REQUESTED: &str = "back_to_requested";
}

/// The state of a late entry.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LateEntryStatus {
    /// Checked and flagged; the record is not created yet.
    #[default]
    Requested,
    /// Waiting for a holder of `late_entries.approve`.
    AwaitingApproval,
    /// The record exists.
    Recorded,
    /// The approval ended without a yes: no record was created.
    Discarded,
}

/// One record entered after the fact.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_late_entries",
    state_machine,
    index(subject_type, record_id),
    index(entered_at)
)]
pub struct LateEntry {
    /// The primary key.
    pub id: i64,
    /// The entity that opted in, e.g. `bookings.check_in`.
    pub subject_type: String,
    /// The record the entry created (or changed); `None` until it exists.
    pub record_id: Option<i64>,
    /// The existing record whose business time was edited, if this is an
    /// edit.
    pub existing_id: Option<i64>,
    /// The type of the parent subject (`bookings.booking`), if any.
    pub parent_type: Option<String>,
    /// The parent subject's id.
    pub parent_id: Option<i64>,
    /// The kind of the scope of the record (`""` when global).
    pub scope_kind: String,
    /// The id of the scope of the record (`""` when global).
    pub scope_id: String,
    /// The business time: when the work really happened.
    pub occurred_at: DateTime,
    /// The entry time: when it was typed in.
    pub entered_at: DateTime,
    /// Whole minutes between the two.
    pub minutes_late: i64,
    /// Whether the entry needed approval.
    pub needs_approval: bool,
    /// Why it is late.
    pub reason: String,
    /// The outage reference, if given.
    pub outage_reference: Option<String>,
    /// The user who entered it.
    pub entered_by: i64,
    /// The approval request, when the entry needed approval.
    pub approval_request_id: Option<i64>,
    /// What the module asked to create (its form).
    pub payload: Json<renox::serde_json::Value>,
    /// The rule runs already fired, to review.
    pub fired_runs: Json<Vec<FiredRun>>,
    /// The state; changed only by a transition.
    pub status: LateEntryStatus,
    /// When the record was created.
    pub recorded_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl LateEntry {
    /// The late entry as a [`Subject`]: `kernel.late_entry:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(SUBJECT_TYPE, self.id)
    }

    /// The record it created, once it exists.
    pub fn record(&self) -> Option<Subject> {
        self.record_id
            .map(|id| Subject::new(&self.subject_type, id))
    }

    /// The parent subject the module named.
    pub fn parent(&self) -> Option<Subject> {
        match (&self.parent_type, self.parent_id) {
            (Some(subject_type), Some(id)) => Some(Subject::new(subject_type, id)),
            _ => None,
        }
    }

    /// The scope of the record, where approvers are looked for.
    pub fn scope(&self) -> renox::auth::permissions::Scope {
        renox::auth::permissions::Scope::new(&self.scope_kind, &self.scope_id)
    }

    /// The late entry of record `record_id` of `subject_type`, if any.
    pub async fn of_record(
        db: &Db,
        subject_type: &str,
        record_id: i64,
    ) -> Result<Option<LateEntry>> {
        LateEntry::query()
            .where_(LateEntry::SUBJECT_TYPE.eq(subject_type))
            .where_(LateEntry::RECORD_ID.eq(record_id))
            .order_by_desc(LateEntry::ID)
            .first(db)
            .await
    }
}

/// Sets columns of late entry `id` inside a transition's transaction.
async fn set(
    tx: &mut Transaction,
    id: i64,
    columns: &[(&str, &(dyn renox::db::ToDbValue + Sync))],
) -> Result {
    let mut all: Vec<(&str, &(dyn renox::db::ToDbValue + Sync))> = columns.to_vec();
    let at = now();
    all.push(("updated_at", &at));
    LateEntry::query()
        .where_(LateEntry::ID.eq(id))
        .update(&mut *tx, &all)
        .await?;
    Ok(())
}

/// The shared effect of `record` and `request_approval_approved`: the
/// module's handler writes the record, then the late entry stores its id and
/// the rule runs already fired between the business time and the entry time.
async fn write_record(entry: &LateEntry, tx: &mut Transaction) -> Result {
    let def = register::require(&entry.subject_type)?;
    let new = NewRecord {
        existing: entry.existing_id,
        payload: entry.payload.0.clone(),
        occurred_at: entry.occurred_at,
        entered_at: entry.entered_at,
        late: true,
        reason: Some(entry.reason.clone()),
        outage_reference: entry.outage_reference.clone(),
        approval_request_id: entry.approval_request_id,
        late_entry_id: Some(entry.id),
        user_id: entry.entered_by,
        parent: entry.parent(),
    };
    let record_id = def.create_in(tx, &new).await?;
    let runs = fired::find(
        tx,
        &entry.subject_type,
        Some(record_id),
        entry.parent().as_ref(),
        entry.occurred_at,
        entry.entered_at,
    )
    .await?;
    set(
        tx,
        entry.id,
        &[
            ("record_id", &Some(record_id)),
            ("fired_runs", &Json(runs)),
            ("recorded_at", &Some(now())),
        ],
    )
    .await
}

/// The effect of `request_approval`: the approval request, in the same
/// transaction. Without an active policy the entry would be accepted
/// unapproved, so the transaction rolls back instead.
async fn request_approval(entry: &LateEntry, tx: &mut Transaction) -> Result {
    let require = Require::new(
        ACTION,
        entry.subject(),
        transitions::REQUEST_APPROVAL,
        entry.entered_by,
        "requested",
    )
    .payload(json!({
        "subject_type": entry.subject_type,
        "occurred_at": entry.occurred_at,
        "entered_at": entry.entered_at,
        "minutes_late": entry.minutes_late,
        "reason": entry.reason,
    }));
    match approvals::require_in(&mut *tx, &require).await? {
        Required::Pending(request) => {
            set(
                tx,
                entry.id,
                &[("approval_request_id", &Some(request.id))],
            )
            .await
        }
        Required::Allowed => Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            texts::en("wl_kernel.late_entries.no_approval_policy"),
        )),
    }
}

impl StateMachine for LateEntry {
    type State = LateEntryStatus;
    const COLUMN: renox::db::Col<LateEntry, LateEntryStatus> = LateEntry::STATUS;

    fn machine() -> Machine<Self> {
        use LateEntryStatus::{AwaitingApproval, Discarded, Recorded, Requested};
        Machine::new()
            .initial([Requested])
            .state(Requested)
            .tone("neutral")
            .state(AwaitingApproval)
            .tone("warning")
            .state(Recorded)
            .tone("success")
            .terminal()
            .state(Discarded)
            .tone("neutral")
            .terminal()
            .transition(
                Transition::new(transitions::RECORD)
                    .label("Record")
                    .from([Requested])
                    .to(Recorded)
                    .system()
                    .effect(|entry: &LateEntry, _ctx, tx| {
                        let entry = entry.clone();
                        Box::pin(async move { write_record(&entry, tx).await })
                    }),
            )
            .transition(
                Transition::new(transitions::REQUEST_APPROVAL)
                    .label("Request approval")
                    .from([Requested])
                    .to(AwaitingApproval)
                    .system()
                    .effect(|entry: &LateEntry, _ctx, tx| {
                        let entry = entry.clone();
                        Box::pin(async move { request_approval(&entry, tx).await })
                    }),
            )
            .transition(
                Transition::new(transitions::REQUEST_APPROVAL_APPROVED)
                    .label("Record (approved)")
                    .from([AwaitingApproval])
                    .to(Recorded)
                    .system()
                    .effect(|entry: &LateEntry, _ctx, tx| {
                        let entry = entry.clone();
                        Box::pin(async move { write_record(&entry, tx).await })
                    }),
            )
            .transition(
                Transition::new(transitions::BACK_TO_REQUESTED)
                    .label("Discard")
                    .from([AwaitingApproval])
                    .to(Discarded)
                    .system(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_has_every_transition() {
        let machine = LateEntry::machine();
        for (name, from, to) in [
            ("record", &["requested"][..], "recorded"),
            ("request_approval", &["requested"], "awaiting_approval"),
            (
                "request_approval_approved",
                &["awaiting_approval"],
                "recorded",
            ),
            ("back_to_requested", &["awaiting_approval"], "discarded"),
        ] {
            let transition = machine.find(name).expect(name);
            assert_eq!(transition.from_states(), from, "{name}");
            assert_eq!(transition.to_state(), Some(to), "{name}");
            assert!(transition.is_system(), "{name}");
        }
        assert_eq!(machine.transitions().len(), 4);
    }

    #[test]
    fn an_entry_names_its_record_parent_and_scope() {
        let entry = LateEntry {
            id: 3,
            subject_type: "bookings.check_in".into(),
            record_id: Some(8),
            parent_type: Some("bookings.booking".into()),
            parent_id: Some(2),
            scope_kind: "scope_entities".into(),
            scope_id: "5".into(),
            ..Default::default()
        };
        assert_eq!(entry.subject(), Subject::new("kernel.late_entry", 3));
        assert_eq!(
            entry.record(),
            Some(Subject::new("bookings.check_in", 8))
        );
        assert_eq!(
            entry.parent(),
            Some(Subject::new("bookings.booking", 2))
        );
        assert_eq!(
            entry.scope(),
            renox::auth::permissions::Scope::new("scope_entities", "5")
        );
        let none = LateEntry::default();
        assert!(none.record().is_none() && none.parent().is_none());
        assert!(none.scope().is_global());
    }
}
