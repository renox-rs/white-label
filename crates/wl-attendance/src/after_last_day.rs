//! Attendance after the last working day of a context (wl-attendance §4
//! `record`, 02 Construction §7.1 *Project* `terminate`). A record for a day
//! after it is refused ([`crate::service::record`] asks the app's
//! [`ContextClosure`](wl_kernel::attendance::ContextClosure)). A termination
//! whose effective date is in the past finds attendance recorded after it
//! already: that attendance is kept, never deleted, and is flagged
//! `after_last_day` for the reviewer ([`flag_after`]), and it stays costed
//! until the reviewer accepts or adjusts it.
//!
//! The module provides [`Reviewer`] as the app's
//! [`AttendanceReview`](wl_kernel::attendance::AttendanceReview).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use wl_kernel::Subject;
use wl_kernel::attendance::{AttendanceRecordId, AttendanceReview, LabourFuture, WorkRejection};
use wl_kernel::workforce::EmployeeId;

use crate::flags::{self, FlagType, Pending};
use crate::location::WorkLocation;
use crate::lock;
use crate::record::{AttendanceRecord, AttendanceStatus, REJECT_BY_SYSTEM};

/// Flags the attendance recorded after `after` at the work locations in
/// `context` with `note`. Returns the records flagged now; a record that
/// carries the flag already is left out.
///
/// # Errors
///
/// A database error.
pub async fn flag_after(
    state: &AppState,
    context: &Subject,
    after: NaiveDate,
    note: &str,
) -> Result<Vec<AttendanceRecordId>> {
    let locations: Vec<WorkLocation> = WorkLocation::query()
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|location| location.scope().as_ref() == Some(context))
        .collect();
    let mut flagged = Vec::new();
    for location in locations {
        let records = AttendanceRecord::query()
            .where_(AttendanceRecord::LOCATION_ID.eq(location.id))
            .where_(AttendanceRecord::DATE.gt(after))
            .order_by(AttendanceRecord::ID)
            .get(&state.db)
            .await?;
        for record in records {
            let mut tx = state.db.begin().await?;
            let mut pending = Pending::default();
            let added = flags::add_in(
                &mut tx,
                record.id,
                FlagType::AfterLastDay,
                note,
                &mut pending,
            )
            .await?;
            tx.commit().await?;
            pending.finish(state).await?;
            if added {
                flagged.push(record.record_id());
            }
        }
    }
    Ok(flagged)
}

/// Whether any attendance was recorded at the work locations in `context`.
///
/// # Errors
///
/// A database error.
pub async fn has_records(state: &AppState, context: &Subject) -> Result<bool> {
    let ids: Vec<i64> = WorkLocation::query()
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|location| location.scope().as_ref() == Some(context))
        .map(|location| location.id)
        .collect();
    if ids.is_empty() {
        return Ok(false);
    }
    Ok(AttendanceRecord::query()
        .where_(AttendanceRecord::LOCATION_ID.is_in(ids))
        .first(&state.db)
        .await?
        .is_some())
}

/// Rejects the attendance of `employee` from `from` on at the work locations
/// in `contexts` with `reason`: a record that is `recorded` or `flagged` is
/// rejected by the system (its proposed hours leave the open timesheet); one
/// that is `accepted` or `locked` is listed in `kept` for an adjustment.
///
/// # Errors
///
/// A database error.
pub async fn reject_work(
    state: &AppState,
    contexts: &[Subject],
    employee: EmployeeId,
    from: NaiveDate,
    reason: &str,
) -> Result<WorkRejection> {
    let ids: Vec<i64> = WorkLocation::query()
        .get(&state.db)
        .await?
        .into_iter()
        .filter(|location| {
            location
                .scope()
                .is_some_and(|scope| contexts.contains(&scope))
        })
        .map(|location| location.id)
        .collect();
    let mut done = WorkRejection::default();
    if ids.is_empty() {
        return Ok(done);
    }
    let records = AttendanceRecord::query()
        .where_(AttendanceRecord::LOCATION_ID.is_in(ids))
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::DATE.gte(from))
        .order_by(AttendanceRecord::ID)
        .get(&state.db)
        .await?;
    for record in records {
        match record.status {
            AttendanceStatus::Rejected => {}
            AttendanceStatus::Accepted | AttendanceStatus::Locked => {
                done.kept.push(record.record_id());
            }
            AttendanceStatus::Recorded | AttendanceStatus::Flagged => {
                let _day = lock::take(record.employee_id, record.date).await;
                let mut tx = state.db.begin().await?;
                let Some(fresh) = AttendanceRecord::find(&mut tx, record.id).await? else {
                    continue;
                };
                let outcome = fresh
                    .transition(REJECT_BY_SYSTEM)
                    .by_system()
                    .reason(reason)
                    .run_in(&mut tx)
                    .await
                    .map_err(|e| e.into_error())?;
                tx.commit().await?;
                outcome.emit(state).await?;
                done.rejected.push(record.record_id());
            }
        }
    }
    Ok(done)
}

/// The module's [`AttendanceReview`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Reviewer;

impl AttendanceReview for Reviewer {
    fn flag_after<'a>(
        &'a self,
        state: &'a AppState,
        context: &'a Subject,
        after: NaiveDate,
        note: &'a str,
    ) -> LabourFuture<'a, Vec<AttendanceRecordId>> {
        Box::pin(flag_after(state, context, after, note))
    }

    fn has_records<'a>(
        &'a self,
        state: &'a AppState,
        context: &'a Subject,
    ) -> LabourFuture<'a, bool> {
        Box::pin(has_records(state, context))
    }

    fn reject_work<'a>(
        &'a self,
        state: &'a AppState,
        contexts: &'a [Subject],
        employee: EmployeeId,
        from: NaiveDate,
        reason: &'a str,
    ) -> LabourFuture<'a, WorkRejection> {
        Box::pin(reject_work(state, contexts, employee, from, reason))
    }
}
