//! The session pages (10 `EDU.BAT.01`, `EDU.ATT.01`, §7.1 *Session*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/sessions` | `edtech.sessions` | `sessions.view` |
//! | `GET /edtech/sessions/{id}` | `edtech.session` | `sessions.view` |
//! | `POST /edtech/sessions/{id}/attendance` | `edtech.session_attendance` | `sessions.teach` (a field `mark_<enrolment>` per student) |
//! | `POST /edtech/sessions/{id}/students` | `edtech.session_student` | `enrolments.manage` or `batches.manage` |
//! | `POST /edtech/sessions/{id}/notice` | `edtech.session_notice` | `enrolments.manage` or `batches.manage` |
//! | `POST /edtech/sessions/{id}/excuse` | `edtech.session_excuse` | `batches.manage` |
//! | `POST /edtech/sessions/{id}/correction` | `edtech.session_correction` | `sessions.teach` within 24 hours of the end, else `batches.manage` |
//! | `POST /edtech/sessions/{id}/reschedule` | `edtech.session_reschedule` | `batches.manage` |
//! | `POST /edtech/sessions/{id}/tutor` | `edtech.session_tutor` | `batches.manage` |
//! | `POST /edtech/sessions/{id}/cancel` | `edtech.session_cancel` | `batches.manage` |
//! | `POST /edtech/sessions/{id}/flag` | `edtech.session_flag` | `batches.approve` |
//!
//! The list shows the sessions from today, soonest first. The transitions are
//! the machine's: the pages only read the form and call [`crate::sessions`].

use std::collections::BTreeMap;

use renox::db::now;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::today;
use wl_kernel::integrations::web::banner_for;

use super::{
    Names, Texts, invalid, names_of, parse_date, parse_local_time, parse_number, require,
    resource_text, text,
};
use crate::attendance::Mark;
use crate::batch::Batch;
use crate::defaults;
use crate::enrolment::EnrolmentKind;
use crate::enrolments::party_name;
use crate::permissions;
use crate::roster;
use crate::session::{Session, SessionStatus};
use crate::sessions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/edtech/sessions", index)
        .name("edtech.sessions")
        .get("/edtech/sessions/{id}", show)
        .name("edtech.session")
        .post("/edtech/sessions/{id}/attendance", attendance)
        .name("edtech.session_attendance")
        .post("/edtech/sessions/{id}/reschedule", reschedule)
        .name("edtech.session_reschedule")
        .post("/edtech/sessions/{id}/tutor", tutor)
        .name("edtech.session_tutor")
        .post("/edtech/sessions/{id}/cancel", cancel)
        .name("edtech.session_cancel")
        .post("/edtech/sessions/{id}/flag", close_flag)
        .name("edtech.session_flag")
        .post("/edtech/sessions/{id}/students", add_student)
        .name("edtech.session_student")
        .post("/edtech/sessions/{id}/notice", record_notice)
        .name("edtech.session_notice")
        .post("/edtech/sessions/{id}/excuse", excuse_late)
        .name("edtech.session_excuse")
        .post("/edtech/sessions/{id}/correction", correct_attendance)
        .name("edtech.session_correction")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: SessionStatus) -> String {
    text(lang, &format!("sessions.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: SessionStatus) -> &'static str {
    match status {
        SessionStatus::Scheduled | SessionStatus::Cancelled => "info",
        SessionStatus::Held => "success",
    }
}

/// `18:00 – 19:30`.
pub(crate) fn time_range(session: &Session) -> String {
    format!(
        "{} – {}",
        sessions::time_text(session.starts_at),
        sessions::time_text(session.ends_at)
    )
}

/// A resource as the page reads it: `Resource 12`, or a dash.
/// One session of a list.
#[derive(Serialize, Debug, Clone)]
pub struct SessionRow {
    /// The primary key.
    pub id: i64,
    /// Its number in the batch.
    pub number: i64,
    /// The batch, `BAT-0007`.
    pub batch: String,
    /// The day.
    pub date: String,
    /// The start and end.
    pub time: String,
    /// The tutor.
    pub tutor: String,
    /// The room.
    pub room: String,
    /// Whether it was added in place of a cancelled one.
    pub make_up: String,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// "Tutor not available or not eligible" while the session is flagged
    /// (`EDU.RUL.12`); empty otherwise.
    pub flag: String,
}

/// The row of `session` of batch `batch`.
pub(crate) fn row_of(lang: &Lang, names: &Names, session: &Session, batch: &str) -> SessionRow {
    SessionRow {
        id: session.id,
        number: session.number,
        batch: batch.to_owned(),
        date: session.session_date.to_string(),
        time: time_range(session),
        tutor: resource_text(lang, names, session.tutor_resource_id),
        room: resource_text(lang, names, session.room_resource_id),
        make_up: if session.make_up {
            text(lang, "sessions.make_up_yes")
        } else {
            String::new()
        },
        status: status_text(lang, session.status),
        status_kind: status_kind(session.status),
        flag: flag_text(lang, session),
    }
}

/// The words of the flag of `session`, empty when it is not flagged.
pub(crate) fn flag_text(lang: &Lang, session: &Session) -> String {
    if session.is_flagged() {
        text(lang, "sessions.flag")
    } else {
        String::new()
    }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/sessions.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    sessions: Vec<SessionRow>,
}

/// `GET /edtech/sessions`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::SESSIONS_VIEW)?;
    let coming = Session::query()
        .where_(Session::SESSION_DATE.gte(today()))
        .order_by(Session::STARTS_AT)
        .get(&state.db)
        .await?;
    let batches = Batch::query().get(&state.db).await?;
    let names = names_of(
        &state,
        coming
            .iter()
            .flat_map(|s| [s.tutor_resource_id, s.room_resource_id]),
    )
    .await;
    let rows = coming
        .iter()
        .map(|s| {
            let batch = batches
                .iter()
                .find(|b| b.id == s.batch_id)
                .map_or_else(|| format!("#{}", s.batch_id), Batch::number);
            row_of(&lang, &names, s, &batch)
        })
        .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        sessions: rows,
    }))
}

