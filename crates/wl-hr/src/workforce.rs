//! The app's [`Workforce`](wl_kernel::workforce::Workforce) (wl-hr §6,
//! issues #216 and #53): the employees with a job role or trade in a scope,
//! every listed employee, the profile of one (role, scopes, account, manager,
//! leave) and the employee of a user.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::today;
use wl_kernel::workforce::{EmployeeId, LeaveCalendar, StaffProfile, Workforce, WorkforceFuture};

use crate::absence::{AbsenceReport, ContactAttempt};
use crate::assignees;
use crate::employee::{Employee, EmployeeScope, EmployeeStatus, EmploymentType};
use crate::leave::{LeaveRequest, LeaveStatus};

/// The workforce of the HR module. `portal_days` is how many days the
/// portal of an employee who exited stays read-only before it closes
/// ([`crate::HrConfig::portal_days_after_exit`]).
#[derive(Debug, Clone, Copy)]
pub struct Staff {
    /// Days the portal stays read-only after `exit`.
    pub portal_days: i64,
}

impl Default for Staff {
    fn default() -> Self {
        Staff {
            portal_days: crate::config::DEFAULT_PORTAL_DAYS_AFTER_EXIT,
        }
    }
}

impl Staff {
    /// The day the portal of `employee` closes: `exit` plus the days, or
    /// `None` for an employee who has not exited.
    pub fn portal_closes_on(&self, employee: &Employee) -> Option<NaiveDate> {
        let exited = employee
            .exited_on
            .filter(|_| employee.status == EmployeeStatus::Exited)?;
        exited.checked_add_days(Days::new(self.portal_days.unsigned_abs()))
    }
}

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

    fn is_active<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            Ok(Employee::find(&state.db, employee.0)
                .await?
                .is_some_and(|row| row.status == EmployeeStatus::Active))
        })
    }

    fn is_freelance<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            Ok(Employee::find(&state.db, employee.0)
                .await?
                .is_some_and(|row| row.employment_type == EmploymentType::Freelance))
        })
    }

    fn failed_contact_days<'a>(
        &'a self,
        db: &'a Db,
        employee: EmployeeId,
        after: NaiveDate,
    ) -> WorkforceFuture<'a, Vec<NaiveDate>> {
        Box::pin(async move {
            let mut days: Vec<NaiveDate> = ContactAttempt::of_employee(db, employee.0)
                .await?
                .into_iter()
                .filter(|attempt| attempt.outcome.failed() && attempt.attempted_on > after)
                .map(|attempt| attempt.attempted_on)
                .collect();
            days.sort_unstable();
            days.dedup();
            Ok(days)
        })
    }

    fn released_pay_paths<'a>(
        &'a self,
        db: &'a Db,
        employee: EmployeeId,
    ) -> WorkforceFuture<'a, Vec<String>> {
        Box::pin(async move {
            let mut runs: Vec<i64> = crate::payroll::PayrollLine::query()
                .where_(crate::payroll::PayrollLine::EMPLOYEE_ID.eq(employee.0))
                .where_(crate::payroll::PayrollLine::RELEASED_INTO.is_not_null())
                .order_by(crate::payroll::PayrollLine::ID)
                .get(db)
                .await?
                .into_iter()
                .filter(|line| {
                    line.held_reason.as_deref() == Some(crate::payroll::HELD_ABSENCE_CASE)
                })
                .filter_map(|line| line.released_into)
                .collect();
            runs.dedup();
            Ok(runs
                .into_iter()
                .map(|id| format!("/hr/payroll/{id}"))
                .collect())
        })
    }

    fn access_disabled<'a>(&'a self, db: &'a Db, user_id: i64) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            let employees = Employee::query()
                .where_(Employee::USER_ID.eq(user_id))
                .get(db)
                .await?;
            for employee in employees {
                // Absconded, or the portal account was closed when the
                // onboarding was cancelled.
                if matches!(
                    employee.status,
                    EmployeeStatus::Absconded | EmployeeStatus::Deceased
                ) || employee.portal_closed_at.is_some()
                {
                    return Ok(true);
                }
                // After `exit` the portal is read-only, then closes.
                if self
                    .portal_closes_on(&employee)
                    .is_some_and(|closes| today() >= closes)
                {
                    return Ok(true);
                }
                let report = AbsenceReport::valid_for(db, employee.id).await?;
                if report.is_some_and(|report| report.access_disabled_at.is_some()) {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    fn read_only<'a>(&'a self, db: &'a Db, user_id: i64) -> WorkforceFuture<'a, bool> {
        Box::pin(async move {
            let employees = Employee::query()
                .where_(Employee::USER_ID.eq(user_id))
                .get(db)
                .await?;
            Ok(employees.iter().any(|employee| {
                self.portal_closes_on(employee)
                    .is_some_and(|closes| today() < closes)
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
