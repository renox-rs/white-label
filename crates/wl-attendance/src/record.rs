//! The attendance record and its machine (wl-attendance §3, §4): presence of
//! one employee on one day at one work location, with the check-in and
//! check-out times.
//!
//! The status moves only through the machine. A record is created
//! `recorded` ([`crate::service::record`]); the system `flag` moves a
//! `recorded` or `accepted` record to `flagged` when a flag rule matches
//! (outside geofence, midday break, missing check-out, overtime cap, permit
//! invalid, overlap), which keeps the history and the event. A reviewer
//! `accept`s, `correct`s or `reject`s it ([`crate::review`], `HRM.ACL.09`);
//! the system `accept`s a record with no flag (`accept_by_system`) and
//! `lock`s an accepted one when its timesheet is approved.
//!
//! ```text
//! [*] --record--> recorded
//! recorded | accepted --flag--> flagged
//! recorded | flagged --accept--> accepted
//! recorded | flagged | accepted --correct--> accepted
//! recorded | flagged --reject--> rejected
//! accepted --lock--> locked
//! ```
//!
//! Times are instants; a day is the Asia/Dubai calendar day of the check-in
//! (no daylight saving).

use renox::chrono::{Days, NaiveDate, NaiveTime, TimeDelta};
use renox::db::{DateTime, Json};
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use serde::{Deserialize, Serialize};
use wl_kernel::BusinessCalendar;
use wl_kernel::attendance::AttendanceRecordId;
use wl_kernel::workforce::EmployeeId;

use crate::review;

/// The system transition that holds a record for review.
pub const FLAG: &str = "flag";
/// A reviewer accepts a record (`attendance.review`).
pub const ACCEPT: &str = "accept";
/// The system accepts a record that has no flag, at check-out.
pub const ACCEPT_BY_SYSTEM: &str = "accept_by_system";
/// A user corrects the times of a record (`attendance.correct`).
pub const CORRECT: &str = "correct";
/// A reviewer rejects a record (`attendance.review`).
pub const REJECT: &str = "reject";
/// The system locks an accepted record when its timesheet is approved.
pub const LOCK: &str = "lock";

/// Where an attendance record stands (§4).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AttendanceStatus {
    /// Checked in; check-out may be missing.
    #[default]
    Recorded,
    /// Needs review.
    Flagged,
    /// Counts towards hours.
    Accepted,
    /// Does not count (duplicate, not worked).
    Rejected,
    /// Used by an approved timesheet; no further change.
    Locked,
}

impl AttendanceStatus {
    /// The states of a record that counts when records are compared:
    /// everything but `rejected`.
    pub const LIVE: [AttendanceStatus; 4] = [
        AttendanceStatus::Recorded,
        AttendanceStatus::Flagged,
        AttendanceStatus::Accepted,
        AttendanceStatus::Locked,
    ];

    /// Whether the system `flag` can move a record in this state.
    pub fn can_be_flagged(self) -> bool {
        matches!(self, AttendanceStatus::Recorded | AttendanceStatus::Accepted)
    }
}

/// How the record was made (§3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AttendanceMethod {
    /// Marked by a supervisor.
    #[default]
    Supervisor,
    /// The employee's own check-in in the PWA (the spec's `self`; `Self` is
    /// not a name a variant can take).
    SelfCheckIn,
    /// A biometric device (Later).
    Device,
    /// A file import.
    Import,
}

/// A break taken, in Asia/Dubai wall-clock time of the record's day.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BreakSpan {
    /// When the break starts.
    pub from: NaiveTime,
    /// When the break ends.
    pub to: NaiveTime,
}

impl BreakSpan {
    /// A break from `from` to `to` (hours and minutes).
    ///
    /// # Panics
    ///
    /// When an hour or minute is out of range.
    pub fn hm(from: (u32, u32), to: (u32, u32)) -> Self {
        BreakSpan {
            from: NaiveTime::from_hms_opt(from.0, from.1, 0).expect("a valid time"),
            to: NaiveTime::from_hms_opt(to.0, to.1, 0).expect("a valid time"),
        }
    }
}

/// The time span of a record, `start` included and `end` excluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// When the presence starts.
    pub start: DateTime,
    /// When it ends.
    pub end: DateTime,
}

