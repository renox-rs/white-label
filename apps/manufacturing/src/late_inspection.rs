//! The decision on a returned lot typed in later (07 Manufacturing §7.1
//! *Late entry*, `PLT.ACL.10`, BL-MFG-14, BL-MFG-07). The QC inspector who
//! looked at the goods in the afternoon may record *pass* or *fail* the next
//! morning. The inspection keeps the time of the decision
//! ([`ReturnInspection::decided_at`]); the kernel flags the entry with a
//! reason when it is more than M minutes late and asks the holder of
//! `late_entries.approve` (the production manager) when it is more than N
//! hours late. M and N are the entity's limits ([`SUBJECT_TYPE`]), seeded by
//! the host.
//!
//! The entity is [`LateInspection`]: what the inspector decided. The result
//! (the lot released, or the NCR raised) is applied from it once the entry is
//! recorded or approved; a rejected entry applies nothing, and while it waits
//! the inspection stays open and takes no other decision.

use renox::auth::permissions::Scope;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::late_entries::{
    self, Entered, EntrySpec, LateEntry, LateEntryDef, LateEntryRecorded, LateEntryStatus,
    NewRecord,
};
use wl_kernel::machines::BoxFuture;

use crate::db::refused;
use crate::permissions;
use crate::plant::Plant;
use crate::returns::{self, ReturnInspection};
use crate::texts::english;

/// The subject type of the late entry of an inspection decision: the key of
/// the limits M and N.
pub const SUBJECT_TYPE: &str = "manufacturing.late_inspection";

/// The subject type of the inspection a late entry is about.
pub const INSPECTION_SUBJECT_TYPE: &str = "manufacturing.return_inspection";

/// What the inspector decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The lot is released.
    Pass,
    /// An NCR is raised on the lot.
    Fail,
}

impl Decision {
    /// The word stored with the decision.
    pub fn key(self) -> &'static str {
        match self {
            Decision::Pass => "pass",
            Decision::Fail => "fail",
        }
    }
}

/// When the decision was made and why it is entered late.
#[derive(Debug, Clone, Copy)]
pub struct Made<'a> {
    /// The time of the decision.
    pub at: DateTime,
    /// Why it is entered late.
    pub reason: Option<&'a str>,
    /// The outage reference.
    pub outage: Option<&'a str>,
}

