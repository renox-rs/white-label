//! The actions of the field PWA and the back office on attendance (wl-attendance
//! §5.1, §5.2, issue #178).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `POST /attendance/mark` | `wl_attendance.mark` | `attendance.mark` at the location |
//! | `POST /attendance/check-in` | `wl_attendance.check_in` | `attendance.self` at the location |
//! | `POST /attendance/records/{id}/check-out` | `wl_attendance.check_out` | the employee themselves, or `attendance.mark` at the location |
//! | `POST /attendance/records/{id}/accept` | `wl_attendance.accept` | `attendance.review` at the location (`HRM.ACL.09`) |
//! | `POST /attendance/records/{id}/reject` | `wl_attendance.reject` | `attendance.review` at the location |
//! | `POST /timesheets/{id}/reopen` | `wl_attendance.timesheet_reopen` | `timesheets.reopen` |
//! | `POST /timesheets/{id}/adjustments` | `wl_attendance.adjustment_open` | `timesheets.adjust` in the scope of the original's context (`HRM.ACL.08`) |
//! | `POST /timesheets/{id}/submit` | `wl_attendance.timesheet_submit` | `timesheets.submit` in the scope of the timesheet's context |
//! | `POST /attendance/records/{id}/correct` | `wl_attendance.correct` | `attendance.correct` at the location, and `attendance.review` when a flag holds the record |
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
use crate::adjustment;
use crate::location::WorkLocation;
use crate::permissions;
use crate::record::AttendanceRecord;
use crate::review::{self, Correction};
use crate::service::{self, NewRecord, refuse};
use crate::submit;
use crate::texts;
use crate::timesheet::{self, AdjustmentOrigin, Affects};

/// The routes of the module.
pub fn routes() -> Routes {
    Routes::new()
        .post("/attendance/mark", mark)
        .name("wl_attendance.mark")
        .post("/attendance/check-in", check_in)
        .name("wl_attendance.check_in")
        .post("/attendance/records/{id}/check-out", check_out)
        .name("wl_attendance.check_out")
        .post("/attendance/records/{id}/accept", accept)
        .name("wl_attendance.accept")
        .post("/attendance/records/{id}/reject", reject)
        .name("wl_attendance.reject")
        .post("/attendance/records/{id}/correct", correct)
        .name("wl_attendance.correct")
        .post("/timesheets/{id}/reopen", reopen)
        .name("wl_attendance.timesheet_reopen")
        .post("/timesheets/{id}/adjustments", open_adjustment)
        .name("wl_attendance.adjustment_open")
        .post("/timesheets/{id}/submit", submit_timesheet)
        .name("wl_attendance.timesheet_submit")
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

/// What a reviewer submits to accept or reject.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct ReviewForm {
    /// Why; required when a flag holds the record, and for a rejection.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /attendance/records/{id}/accept`: a reviewer accepts the record.
async fn accept(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReviewForm>,
) -> Result<Response> {
    review::accept(&state, &user, id, &form.reason).await?;
    Ok((Toast::success(text(&lang, "review.saved")), renox::HxRefresh).into_response())
}

/// `POST /attendance/records/{id}/reject`: a reviewer rejects the record.
async fn reject(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReviewForm>,
) -> Result<Response> {
    review::reject(&state, &user, id, &form.reason).await?;
    Ok((Toast::success(text(&lang, "review.saved")), renox::HxRefresh).into_response())
}

/// What a user submits to correct a record.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct CorrectForm {
    /// The corrected check-in.
    #[validate(max = 40)]
    pub check_in: String,
    /// The corrected check-out, if any.
    #[validate(max = 40)]
    #[serde(default)]
    pub check_out: String,
    /// Why the record is corrected.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
}

/// `POST /attendance/records/{id}/correct`: a user corrects the times.
async fn correct(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<CorrectForm>,
) -> Result<Response> {
    let check_in = parse_time(&state, &lang, "check_in", &form.check_in)?;
    let check_out = if form.check_out.trim().is_empty() {
        None
    } else {
        Some(parse_time(&state, &lang, "check_out", &form.check_out)?)
    };
    review::correct(
        &state,
        &user,
        id,
        Correction {
            check_in,
            check_out,
            breaks: None,
        },
        &form.reason,
    )
    .await?;
    Ok((Toast::success(text(&lang, "review.saved")), renox::HxRefresh).into_response())
}

/// `POST /timesheets/{id}/reopen`: a user reopens an approved timesheet that
/// nothing has used.
async fn reopen(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReviewForm>,
) -> Result<Response> {
    timesheet::reopen(&state, &user, id, &form.reason).await?;
    Ok((Toast::success(text(&lang, "review.saved")), renox::HxRefresh).into_response())
}

/// `POST /timesheets/{id}/submit`: a user submits an open timesheet.
async fn submit_timesheet(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    submit::submit(&state, &user, id).await?;
    Ok((Toast::success(text(&lang, "review.saved")), renox::HxRefresh).into_response())
}

/// What a user with `timesheets.adjust` submits to correct a locked
/// timesheet.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct AdjustmentForm {
    /// Why.
    #[validate(max = 500)]
    #[serde(default)]
    pub reason: String,
    /// `client_dispute`, `attendance_error`, `payroll_error` or
    /// `provisional_reconciliation`.
    #[validate(max = 40)]
    #[serde(default)]
    pub origin: String,
    /// `billing`, `payroll` or `both`; the origin's default when empty.
    #[validate(max = 20)]
    #[serde(default)]
    pub affects: String,
    /// The linked ticket or invoice dispute, as `type:id`.
    #[validate(max = 80)]
    #[serde(default)]
    pub linked: String,
    /// The signed entries, a JSON list ([`crate::adjustment::parse_entries`]).
    #[validate(max = 8000)]
    #[serde(default)]
    pub entries: String,
}

/// `POST /timesheets/{id}/adjustments`: a user opens an adjustment on a
/// locked timesheet.
async fn open_adjustment(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<AdjustmentForm>,
) -> Result<Response> {
    let linked = if form.linked.trim().is_empty() {
        None
    } else {
        Some(
            form.linked
                .trim()
                .parse::<wl_kernel::Subject>()
                .map_err(|_| refuse(text(&lang, "adjustment.link_required")))?,
        )
    };
    let entries = if form.entries.trim().is_empty() {
        Vec::new()
    } else {
        adjustment::parse_entries(&form.entries)?
    };
    adjustment::open(
        &state,
        &user,
        adjustment::OpenAdjustment {
            original: id,
            reason: form.reason,
            origin: AdjustmentOrigin::parse(&form.origin),
            linked,
            affects: Affects::parse(&form.affects),
            entries,
        },
    )
    .await?;
    Ok((Toast::success(text(&lang, "adjustment.opened")), renox::HxRefresh).into_response())
}
