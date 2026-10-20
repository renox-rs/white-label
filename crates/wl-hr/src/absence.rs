//! The absence of an employee (wl-hr §3 *Employee*: contact attempts during
//! an absence and the MoHRE absence report; §6 *Consumes* apps absence case;
//! issue #197).
//!
//! - **Absence case.** The verticals open one after N consecutive days
//!   without attendance or approved leave (`MPW.RUL.09`, `CON.RUL.08`) and say
//!   so with [`AbsenceCaseOpened`] and [`AbsenceCaseClosed`]; [`AbsenceCase`]
//!   keeps them. `report_absconding` needs one open, and counts its days from
//!   the last day with attendance that the event carries: the attendance
//!   records belong to wl-attendance, and the vertical's rule counts from
//!   them, so wl-hr does not read them a second time. The guard has to run
//!   inside the approval's transaction, where only the database is at hand.
//! - **Contact attempts.** HR records each attempt to reach the worker: date,
//!   method, by whom, outcome ([`record_contact_attempt`]). Two failed
//!   attempts on two different days after the last attended day show that the
//!   worker cannot be reached (ER Art. 28).
//! - **The MoHRE absence report** ([`AbsenceReport`]) is written when the
//!   exit is approved; it carries the report number and date, the report file,
//!   the withdrawal reference and the day the worker returned (for
//!   `reinstate`), and the time the portal and attendance were switched off.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::workforce::{AbsenceCaseClosed, AbsenceCaseOpened};

use crate::employee::{Employee, EmployeeStatus};
use crate::texts::english;

/// An absence case of a vertical, as wl-hr keeps it.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_hr_absence_cases",
    unique(case_type, case_id),
    index(employee_id, closed_on)
)]
pub struct AbsenceCase {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The case's subject type, e.g. `ticketing.ticket`.
    pub case_type: String,
    /// The case's id.
    pub case_id: i64,
    /// The day it was opened.
    pub opened_on: NaiveDate,
    /// The last day with attendance or approved leave.
    pub last_present_on: NaiveDate,
    /// The day it was closed; empty while it is open.
    pub closed_on: Option<NaiveDate>,
    /// Why it was closed.
    pub close_reason: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AbsenceCase {
    /// The open cases of `employee`, oldest first.
    pub async fn open_of(db: &Db, employee: i64) -> Result<Vec<AbsenceCase>> {
        AbsenceCase::query()
            .where_(AbsenceCase::EMPLOYEE_ID.eq(employee))
            .where_(AbsenceCase::CLOSED_ON.is_null())
            .order_by(AbsenceCase::ID)
            .get(db)
            .await
    }
}

/// How an attempt to reach the worker ended.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ContactOutcome {
    /// Nobody answered.
    #[default]
    NoAnswer,
    /// The number or address does not work.
    Unreachable,
    /// The worker was reached.
    Reached,
}

impl ContactOutcome {
    /// Whether the attempt failed: *no answer* or an equivalent.
    pub fn failed(self) -> bool {
        matches!(self, ContactOutcome::NoAnswer | ContactOutcome::Unreachable)
    }
}

