//! The app's [`Workforce`](wl_kernel::workforce::Workforce) (wl-hr §6,
//! issues #216 and #53): the employees with a job role or trade in a scope,
//! every listed employee, the profile of one (role, scopes, account, manager,
//! leave) and the employee of a user.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::workforce::{
    EmployeeId, LeaveCalendar, StaffProfile, Workforce, WorkforceFuture,
};

use crate::assignees;
use crate::employee::{Employee, EmployeeScope, EmployeeStatus};
use crate::leave::{LeaveRequest, LeaveStatus};

/// The workforce of the HR module.
#[derive(Debug, Clone, Copy, Default)]
pub struct Staff;

impl LeaveCalendar for Staff {
    fn approved_leave<'a>(
        &'a self,
        db: &'a Db,
        employee: EmployeeId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> WorkforceFuture<'a, Vec<(NaiveDate, NaiveDate)>> {
        Box::pin(async move {
            Ok(LeaveRequest::query()
                .where_(LeaveRequest::EMPLOYEE_ID.eq(employee.0))
                .where_(LeaveRequest::STATUS.is_in([LeaveStatus::Approved, LeaveStatus::Completed]))
                .order_by(LeaveRequest::FROM_DATE)
                .get(db)
                .await?
                .into_iter()
                .filter(|leave| leave.days() > 0 && leave.from_date <= to && leave.to_date >= from)
                .map(|leave| (leave.from_date, leave.to_date))
                .collect())
        })
    }
}

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

    fn all_staff<'a>(&'a self, db: &'a Db) -> WorkforceFuture<'a, Vec<EmployeeId>> {
        Box::pin(async move {
            Ok(Employee::query()
                .where_(Employee::STATUS.is_in(EmployeeStatus::WORKING))
                .order_by(Employee::ID)
                .get(db)
                .await?
                .into_iter()
                .map(|e| e.employee_id())
                .collect())
        })
    }

    fn staff<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Option<StaffProfile>> {
        Box::pin(async move {
            let Some(row) = Employee::find(&state.db, employee.0).await? else {
                return Ok(None);
            };
            let scopes = row.scopes(&state.db).await?;
            let managers = assignees::resolve(state, &row, &scopes, "manager").await?;
            let on_leave = row.status == EmployeeStatus::OnLeave;
            let returns_on = if on_leave {
                let day = today();
                LeaveRequest::query()
                    .where_(LeaveRequest::EMPLOYEE_ID.eq(row.id))
                    .where_(
                        LeaveRequest::STATUS.is_in([LeaveStatus::Approved, LeaveStatus::Completed]),
                    )
                    .get(&state.db)
                    .await?
                    .into_iter()
                    .filter(|leave| leave.from_date <= day && day <= leave.to_date)
                    .map(|leave| leave.to_date)
                    .max()
                    .and_then(|last| last.checked_add_days(Days::new(1)))
            } else {
                None
            };
            Ok(Some(StaffProfile {
                employee,
                party: row.party(),
                user_id: row.user_id,
                role: row.job_role.clone(),
                scopes,
                legal_entity_id: row.legal_entity_id,
                listed: EmployeeStatus::WORKING.contains(&row.status),
                on_leave,
                returns_on,
                managers,
            }))
        })
    }

    fn employee_of_user<'a>(
        &'a self,
        db: &'a Db,
        user_id: i64,
    ) -> WorkforceFuture<'a, Option<EmployeeId>> {
        Box::pin(async move {
            Ok(Employee::query()
                .where_(Employee::USER_ID.eq(user_id))
                .order_by(Employee::ID)
                .first(db)
                .await?
                .map(|e| e.employee_id()))
        })
    }
}
