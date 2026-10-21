//! Hours for payroll at the cut-off (wl-attendance §5.7 points 1 and 2, §6
//! `Timesheets`, §7 *Payroll basis*; BL-ATT-12).
//!
//! Salary does not wait for the client. [`AttendanceTimesheets`] is the
//! module's answer to the kernel's
//! [`Timesheets`](wl_kernel::timesheets::Timesheets):
//!
//! - **`hours_for_payroll(period)`** gives, for the regular timesheets whose
//!   last day lies in the payroll period, the hours of every `approved` (or
//!   `locked`) one that no payroll run used yet, as they are, and for a
//!   timesheet still `open` or `submitted` the accepted attendance hours of
//!   its owner, marked *provisional*. Only `accepted` attendance counts: a
//!   record still `flagged` at the cut-off is left out and its hours come
//!   later through the reconciliation (BL-ATT-13) once the timesheet is
//!   approved. It has no side effect and gives the same answer
//!   until the data changes.
//! - **`record_payroll_use(run, used)`**, told by payroll at `submit`, marks
//!   the timesheets: an approved one is `lock`ed with `paid_by` = the run; one
//!   paid on provisional hours gets `paid_provisionally_by` = the run and the
//!   hours that were paid are kept ([`ProvisionalHours`]). From then on a
//!   provisionally paid timesheet counts as consumed and is not reopened
//!   without an adjustment. A run that is returned or cancelled marks
//!   nothing, because payroll tells only at `submit`. Told twice, the same run
//!   changes nothing.
//! - When a provisionally paid timesheet is approved it is `lock`ed at once,
//!   payroll being its first consumer; `paid_provisionally_by` stays as the
//!   consumer and no `paid_by` is added ([`lock_when_paid_provisionally`]).
//!
//! The payroll basis is a setting ([`crate::config::PayrollBasis`]):
//! `attendance` (default) or `wait`, which is only for approvers who are the
//! employer's own staff. With `wait`, an unapproved timesheet gives no hours.
//! An external approver's timesheet is always paid on attendance, and choosing
//! `wait` for it is refused in the settings.
//!
//! Decisions:
//!
//! - **Which timesheets belong to a payroll period.** A timesheet belongs to
//!   the payroll period that holds its last day, so a timesheet is paid by
//!   one run only. The provisional hours of an unapproved timesheet are those
//!   of its days up to the cut-off (and up to the period's end); what happens
//!   after the cut-off is picked up by the reconciliation.
//! - **Provisional hours are `normal`.** The split into overtime, rest days
//!   and holidays is the submitter's, on the timesheet; attendance gives the
//!   worked minutes, less the breaks, as the proposal of
//!   [`crate::proposal`] does. The reconciliation then pays the difference
//!   per category.
//! - **Only attendance timesheets get provisional hours.** A timesheet of
//!   hours entered by hand or generated from sessions has no attendance to
//!   pay on, so it gives nothing until it is approved.
//! - **One record is paid once** in a call, even when two timesheets of the
//!   owner overlap.
//! - **A timesheet with `paid_provisionally_by` is not offered again**, also
//!   when it was sent back to `open`: its hours were paid.

use std::collections::{BTreeMap, BTreeSet};

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::BusinessCalendar;
use wl_kernel::Subject;
use wl_kernel::attendance::LabourFuture;
use wl_kernel::calendar::calendar;
use wl_kernel::timesheets::{
    HoursLine, PayrollHours, PayrollPeriod, TIMESHEET_SUBJECT_TYPE, Timesheets,
};

use crate::approval::kind_text;
use crate::config::{PayrollBasis, config_of};
use crate::entry::{Category, TimesheetEntry};
use crate::overtime::worked_minutes;
use crate::record::{AttendanceRecord, AttendanceStatus};
use crate::service::refuse;
use crate::texts::english;
use crate::timesheet::{self, Consumer, HoursSource, Timesheet, TimesheetKind, TimesheetStatus};

/// The hours a payroll run paid for a timesheet that the client had not yet
/// approved: one row per day and category, as paid.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_attendance_provisional_hours", index(timesheet_id))]
pub struct ProvisionalHours {
    /// The primary key.
    pub id: i64,
    /// The timesheet.
    #[model(references = "wl_attendance_timesheets")]
    pub timesheet_id: i64,
    /// The payroll run, `hr.payroll_run:5`.
    pub payroll_run: String,
    /// The day.
    pub date: NaiveDate,
    /// `normal`, `overtime`, ...
    pub category: String,
    /// The minutes paid.
    pub minutes: i64,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ProvisionalHours {
    /// The hours paid provisionally for `timesheet_id`, by day and category.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn of_timesheet(db: &Db, timesheet_id: i64) -> Result<Vec<ProvisionalHours>> {
        ProvisionalHours::query()
            .where_(ProvisionalHours::TIMESHEET_ID.eq(timesheet_id))
            .order_by(ProvisionalHours::DATE)
            .order_by(ProvisionalHours::ID)
            .get(db)
            .await
    }
}