/// What the page of one session needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/session.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    title: String,
    batch_id: i64,
    batch: String,
    date: String,
    time: String,
    tutor: String,
    room: String,
    status: String,
    status_kind: &'static str,
    cancel_reason: String,
    start_time: String,
    can_attend: bool,
    can_change: bool,
    flagged: bool,
    flag: String,
    flag_reason: String,
    show_doc_flags: bool,
    doc_flags: Vec<DocFlagRow>,
    can_close_flag: bool,
    flag_options: Vec<[String; 2]>,
    students: Vec<StudentRow>,
    show_students: bool,
    attend_rows: Vec<AttendRow>,
    mark_options: Vec<[String; 2]>,
    student_options: Vec<[String; 2]>,
    can_roster: bool,
    can_excuse: bool,
    can_correct: bool,
    integration_banner: Option<renox::serde_json::Value>,
}

/// One student of the roster of a session.
#[derive(Serialize, Debug, Clone)]
pub struct StudentRow {
    /// The enrolment's number, `ENR-0007`.
    pub enrolment: String,
    /// The student's name.
    pub student: String,
    /// The mark in words.
    pub mark: String,
    /// The badge kind of the mark.
    pub mark_kind: &'static str,
    /// The notice of absence and its time, or a dash.
    pub notice: String,
    /// Whether the mark used a credit of the package.
    pub credit: String,
}

/// One student of the attendance form.
#[derive(Serialize, Debug, Clone)]
pub struct AttendRow {
    /// The enrolment's number, which names the form field.
    pub id: i64,
    /// `ENR-0007 · Sara Ali`.
    pub label: String,
    /// The mark the form starts with.
    pub selected: String,
}

/// One flag of the documents of a session, closed or not.
#[derive(Serialize, Debug, Clone)]
pub struct DocFlagRow {
    /// The flag's number.
    pub id: i64,
    /// "Police clearance expired on 2026-12-15".
    pub label: String,
    /// The day it was judged on.
    pub on_date: String,
    /// Open or closed.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
    /// Who closed it.
    pub reviewed_by: String,
    /// The note it was closed with.
    pub note: String,
}