impl Span {
    /// Whether the two spans share at least one minute. Spans that touch
    /// (one ends when the other starts) do not overlap, and an empty span
    /// overlaps nothing.
    pub fn overlaps(&self, other: &Span) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// One employee on one day at one work location.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_attendance_records",
    state_machine,
    index(employee_id, date),
    index(location_id, date)
)]
pub struct AttendanceRecord {
    /// The primary key.
    pub id: i64,
    /// The employee (a wl-hr employee id).
    pub employee_id: i64,
    /// The work location.
    #[model(references = "wl_attendance_work_locations")]
    pub location_id: i64,
    /// The day, in Asia/Dubai.
    pub date: NaiveDate,
    /// When the employee checked in.
    pub check_in: DateTime,
    /// When the employee checked out; missing while checked in.
    pub check_out: Option<DateTime>,
    /// How the record was made.
    pub method: AttendanceMethod,
    /// The user who made it: the supervisor, or the employee for a self
    /// check-in; none for an import.
    pub recorded_by: Option<i64>,
    /// When the record was saved.
    pub recorded_at: DateTime,
    /// The breaks taken.
    pub breaks: Json<Vec<BreakSpan>>,
    /// A note.
    pub note: String,
    /// Where it stands; changed only by the machine.
    pub status: AttendanceStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl StateMachine for AttendanceRecord {
    type State = AttendanceStatus;
    const COLUMN: renox::db::Col<AttendanceRecord, AttendanceStatus> = AttendanceRecord::STATUS;

    fn machine() -> Machine<Self> {
        use AttendanceStatus::{Accepted, Flagged, Locked, Recorded, Rejected};
        Machine::new()
            .initial([Recorded])
            .state(Recorded)
            .tone("neutral")
            .state(Flagged)
            .tone("warning")
            .state(Accepted)
            .tone("success")
            .state(Rejected)
            .tone("neutral")
            .terminal()
            .state(Locked)
            .tone("success")
            .terminal()
            .transition(
                Transition::new(FLAG)
                    .label("Flag")
                    .system()
                    .from([Recorded, Accepted])
                    .to(Flagged),
            )
            .transition(
                Transition::new(ACCEPT)
                    .label("Accept")
                    .tone("primary")
                    .from([Recorded, Flagged])
                    .to(Accepted)
                    .effect(|record: &AttendanceRecord, ctx, tx| {
                        let record = record.clone();
                        let who = review::Who::of(ctx);
                        Box::pin(async move { review::accept_in(tx, &record, &who).await })
                    }),
            )
            .transition(
                Transition::new(ACCEPT_BY_SYSTEM)
                    .label("Accept")
                    .system()
                    .from([Recorded])
                    .to(Accepted)
                    .effect(|record: &AttendanceRecord, _ctx, tx| {
                        let record = record.clone();
                        Box::pin(async move { review::accept_by_system_in(tx, &record).await })
                    }),
            )
            .transition(
                Transition::new(CORRECT)
                    .label("Correct")
                    .from([Recorded, Flagged, Accepted])
                    .to(Accepted)
                    .reason_required()
                    .effect(|record: &AttendanceRecord, ctx, tx| {
                        let record = record.clone();
                        let who = review::Who::of(ctx);
                        let input = ctx.input_as::<review::CorrectInput>();
                        Box::pin(async move {
                            // Without the corrected values there is nothing to
                            // write: refuse rather than accept unchanged.
                            let input = input.map_err(|_| {
                                crate::service::refuse(crate::texts::english(
                                    "wl_attendance.review.no_correction",
                                    &[],
                                ))
                            })?;
                            review::correct_in(tx, &record, &who, &input).await
                        })
                    }),
            )
            .transition(
                Transition::new(REJECT)
                    .label("Reject")
                    .tone("danger")
                    .from([Recorded, Flagged])
                    .to(Rejected)
                    .reason_required()
                    .effect(|record: &AttendanceRecord, ctx, tx| {
                        let record = record.clone();
                        let who = review::Who::of(ctx);
                        Box::pin(async move { review::reject_in(tx, &record, &who).await })
                    }),
            )
            .transition(
                Transition::new(LOCK)
                    .label("Lock")
                    .system()
                    .from([Accepted])
                    .to(Locked),
            )
    }
}

impl AttendanceRecord {
    /// The record as an id.
    pub fn record_id(&self) -> AttendanceRecordId {
        AttendanceRecordId(self.id)
    }

    /// The employee as an id.
    pub fn employee(&self) -> EmployeeId {
        EmployeeId(self.employee_id)
    }

    /// Whether the record has a check-in and no check-out.
    pub fn is_open(&self) -> bool {
        self.check_out.is_none()
    }