/// The hours of an `approved` or `locked` timesheet: its entries, per day
/// and category.
async fn approved_lines(db: &Db, sheet: &Timesheet) -> Result<Vec<HoursLine>> {
    let mut sums: BTreeMap<(NaiveDate, usize), i64> = BTreeMap::new();
    for entry in TimesheetEntry::of_timesheet(db, sheet.id).await? {
        for (n, category) in Category::ALL.iter().enumerate() {
            *sums.entry((entry.date, n)).or_default() += entry.minutes_of(*category);
        }
    }
    Ok(sums
        .into_iter()
        .filter(|(_, minutes)| *minutes != 0)
        .map(|((date, n), minutes)| HoursLine {
            date,
            category: Category::ALL[n].as_str().to_owned(),
            minutes,
        })
        .collect())
}

/// The provisional hours of an unapproved `sheet`: the worked minutes of the
/// owner's `accepted` records from the first day of the timesheet to the
/// cut-off (or its last day, or the end of the payroll period), as `normal`
/// hours. A record already counted for another timesheet in `counted` is
/// left out.
async fn provisional_lines(
    state: &AppState,
    calendar: &BusinessCalendar,
    sheet: &Timesheet,
    period: &PayrollPeriod,
    counted: &mut BTreeSet<i64>,
) -> Result<Vec<HoursLine>> {
    let to = sheet.period_to.min(period.cut_off).min(period.to);
    let records = AttendanceRecord::query()
        .where_(AttendanceRecord::EMPLOYEE_ID.eq(sheet.owner_employee_id))
        .where_(AttendanceRecord::STATUS.eq(AttendanceStatus::Accepted))
        .where_(AttendanceRecord::DATE.gte(sheet.period_from))
        .where_(AttendanceRecord::DATE.lte(to))
        .order_by(AttendanceRecord::DATE)
        .order_by(AttendanceRecord::ID)
        .get(&state.db)
        .await?;
    let mut by_day: BTreeMap<NaiveDate, i64> = BTreeMap::new();
    for record in records {
        if !counted.insert(record.id) {
            continue;
        }
        let minutes = worked_minutes(calendar, &record);
        if minutes > 0 {
            *by_day.entry(record.date).or_default() += minutes;
        }
    }
    Ok(by_day
        .into_iter()
        .map(|(date, minutes)| HoursLine {
            date,
            category: Category::Normal.as_str().to_owned(),
            minutes,
        })
        .collect())
}

/// The hours payroll pays for `period` (see the module docs).
///
/// # Errors
///
/// The database's errors.
pub async fn hours_for_payroll(
    state: &AppState,
    period: &PayrollPeriod,
) -> Result<Vec<PayrollHours>> {
    let config = config_of(state);
    let calendar = calendar(state);
    let sheets = Timesheet::query()
        .where_(Timesheet::KIND.eq(TimesheetKind::Regular))
        .where_(Timesheet::PERIOD_TO.gte(period.from))
        .where_(Timesheet::PERIOD_TO.lte(period.to))
        .order_by(Timesheet::ID)
        .get(&state.db)
        .await?;
    let mut counted = BTreeSet::new();
    let mut hours = Vec::new();
    for sheet in sheets {
        // What payroll paid already is not offered again.
        if sheet.paid_by.is_some() || sheet.paid_provisionally_by.is_some() {
            continue;
        }
        let (lines, provisional) = match sheet.status {
            TimesheetStatus::Approved | TimesheetStatus::Locked => {
                (approved_lines(&state.db, &sheet).await?, false)
            }
            TimesheetStatus::Open | TimesheetStatus::Submitted => {
                if sheet.hours_source != HoursSource::Attendance
                    || config.payroll_basis_for(sheet.approver_kind) == PayrollBasis::Wait
                {
                    continue;
                }
                let lines =
                    provisional_lines(state, &calendar, &sheet, period, &mut counted).await?;
                if lines.is_empty() {
                    continue;
                }
                (lines, true)
            }
        };
        hours.push(PayrollHours {
            timesheet: sheet.subject(),
            owner: sheet.owner(),
            period_from: sheet.period_from,
            period_to: sheet.period_to,
            context: sheet.context(),
            approver_kind: kind_text(sheet.approver_kind).to_owned(),
            provisional,
            lines,
        });
    }
    Ok(hours)
}

/// The id of the timesheet `subject`, refused when it is another record.
fn timesheet_id(subject: &Subject) -> Result<i64> {
    if subject.subject_type == TIMESHEET_SUBJECT_TYPE {
        Ok(subject.subject_id)
    } else {
        Err(refuse(english(
            "wl_attendance.payroll.not_a_timesheet",
            &[("subject", subject)],
        )))
    }
}

fn already_paid(sheet: &Timesheet, run: &Subject) -> Error {
    refuse(english(
        "wl_attendance.payroll.already_paid",
        &[("timesheet", &sheet.subject()), ("run", run)],
    ))
}

