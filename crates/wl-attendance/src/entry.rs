//! Timesheet entries (wl-attendance §3 *Timesheet entry*): hours on one day
//! against one cost object, by category, in minutes.
//!
//! - **Proposed from attendance.** `accept` (and `correct`) of a record
//!   proposes its worked minutes, less the breaks, on the owner's `open`
//!   attendance timesheet whose period holds the day ([`propose_in`]): one
//!   entry per record, category *normal*, the cost object the work
//!   location's scope (the project), non-billable when the record carries
//!   `permit_invalid`. The submitter then splits the hours into categories
//!   and cost codes. `reject` removes the entry again ([`unpropose_in`]).
//!   With no open timesheet nothing is proposed.
//! - **Entered by hand or generated** ([`add`]). Saving one runs the
//!   overtime cap of `HRM.RUL.13` on the day again ([`crate::overtime`]),
//!   through [`TimesheetEntryHours`], which tells that check the minutes of
//!   such entries (the ones proposed from attendance would count twice).
//! - **Signed in an adjustment.** The entries of an adjustment hold a signed
//!   delta; one that takes the original plus its approved adjustments below
//!   0 h for a day and category is refused on save (`HRM.ACL.08`).

use std::collections::BTreeSet;

use renox::chrono::NaiveDate;
use renox::db::{DateTime, Transaction, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::BusinessCalendar;
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

use crate::flags::{AttendanceFlag, FlagType};
use crate::location::WorkLocation;
use crate::lock;
use crate::overtime::{EntryHours, HoursFuture, worked_minutes};
use crate::record::AttendanceRecord;
use crate::service::refuse;
use crate::texts::english;
use crate::timesheet::{HoursSource, Timesheet, TimesheetKind, TimesheetStatus};

/// The category of hours (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    /// Normal hours.
    Normal,
    /// Overtime.
    Overtime,
    /// Night hours.
    Night,
    /// Work on a rest day.
    RestDay,
    /// Work on a public holiday.
    PublicHoliday,
}

impl Category {
    /// Every category.
    pub const ALL: [Category; 5] = [
        Category::Normal,
        Category::Overtime,
        Category::Night,
        Category::RestDay,
        Category::PublicHoliday,
    ];

    /// The category named `name` (`normal`, `overtime`, `night`, `rest_day`,
    /// `public_holiday`).
    pub fn parse(name: &str) -> Option<Category> {
        Some(match name.trim() {
            "normal" => Category::Normal,
            "overtime" => Category::Overtime,
            "night" => Category::Night,
            "rest_day" => Category::RestDay,
            "public_holiday" => Category::PublicHoliday,
            _ => return None,
        })
    }

    /// The name, as in a form.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Normal => "normal",
            Category::Overtime => "overtime",
            Category::Night => "night",
            Category::RestDay => "rest_day",
            Category::PublicHoliday => "public_holiday",
        }
    }
}

/// Minutes as hours for a message: `1 h`, `1 h 30 min`, `45 min`; a sign
/// kept.
pub fn hours_text(minutes: i64) -> String {
    let sign = if minutes < 0 { "-" } else { "" };
    let abs = minutes.abs();
    match (abs / 60, abs % 60) {
        (0, m) => format!("{sign}{m} min"),
        (h, 0) => format!("{sign}{h} h"),
        (h, m) => format!("{sign}{h} h {m} min"),
    }
}

