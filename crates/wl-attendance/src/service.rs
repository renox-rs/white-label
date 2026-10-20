//! Recording attendance (wl-attendance §4 *Attendance record*, `record`):
//! the services behind supervisor marking, imports and the employee's own
//! check-in and check-out.
//!
//! `record` is refused when the employee is not at work (the kernel's
//! [`Workforce`](wl_kernel::workforce::Workforce) says so: not listed, or on
//! leave), or when the day is outside the late-entry window (`HRM.ACL.01`:
//! today, or up to [`AttendanceConfig::late_entry_days`] back, 1 by default;
//! a day that has not come is refused too). A self check-in is refused while
//! the employee is checked in at another location (naming it) or at the same
//! one (a duplicate); nothing is created then.
//!
//! The overlap check and the insert of one employee and day run under one
//! lock ([`crate::lock`]) and in one transaction; the checks of
//! [`crate::checks`] flag through the `flag` transition and never refuse.
//! After the commit `AttendanceRecorded` and one `AttendanceFlagged` per
//! flag are emitted, the people are told, and the overlap check runs once
//! more on what is stored, so that a second server saving at the same moment
//! cannot leave both records unflagged.
//!
//! Decision: the spec's method `self` is [`AttendanceMethod::SelfCheckIn`]
//! (`self` is not a name a variant can take).

use renox::chrono::{Days, NaiveDate};
use renox::db::{DateTime, Json, now};
use renox::prelude::*;
use wl_kernel::attendance::AttendanceRecorded;
use wl_kernel::documents::{dubai_date, today};
use wl_kernel::workforce::{EmployeeId, workforce};

use crate::checks;
use crate::config::{AttendanceConfig, config_of};
use crate::flags::{AttendanceFlag, Pending};
use crate::location::WorkLocation;
use crate::lock;
use crate::overlap;
use crate::record::{AttendanceMethod, AttendanceRecord, AttendanceStatus, BreakSpan};
use crate::texts::english;

/// What a supervisor, an import or a check-in saves.
#[derive(Debug, Clone)]
pub struct NewRecord {
    /// The employee.
    pub employee: EmployeeId,
    /// The work location.
    pub location: i64,
    /// When the employee was checked in.
    pub check_in: DateTime,
    /// When the employee checked out, when known.
    pub check_out: Option<DateTime>,
    /// The breaks taken.
    pub breaks: Vec<BreakSpan>,
    /// How the record is made.
    pub method: AttendanceMethod,
    /// The user who makes it.
    pub by: Option<i64>,
    /// A note.
    pub note: String,
}

impl NewRecord {
    /// A supervisor `by` marks `employee` present at `location` from
    /// `check_in` to `check_out`.
    pub fn supervisor(
        employee: EmployeeId,
        location: i64,
        check_in: DateTime,
        check_out: Option<DateTime>,
        by: i64,
    ) -> Self {
        NewRecord {
            employee,
            location,
            check_in,
            check_out,
            breaks: Vec::new(),
            method: AttendanceMethod::Supervisor,
            by: Some(by),
            note: String::new(),
        }
    }

    /// An import (no user) of the same.
    pub fn import(
        employee: EmployeeId,
        location: i64,
        check_in: DateTime,
        check_out: Option<DateTime>,
    ) -> Self {
        NewRecord {
            method: AttendanceMethod::Import,
            by: None,
            ..NewRecord::supervisor(employee, location, check_in, check_out, 0)
        }
    }

    /// With these breaks taken.
    pub fn breaks(mut self, breaks: Vec<BreakSpan>) -> Self {
        self.breaks = breaks;
        self
    }

    /// With this note.
    pub fn note(mut self, note: &str) -> Self {
        self.note = note.trim().to_owned();
        self
    }
}

