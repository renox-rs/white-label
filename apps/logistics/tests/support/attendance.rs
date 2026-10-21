//! Attendance for the logistics tests: the module does not depend on
//! `wl-attendance`, so the tests provide an `AttendanceHistory` that knows
//! the check-ins they record, and a `ShiftCalendar` (rosters are a later
//! issue, `HRM.ROS.01`) that knows the shift ends they set.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::attendance::{AttendanceHistory, LabourFuture, ShiftCalendar};
use wl_kernel::workforce::EmployeeId;

/// The end of a user's shift on a day.
#[derive(Clone, Default)]
pub struct Shifts(Arc<Mutex<BTreeMap<(i64, NaiveDate), DateTime>>>);

impl Shifts {
    /// The shift of `user_id` on `date` ends at `end`.
    pub fn set(&self, user_id: i64, date: NaiveDate, end: DateTime) {
        self.0.lock().unwrap().insert((user_id, date), end);
    }
}

impl ShiftCalendar for Shifts {
    fn shift_end<'a>(
        &'a self,
        _state: &'a AppState,
        user_id: i64,
        date: NaiveDate,
    ) -> LabourFuture<'a, Option<DateTime>> {
        let found = self.0.lock().unwrap().get(&(user_id, date)).copied();
        Box::pin(async move { Ok(found) })
    }
}

/// The check-ins recorded for employees.
#[derive(Clone, Default)]
pub struct Attendance(Arc<Mutex<Vec<(EmployeeId, DateTime)>>>);

impl Attendance {
    /// `employee` checked in at `at`.
    pub fn check_in(&self, employee: EmployeeId, at: DateTime) {
        self.0.lock().unwrap().push((employee, at));
    }
}

impl AttendanceHistory for Attendance {
    fn checked_in_since<'a>(
        &'a self,
        _state: &'a AppState,
        employee: EmployeeId,
        since: DateTime,
    ) -> LabourFuture<'a, bool> {
        let found = self
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|(who, at)| *who == employee && *at > since);
        Box::pin(async move { Ok(found) })
    }
}
