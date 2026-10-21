//! The days an employee was recorded as present (wl-attendance §6; the last
//! attended day an absence case counts from, `CON.RUL.08`).
//!
//! The module provides [`Attended`] as the app's
//! [`AttendanceHistory`](wl_kernel::attendance::AttendanceHistory). A record
//! counts unless it is `rejected`; a day with several records is listed once.

use std::collections::BTreeSet;

use renox::chrono::{Duration, NaiveDate};
use renox::db::DateTime;
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

/// Whether `employee` has a record that counts whose check-in is after
/// `since`.
///
/// # Errors
///
/// A database error.
pub async fn checked_in_since(
    state: &AppState,
    employee: EmployeeId,
    since: DateTime,
) -> Result<bool> {
    // A record belongs to the day of its check-in (Asia/Dubai, UTC+4): a day
    // before the day of `since`, less one for the offset, cannot hold one.
    let from = (since - Duration::days(1)).date_naive();
    let records = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::DATE.gte(from))
        .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
        .get(&state.db)
        .await?;
    Ok(records.iter().any(|record| record.check_in > since))
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

    fn checked_in_since<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
        since: DateTime,
    ) -> LabourFuture<'a, bool> {
        Box::pin(checked_in_since(state, employee, since))
    }
}