/// Hours on one day against one cost object.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_attendance_timesheet_entries",
    index(employee_id, date),
    index(timesheet_id)
)]
pub struct TimesheetEntry {
    /// The primary key.
    pub id: i64,
    /// The timesheet.
    #[model(references = "wl_attendance_timesheets")]
    pub timesheet_id: i64,
    /// The employee whose hours these are.
    pub employee_id: i64,
    /// The day.
    pub date: NaiveDate,
    /// The type of the cost object's `Subject` (cost code, work order...).
    pub cost_object_type: Option<String>,
    /// The id of the cost object's `Subject`.
    pub cost_object_id: Option<i64>,
    /// Normal minutes.
    pub normal_minutes: i64,
    /// Overtime minutes.
    pub overtime_minutes: i64,
    /// Night minutes.
    pub night_minutes: i64,
    /// Minutes on a rest day.
    pub rest_day_minutes: i64,
    /// Minutes on a public holiday.
    pub public_holiday_minutes: i64,
    /// Whether the hours are billable.
    pub billable: bool,
    /// Where the hours come from.
    pub source: HoursSource,
    /// The attendance record the hours were proposed from.
    #[model(references = "wl_attendance_records")]
    pub attendance_record_id: Option<i64>,
    /// A note.
    pub note: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl TimesheetEntry {
    /// The cost object, if allocated.
    pub fn cost_object(&self) -> Option<Subject> {
        match (&self.cost_object_type, self.cost_object_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The minutes of `category`.
    pub fn minutes_of(&self, category: Category) -> i64 {
        match category {
            Category::Normal => self.normal_minutes,
            Category::Overtime => self.overtime_minutes,
            Category::Night => self.night_minutes,
            Category::RestDay => self.rest_day_minutes,
            Category::PublicHoliday => self.public_holiday_minutes,
        }
    }

    /// The minutes of every category.
    pub fn total_minutes(&self) -> i64 {
        Category::ALL.iter().map(|c| self.minutes_of(*c)).sum()
    }

    /// The entries of a timesheet, by day and id.
    pub async fn of_timesheet(db: &Db, timesheet_id: i64) -> Result<Vec<TimesheetEntry>> {
        TimesheetEntry::query()
            .where_(TimesheetEntry::TIMESHEET_ID.eq(timesheet_id))
            .order_by(TimesheetEntry::DATE)
            .order_by(TimesheetEntry::ID)
            .get(db)
            .await
    }
}

/// An entry to save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
    /// The day.
    pub date: NaiveDate,
    /// The cost object.
    pub cost_object: Option<Subject>,
    /// Minutes per category; signed in an adjustment.
    pub minutes: Vec<(Category, i64)>,
    /// Whether the hours are billable.
    pub billable: bool,
    /// Where the hours come from.
    pub source: HoursSource,
    /// A note.
    pub note: String,
}

impl NewEntry {
    /// `minutes` of normal hours on `date`, billable, entered by hand.
    pub fn normal(date: NaiveDate, minutes: i64) -> Self {
        NewEntry {
            date,
            cost_object: None,
            minutes: vec![(Category::Normal, minutes)],
            billable: true,
            source: HoursSource::Manual,
            note: String::new(),
        }
    }

    /// `minutes` of `category` on `date`.
    pub fn of(date: NaiveDate, category: Category, minutes: i64) -> Self {
        NewEntry {
            minutes: vec![(category, minutes)],
            ..NewEntry::normal(date, 0)
        }
    }

    /// Against `cost_object`.
    pub fn on(mut self, cost_object: &Subject) -> Self {
        self.cost_object = Some(cost_object.clone());
        self
    }

    /// Taken from attendance.
    pub fn from_attendance(mut self) -> Self {
        self.source = HoursSource::Attendance;
        self
    }

    /// Not billable.
    pub fn non_billable(mut self) -> Self {
        self.billable = false;
        self
    }

    /// With this note.
    pub fn note(mut self, note: &str) -> Self {
        self.note = note.trim().to_owned();
        self
    }

    pub(crate) fn minutes_of(&self, category: Category) -> i64 {
        self.minutes
            .iter()
            .filter(|(c, _)| *c == category)
            .map(|(_, m)| *m)
            .sum()
    }
}

/// Saves `new` on timesheet `timesheet_id`.
///
/// # Errors
///
/// `NotFound`; `422` when the timesheet is not `open`, the entry has no
/// hours, a regular entry has negative hours or a day outside the period, or
/// an adjustment's delta would take the original plus its approved
/// adjustments below 0 h for the day and category.
pub async fn add(state: &AppState, timesheet_id: i64, new: NewEntry) -> Result<TimesheetEntry> {
    let sheet = Timesheet::find_or_404(&state.db, timesheet_id).await?;
    if sheet.status != TimesheetStatus::Open {
        return Err(refuse(english("wl_attendance.entry.not_open", &[])));
    }
    if new.minutes.iter().all(|(_, m)| *m == 0) {
        return Err(refuse(english("wl_attendance.entry.empty", &[])));
    }
    if sheet.kind == TimesheetKind::Regular {
        if new.minutes.iter().any(|(_, m)| *m < 0) {
            return Err(refuse(english("wl_attendance.entry.negative", &[])));
        }
        if new.date < sheet.period_from || new.date > sheet.period_to {
            return Err(refuse(english(
                "wl_attendance.entry.outside_period",
                &[
                    ("date", &new.date),
                    ("from", &sheet.period_from),
                    ("to", &sheet.period_to),
                ],
            )));
        }
    }
    let saved = {
        // The same lock as `submit`: an entry saved while the timesheet is
        // checked waits for it.
        let _lock = lock::take(sheet.owner_employee_id, new.date).await;
        if sheet.kind == TimesheetKind::Adjustment {
            ensure_not_below_zero(state, &sheet, &new).await?;
        }
        let mut entry = TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: sheet.owner_employee_id,
            date: new.date,
            cost_object_type: new.cost_object.as_ref().map(|s| s.subject_type.clone()),
            cost_object_id: new.cost_object.as_ref().map(|s| s.subject_id),
            billable: new.billable,
            source: new.source,
            note: new.note.clone(),
            ..TimesheetEntry::default()
        };
        for category in Category::ALL {
            let minutes = new.minutes_of(category);
            match category {
                Category::Normal => entry.normal_minutes = minutes,
                Category::Overtime => entry.overtime_minutes = minutes,
                Category::Night => entry.night_minutes = minutes,
                Category::RestDay => entry.rest_day_minutes = minutes,
                Category::PublicHoliday => entry.public_holiday_minutes = minutes,
            }
        }
        TimesheetEntry::create(&state.db, entry).await?
    };
    if sheet.kind == TimesheetKind::Regular && saved.source != HoursSource::Attendance {
        crate::overtime::on_entry_saved(state, sheet.owner(), saved.date).await?;
    }
    Ok(saved)
}

/// Refuses a delta that takes the original plus its approved adjustments, and
/// this adjustment's own entries, below 0 h for the day and category.
async fn ensure_not_below_zero(state: &AppState, sheet: &Timesheet, new: &NewEntry) -> Result {
    let Some(original) = sheet.adjusts_id else {
        return Ok(());
    };
    let mut standing = standing_entries(&state.db, original, Some(sheet.id)).await?;
    standing.retain(|e| e.date == new.date);
    match below_zero(&standing, std::slice::from_ref(new)) {
        Some(message) => Err(refuse(message)),
        None => Ok(()),
    }
}

/// The entries that stand for the original `original` of an adjustment: its
/// own, those of its approved (or locked) adjustments, and, when `sheet` is
/// given, those of that adjustment so far.
pub(crate) async fn standing_entries(
    db: &Db,
    original: i64,
    sheet: Option<i64>,
) -> Result<Vec<TimesheetEntry>> {
    let mut sheets: Vec<i64> = Timesheet::query()
        .where_(Timesheet::ADJUSTS_ID.eq(original))
        .where_(Timesheet::STATUS.is_in([TimesheetStatus::Approved, TimesheetStatus::Locked]))
        .get(db)
        .await?
        .into_iter()
        .map(|t| t.id)
        .collect();
    sheets.push(original);
    sheets.extend(sheet);
    TimesheetEntry::query()
        .where_(TimesheetEntry::TIMESHEET_ID.is_in(sheets))
        .get(db)
        .await
}

/// The message that refuses `news` when, for a day and category, the minutes
/// that `standing` holds plus the deltas of `news` fall below zero; none when
/// they do not.
pub(crate) fn below_zero(standing: &[TimesheetEntry], news: &[NewEntry]) -> Option<String> {
    let mut keys: BTreeSet<(NaiveDate, &'static str)> = BTreeSet::new();
    for new in news {
        for (category, _) in &new.minutes {
            keys.insert((new.date, category.as_str()));
        }
    }
    for (date, name) in keys {
        let Some(category) = Category::parse(name) else {
            continue;
        };
        let delta: i64 = news
            .iter()
            .filter(|n| n.date == date)
            .map(|n| n.minutes_of(category))
            .sum();
        if delta >= 0 {
            continue;
        }
        let held: i64 = standing
            .iter()
            .filter(|e| e.date == date)
            .map(|e| e.minutes_of(category))
            .sum();
        if held + delta < 0 {
            return Some(english(
                "wl_attendance.entry.below_zero",
                &[
                    ("date", &date),
                    ("category", &name),
                    ("standing", &hours_text(held)),
                    ("delta", &hours_text(delta)),
                ],
            ));
        }
    }
    None
}

/// The open attendance timesheet of `employee` whose period holds `date`.
async fn open_sheet_in(
    tx: &mut Transaction,
    employee: i64,
    date: NaiveDate,
) -> Result<Option<Timesheet>> {
    Ok(Timesheet::query()
        .where_(Timesheet::OWNER_EMPLOYEE_ID.eq(employee))
        .where_(Timesheet::STATUS.eq(TimesheetStatus::Open))
        .where_(Timesheet::KIND.eq(TimesheetKind::Regular))
        .where_(Timesheet::HOURS_SOURCE.eq(HoursSource::Attendance))
        .where_(Timesheet::PERIOD_FROM.lte(date))
        .where_(Timesheet::PERIOD_TO.gte(date))
        .order_by(Timesheet::ID)
        .first(&mut *tx)
        .await?)
}

/// Proposes the hours of the `accepted` `record` on the open timesheet of
/// its employee, inside `tx` (§4 `accept` effects): a record with no
/// check-out has no hours yet and proposes none. Called again when the
/// record is corrected or checked out, it updates its entry.
///
/// # Errors
///
/// The database's errors.
pub async fn propose_in(tx: &mut Transaction, record: &AttendanceRecord) -> Result {
    let minutes = worked_minutes(&BusinessCalendar::default(), record);
    if minutes == 0 {
        return unpropose_in(tx, record.id).await;
    }
    let non_billable = AttendanceFlag::query()
        .where_(AttendanceFlag::RECORD_ID.eq(record.id))
        .where_(AttendanceFlag::FLAG_TYPE.eq(FlagType::PermitInvalid))
        .count(&mut *tx)
        .await?
        > 0;
    let held = TimesheetEntry::query()
        .where_(TimesheetEntry::ATTENDANCE_RECORD_ID.eq(record.id))
        .first(&mut *tx)
        .await?;
    if let Some(held) = held {
        let in_open = Timesheet::find(&mut *tx, held.timesheet_id)
            .await?
            .is_some_and(|sheet| sheet.status == TimesheetStatus::Open);
        if in_open {
            TimesheetEntry::query()
                .where_(TimesheetEntry::ID.eq(held.id))
                .update(
                    &mut *tx,
                    &[
                        ("date", &record.date),
                        ("normal_minutes", &minutes),
                        ("billable", &!non_billable),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
        }
        return Ok(());
    }
    let Some(sheet) = open_sheet_in(tx, record.employee_id, record.date).await? else {
        return Ok(());
    };
    let scope = WorkLocation::find(&mut *tx, record.location_id)
        .await?
        .and_then(|location| location.scope());
    TimesheetEntry::create(
        &mut *tx,
        TimesheetEntry {
            timesheet_id: sheet.id,
            employee_id: record.employee_id,
            date: record.date,
            cost_object_type: scope.as_ref().map(|s| s.subject_type.clone()),
            cost_object_id: scope.as_ref().map(|s| s.subject_id),
            normal_minutes: minutes,
            billable: !non_billable,
            source: HoursSource::Attendance,
            attendance_record_id: Some(record.id),
            ..TimesheetEntry::default()
        },
    )
    .await?;
    Ok(())
}

/// Removes the hours proposed from record `record_id` from the timesheets that
/// are still `open` (§4 `reject` effects).
///
/// # Errors
///
/// The database's errors.
pub async fn unpropose_in(tx: &mut Transaction, record_id: i64) -> Result {
    let entries = TimesheetEntry::query()
        .where_(TimesheetEntry::ATTENDANCE_RECORD_ID.eq(record_id))
        .get(&mut *tx)
        .await?;
    for entry in entries {
        let open = Timesheet::find(&mut *tx, entry.timesheet_id)
            .await?
            .is_some_and(|sheet| sheet.status == TimesheetStatus::Open);
        if open {
            TimesheetEntry::query()
                .where_(TimesheetEntry::ID.eq(entry.id))
                .delete(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

/// The minutes of the entries that do not come from attendance, for the
/// overtime cap (`HRM.RUL.13`): manual and generated entries of regular
/// timesheets, in any state. Adjustments are left out, and so are the hours
/// proposed from attendance, which the records count already.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimesheetEntryHours;

impl EntryHours for TimesheetEntryHours {
    fn minutes_on<'a>(
        &'a self,
        state: &'a AppState,
        employee: EmployeeId,
        date: NaiveDate,
    ) -> HoursFuture<'a> {
        Box::pin(async move {
            let entries = TimesheetEntry::query()
                .where_(TimesheetEntry::EMPLOYEE_ID.eq(employee.0))
                .where_(TimesheetEntry::DATE.eq(date))
                .where_(TimesheetEntry::SOURCE.is_in([HoursSource::Manual, HoursSource::Generated]))
                .get(&state.db)
                .await?;
            if entries.is_empty() {
                return Ok(0);
            }
            let ids: BTreeSet<i64> = entries.iter().map(|e| e.timesheet_id).collect();
            let regular: BTreeSet<i64> = Timesheet::query()
                .where_(Timesheet::ID.is_in(ids.into_iter().collect::<Vec<_>>()))
                .where_(Timesheet::KIND.eq(TimesheetKind::Regular))
                .get(&state.db)
                .await?
                .into_iter()
                .map(|t| t.id)
                .collect();
            Ok(entries
                .iter()
                .filter(|e| regular.contains(&e.timesheet_id))
                .map(TimesheetEntry::total_minutes)
                .sum())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_read_as_hours() {
        assert_eq!(hours_text(60), "1 h");
        assert_eq!(hours_text(90), "1 h 30 min");
        assert_eq!(hours_text(45), "45 min");
        assert_eq!(hours_text(-480), "-8 h");
    }

    #[test]
    fn an_entry_adds_up_its_categories() {
        let entry = TimesheetEntry {
            normal_minutes: 480,
            overtime_minutes: 60,
            ..TimesheetEntry::default()
        };
        assert_eq!(entry.total_minutes(), 540);
        assert_eq!(entry.minutes_of(Category::Overtime), 60);
        assert_eq!(Category::parse("rest_day"), Some(Category::RestDay));
        assert_eq!(Category::parse("x"), None);
    }

    #[test]
    fn a_new_entry_knows_its_minutes() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let new = NewEntry::of(day, Category::Overtime, -120);
        assert_eq!(new.minutes_of(Category::Overtime), -120);
        assert_eq!(new.minutes_of(Category::Normal), 0);
    }
}
