//! Entering a record after the fact (wl-kernel §5.13, `PLT.ACL.10`).
//!
//! A module's save handler for an opted-in entity ([`super::LateEntryDef`])
//! gives the kernel the business time and the form instead of writing the
//! record itself:
//!
//! ```ignore
//! let spec = EntrySpec::new("bookings.check_in", user.id, checked_in_at)
//!     .parent(Subject::new("bookings.booking", booking.id))
//!     .scope(Scope::of_id::<Building>(booking.building_id))
//!     .reason(&form.late_reason)
//!     .outage_reference(&form.outage_reference)
//!     .payload(json!({ "room": form.room }));
//! match late_entries::enter(&state, spec).await? {
//!     Entered::OnTime { record_id } | Entered::Recorded { record_id, .. } => { /* saved */ }
//!     Entered::Pending { .. } => { /* "sent for approval" */ }
//! }
//! ```
//!
//! [`enter`] decides with [`super::assess`], against the limits of the entity
//! and the period the module reports:
//!
//! - **on time**: the module's `create` handler writes the record; no late
//!   entry is stored;
//! - **flagged** (more than M minutes, up to N hours): a reason is required;
//!   the late entry is stored, the handler writes the record in the same
//!   transaction and `LateEntryRecorded` is emitted after the commit;
//! - **beyond N hours, or in a closed period the module allows to reopen**: a
//!   reason is required; the late entry is stored and an approval request
//!   for a holder of `late_entries.approve` is created. The record is
//!   created only on approval; on reject, withdraw or expiry nothing is
//!   created;
//! - **in a period the module forbids to reopen**: refused (422) with the
//!   module's message; nothing is stored.

use renox::auth::permissions::Scope;
use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use renox::serde_json::Value;
use renox::state_machine::{Outcome, StateMachine, TransitionError};

use super::SUBJECT_TYPE;
use super::assess::{Assessment, assess};
use super::entry::{LateEntry, LateEntryStatus, transitions};
use super::events::LateEntryRecorded;
use super::limits::limits_for;
use super::register::{self, NewRecord};
use crate::approvals::{self, ApprovalRequest};
use crate::subject::Subject;
use crate::texts;

/// A record to enter, with its business time. Built with
/// [`EntrySpec::new`] and the builder methods.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct EntrySpec {
    /// The entity, which must be registered with
    /// [`LateEntryDef`](super::LateEntryDef).
    pub subject_type: String,
    /// The user who enters it.
    pub user_id: i64,
    /// The business time: when the work really happened.
    pub occurred_at: DateTime,
    /// The existing record whose business time is being changed; `None` for
    /// a new record.
    pub existing: Option<i64>,
    /// The record the entry belongs to (`booking` on a check-in).
    pub parent: Option<Subject>,
    /// The scope of the record, where approvers are looked for.
    pub scope: Scope,
    /// Why it is late.
    pub reason: Option<String>,
    /// A link to an outage: an integration `down` period, or a note such as
    /// "outage at branch X from-to".
    pub outage_reference: Option<String>,
    /// The form, handed to the module's `create` handler.
    pub payload: Value,
}

impl EntrySpec {
    /// A new `subject_type` record entered by `user_id` whose work happened
    /// at `occurred_at`.
    pub fn new(subject_type: &str, user_id: i64, occurred_at: DateTime) -> Self {
        EntrySpec {
            subject_type: subject_type.to_owned(),
            user_id,
            occurred_at,
            existing: None,
            parent: None,
            scope: Scope::global(),
            reason: None,
            outage_reference: None,
            payload: Value::Null,
        }
    }

    /// The business time of existing record `id` is being changed; the
    /// change is checked the same way, against the time of the edit.
    pub fn existing(mut self, id: i64) -> Self {
        self.existing = Some(id);
        self
    }

    /// The record the entry belongs to.
    pub fn parent(mut self, parent: Subject) -> Self {
        self.parent = Some(parent);
        self
    }

    /// The scope of the record.
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    /// Why it is late (blank counts as none).
    pub fn reason(mut self, reason: &str) -> Self {
        let reason = reason.trim();
        self.reason = (!reason.is_empty()).then(|| reason.to_owned());
        self
    }

    /// The outage reference (blank counts as none).
    pub fn outage_reference(mut self, reference: &str) -> Self {
        let reference = reference.trim();
        self.outage_reference = (!reference.is_empty()).then(|| reference.to_owned());
        self
    }

    /// The form for the module's `create` handler.
    pub fn payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }
}

/// What [`enter`] came to.
#[derive(Debug, Clone)]
pub enum Entered {
    /// Not late: the record was written, no late entry stored.
    OnTime {
        /// The record's id, as the module's handler returned it.
        record_id: i64,
    },
    /// Flagged and recorded; `LateEntryRecorded` was emitted.
    Recorded {
        /// The record's id.
        record_id: i64,
        /// The late entry.
        entry: Box<LateEntry>,
    },
    /// The entry waits for approval; no record exists yet.
    Pending {
        /// The late entry, `awaiting_approval`.
        entry: Box<LateEntry>,
        /// The approval request.
        approval_request_id: i64,
    },
}

impl Entered {
    /// The record's id, once it exists.
    pub fn record_id(&self) -> Option<i64> {
        match self {
            Entered::OnTime { record_id } | Entered::Recorded { record_id, .. } => Some(*record_id),
            Entered::Pending { .. } => None,
        }
    }

    /// Whether the entry waits for approval.
    pub fn is_pending(&self) -> bool {
        matches!(self, Entered::Pending { .. })
    }
}

