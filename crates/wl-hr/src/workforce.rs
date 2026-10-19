//! The app's [`Workforce`](wl_kernel::workforce::Workforce) (wl-hr §6,
//! issue #216): the employees with a job role or trade in a scope.

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::workforce::{EmployeeId, Workforce, WorkforceFuture};

use crate::employee::{Employee, EmployeeScope, EmployeeStatus};

/// The workforce of the HR module.
#[derive(Debug, Clone, Copy, Default)]
pub struct Staff;

impl Workforce for Staff {
    fn employees_in<'a>(
        &'a self,
        db: &'a Db,
        role: &'a str,
        scope: &'a Subject,
    ) -> WorkforceFuture<'a, Vec<EmployeeId>> {
        Box::pin(async move {
            let ids: Vec<i64> = EmployeeScope::query()
                .where_(EmployeeScope::SCOPE_TYPE.eq(scope.subject_type.clone()))
                .where_(EmployeeScope::SCOPE_ID.eq(scope.subject_id.to_string()))
                .get(db)
                .await?
                .into_iter()
                .map(|s| s.employee_id)
                .collect();
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            Ok(Employee::query()
                .where_(Employee::ID.is_in(ids))
                .where_(Employee::JOB_ROLE.eq(role.trim().to_lowercase()))
                .where_(Employee::STATUS.is_in(EmployeeStatus::WORKING))
                .order_by(Employee::ID)
                .get(db)
                .await?
                .into_iter()
                .map(|e| e.employee_id())
                .collect())
        })
    }
}
