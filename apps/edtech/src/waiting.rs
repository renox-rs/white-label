//! The waiting list of a batch (10 §4.1 *Two people take the last seat*,
//! `EDU.ACL.17`, BL-EDU-02).
//!
//! A student refused a seat of a full batch can be put on its waiting list: a
//! plain list of the student, the payer and when they were added, which the
//! counsellors see. No seat is held and nothing is offered when a seat frees;
//! the counsellor looks at the list and enrols the student again.

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::PartyId;
use wl_kernel::parties::directory;

use crate::batch::{Batch, BatchStatus};
use crate::db::refused;
use crate::enrolment::Enrolment;
use crate::permissions;
use crate::texts::english;

/// One entry of the waiting list of a batch.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "edtech_batch_waiting",
    unique(batch_id, student_party_id),
    index(batch_id)
)]
pub struct BatchWaiting {
    /// The primary key.
    pub id: i64,
    /// The batch that is full.
    #[model(references = "edtech_batches")]
    pub batch_id: i64,
    /// The student, a party.
    pub student_party_id: i64,
    /// Who pays, a party.
    pub payer_party_id: i64,
    /// The user who added the entry.
    pub created_by: Option<i64>,
    /// When the entry was added.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl BatchWaiting {
    /// The waiting list of `batch_id`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_batch(db: &Db, batch_id: i64) -> Result<Vec<BatchWaiting>> {
        BatchWaiting::query()
            .where_(BatchWaiting::BATCH_ID.eq(batch_id))
            .order_by(BatchWaiting::ID)
            .get(db)
            .await
    }
}

/// `Forbidden` unless `user` holds `permission`.
fn require(user: &AuthUser, permission: &str) -> Result {
    if user.has_permission(permission) {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}

/// Puts `student` on the waiting list of batch `batch_id`; the payer is the
/// student unless another is named.
///
/// # Errors
///
/// A 403 without `enrolments.manage`; a 404 for an unknown batch; a 422 for a
/// batch that is not `open`, an unknown student or payer, a student who holds a
/// seat of the batch already or is on the list already.
pub async fn join(
    state: &AppState,
    by: &AuthUser,
    batch_id: i64,
    student: i64,
    payer: Option<i64>,
) -> Result<BatchWaiting> {
    require(by, permissions::ENROLMENTS_MANAGE)?;
    let batch = Batch::find_or_404(&state.db, batch_id).await?;
    if batch.status != BatchStatus::Open {
        return Err(refused(english("edtech.waiting.problem.not_open", &[])));
    }
    let payer = payer.unwrap_or(student);
    for (party, key) in [
        (student, "edtech.enrolments.problem.student"),
        (payer, "edtech.enrolments.problem.payer"),
    ] {
        let found = directory(state)
            .display_name(&state.db, PartyId(party), "en")
            .await?;
        if found.is_none() {
            return Err(refused(english(key, &[])));
        }
    }
    let seated = Enrolment::seated_in(&state.db, batch.id)
        .await?
        .iter()
        .any(|e| e.student_party_id == student);
    if seated {
        return Err(refused(english("edtech.waiting.problem.seated", &[])));
    }
    let listed = BatchWaiting::query()
        .where_(BatchWaiting::BATCH_ID.eq(batch.id))
        .where_(BatchWaiting::STUDENT_PARTY_ID.eq(student))
        .exists(&state.db)
        .await?;
    if listed {
        return Err(refused(english("edtech.waiting.problem.listed", &[])));
    }
    let mut entry = BatchWaiting {
        batch_id: batch.id,
        student_party_id: student,
        payer_party_id: payer,
        created_by: Some(by.id),
        ..BatchWaiting::default()
    };
    entry.save(&state.db).await?;
    Ok(entry)
}

/// Takes entry `id` off the waiting list (the student was enrolled, or no longer
/// wants a seat).
///
/// # Errors
///
/// A 403 without `enrolments.manage`; a 404 for an unknown entry.
pub async fn remove(state: &AppState, by: &AuthUser, id: i64) -> Result<i64> {
    require(by, permissions::ENROLMENTS_MANAGE)?;
    let entry = BatchWaiting::find_or_404(&state.db, id).await?;
    BatchWaiting::query()
        .where_(BatchWaiting::ID.eq(id))
        .delete(&state.db)
        .await?;
    Ok(entry.batch_id)
}
