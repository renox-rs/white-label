//! The attendance contract (wl-attendance §6 *Events emitted*): what other
//! modules and the verticals may know about attendance records without
//! depending on the `wl-attendance` crate.
//!
//! `wl-attendance` emits [`AttendanceRecorded`] when a check-in, a check-out
//! or a supervisor marking is saved, and [`AttendanceFlagged`] once for every
//! flag it adds to a record (`overlap`, `overtime_cap`, `midday_break`,
//! `permit_invalid`, ...). A record is referred to by [`AttendanceRecordId`]
//! or as a [`Subject`] of type [`ATTENDANCE_SUBJECT_TYPE`].

use std::fmt;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::subject::Subject;
use crate::workforce::EmployeeId;

/// The subject type of an attendance record, for [`Subject`] references.
pub const ATTENDANCE_SUBJECT_TYPE: &str = "attendance.record";

/// The subject type of a work location.
pub const WORK_LOCATION_SUBJECT_TYPE: &str = "attendance.work_location";

/// The id of an attendance record.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct AttendanceRecordId(pub i64);

impl AttendanceRecordId {
    /// The record as a [`Subject`]: `attendance.record:7`.
    pub fn subject(self) -> Subject {
        Subject::new(ATTENDANCE_SUBJECT_TYPE, self.0)
    }
}

impl fmt::Display for AttendanceRecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<i64> for AttendanceRecordId {
    fn from(id: i64) -> Self {
        AttendanceRecordId(id)
    }
}

/// An attendance record was saved: a check-in, a check-out or a supervisor
/// marking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendanceRecorded {
    /// The record.
    pub record: AttendanceRecordId,
    /// The employee.
    pub employee: EmployeeId,
    /// The work location, `attendance.work_location:3`.
    pub location: Subject,
    /// The day, in Asia/Dubai.
    pub date: NaiveDate,
    /// `supervisor`, `self_check_in`, `device` or `import`.
    pub method: String,
    /// Whether the position was inside the geofence; `None` when unknown
    /// (no geofence is checked yet).
    pub within_geofence: Option<bool>,
    /// The flags the record carries when it is saved, e.g. `overlap`.
    pub flags: Vec<String>,
}

impl Event for AttendanceRecorded {}

/// A flag was added to an attendance record, once per record and flag type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttendanceFlagged {
    /// The record.
    pub record: AttendanceRecordId,
    /// The employee.
    pub employee: EmployeeId,
    /// The day, in Asia/Dubai.
    pub date: NaiveDate,
    /// The flag type: `overlap`, `overtime_cap`, `midday_break`,
    /// `permit_invalid`, ...
    pub flag: String,
}

impl Event for AttendanceFlagged {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_a_subject_and_a_plain_number() {
        let id = AttendanceRecordId::from(7);
        assert_eq!(id.to_string(), "7");
        assert_eq!(id.subject(), Subject::new("attendance.record", 7));
        assert_eq!(renox::serde_json::to_string(&id).unwrap(), "7");
    }

    #[test]
    fn the_events_round_trip_through_json() {
        let flagged = AttendanceFlagged {
            record: AttendanceRecordId(7),
            employee: EmployeeId(3),
            date: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap(),
            flag: "overlap".into(),
        };
        let json = renox::serde_json::to_string(&flagged).unwrap();
        assert_eq!(
            renox::serde_json::from_str::<AttendanceFlagged>(&json).unwrap(),
            flagged
        );
    }
}
