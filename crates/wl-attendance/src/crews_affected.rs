//! The crews a document's expiry affects (`DOC.RUL.02`, wl-attendance §5.5
//! point 3, issue #297 of 02).
//!
//! A work permit or residence visa kind flags `attendance_record.accept`
//! ([`GUARDED`]). When such a document is within its look-ahead window,
//! wl-documents asks the [`FutureWork`] provider of that transition which
//! planned work needs the document after the expiry date. For attendance it
//! is the crew the employee works in: an employee who is still listed (not
//! gone, absconded or deceased) and has a scope, a project in 02, is planned
//! to be on site the day after the expiry date, so they are listed, once,
//! under their first scope. The responsible staff of that scope are told by
//! the expiry run (`documents.expiring`).
//!
//! Decision: there is no crew roster to read yet, so the crew is the employee
//! assigned to the scope; a document of any other subject lists nothing.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::{BlockEntry, DocumentFuture, FutureWork, PlannedRecord};
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, EmployeeId, workforce};

use crate::texts::english;

/// The transition the work permit and residence visa kinds flag.
pub const GUARDED: &str = "attendance_record.accept";

/// The provider registered for [`GUARDED`].
pub(crate) struct CrewsAffected;

impl FutureWork for CrewsAffected {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(async move {
            if subject.subject_type != EMPLOYEE_SUBJECT_TYPE || !entry.applies_to(None) {
                return Ok(Vec::new());
            }
            let employee = EmployeeId(subject.subject_id);
            let Some(profile) = workforce(state).staff(state, employee).await? else {
                return Ok(Vec::new());
            };
            let Some(scope) = profile.scopes.first().filter(|_| profile.listed) else {
                return Ok(Vec::new());
            };
            Ok(vec![PlannedRecord {
                record: subject.clone(),
                date: from,
                label: english("wl_attendance.crews.label", &[("employee", &employee.0)]),
                scope: Some(renox::auth::permissions::Scope::new(
                    &scope.subject_type,
                    scope.subject_id,
                )),
            }])
        })
    }
}