/// `GET /edtech/sessions/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::SESSIONS_VIEW)?;
    let session = Session::find_or_404(&state.db, id).await?;
    let batch = Batch::find_or_404(&state.db, session.batch_id).await?;
    let names = names_of(
        &state,
        [session.tutor_resource_id, session.room_resource_id],
    )
    .await;
    let scheduled = session.status == SessionStatus::Scheduled;
    let students = roster::roster(&state.db, &session).await?;
    let mut student_rows = Vec::new();
    let mut attend_rows = Vec::new();
    let mut student_options: Vec<[String; 2]> = Vec::new();
    for student in &students {
        let name = party_name(&state, &lang.locale, student.enrolment.student_party_id).await;
        let label = format!("{} · {name}", student.enrolment.number());
        let mark = student.mark();
        let credit = if student.enrolment.kind != EnrolmentKind::Package {
            text(&lang, "none")
        } else if student.row.as_ref().is_some_and(|row| row.consumed) {
            text(&lang, "attendance.credit_used")
        } else if student.row.as_ref().is_some_and(|row| row.credit_due) {
            text(&lang, "attendance.credit_waiting")
        } else {
            text(&lang, "attendance.credit_not_used")
        };
        let notice = student
            .row
            .as_ref()
            .and_then(|row| row.notice_at)
            .map_or_else(
                || text(&lang, "none"),
                |at| {
                    format!(
                        "{} {}",
                        crate::sessions::date_text(at),
                        crate::sessions::time_text(at)
                    )
                },
            );
        student_rows.push(StudentRow {
            enrolment: student.enrolment.number(),
            student: name,
            mark: text(&lang, &format!("attendance.mark.{}", mark.as_str())),
            mark_kind: match mark {
                Mark::Present | Mark::Late => "success",
                Mark::Absent => "error",
                Mark::Excused => "info",
                Mark::Pending => "warning",
            },
            notice,
            credit,
        });
        attend_rows.push(AttendRow {
            id: student.enrolment.id,
            label: label.clone(),
            selected: if mark == Mark::Pending {
                "present".to_owned()
            } else {
                mark.as_str().to_owned()
            },
        });
        student_options.push([student.enrolment.id.to_string(), label]);
    }
    let mark_options: Vec<[String; 2]> = [Mark::Present, Mark::Late, Mark::Absent, Mark::Excused]
        .iter()
        .map(|mark| {
            [
                mark.as_str().to_owned(),
                text(&lang, &format!("attendance.mark.{}", mark.as_str())),
            ]
        })
        .collect();
    let found = sessions::document_flags(&state, session.id).await?;
    let assigned = sessions::may_mark(&state, &user, &session).await?;
    let none = text(&lang, "none");
    let doc_flags: Vec<DocFlagRow> = found
        .iter()
        .map(|flag| DocFlagRow {
            id: flag.id,
            label: flag.label.clone(),
            on_date: flag.on_date.to_string(),
            status: text(
                &lang,
                if flag.is_reviewed() {
                    "sessions.flag_closed_status"
                } else {
                    "sessions.flag_open"
                },
            ),
            status_kind: if flag.is_reviewed() {
                "success"
            } else {
                "warning"
            },
            reviewed_by: flag.reviewer_name.clone().unwrap_or_else(|| none.clone()),
            note: flag.review_note.clone().unwrap_or_else(|| none.clone()),
        })
        .collect();
    let flag_options: Vec<[String; 2]> = found
        .iter()
        .filter(|flag| !flag.is_reviewed())
        .map(|flag| [flag.id.to_string(), flag.label.clone()])
        .collect();
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: session.id,
        title: format!("{} · {}", batch.number(), session.number),
        batch_id: batch.id,
        batch: batch.number(),
        date: session.session_date.to_string(),
        time: time_range(&session),
        tutor: resource_text(&lang, &names, session.tutor_resource_id),
        room: resource_text(&lang, &names, session.room_resource_id),
        status: status_text(&lang, session.status),
        status_kind: status_kind(session.status),
        cancel_reason: session
            .cancel_reason
            .clone()
            .unwrap_or_else(|| text(&lang, "none")),
        start_time: sessions::time_text(session.starts_at),
        can_attend: scheduled
            && session.starts_at <= now()
            && user.has_permission(permissions::SESSIONS_TEACH)
            && assigned,
        can_change: scheduled && user.has_permission(permissions::BATCHES_MANAGE),
        flagged: session.is_flagged(),
        flag: flag_text(&lang, &session),
        flag_reason: session
            .flag_reason
            .as_deref()
            .map(crate::tutor_alerts::reason_text)
            .unwrap_or_default(),
        show_doc_flags: !doc_flags.is_empty(),
        doc_flags,
        can_close_flag: !flag_options.is_empty()
            && user.has_permission(permissions::BATCHES_APPROVE),
        flag_options,
        show_students: !student_rows.is_empty(),
        students: student_rows,
        attend_rows,
        mark_options,
        student_options,
        can_roster: scheduled
            && (user.has_permission(permissions::ENROLMENTS_MANAGE)
                || user.has_permission(permissions::BATCHES_MANAGE)),
        can_excuse: scheduled && user.has_permission(permissions::BATCHES_MANAGE),
        can_correct: crate::corrections::may_correct(&user, &session) && assigned,
        integration_banner: banner_for(&state.db, defaults::SESSION_SCREEN, &lang).await?,
    }))
}

/// The toast and refresh that answer a transition.
fn done(lang: &Lang, key: &str) -> Result<Response> {
    Ok((Toast::success(text(lang, key)), renox::HxRefresh).into_response())
}