/// 422 with the kernel's text `key`.
fn refused(key: &str) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, texts::en(key))
}

/// What the check says about a business time right now, for the entry form
/// (whether to ask for a reason): the module's period, the entity's limits
/// and the current time.
pub async fn preview(
    state: &AppState,
    subject_type: &str,
    occurred_at: DateTime,
) -> Result<Assessment> {
    let def = register::require(subject_type)?;
    let limits = limits_for(&state.db, subject_type).await?;
    let mut tx = state.db.begin().await?;
    let period = def.period_of(&mut tx, occurred_at).await?;
    drop(tx);
    Ok(assess(limits, occurred_at, now(), &period))
}

/// Enters the record of `spec`, as described in the module's docs.
///
/// # Errors
///
/// 400 when the entity does not opt in; 422 when the entry is late and has
/// no reason, when the period is locked (the module's message), when no
/// approval policy is active for an entry that needs approval, or when the
/// module's handler refuses. Nothing is stored then.
pub async fn enter(state: &AppState, spec: EntrySpec) -> Result<Entered> {
    let def = register::require(&spec.subject_type)?;
    let limits = limits_for(&state.db, &spec.subject_type).await?;
    let entered_at = now();
    let mut tx = state.db.begin().await?;
    let period = def.period_of(&mut tx, spec.occurred_at).await?;
    let found = assess(limits, spec.occurred_at, entered_at, &period);
    if let Assessment::Refused(message) = &found {
        return Err(Error::Status(
            StatusCode::UNPROCESSABLE_ENTITY,
            message.clone(),
        ));
    }
    if !found.is_late() {
        let new = NewRecord {
            existing: spec.existing,
            payload: spec.payload.clone(),
            occurred_at: spec.occurred_at,
            entered_at,
            late: false,
            reason: None,
            outage_reference: None,
            approval_request_id: None,
            late_entry_id: None,
            user_id: spec.user_id,
            parent: spec.parent.clone(),
        };
        let record_id = def.create_in(&mut tx, &new).await?;
        tx.commit().await?;
        return Ok(Entered::OnTime { record_id });
    }
    let Some(reason) = spec.reason.clone() else {
        return Err(refused("wl_kernel.late_entries.reason_required"));
    };
    let entry = LateEntry::create(
        &mut tx,
        LateEntry {
            subject_type: spec.subject_type.clone(),
            existing_id: spec.existing,
            parent_type: spec.parent.as_ref().map(|p| p.subject_type.clone()),
            parent_id: spec.parent.as_ref().map(|p| p.subject_id),
            scope_kind: spec.scope.kind().to_owned(),
            scope_id: spec.scope.id().to_owned(),
            occurred_at: spec.occurred_at,
            entered_at,
            minutes_late: found.minutes_late(),
            needs_approval: found.needs_approval(),
            reason,
            outage_reference: spec.outage_reference.clone(),
            entered_by: spec.user_id,
            payload: Json(spec.payload.clone()),
            status: LateEntryStatus::Requested,
            ..Default::default()
        },
    )
    .await?;

    if found.needs_approval() {
        let outcome = entry
            .transition(transitions::REQUEST_APPROVAL)
            .by_system()
            .run_in(&mut tx)
            .await
            .map_err(TransitionError::into_error)?;
        tx.commit().await?;
        // The request the effect created is announced only now.
        approvals::announce(state, &outcome).await?;
        if let Outcome::Done(done) = outcome {
            done.emit(state).await?;
        }
        let entry = reload(state, entry.id).await?;
        let approval_request_id = entry.approval_request_id.unwrap_or_default();
        return Ok(Entered::Pending {
            entry: Box::new(entry),
            approval_request_id,
        });
    }

    let outcome = entry
        .transition(transitions::RECORD)
        .by_system()
        .run_in(&mut tx)
        .await
        .map_err(TransitionError::into_error)?;
    tx.commit().await?;
    if let Outcome::Done(done) = outcome {
        done.emit(state).await?;
    }
    let entry = reload(state, entry.id).await?;
    state.emit(LateEntryRecorded::of(&entry)).await?;
    let record_id = entry.record_id.unwrap_or_default();
    Ok(Entered::Recorded {
        record_id,
        entry: Box::new(entry),
    })
}

async fn reload(state: &AppState, id: i64) -> Result<LateEntry> {
    LateEntry::find(&state.db, id).await?.ok_or(Error::NotFound)
}

impl LateEntryRecorded {
    /// The event of a stored late entry whose record exists.
    pub fn of(entry: &LateEntry) -> Self {
        LateEntryRecorded {
            subject: entry.record().unwrap_or_else(|| entry.subject()),
            late_entry: entry.id,
            occurred_at: entry.occurred_at,
            entered_at: entry.entered_at,
            reason: entry.reason.clone(),
            outage_reference: entry.outage_reference.clone(),
            user_id: entry.entered_by,
            approval_request_id: entry.approval_request_id,
            fired_runs: entry.fired_runs.0.clone(),
        }
    }
}

/// After the commit of a final approval: emits [`LateEntryRecorded`] when
/// the request was about a late entry whose record was created. Anything
/// else announces nothing.
pub async fn announce_approved(state: &AppState, request: &ApprovalRequest) -> Result {
    if request.subject_type != SUBJECT_TYPE {
        return Ok(());
    }
    if let Some(entry) = LateEntry::find(&state.db, request.subject_id).await?
        && entry.status == LateEntryStatus::Recorded
    {
        state.emit(LateEntryRecorded::of(&entry)).await?;
    }
    Ok(())
}