/// A decision on a returned lot typed in later, as it was typed.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "manufacturing_late_inspections",
    index(inspection_id),
    index(late_entry_id)
)]
pub struct LateInspection {
    /// The primary key.
    pub id: i64,
    /// The inspection decided.
    pub inspection_id: i64,
    /// `pass` or `fail`.
    pub decision: String,
    /// The note of a pass, or what is wrong on a fail.
    pub text: Option<String>,
    /// The plant where the NCR of a fail is raised, and the entry approved.
    pub plant_id: Option<i64>,
    /// When the decision was made (the business time).
    pub occurred_at: DateTime,
    /// When it was typed in.
    pub entered_at: DateTime,
    /// Whether it carries the late-entry flag.
    pub late: bool,
    /// Why it is late.
    pub late_reason: Option<String>,
    /// The late entry of the kernel.
    pub late_entry_id: Option<i64>,
    /// The user who typed it in.
    pub entered_by: i64,
    /// When the result was applied to the inspection.
    pub applied_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

fn create<'a>(tx: &'a mut Transaction, new: &'a NewRecord) -> BoxFuture<'a, Result<i64>> {
    Box::pin(async move {
        let payload = &new.payload;
        let row = LateInspection::create(
            &mut *tx,
            LateInspection {
                inspection_id: payload["inspection_id"].as_i64().unwrap_or_default(),
                decision: payload["decision"].as_str().unwrap_or_default().to_owned(),
                text: payload["text"].as_str().map(str::to_owned),
                plant_id: payload["plant_id"].as_i64(),
                occurred_at: new.occurred_at,
                entered_at: new.entered_at,
                late: new.late,
                late_reason: new.reason.clone(),
                late_entry_id: new.late_entry_id,
                entered_by: new.user_id,
                ..LateInspection::default()
            },
        )
        .await?;
        Ok(row.id)
    })
}

/// Registers the entity with the kernel's late-entry check, and the applying
/// of the decision once the entry is recorded or approved.
pub(crate) fn register(app: &mut Registry) {
    LateEntryDef::of(SUBJECT_TYPE, create)
        .business_time("occurred_at")
        .parent("inspection")
        .register();
    app.listen(|event: LateEntryRecorded, state: AppState| async move {
        if event.subject.subject_type == SUBJECT_TYPE {
            apply_from(&state, event.subject.subject_id).await?;
        }
        Ok(())
    });
}

/// The late entry that keeps inspection `id` waiting for the production
/// manager, when there is one.
///
/// # Errors
///
/// A database error.
pub async fn waiting(state: &AppState, id: i64) -> Result<Option<LateEntry>> {
    LateEntry::query()
        .where_(LateEntry::SUBJECT_TYPE.eq(SUBJECT_TYPE))
        .where_(LateEntry::PARENT_TYPE.eq(INSPECTION_SUBJECT_TYPE))
        .where_(LateEntry::PARENT_ID.eq(id))
        .where_(LateEntry::STATUS.eq(LateEntryStatus::AwaitingApproval))
        .first(&state.db)
        .await
}

/// The decision typed in later for inspection `id` that carries the
/// late-entry flag, when there is one.
///
/// # Errors
///
/// A database error.
pub async fn flagged(state: &AppState, id: i64) -> Result<Option<LateInspection>> {
    LateInspection::query()
        .where_(LateInspection::INSPECTION_ID.eq(id))
        .where_(LateInspection::LATE.eq(true))
        .order_by_desc(LateInspection::ID)
        .first(&state.db)
        .await
}

/// Applies the decision of late inspection `id` to its inspection, once.
///
/// # Errors
///
/// The 422 refusals of the pass or fail (the lot cannot be released, the NCR
/// cannot be raised); the row stays unapplied.
pub async fn apply_from(state: &AppState, id: i64) -> Result {
    let Some(found) = LateInspection::find(&state.db, id).await? else {
        return Ok(());
    };
    if found.applied_at.is_some() {
        return Ok(());
    }
    let inspection = ReturnInspection::find_or_404(&state.db, found.inspection_id).await?;
    if inspection.is_open() {
        let text = found.text.clone().unwrap_or_default();
        if found.decision == Decision::Pass.key() {
            returns::apply_pass(
                state,
                found.entered_by,
                found.occurred_at,
                &inspection,
                &text,
            )
            .await?;
        } else {
            let plant = found
                .plant_id
                .ok_or_else(|| refused(english("manufacturing.create.plant", &[])))?;
            returns::apply_fail(
                state,
                found.entered_by,
                found.occurred_at,
                &inspection,
                plant,
                &text,
            )
            .await?;
        }
    }
    LateInspection::query()
        .where_(LateInspection::ID.eq(found.id))
        .update(
            &state.db,
            &[("applied_at", &Some(now())), ("updated_at", &now())],
        )
        .await?;
    Ok(())
}

/// `by` records `decision` on inspection `id`, made at `made_at`, with the
/// `text` (the note of a pass, what is wrong on a fail).
///
/// Not late: it is applied at once. Late by more than M minutes: it is
/// flagged and applied with the reason (required). Late by more than N
/// hours: an approval request goes to the holders of `late_entries.approve`
/// and it is applied when one approves; until then the inspection is open
/// and takes no other decision.
///
/// # Errors
///
/// `Forbidden` without `inspections.record` (and, for a fail, `ncrs.raise` in
/// a plant); a 422 for a time in the future, an inspection that has its result
/// or waits already, or a late entry without a reason.
pub async fn enter(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    decision: Decision,
    text: &str,
    made: Made<'_>,
) -> Result<Entered> {
    let Made {
        at: made_at,
        reason,
        outage,
    } = made;
    if !returns::allows(by, permissions::INSPECTIONS_RECORD) {
        return Err(Error::Forbidden);
    }
    let found = ReturnInspection::find_or_404(&state.db, id).await?;
    if !found.is_open() {
        return Err(refused(english("manufacturing.returns.inspected", &[])));
    }
    if waiting(state, id).await?.is_some() {
        return Err(refused(english("manufacturing.returns.waiting", &[])));
    }
    if made_at > now() {
        return Err(refused(english(
            "manufacturing.late_inspection.future",
            &[],
        )));
    }
    let plant = match decision {
        Decision::Fail => Some(returns::ncr_plant(state, by).await?),
        Decision::Pass => Plant::query()
            .where_(Plant::ACTIVE.eq(true))
            .order_by(Plant::ID)
            .first(&state.db)
            .await?
            .map(|plant| plant.id),
    };
    let scope = plant.map_or_else(Scope::global, permissions::plant_scope);
    let mut spec = EntrySpec::new(SUBJECT_TYPE, by.id, made_at)
        .parent(Subject::new(INSPECTION_SUBJECT_TYPE, id))
        .scope(scope)
        .payload(renox::serde_json::json!({
            "inspection_id": id,
            "decision": decision.key(),
            "text": Some(text.trim()).filter(|t| !t.is_empty()),
            "plant_id": plant,
        }));
    if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
        spec = spec.reason(reason);
    }
    if let Some(outage) = outage.map(str::trim).filter(|r| !r.is_empty()) {
        spec = spec.outage_reference(outage);
    }
    let entered = late_entries::enter(state, spec).await?;
    // Recorded entries are applied by the listener; this is the same
    // applying, done once.
    if let Some(record) = entered.record_id() {
        apply_from(state, record).await?;
    }
    Ok(entered)
}
