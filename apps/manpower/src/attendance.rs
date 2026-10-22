//! Attendance at a site while a deployment is suspended (09 Manpower §7.1
//! *Deployment* `suspend`: "attendance at the site disabled", BL-MPW-11).
//!
//! The vertical provides the kernel's
//! [`ContextClosure`](wl_kernel::attendance::ContextClosure), which
//! wl-attendance asks before it saves a record in a work location whose scope
//! is a context: a site (`manpower.site`) or a deployment
//! (`manpower.deployment`). A worker whose deployment there is `suspended`
//! from a day on or before the day of the record, and who has no `active`
//! deployment there, is refused: a check-in at the site, a supervisor's mark
//! and an import all fail with the same text. A deployment that resumes
//! ([`crate::resumption`]) lets the worker check in again. Nobody else is
//! affected, and a worker with no deployment is not refused here (that is the
//! attendance module's own business).

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::attendance::{ContextClosure, LabourFuture};
use wl_kernel::workforce::EmployeeId;

use crate::deployment::{self, Deployment, DeploymentStatus};
use crate::site;
use crate::texts::english;

/// Refuses attendance of a worker whose deployment at the context is suspended.
pub(crate) struct SuspendedDeployments;

impl ContextClosure for SuspendedDeployments {
    fn refuses<'a>(
        &'a self,
        state: &'a AppState,
        context: &'a Subject,
        employee: EmployeeId,
        date: NaiveDate,
    ) -> LabourFuture<'a, Option<String>> {
        Box::pin(async move {
            let here = match context.subject_type.as_str() {
                site::SUBJECT_TYPE => {
                    Deployment::query()
                        .where_(Deployment::EMPLOYEE_ID.eq(employee.0))
                        .where_(Deployment::SITE_ID.eq(context.subject_id))
                        .get(&state.db)
                        .await?
                }
                deployment::SUBJECT_TYPE => Deployment::find(&state.db, context.subject_id)
                    .await?
                    .filter(|found| found.employee_id == employee.0)
                    .into_iter()
                    .collect(),
                _ => return Ok(None),
            };
            let working = here.iter().any(|found| {
                found.status == DeploymentStatus::Active
                    && found.start_date <= date
                    && found.end_date.is_none_or(|end| date <= end)
            });
            let suspended = here.iter().any(|found| {
                found.status == DeploymentStatus::Suspended
                    && found.suspended_on.is_some_and(|since| since <= date)
            });
            Ok((suspended && !working).then(|| english("manpower.attendance.suspended", &[])))
        })
    }
}