/// The approved timesheet `sheet` was paid by `run`: it is `lock`ed with
/// `paid_by` = the run; one billing locked first only gets `paid_by`.
async fn mark_paid(state: &AppState, sheet: Timesheet, run: &Subject) -> Result {
    let reference = run.to_string();
    if sheet.paid_by.as_deref() == Some(reference.as_str()) {
        return Ok(());
    }
    if sheet.paid_by.is_some() || sheet.paid_provisionally_by.is_some() {
        return Err(already_paid(&sheet, run));
    }
    match sheet.status {
        TimesheetStatus::Approved => {
            timesheet::lock(state, sheet.id, Consumer::Payroll, &reference).await?;
        }
        TimesheetStatus::Locked => {
            Timesheet::query()
                .where_(Timesheet::ID.eq(sheet.id))
                .where_(Timesheet::PAID_BY.is_null())
                .update(&state.db, &[("paid_by", &Some(reference))])
                .await?;
        }
        _ => {
            return Err(refuse(english(
                "wl_attendance.payroll.not_approved",
                &[("timesheet", &sheet.subject())],
            )));
        }
    }
    Ok(())
}

/// The unapproved timesheet `sheet` was paid on the provisional `hours` by
/// `run`: `paid_provisionally_by` is set and the hours paid are kept, in one
/// transaction.
async fn mark_provisional(
    state: &AppState,
    sheet: Timesheet,
    run: &Subject,
    hours: &PayrollHours,
) -> Result {
    let reference = run.to_string();
    if sheet.paid_provisionally_by.as_deref() == Some(reference.as_str()) {
        return Ok(());
    }
    if sheet.paid_by.is_some() || sheet.paid_provisionally_by.is_some() {
        return Err(already_paid(&sheet, run));
    }
    if !matches!(
        sheet.status,
        TimesheetStatus::Open | TimesheetStatus::Submitted | TimesheetStatus::Approved
    ) {
        return Err(refuse(english(
            "wl_attendance.payroll.not_unapproved",
            &[("timesheet", &sheet.subject())],
        )));
    }
    let mut tx = state.db.begin().await?;
    let claimed = Timesheet::query()
        .where_(Timesheet::ID.eq(sheet.id))
        .where_(Timesheet::PAID_PROVISIONALLY_BY.is_null())
        .update(
            &mut tx,
            &[("paid_provisionally_by", &Some(reference.clone()))],
        )
        .await?;
    if claimed == 0 {
        return Err(already_paid(&sheet, run));
    }
    for line in &hours.lines {
        ProvisionalHours::create(
            &mut tx,
            ProvisionalHours {
                timesheet_id: sheet.id,
                payroll_run: reference.clone(),
                date: line.date,
                category: line.category.clone(),
                minutes: line.minutes,
                ..ProvisionalHours::default()
            },
        )
        .await?;
    }
    tx.commit().await?;
    // Approved between the calculation and the submit: payroll is the first
    // consumer now.
    let stored = Timesheet::find_or_404(&state.db, sheet.id).await?;
    lock_when_paid_provisionally(state, stored).await?;
    Ok(())
}

/// Tells the timesheets that payroll run `run` used `used` (see the module
/// docs).
///
/// # Errors
///
/// `422` for something that is not a timesheet, a timesheet another run
/// already paid, or a state that cannot be paid; the database's errors.
pub async fn record_use(state: &AppState, run: &Subject, used: &[PayrollHours]) -> Result {
    for hours in used {
        let id = timesheet_id(&hours.timesheet)?;
        let sheet = Timesheet::find_or_404(&state.db, id).await?;
        if hours.provisional {
            mark_provisional(state, sheet, run, hours).await?;
        } else {
            mark_paid(state, sheet, run).await?;
        }
    }
    Ok(())
}

/// Locks `sheet` at once when it is `approved` and payroll paid it on
/// provisional hours: payroll is its first consumer, `paid_provisionally_by`
/// stays and no `paid_by` is added. Any other timesheet is returned as it
/// is.
///
/// # Errors
///
/// The errors of `lock`.
pub async fn lock_when_paid_provisionally(state: &AppState, sheet: Timesheet) -> Result<Timesheet> {
    match (&sheet.status, sheet.paid_provisionally_by.clone()) {
        (TimesheetStatus::Approved, Some(reference)) => {
            timesheet::lock(state, sheet.id, Consumer::PayrollProvisional, &reference).await
        }
        _ => Ok(sheet),
    }
}

/// The module's [`Timesheets`] for payroll.
#[derive(Debug, Clone, Copy, Default)]
pub struct AttendanceTimesheets;

impl Timesheets for AttendanceTimesheets {
    fn hours_for_payroll<'a>(
        &'a self,
        state: &'a AppState,
        period: &'a PayrollPeriod,
    ) -> LabourFuture<'a, Vec<PayrollHours>> {
        Box::pin(async move { hours_for_payroll(state, period).await })
    }

    fn record_payroll_use<'a>(
        &'a self,
        state: &'a AppState,
        run: &'a Subject,
        used: &'a [PayrollHours],
    ) -> LabourFuture<'a, ()> {
        Box::pin(async move { record_use(state, run, used).await })
    }
}
