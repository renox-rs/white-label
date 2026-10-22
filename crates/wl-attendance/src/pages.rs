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
//! | `POST /timesheets/{id}/approve` | `wl_attendance.timesheet_approve` | `timesheets.approve` in the scope of the context (internal), or the timesheet's approver role (external) |
//! | `POST /timesheets/{id}/approve-on-behalf` | `wl_attendance.timesheet_approve_on_behalf` | `timesheets.approve_on_behalf` in the scope of the context, with the signed sheet |
//! | `POST /timesheets/{id}/reject` | `wl_attendance.timesheet_reject` | an approver of the timesheet |
//! | `POST /timesheets/{id}/submit` | `wl_attendance.timesheet_submit` | `timesheets.submit` in the scope of the timesheet's context |
//! | `POST /attendance/records/{id}/correct` | `wl_attendance.correct` | `attendance.correct` at the location, and `attendance.review` when a flag holds the record |
//!
//! The routes need a signed-in user and carry no permission guard of their
//! own: the guard counts global roles only, while a supervisor may hold the
//! permission in one project. The handlers check it in the scope of the
//! location and answer 403. Times are sent as `2026-11-02T07:00` in
//! Asia/Dubai. A marking may carry the break (`break_from`, `break_to` as
//! `12:30`); without it an outdoor block in the midday season is flagged. The
//! crew list page is in [`crate::crew`]; the HTML of the field PWA comes later.

use renox::chrono::{NaiveDateTime, NaiveTime};
use renox::db::{DateTime, now};
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::calendar::calendar;
use wl_kernel::workforce::{EmployeeId, workforce};

use crate::access;
use crate::adjustment;
use crate::approval;
use crate::location::WorkLocation;
use crate::permissions;
use crate::record::{AttendanceRecord, BreakSpan};
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
        .post("/timesheets/{id}/approve", approve_timesheet)
        .name("wl_attendance.timesheet_approve")
        .post("/timesheets/{id}/approve-on-behalf", approve_on_behalf)
        .name("wl_attendance.timesheet_approve_on_behalf")
        .post("/timesheets/{id}/reject", reject_timesheet)
        .name("wl_attendance.timesheet_reject")
        .require_auth()
}

/// The package text `key`.
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("wl_attendance.{key}"))
}

/// `2026-11-02T07:00` in Asia/Dubai as an instant.
pub(crate) fn parse_time(
    state: &AppState,
    lang: &Lang,
    field: &str,
    value: &str,
) -> Result<DateTime> {
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
    /// The break taken, from `12:30`; empty when the break is removed. The
    /// crew page pre-fills it in the midday season (`HRM.RUL.13`).
    #[validate(max = 10)]
    #[serde(default)]
    pub break_from: String,
    /// The break taken, to `15:00`; both or none of the break times.
    #[validate(max = 10)]
    #[serde(default)]
    pub break_to: String,
}

