//! Using an approved override (wl-documents §4.3, #50 A6/A7): which override
//! covers a failing block for the acting user, and the guard's claim, give
//! back and report steps around the guarded run.

use renox::db::{DateTime, now};
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::Subject;
use wl_kernel::documents::{CoveredBlock, DocumentBlockOverridden};

use super::machine::{UNUSE, USE};
use super::model::{BlockOverride, BlockOverrideStatus};

/// The approved override of `kind_id` for `subject` on `guarded` and
/// `record_id` that `actor` may use at `at`: unused, not past its valid
/// until, and requested by `actor`. The oldest first.
pub(crate) async fn covering(
    db: &Db,
    kind_id: i64,
    subject: &Subject,
    guarded: &str,
    record_id: i64,
    actor: i64,
    at: DateTime,
) -> Result<Option<BlockOverride>> {
    let rows = BlockOverride::query()
        .where_(BlockOverride::KIND_ID.eq(kind_id))
        .where_(BlockOverride::SUBJECT_TYPE.eq(subject.subject_type.as_str()))
        .where_(BlockOverride::SUBJECT_ID.eq(subject.subject_id))
        .where_(BlockOverride::GUARDED.eq(guarded))
        .where_(BlockOverride::RECORD_ID.eq(record_id))
        .where_(BlockOverride::REQUESTED_BY.eq(actor))
        .where_(BlockOverride::STATUS.eq(BlockOverrideStatus::Approved))
        .order_by(BlockOverride::ID)
        .get(db)
        .await?;
    Ok(rows.into_iter().find(|o| o.usable_at(at)))
}

/// Claims the overrides `covered` for `actor`, all or none, in one
/// transaction: each row is locked and loaded, must be requested by
/// `actor` and still usable, and runs `use` (which stamps used at). `false`
/// when any cannot be claimed (a concurrent attempt got it first, or it
/// ran out); then nothing stays claimed.
pub(crate) async fn claim(db: &Db, covered: &[CoveredBlock], actor: i64) -> Result<bool> {
    let mut tx = db.begin().await?;
    for block in covered {
        let Some(record) = BlockOverride::query()
            .where_eq("id", block.override_id.0)
            .lock_for_update()
            .first(&mut tx)
            .await?
        else {
            return Ok(false);
        };
        if record.requested_by != actor || !record.usable_at(now()) {
            return Ok(false);
        }
        match record
            .transition(USE)
            .by_system()
            .reason("Used by the guarded transition.")
            .run_in(&mut tx)
            .await
        {
            Ok(_) => {}
            Err(
                TransitionError::Guard(_) | TransitionError::NotFrom(_) | TransitionError::Conflict,
            ) => {
                return Ok(false);
            }
            Err(other) => return Err(other.into_error()),
        }
    }
    tx.commit().await?;
    Ok(true)
}

/// Gives the claimed overrides `covered` back after the guarded run failed:
/// `unuse` puts each `used` one back to `approved` with used at empty.
pub(crate) async fn unclaim(db: &Db, covered: &[CoveredBlock]) -> Result {
    let mut tx = db.begin().await?;
    for block in covered {
        let Some(record) = BlockOverride::find(&mut tx, block.override_id.0).await? else {
            continue;
        };
        if record.status != BlockOverrideStatus::Used {
            continue;
        }
        // The transition event needs an `AppState`; the guard has only the `Db`.
        let _outcome = record
            .transition(UNUSE)
            .by_system()
            .reason("The guarded transition failed.")
            .run_in(&mut tx)
            .await
            .map_err(TransitionError::into_error)?;
    }
    tx.commit().await?;
    Ok(())
}

/// Emits [`DocumentBlockOverridden`] for each override of `covered`, once
/// the guarded transition `guarded` ran on `record_id`.
pub(crate) async fn report(
    state: &AppState,
    guarded: &str,
    record_id: i64,
    covered: &[CoveredBlock],
) -> Result {
    for block in covered {
        let Some(record) = BlockOverride::find(&state.db, block.override_id.0).await? else {
            continue;
        };
        state
            .emit(DocumentBlockOverridden {
                override_id: block.override_id,
                kind: block.entry.kind.clone(),
                subject: block.subject.clone(),
                guarded: guarded.to_owned(),
                record_id,
                reason: record.reason.clone(),
                requested_by: record.requested_by,
                approved_by: record.approved_by.unwrap_or_default(),
            })
            .await?;
    }
    Ok(())
}
