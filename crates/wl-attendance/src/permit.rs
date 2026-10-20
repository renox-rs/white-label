//! Work permit, residence visa and qualifications (`HRM.RUL.14`,
//! wl-attendance §5.5 point 3, §6, §7, BL-ATT-04): when a worker is marked
//! present on a day on which their work permit or residence visa is not valid,
//! or a qualification the context requires (a SIRA licence, say) is not, the
//! record is flagged `permit_invalid` the same day. HR and the supervisor are
//! told.
//!
//! - **Through the kernel.** The documents are asked with `document_valid`
//!   ([`DocumentGuard::is_valid`](wl_kernel::documents::DocumentGuard::is_valid))
//!   and the qualifications with `qualification_valid`
//!   ([`Qualifications`](wl_kernel::workforce::Qualifications)); this module
//!   knows neither wl-documents nor wl-hr.
//! - **`on_date` is the attendance date**, also for a late entry made the
//!   next day: the guard answers with the document valid on that date. A
//!   renewal that waits for review answers *no*, and the flag stays when the
//!   document is renewed later the same day; HR clears it by review.
//! - **Which kinds.** [`AttendanceConfig::permit_kinds`](crate::AttendanceConfig)
//!   (work permit and residence visa by default) for every employee, plus
//!   the qualifications the work location's context requires
//!   ([`WorkLocation::required_qualifications`]); for those the role is the
//!   location's, else the employee's job role, and the scope is the
//!   location's scope, else the location itself. With
//!   [`AttendanceConfig::permit_check`](crate::AttendanceConfig) off nothing runs.
//!   Decision: the staff profile of the kernel does not carry the employment
//!   type, so a freelancer is not told apart; a client whose freelancers have no
//!   company permit leaves the kind out of `permit_kinds`.
//! - **It flags, it never refuses**: the worker did turn up, and the hours
//!   are not dropped from payroll.
//! - **Asked before the transaction.** The guards read the database, so they
//!   are asked before the record's transaction opens ([`failures`]), and the
//!   flag is added inside it.
//!
//! What a `permit_invalid` record does to the timesheet is in
//! [`crate::proposal`].

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::guard;
use wl_kernel::workforce::{EmployeeId, qualifications, workforce};

use crate::config::config_of;
use crate::location::WorkLocation;
use crate::texts::english;

/// The document kinds and qualification kinds of `employee` that are not
/// valid on `date` for work at `location`; empty when all are, or when the
/// check is off.
///
/// # Errors
///
/// The database's errors, and the failures of the guards.
pub async fn failures(
    state: &AppState,
    employee: EmployeeId,
    location: &WorkLocation,
    date: NaiveDate,
) -> Result<Vec<String>> {
    let config = config_of(state);
    if !config.permit_check {
        return Ok(Vec::new());
    }
    let mut failed = Vec::new();
    let documents = guard(state);
    for kind in &config.permit_kinds {
        let validity = documents
            .is_valid(&state.db, &employee.subject(), kind, date)
            .await?;
        if !validity.is_valid() {
            failed.push(kind.clone());
        }
    }
    let required: Vec<String> = location.required_qualifications.to_vec();
    if !required.is_empty() {
        let asked = qualifications(state);
        let profile = workforce(state).staff(state, employee).await?;
        let role = location
            .qualification_role
            .clone()
            .or_else(|| profile.map(|p| p.role));
        let scope = location.scope().unwrap_or_else(|| location.subject());
        for kind in &required {
            let valid = match &role {
                Some(role) => {
                    asked
                        .qualification_valid_for(state, employee, kind, date, role, &scope)
                        .await?
                }
                None => {
                    asked
                        .qualification_valid(state, employee, kind, date)
                        .await?
                }
            };
            if !valid {
                failed.push(kind.clone());
            }
        }
    }
    Ok(failed)
}

/// The note of the flag: what is not valid on which day.
pub fn note_of(failed: &[String], date: NaiveDate) -> String {
    english(
        "wl_attendance.permit.note",
        &[("kinds", &failed.join(", ")), ("date", &date)],
    )
}