/// `POST /edtech/sessions/{id}/attendance`: a `mark_<enrolment>` per student; a
/// student without one is present (or excused, if a notice 24 hours ahead said so).
async fn attendance(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut given = Vec::new();
    for (name, value) in &form {
        let Some(enrolment) = name.strip_prefix("mark_") else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let mark =
            Mark::parse(value).ok_or_else(|| invalid(&text(&lang, "attendance.problem.mark")))?;
        given.push((parse_number(enrolment)?, mark));
    }
    sessions::submit_attendance_marked(&state, &user, id, &given, true).await?;
    done(&lang, "sessions.attendance_done")
}

/// `POST /edtech/sessions/{id}/correction`: a `mark_<enrolment>` per student and a
/// `reason`.
async fn correct_attendance(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let mut given = Vec::new();
    for (name, value) in &form {
        let Some(enrolment) = name.strip_prefix("mark_") else {
            continue;
        };
        if value.trim().is_empty() {
            continue;
        }
        let mark =
            Mark::parse(value).ok_or_else(|| invalid(&text(&lang, "attendance.problem.mark")))?;
        given.push((parse_number(enrolment)?, mark));
    }
    let reason = form.get("reason").map_or("", String::as_str);
    crate::corrections::correct_attendance(&state, &user, id, &given, reason).await?;
    done(&lang, "corrections.done")
}

/// The field `name` of a form as a number.
fn number_of(form: &BTreeMap<String, String>, name: &str) -> std::result::Result<i64, Error> {
    parse_number(form.get(name).map_or("", String::as_str))
}

/// `POST /edtech/sessions/{id}/students`: an `enrolment` takes the session.
async fn add_student(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    roster::add_student(&state, &user, id, number_of(&form, "enrolment")?).await?;
    done(&lang, "attendance.added")
}

/// `POST /edtech/sessions/{id}/notice`: the `enrolment` gave notice of an absence
/// at `given_at` (now when blank).
async fn record_notice(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let given_at =
        parse_local_time(form.get("given_at").map_or("", String::as_str))?.unwrap_or_else(now);
    roster::record_notice(&state, &user, id, number_of(&form, "enrolment")?, given_at).await?;
    done(&lang, "attendance.notice_recorded")
}

/// `POST /edtech/sessions/{id}/excuse`: the coordinator excuses a late notice
/// with a `reason`.
async fn excuse_late(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").map_or("", String::as_str);
    roster::excuse_late(&state, &user, id, number_of(&form, "enrolment")?, reason).await?;
    done(&lang, "attendance.excused")
}

/// The form of `reschedule`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct RescheduleForm {
    /// The new day, `2026-11-10`.
    #[serde(default)]
    pub date: String,
    /// The new start time, `18:00`.
    #[serde(default)]
    pub start_time: String,
}

/// `POST /edtech/sessions/{id}/reschedule`.
async fn reschedule(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<RescheduleForm>,
) -> Result<Response> {
    let date = parse_date(&form.date)?;
    sessions::reschedule(&state, &user, id, date, &form.start_time).await?;
    done(&lang, "sessions.rescheduled")
}

/// The form of `change_tutor`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct TutorForm {
    /// The new tutor's resource number.
    #[serde(default)]
    pub tutor: String,
    /// The new tutor's employee number.
    #[serde(default)]
    pub employee: String,
}

/// `POST /edtech/sessions/{id}/tutor`.
async fn tutor(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<TutorForm>,
) -> Result<Response> {
    let resource = parse_number(&form.tutor)?;
    let employee = super::parse_optional_id(&form.employee)?;
    sessions::change_tutor(&state, &user, id, resource, employee).await?;
    done(&lang, "sessions.tutor_changed")
}

/// The form of the closing of a flag.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct FlagForm {
    /// The flag's number.
    #[serde(default)]
    pub flag: String,
    /// The note it is closed with.
    #[serde(default)]
    pub note: String,
}

/// `POST /edtech/sessions/{id}/flag`.
async fn close_flag(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<FlagForm>,
) -> Result<Response> {
    let flag = parse_number(&form.flag)?;
    sessions::close_flag(&state, &user, id, flag, &form.note).await?;
    done(&lang, "sessions.flag_closed")
}

/// The form of `cancel`.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CancelForm {
    /// Why the session does not take place.
    #[serde(default)]
    pub reason: String,
    /// The day of the make-up session, if one is added.
    #[serde(default)]
    pub make_up_on: String,
}

/// `POST /edtech/sessions/{id}/cancel`.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<CancelForm>,
) -> Result<Response> {
    let make_up = if form.make_up_on.trim().is_empty() {
        None
    } else {
        Some(parse_date(&form.make_up_on)?)
    };
    sessions::cancel(&state, &user, id, &form.reason, make_up).await?;
    done(&lang, "sessions.cancelled")
}