/// A refusal with a message, as a 422.
pub(crate) fn refuse(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// The earliest day a supervisor may mark on `today`.
pub fn earliest_day(config: &AttendanceConfig, today: NaiveDate) -> NaiveDate {
    today
        .checked_sub_days(Days::new(u64::try_from(config.late_entry_days).unwrap_or(0)))
        .unwrap_or(today)
}

/// Refuses an employee who is not at work.
async fn ensure_at_work(state: &AppState, employee: EmployeeId) -> Result {
    let profile = workforce(state).staff(state, employee).await?;
    match profile {
        Some(profile) if profile.listed && !profile.on_leave => Ok(()),
        _ => Err(refuse(english("wl_attendance.record.not_working", &[]))),
    }
}

/// Refuses a day outside the late-entry window (`HRM.ACL.01`).
fn ensure_in_window(state: &AppState, date: NaiveDate) -> Result {
    let config = config_of(state);
    let day = today();
    if date > day {
        return Err(refuse(english(
            "wl_attendance.record.in_future",
            &[("date", &date)],
        )));
    }
    if date < earliest_day(&config, day) {
        return Err(refuse(english(
            "wl_attendance.record.too_old",
            &[("days", &config.late_entry_days), ("date", &date)],
        )));
    }
    Ok(())
}

/// Refuses a self check-in while the employee is checked in on `date` at the
/// same location (a duplicate) or at another one.
async fn ensure_not_checked_in(
    tx: &mut renox::db::Transaction,
    employee: EmployeeId,
    location: &WorkLocation,
    date: NaiveDate,
) -> Result {
    let open: Vec<AttendanceRecord> = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(employee.0))
        .where_(AttendanceRecord::DATE.eq(date))
        .where_(AttendanceRecord::STATUS.is_in(AttendanceStatus::LIVE))
        .order_by(AttendanceRecord::ID)
        .get(&mut *tx)
        .await?
        .into_iter()
        .filter(AttendanceRecord::is_open)
        .collect();
    if open.iter().any(|r| r.location_id == location.id) {
        return Err(refuse(english("wl_attendance.self.already_here", &[])));
    }
    if let Some(elsewhere) = open.first() {
        let name = WorkLocation::find(&mut *tx, elsewhere.location_id)
            .await?
            .map_or_else(|| format!("#{}", elsewhere.location_id), |l| l.name);
        return Err(refuse(english(
            "wl_attendance.self.open_elsewhere",
            &[("location", &name)],
        )));
    }
    Ok(())
}

/// Emits `AttendanceRecorded` for what is stored of `record`.
async fn emit_recorded(state: &AppState, record: &AttendanceRecord) -> Result {
    let flags: Vec<String> = AttendanceFlag::of_record(&state.db, record.id)
        .await?
        .into_iter()
        .map(|f| f.flag_type.as_str().to_owned())
        .collect();
    state
        .emit(AttendanceRecorded {
            record: record.record_id(),
            employee: record.employee(),
            location: wl_kernel::Subject::new(
                wl_kernel::attendance::WORK_LOCATION_SUBJECT_TYPE,
                record.location_id,
            ),
            date: record.date,
            method: record.method.as_str().to_owned(),
            within_geofence: None,
            flags,
        })
        .await
}

/// Runs the overlap check once more on what is stored, in its own
/// transaction (see the module docs).
async fn settle(state: &AppState, record: &AttendanceRecord) -> Result {
    let mut tx = state.db.begin().await?;
    let mut pending = Pending::default();
    overlap::flag_in(state, &mut tx, record, &mut pending).await?;
    tx.commit().await?;
    pending.finish(state).await
}

