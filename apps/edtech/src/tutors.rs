//! Assigning a tutor (10 §7.1 *Batch* `change_tutor`, *Session*
//! `change_tutor`, `EDU.ACL.14`): the assignments kept as history
//! ([`crate::tutor_assignment`]), the change of the tutor of a batch from a
//! date, and the guard [`tutor_eligible`] (BL-EDU-08) that the tutor's documents
//! are valid on every session date ([`crate::eligibility`]).
//!
//! Decisions:
//!
//! - A tutor is a wl-scheduling resource with the wl-hr employee behind it;
//!   the batch and its sessions keep the tutor they have now in their own
//!   columns and every change adds a row of history.
//! - Changing the tutor of a batch moves the `scheduled` sessions of the
//!   batch's tutor from the date to the new one, all or nothing: the new tutor
//!   must be free for every one of those sessions and eligible on every one of
//!   those dates, else nothing changes. BL-EDU-09 adds the notification and
//!   the cases around it; BL-EDU-08's [`tutor_eligible`] is the eligibility check.
//! - A `planned` batch has no sessions yet: its tutor is replaced in place and
//!   checked on the dates of its plan.

use renox::chrono::{Duration, NaiveDate};
use renox::db::{Transaction, now};
use renox::prelude::*;
use wl_kernel::documents::today;

use crate::batch::{Batch, BatchStatus};
use crate::batch_machine::{BatchInput, CHANGE_TUTOR, CHANGE_TUTOR_RUNNING};
use crate::batches;
use crate::db::{refused, set_batch};
use crate::permissions;
use crate::session::{Session, SessionStatus};
use crate::sessions::{self, Except};
use crate::texts::english;
use crate::tutor_assignment::TutorAssignment;

/// A tutor: the wl-scheduling resource and the employee behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tutor {
    /// The resource.
    pub resource_id: i64,
    /// The wl-hr employee, when known.
    pub employee_id: Option<i64>,
}

/// The guard of `EDU.ACL.14`: the tutor's documents are valid on every one of
/// `dates` for `batch` (police clearance when minors are enrolled; the permit or
/// the freelance visa for a freelance tutor), by `qualification_valid`
/// ([`crate::eligibility`]).
///
/// # Errors
///
/// A 422 naming the missing kind and the first failing date, and the other
/// dates it fails on.
pub async fn tutor_eligible(
    state: &AppState,
    tutor: &Tutor,
    batch: &Batch,
    dates: &[NaiveDate],
) -> Result {
    crate::eligibility::check(state, tutor, batch, dates).await
}

/// Records `resource` as the tutor of batch `batch_id` from `from`: the open
/// batch-level assignment ends the day before and a new one starts.
pub(crate) async fn record_batch(
    tx: &mut Transaction,
    batch_id: i64,
    tutor: &Tutor,
    from: NaiveDate,
    by: Option<i64>,
) -> Result {
    let day_before = from - Duration::days(1);
    TutorAssignment::query()
        .where_(TutorAssignment::BATCH_ID.eq(batch_id))
        .where_(TutorAssignment::SESSION_ID.is_null())
        .where_(TutorAssignment::VALID_TO.is_null())
        .update(
            &mut *tx,
            &[("valid_to", &Some(day_before)), ("updated_at", &now())],
        )
        .await?;
    let mut row = TutorAssignment {
        batch_id,
        tutor_resource_id: tutor.resource_id,
        tutor_employee_id: tutor.employee_id,
        valid_from: from,
        assigned_by: by,
        ..TutorAssignment::default()
    };
    row.insert(&mut *tx).await
}

/// Records `tutor` as the tutor of one session, for its day.
pub(crate) async fn record_session(
    tx: &mut Transaction,
    session: &Session,
    tutor: &Tutor,
    by: Option<i64>,
) -> Result {
    let mut row = TutorAssignment {
        batch_id: session.batch_id,
        session_id: Some(session.id),
        tutor_resource_id: tutor.resource_id,
        tutor_employee_id: tutor.employee_id,
        valid_from: session.session_date,
        valid_to: Some(session.session_date),
        assigned_by: by,
        ..TutorAssignment::default()
    };
    row.insert(&mut *tx).await
}

/// `change_tutor` of batch `id`: from `from`, the sessions of the batch's
/// tutor are taught by `resource` (and the employee behind it).
///
/// # Errors
///
/// A 403 without `batches.manage`; a 422 for a batch that is completed,
/// cancelled or terminated, a tutor taken on one of the dates, or a tutor
/// that is not eligible on every date (BL-EDU-08).
pub async fn change_batch_tutor(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    resource: i64,
    employee: Option<i64>,
    from: NaiveDate,
) -> Result<Batch> {
    if !by.has_permission(permissions::BATCHES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let batch = Batch::find_or_404(&state.db, id).await?;
    if !batch.status.is_live() {
        return Err(refused(english("edtech.tutors.problem.batch_over", &[])));
    }
    let tutor = Tutor {
        resource_id: resource,
        employee_id: employee,
    };
    let (dates, slots) = if batch.status == BatchStatus::Planned {
        let plan = batches::plan_for(state, &batch)?;
        let kept: Vec<_> = plan.iter().filter(|p| p.date >= from).collect();
        (
            kept.iter().map(|p| p.date).collect::<Vec<_>>(),
            kept.iter()
                .map(|p| (p.starts_at, p.ends_at))
                .collect::<Vec<_>>(),
        )
    } else {
        let moving: Vec<Session> = Session::of_batch(&state.db, id)
            .await?
            .into_iter()
            .filter(|s| {
                s.status == SessionStatus::Scheduled
                    && s.session_date >= from
                    && s.tutor_resource_id == batch.tutor_resource_id
            })
            .collect();
        (
            moving.iter().map(|s| s.session_date).collect::<Vec<_>>(),
            moving
                .iter()
                .map(|s| (s.starts_at, s.ends_at))
                .collect::<Vec<_>>(),
        )
    };
    tutor_eligible(state, &tutor, &batch, &dates).await?;
    let clashes = sessions::conflicts(&state.db, Some(resource), None, &slots, Except::default())
        .await?;
    sessions::no_clash(&clashes)?;
    if batch.status == BatchStatus::Planned {
        let mut tx = state.db.begin().await?;
        set_batch(
            &mut tx,
            id,
            &[
                ("tutor_resource_id", &Some(resource)),
                ("tutor_employee_id", &employee),
            ],
        )
        .await?;
        record_batch(&mut tx, id, &tutor, from.max(batch.start_date), Some(by.id)).await?;
        tx.commit().await?;
        return Batch::find_or_404(&state.db, id).await;
    }
    let name = if batch.status == BatchStatus::Running {
        CHANGE_TUTOR_RUNNING
    } else {
        CHANGE_TUTOR
    };
    let input = BatchInput {
        from: Some(from),
        tutor_resource_id: Some(resource),
        tutor_employee_id: employee,
        by: Some(by.id),
        ..BatchInput::default()
    };
    batches::run(state, Some(by), id, name, input).await
}

/// The assignments of batch `batch_id`, for its page.
///
/// # Errors
///
/// The database's errors.
pub async fn history(db: &Db, batch_id: i64) -> Result<Vec<TutorAssignment>> {
    TutorAssignment::of_batch(db, batch_id).await
}

/// The day the change takes effect when none is named: today.
pub fn default_from() -> NaiveDate {
    today()
}