    /// The time span of the record. A record with no check-out counts as
    /// running until `now` for a self check-in, and until the end of its day
    /// for any other method (§4 *Terms*, *Overlap*). The span never ends
    /// before it starts.
    pub fn span(&self, calendar: &BusinessCalendar, now: DateTime) -> Span {
        let end = match self.check_out {
            Some(out) => out,
            None if self.method == AttendanceMethod::SelfCheckIn => now,
            None => end_of_day(calendar, self.date),
        };
        Span {
            start: self.check_in,
            end: end.max(self.check_in),
        }
    }

    /// Whether the record counts when records are compared: it is not
    /// `rejected`.
    pub fn counts(&self) -> bool {
        AttendanceStatus::LIVE.contains(&self.status)
    }
}

/// The instant a day ends: 00:00 of the next day in Asia/Dubai.
pub fn end_of_day(calendar: &BusinessCalendar, date: NaiveDate) -> DateTime {
    let next = date.checked_add_days(Days::new(1)).unwrap_or(date);
    calendar.from_local(next.and_time(NaiveTime::MIN))
}

/// The minutes between two instants, zero when `to` is not after `from`.
pub fn minutes_between(from: DateTime, to: DateTime) -> i64 {
    (to - from).max(TimeDelta::zero()).num_minutes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime {
        text.parse().unwrap()
    }

    #[test]
    fn a_new_record_is_recorded_and_flag_is_a_system_transition() {
        let machine = AttendanceRecord::machine();
        assert_eq!(machine.initial_states(), ["recorded"]);
        let flag = machine.find(FLAG).unwrap();
        assert!(flag.is_system());
        assert_eq!(flag.from_states(), ["recorded", "accepted"]);
        assert_eq!(flag.to_state(), Some("flagged"));
        let names: Vec<&str> = machine.transitions().iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            ["flag", "accept", "accept_by_system", "correct", "reject", "lock"]
        );
        assert_eq!(machine.find(ACCEPT).unwrap().from_states(), ["recorded", "flagged"]);
        assert_eq!(
            machine.find(CORRECT).unwrap().from_states(),
            ["recorded", "flagged", "accepted"]
        );
        assert_eq!(machine.find(REJECT).unwrap().to_state(), Some("rejected"));
        assert!(machine.find(ACCEPT_BY_SYSTEM).unwrap().is_system());
        assert!(machine.find(LOCK).unwrap().is_system());
        assert!(!machine.find(ACCEPT).unwrap().is_system());
    }

    #[test]
    fn spans_that_touch_do_not_overlap() {
        let a = Span {
            start: at("2026-11-02T03:00:00Z"),
            end: at("2026-11-02T08:00:00Z"),
        };
        let touching = Span {
            start: at("2026-11-02T08:00:00Z"),
            end: at("2026-11-02T12:00:00Z"),
        };
        let inside = Span {
            start: at("2026-11-02T07:59:00Z"),
            end: at("2026-11-02T12:00:00Z"),
        };
        assert!(!a.overlaps(&touching));
        assert!(a.overlaps(&inside) && inside.overlaps(&a));
        let empty = Span {
            start: at("2026-11-02T05:00:00Z"),
            end: at("2026-11-02T05:00:00Z"),
        };
        assert!(!a.overlaps(&empty));
    }

    #[test]
    fn an_open_record_runs_to_now_for_a_self_check_in_and_to_midnight_otherwise() {
        let calendar = BusinessCalendar::default();
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let open = AttendanceRecord {
            date: day,
            check_in: at("2026-11-02T03:00:00Z"),
            method: AttendanceMethod::SelfCheckIn,
            ..AttendanceRecord::default()
        };
        let now = at("2026-11-02T05:00:00Z");
        assert_eq!(open.span(&calendar, now).end, now);
        // Before its own check-in the span is empty, never negative.
        assert_eq!(open.span(&calendar, at("2026-11-02T01:00:00Z")).end, open.check_in);
        let marked = AttendanceRecord {
            method: AttendanceMethod::Supervisor,
            ..open
        };
        // Midnight in Dubai is 20:00 UTC the evening before.
        assert_eq!(marked.span(&calendar, now).end, at("2026-11-02T20:00:00Z"));
    }

    #[test]
    fn minutes_never_go_below_zero() {
        assert_eq!(
            minutes_between(at("2026-11-02T03:00:00Z"), at("2026-11-02T04:30:00Z")),
            90
        );
        assert_eq!(
            minutes_between(at("2026-11-02T04:30:00Z"), at("2026-11-02T03:00:00Z")),
            0
        );
    }
}