/// One attempt to reach an absent worker.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_contact_attempts", index(employee_id, attempted_on))]
pub struct ContactAttempt {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The day of the attempt.
    pub attempted_on: NaiveDate,
    /// How: phone, SMS, visit, ...
    pub method: String,
    /// The user who tried.
    pub by_user_id: Option<i64>,
    /// How it ended.
    pub outcome: ContactOutcome,
    /// A note.
    pub note: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl ContactAttempt {
    /// The attempts at `employee`, oldest first.
    pub async fn of_employee(db: &Db, employee: i64) -> Result<Vec<ContactAttempt>> {
        ContactAttempt::query()
            .where_(ContactAttempt::EMPLOYEE_ID.eq(employee))
            .order_by(ContactAttempt::ATTEMPTED_ON)
            .order_by(ContactAttempt::ID)
            .get(db)
            .await
    }
}

/// The MoHRE absence report of an employee.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_hr_absence_reports", index(employee_id, status))]
pub struct AbsenceReport {
    /// The primary key.
    pub id: i64,
    /// The employee.
    #[model(references = "wl_hr_employees")]
    pub employee_id: i64,
    /// The report number.
    pub report_number: String,
    /// The report date.
    pub report_date: NaiveDate,
    /// The report file, a stored document.
    pub report_file_id: Option<i64>,
    /// The day the exit was approved.
    pub reported_on: NaiveDate,
    /// `valid` or `withdrawn`.
    pub status: String,
    /// The authority's reference of the withdrawal or cancellation.
    pub withdrawal_reference: Option<String>,
    /// The day the worker returned.
    pub returned_on: Option<NaiveDate>,
    /// When the portal and attendance were switched off.
    pub access_disabled_at: Option<DateTime>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl AbsenceReport {
    /// The report in force for `employee`, if any.
    pub async fn valid_for(db: &Db, employee: i64) -> Result<Option<AbsenceReport>> {
        AbsenceReport::query()
            .where_(AbsenceReport::EMPLOYEE_ID.eq(employee))
            .where_(AbsenceReport::STATUS.eq("valid"))
            .order_by_desc(AbsenceReport::ID)
            .first(db)
            .await
    }
}

/// `AbsenceCaseOpened`: keeps the case; a second event for the same case
/// changes nothing.
pub(crate) async fn on_case_opened(state: &AppState, event: &AbsenceCaseOpened) -> Result {
    if Employee::find(&state.db, event.employee.0).await?.is_none() {
        return Ok(());
    }
    let known = AbsenceCase::query()
        .where_(AbsenceCase::CASE_TYPE.eq(event.case.subject_type.as_str()))
        .where_(AbsenceCase::CASE_ID.eq(event.case.subject_id))
        .first(&state.db)
        .await?;
    if known.is_some() {
        return Ok(());
    }
    AbsenceCase::create(
        &state.db,
        AbsenceCase {
            employee_id: event.employee.0,
            case_type: event.case.subject_type.clone(),
            case_id: event.case.subject_id,
            opened_on: event.opened_on,
            last_present_on: event.last_present_on,
            ..AbsenceCase::default()
        },
    )
    .await?;
    Ok(())
}

/// `AbsenceCaseClosed`: the case is no longer open.
pub(crate) async fn on_case_closed(state: &AppState, event: &AbsenceCaseClosed) -> Result {
    AbsenceCase::query()
        .where_(AbsenceCase::CASE_TYPE.eq(event.case.subject_type.as_str()))
        .where_(AbsenceCase::CASE_ID.eq(event.case.subject_id))
        .where_(AbsenceCase::CLOSED_ON.is_null())
        .update(
            &state.db,
            &[
                ("closed_on", &Some(event.closed_on)),
                ("close_reason", &Some(event.reason.clone())),
                ("updated_at", &renox::db::now()),
            ],
        )
        .await?;
    Ok(())
}

/// Why a contact attempt was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptRefused {
    /// The method (phone, SMS, visit, ...) is blank.
    MethodRequired,
    /// The day is after today.
    FutureDate,
    /// The employee is not working or absconded: there is nobody to reach.
    NotAbsent,
}

impl AttemptRefused {
    /// The text for the refusal.
    pub fn message(self) -> String {
        english(
            match self {
                AttemptRefused::MethodRequired => "wl_hr.absence.method_required",
                AttemptRefused::FutureDate => "wl_hr.absence.future_date",
                AttemptRefused::NotAbsent => "wl_hr.absence.not_absent",
            },
            &[],
        )
    }

    /// The refusal as a 422 error.
    pub fn into_error(self) -> Error {
        Error::Status(StatusCode::UNPROCESSABLE_ENTITY, self.message())
    }
}

/// HR (`by`) records an attempt to reach `employee_id` on `attempted_on`
/// (today at the latest): how, and how it ended.
///
/// # Errors
///
/// `NotFound` for an unknown employee; the inner [`AttemptRefused`] for a
/// blank method, a future day, or an employee who is not working.
pub async fn record_contact_attempt(
    state: &AppState,
    by: i64,
    employee_id: i64,
    attempted_on: NaiveDate,
    method: &str,
    outcome: ContactOutcome,
    note: Option<&str>,
) -> Result<std::result::Result<ContactAttempt, AttemptRefused>> {
    let employee = Employee::find_or_404(&state.db, employee_id).await?;
    if method.trim().is_empty() {
        return Ok(Err(AttemptRefused::MethodRequired));
    }
    if attempted_on > today() {
        return Ok(Err(AttemptRefused::FutureDate));
    }
    let reachable = matches!(
        employee.status,
        EmployeeStatus::Active
            | EmployeeStatus::OnLeave
            | EmployeeStatus::Notice
            | EmployeeStatus::Absconded
    );
    if !reachable {
        return Ok(Err(AttemptRefused::NotAbsent));
    }
    let note = note
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned);
    let attempt = ContactAttempt::create(
        &state.db,
        ContactAttempt {
            employee_id,
            attempted_on,
            method: method.trim().to_owned(),
            by_user_id: Some(by),
            outcome,
            note,
            ..ContactAttempt::default()
        },
    )
    .await?;
    Ok(Ok(attempt))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_answer_and_unreachable_are_failed_attempts() {
        assert!(ContactOutcome::NoAnswer.failed());
        assert!(ContactOutcome::Unreachable.failed());
        assert!(!ContactOutcome::Reached.failed());
    }

    #[test]
    fn a_refusal_is_a_422_with_its_text() {
        assert!(matches!(
            AttemptRefused::FutureDate.into_error(),
            Error::Status(StatusCode::UNPROCESSABLE_ENTITY, _)
        ));
        assert_eq!(
            AttemptRefused::MethodRequired.message(),
            "Say how you tried to reach the worker."
        );
    }
}