/// Saves attendance: `record` of §4.
///
/// # Errors
///
/// `NotFound` for an unknown location; `422` when the employee is not at
/// work, the day is outside the late-entry window, the check-out is not
/// after the check-in, or (a self check-in) the employee is checked in
/// already.
pub async fn record(state: &AppState, input: NewRecord) -> Result<AttendanceRecord> {
    let location = WorkLocation::find(&state.db, input.location)
        .await?
        .ok_or(Error::NotFound)?;
    let date = dubai_date(input.check_in);
    ensure_at_work(state, input.employee).await?;
    ensure_in_window(state, date)?;
    if input.check_out.is_some_and(|out| out <= input.check_in) {
        return Err(refuse(english("wl_attendance.record.times", &[])));
    }
    let prepared = checks::prepare(state, input.employee, &location, date).await?;
    let _lock = lock::take(input.employee.0, date).await;
    let mut tx = state.db.begin().await?;
    if input.method == AttendanceMethod::SelfCheckIn {
        ensure_not_checked_in(&mut tx, input.employee, &location, date).await?;
    }
    let mut record = AttendanceRecord {
        employee_id: input.employee.0,
        location_id: location.id,
        date,
        check_in: input.check_in,
        check_out: input.check_out,
        method: input.method,
        recorded_by: input.by,
        recorded_at: now(),
        breaks: Json(input.breaks),
        note: input.note,
        status: AttendanceStatus::Recorded,
        ..AttendanceRecord::default()
    };
    record.insert(&mut tx).await?;
    let mut pending = Pending::default();
    checks::run_in(state, &mut tx, &record, &location, &prepared, &mut pending).await?;
    tx.commit().await?;
    let stored = AttendanceRecord::find_or_404(&state.db, record.id).await?;
    emit_recorded(state, &stored).await?;
    pending.finish(state).await?;
    settle(state, &stored).await?;
    AttendanceRecord::find_or_404(&state.db, record.id).await
}

/// The employee checks in themselves at `at` (`attendance.self`): `record`
/// with the method `self_check_in`.
///
/// # Errors
///
/// As [`record`]; the message of a refusal names the location the employee is
/// still checked in at.
pub async fn check_in_self(
    state: &AppState,
    employee: EmployeeId,
    location: i64,
    at: DateTime,
    by: Option<i64>,
) -> Result<AttendanceRecord> {
    record(
        state,
        NewRecord {
            employee,
            location,
            check_in: at,
            check_out: None,
            breaks: Vec::new(),
            method: AttendanceMethod::SelfCheckIn,
            by,
            note: String::new(),
        },
    )
    .await
}

/// Checks the employee out of record `record_id` at `at`, and runs the
/// checks again on the closed span. The status does not change.
///
/// # Errors
///
/// `NotFound` for an unknown record; `422` for a record that is `locked` or
/// `rejected`, has a check-out already, or when `at` is not after the
/// check-in.
pub async fn check_out(state: &AppState, record_id: i64, at: DateTime) -> Result<AttendanceRecord> {
    let found = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    if matches!(
        found.status,
        AttendanceStatus::Locked | AttendanceStatus::Rejected
    ) {
        return Err(refuse(english("wl_attendance.record.locked", &[])));
    }
    if found.check_out.is_some() {
        return Err(refuse(english("wl_attendance.record.checked_out", &[])));
    }
    if at <= found.check_in {
        return Err(refuse(english("wl_attendance.record.times", &[])));
    }
    let location = WorkLocation::find(&state.db, found.location_id)
        .await?
        .ok_or(Error::NotFound)?;
    let prepared = checks::prepare(state, found.employee(), &location, found.date).await?;
    let _lock = lock::take(found.employee_id, found.date).await;
    let mut tx = state.db.begin().await?;
    AttendanceRecord::query()
        .where_(AttendanceRecord::ID.eq(record_id))
        .update(&mut tx, &[("check_out", &Some(at)), ("updated_at", &now())])
        .await?;
    let closed = AttendanceRecord::find_or_404(&mut tx, record_id).await?;
    let mut pending = Pending::default();
    checks::run_in(state, &mut tx, &closed, &location, &prepared, &mut pending).await?;
    tx.commit().await?;
    let stored = AttendanceRecord::find_or_404(&state.db, record_id).await?;
    emit_recorded(state, &stored).await?;
    pending.finish(state).await?;
    settle(state, &stored).await?;
    AttendanceRecord::find_or_404(&state.db, record_id).await
}
