//! The days an employee was recorded as present (wl-attendance §6; the last
//! attended day an absence case counts from, `CON.RUL.08`).
//!
//! The module provides [`Attended`] as the app's
//! [`AttendanceHistory`](wl_kernel::attendance::AttendanceHistory). A record
//! counts unless it is `rejected`; a day with several records is listed once.

use std::collections::BTreeSet;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::attendance::{AttendanceHistory, LabourFuture};
use wl_kernel::workforce::EmployeeId;

use crate::record::{AttendanceRecord, AttendanceStatus};

/// The days from `from` to `to` (included) on which `employee` has a record
/// that counts, oldest first.
///
/// # Errors
///
/// A database error.
pub async fn attended_days(
    state: &AppState,
    employee: EmployeeId,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<NaiveDate>> {
    let days: BTreeSet<NaiveDate> = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::DATE.gte(from))
        .where_(AttendanceRecord::DATE.lte(to))
        .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
        .get(&state.db)
        .await?
        .into_iter()
        .map(|record| record.date)
        .collect();
    Ok(days.into_iter().collect())
}

/// The module's [`AttendanceHistory`].
#[derive(Debug, Clone, Copy, Default)]
pub struct Attended;

impl AttendanceHistory for Attended {
    fn attended_days<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> LabourFuture<'a, Vec<NaiveDate>> {
        Box::pin(attended_days(state, employee, from, to))
    }
}
