//! The actions of the field PWA and the back office on attendance (wl-attendance
//! §5.1, §5.2, issue #178).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /attendance/mark` | `wl_attendance.mark` | `attendance.mark` at the location |
//! | `POST /attendance/check-in` | `wl_attendance.check_in` | `attendance.self` at the location |
//! | `POST /attendance/records/{id}/check-out` | `wl_attendance.check_out` | the employee themselves, or `attendance.mark` at the location |
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a supervisor may hold the
//! permission in one project. The handlers check it in the scope of the
//! location and answer 403. Times are sent as `2026-11-02T07:00` in
//! Asia/Dubai. The crew page and the HTML of the field PWA come later; the
//! crew list's clashes are [`crate::overlap::clashes_for`].

use renox::chrono::NaiveDateTime;
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::calendar::calendar;
use wl_kernel::workforce::{EmployeeId, workforce};

use crate::access;
use crate::location::WorkLocation;
use crate::permissions;
use crate::record::AttendanceRecord;
use crate::service::{self, NewRecord, refuse};
use crate::texts;

/// The routes of the module.
pub fn routes() -> Routes {
    Routes::new()
        .post("/attendance/mark", mark)
        .name("wl_attendance.mark")
        .post("/attendance/check-in", check_in)
        .name("wl_attendance.check_in")
        .post("/attendance/records/{id}/check-out", check_out)
        .name("wl_attendance.check_out")
        .require_auth()
}

/// The package text `key`.
fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_attendance.{key}"))
}

/// `2026-11-02T07:00` in Asia/Dubai as an instant.
fn parse_time(state: &AppState, lang: &Lang, field: &str, value: &str) -> Result<DateTime> {
    let local = NaiveDateTime::parse_from_str(value.trim(), "%Y-%m-%dT%H:%M")
        .map_err(|_| refuse(format!("{field}: {}", text(lang, "record.bad_time"))))?;
    Ok(calendar(state).from_local(local))
}

/// What a supervisor submits.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct MarkForm {
    /// The employee.
    pub employee_id: i64,
    /// The work location.
    pub location_id: i64,
    /// When the employee was checked in.
    #[validate(max = 40)]
    pub check_in: String,
    /// When the employee checked out, if known.
    #[validate(max = 40)]
    #[serde(default)]
    pub check_out: String,
    /// A note.
    #[validate(max = 500)]
    #[serde(default)]
    pub note: String,
}

/// `POST /attendance/mark`: a supervisor marks an employee present.
async fn mark(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<MarkForm>,
) -> Result<Response> {
    let location = WorkLocation::find(&state.db, form.location_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !access::holds(&state, user.id, permissions::MARK, &location).await? {
        return Err(Error::Forbidden);
    }
    let check_in = parse_time(&state, &lang, "check_in", &form.check_in)?;
    let check_out = if form.check_out.trim().is_empty() {
        None
    } else {
        Some(parse_time(&state, &lang, "check_out", &form.check_out)?)
    };
    service::record(
        &state,
        NewRecord::supervisor(
            EmployeeId(form.employee_id),
            location.id,
            check_in,
            check_out,
            user.id,
        )
        .note(&form.note),
    )
    .await?;
    Ok((Toast::success(text(&lang, "record.saved")), renox::HxRefresh).into_response())
}

/// What an employee submits to check in.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CheckInForm {
    /// The work location.
    pub location_id: i64,
}

/// The employee whose account is `user`.
async fn employee_of(state: &AppState, lang: &Lang, user: i64) -> Result<EmployeeId> {
    workforce(state)
        .employee_of_user(&state.db, user)
        .await?
        .ok_or_else(|| refuse(text(lang, "self.no_employee")))
}

/// `POST /attendance/check-in`: the employee checks in at a location.
async fn check_in(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Valid(form): Valid<CheckInForm>,
) -> Result<Response> {
    let location = WorkLocation::find(&state.db, form.location_id)
        .await?
        .ok_or(Error::NotFound)?;
    if !access::holds(&state, user.id, permissions::SELF_CHECK_IN, &location).await? {
        return Err(Error::Forbidden);
    }
    let employee = employee_of(&state, &lang, user.id).await?;
    service::check_in_self(&state, employee, location.id, now(), Some(user.id)).await?;
    Ok((Toast::success(text(&lang, "record.saved")), renox::HxRefresh).into_response())
}

/// `POST /attendance/records/{id}/check-out`: the employee checks out, or a
/// supervisor does for them.
async fn check_out(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    let record = AttendanceRecord::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let location = WorkLocation::find(&state.db, record.location_id)
        .await?
        .ok_or(Error::NotFound)?;
    let own = workforce(&state)
        .employee_of_user(&state.db, user.id)
        .await?
        .is_some_and(|employee| employee.0 == record.employee_id);
    if !own && !access::holds(&state, user.id, permissions::MARK, &location).await? {
        return Err(Error::Forbidden);
    }
    service::check_out(&state, id, now()).await?;
    Ok((Toast::success(text(&lang, "record.saved")), renox::HxRefresh).into_response())
}
