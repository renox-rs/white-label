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
use wl_kernel::Subject;
use wl_kernel::attendance::{AttendanceRecordId, AttendanceReview, LabourFuture};

use crate::flags::{self, FlagType, Pending};
use crate::location::WorkLocation;
use crate::record::AttendanceRecord;

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
}