/// The break of the form: none when both times are empty, else `from` and
/// `to` as `12:30`, refused when only one is given or `to` is not after
/// `from`.
fn parse_break(lang: &Lang, form: &MarkForm) -> Result<Vec<BreakSpan>> {
    let (from, to) = (form.break_from.trim(), form.break_to.trim());
    if from.is_empty() && to.is_empty() {
        return Ok(Vec::new());
    }
    let time = |field: &str, value: &str| {
        NaiveTime::parse_from_str(value, "%H:%M")
            .map_err(|_| refuse(format!("{field}: {}", text(lang, "record.bad_break"))))
    };
    let (from, to) = (time("break_from", from)?, time("break_to", to)?);
    if to <= from {
        return Err(refuse(text(lang, "record.bad_break")));
    }
    Ok(vec![BreakSpan { from, to }])
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
    let breaks = parse_break(&lang, &form)?;
    service::record(
        &state,
        NewRecord::supervisor(
            EmployeeId(form.employee_id),
            location.id,
            check_in,
            check_out,
            user.id,
        )
        .breaks(breaks)
        .note(&form.note),
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "record.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    Ok((
        Toast::success(text(&lang, "record.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    Ok((
        Toast::success(text(&lang, "record.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    Ok((
        Toast::success(text(&lang, "review.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    Ok((
        Toast::success(text(&lang, "review.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    Ok((
        Toast::success(text(&lang, "review.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    Ok((
        Toast::success(text(&lang, "review.saved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /timesheets/{id}/submit`: a user submits an open timesheet.
async fn submit_timesheet(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    submit::submit(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "review.saved")),
        renox::HxRefresh,
    )
        .into_response())
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
    /// The attendance evidence, as `type:id` separated by commas
    /// ([`crate::dispute::parse_evidence`]).
    #[validate(max = 500)]
    #[serde(default)]
    pub evidence: String,
    /// The page's one entry: the day to correct, `2026-11-02` (used when
    /// `entries` is empty).
    #[validate(max = 10)]
    #[serde(default)]
    pub entry_date: String,
    /// The page's one entry: the category (`normal`, `overtime`...).
    #[validate(max = 20)]
    #[serde(default)]
    pub entry_category: String,
    /// The page's one entry: the signed hours, `-1.5`.
    #[validate(max = 12)]
    #[serde(default)]
    pub entry_hours: String,
}

/// `-1.5` hours as minutes (`-90`); none when it is not a number with at most
/// two decimals.
fn minutes_of_hours(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty()
        || fraction.len() > 2
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let hundredths =
        whole.parse::<i64>().ok()? * 100 + format!("{fraction:0<2}").parse::<i64>().ok()?;
    let minutes = (hundredths * 60 + 50) / 100;
    Some(if negative { -minutes } else { minutes })
}

/// The one entry the adjustment sheet of the timesheet page sends.
fn page_entry(lang: &Lang, form: &AdjustmentForm) -> Result<Vec<crate::entry::NewEntry>> {
    let date = renox::chrono::NaiveDate::parse_from_str(form.entry_date.trim(), "%Y-%m-%d")
        .map_err(|_| refuse(texts::t(lang, "wl_attendance.sheet.bad_day")))?;
    let minutes = minutes_of_hours(&form.entry_hours)
        .ok_or_else(|| refuse(texts::t(lang, "wl_attendance.sheet.bad_hours")))?;
    let category = crate::entry::Category::parse(&form.entry_category)
        .unwrap_or(crate::entry::Category::Normal);
    Ok(vec![crate::entry::NewEntry::of(date, category, minutes)])
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
    let entries = if form.entries.trim().is_empty() && !form.entry_date.trim().is_empty() {
        page_entry(&lang, &form)?
    } else if form.entries.trim().is_empty() {
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
            evidence: crate::dispute::parse_evidence(&form.evidence)?,
        },
    )
    .await?;
    Ok((
        Toast::success(text(&lang, "adjustment.opened")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /timesheets/{id}/approve`: the approver approves a submitted
/// timesheet.
async fn approve_timesheet(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    // A client user of another company meets "not found" (`MPW.ACL.01`).
    let sheet = crate::timesheet::Timesheet::find_or_404(&state.db, id).await?;
    crate::timesheet_pages::not_of_another_client(&state, &user, &sheet).await?;
    approval::approve(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "approval.approved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The signed sheet that goes with an approval on behalf.
#[derive(Deserialize, Serialize, Validate, Debug, Default)]
pub struct OnBehalfForm {
    /// The document that holds the signed sheet; 0 or none when not given.
    #[serde(default)]
    pub document_id: i64,
    /// Who signed for the client.
    #[validate(max = 120)]
    #[serde(default)]
    pub approver_name: String,
    /// The day they signed, `2026-11-09`.
    #[validate(max = 10)]
    #[serde(default)]
    pub signed_on: String,
}

/// `POST /timesheets/{id}/approve-on-behalf`: staff approve for the external
/// approver with the signed sheet.
async fn approve_on_behalf(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<OnBehalfForm>,
) -> Result<Response> {
    let evidence = renox::chrono::NaiveDate::parse_from_str(form.signed_on.trim(), "%Y-%m-%d")
        .ok()
        .map(|signed_on| approval::Evidence {
            document_id: form.document_id,
            approver_name: form.approver_name.clone(),
            signed_on,
        });
    approval::approve_on_behalf(&state, &user, id, evidence).await?;
    Ok((
        Toast::success(text(&lang, "approval.approved")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /timesheets/{id}/reject`: the approver sends a submitted timesheet
/// back with a reason.
async fn reject_timesheet(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Valid(form): Valid<ReviewForm>,
) -> Result<Response> {
    // A client user of another company meets "not found" (`MPW.ACL.01`).
    let sheet = crate::timesheet::Timesheet::find_or_404(&state.db, id).await?;
    crate::timesheet_pages::not_of_another_client(&state, &user, &sheet).await?;
    approval::reject(&state, &user, id, &form.reason).await?;
    Ok((
        Toast::success(text(&lang, "approval.rejected")),
        renox::HxRefresh,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::minutes_of_hours;

    #[test]
    fn hours_are_read_as_signed_minutes() {
        assert_eq!(minutes_of_hours("1.5"), Some(90));
        assert_eq!(minutes_of_hours("-2"), Some(-120));
        assert_eq!(minutes_of_hours(" +0.25 "), Some(15));
        assert_eq!(minutes_of_hours("1.255"), None);
        assert_eq!(minutes_of_hours("x"), None);
        assert_eq!(minutes_of_hours(""), None);
    }
}
