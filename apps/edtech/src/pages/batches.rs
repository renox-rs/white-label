//! The batch pages (10 `EDU.BAT.01`, §7.1 *Batch*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/batches` | `edtech.batches` | `batches.view` |
//! | `POST /edtech/batches` | `edtech.batch_create` | `batches.manage` |
//! | `GET /edtech/batches/{id}` | `edtech.batch` | `batches.view` |
//! | `POST /edtech/batches/{id}/open` | `edtech.batch_open` | `batches.manage` |
//! | `POST /edtech/batches/{id}/tutor` | `edtech.batch_tutor` | `batches.manage` |
//! | `POST /edtech/batches/{id}/waiting` | `edtech.batch_waiting` | `enrolments.manage` |
//! | `POST /edtech/waiting/{id}/remove` | `edtech.waiting_remove` | `enrolments.manage` |
//!
//! The page of a batch lists its sessions (see [`super::sessions`]).
//!
//! The list shows each batch with its dates, its weekly pattern, its tutor and
//! room, its seats and its status in words. The transitions are the
//! machine's: the pages only read the form and call [`crate::batches`].

use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::documents::{dubai_text, today};

use super::sessions::{SessionRow, row_of};
use super::{
    Texts, optional_party_of, parse_date, parse_number, parse_optional_id, parse_weekdays,
    party_of, require, text,
};
use crate::batch::{Batch, BatchStatus};
use crate::batches::{self, NewBatch};
use crate::course::Course;
use crate::enrolments::party_name;
use crate::permissions;
use crate::seats;
use crate::session::Session;
use crate::texts;
use crate::tutors;
use crate::waiting::{self, BatchWaiting};

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/edtech/batches", index)
        .name("edtech.batches")
        .post("/edtech/batches", create)
        .name("edtech.batch_create")
        .get("/edtech/batches/{id}", show)
        .name("edtech.batch")
        .post("/edtech/batches/{id}/open", open)
        .name("edtech.batch_open")
        .post("/edtech/batches/{id}/tutor", tutor)
        .name("edtech.batch_tutor")
        .post("/edtech/batches/{id}/waiting", join_waiting)
        .name("edtech.batch_waiting")
        .post("/edtech/waiting/{id}/remove", remove_waiting)
        .name("edtech.waiting_remove")
        .require_auth()
}

/// The status as staff read it.
pub(crate) fn status_text(lang: &Lang, status: BatchStatus) -> String {
    text(lang, &format!("batches.status.{}", status.as_str()))
}

/// The badge kind of a status.
pub(crate) fn status_kind(status: BatchStatus) -> &'static str {
    match status {
        BatchStatus::Planned | BatchStatus::Cancelled => "info",
        BatchStatus::Open => "info",
        BatchStatus::Running | BatchStatus::Completed => "success",
        BatchStatus::Terminated => "error",
    }
}

/// The weekly pattern in words: `Mon, Wed 18:00 for 90 min`.
pub(crate) fn pattern_text(lang: &Lang, batch: &Batch) -> String {
    let days: Vec<String> = batch
        .weekday_list()
        .iter()
        .map(|day| text(lang, &format!("batches.weekday.{day}")))
        .collect();
    format!(
        "{} {} · {} {}",
        days.join(", "),
        batch.start_time,
        batch.duration_minutes,
        text(lang, "batches.minutes")
    )
}

/// The seats in words: `3 of 12`.
pub(crate) fn seats_text(lang: &Lang, batch: &Batch) -> String {
    format!(
        "{} {} {}",
        seats::taken(batch),
        text(lang, "batches.of"),
        batch.capacity
    )
}

/// One batch of the list.
#[derive(Serialize, Debug, Clone)]
pub struct BatchRow {
    /// The primary key.
    pub id: i64,
    /// `BAT-0007`.
    pub number: String,
    /// The course.
    pub course: String,
    /// The first and last day.
    pub dates: String,
    /// The weekly pattern.
    pub pattern: String,
    /// The seats taken of the capacity.
    pub seats: String,
    /// The status.
    pub status: String,
    /// The badge kind.
    pub status_kind: &'static str,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/batches.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    batches: Vec<BatchRow>,
    course_options: Vec<[String; 2]>,
    course_selected: String,
    can_manage: bool,
}

/// The name of course `id` for the list.
fn course_title(courses: &[Course], id: i64) -> String {
    courses
        .iter()
        .find(|c| c.id == id)
        .map_or_else(|| format!("#{id}"), Course::title)
}

/// `GET /edtech/batches`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::BATCHES_VIEW)?;
    let all = Batch::query()
        .order_by_desc(Batch::ID)
        .get(&state.db)
        .await?;
    let courses = Course::query()
        .order_by(Course::CODE)
        .get(&state.db)
        .await?;
    let rows = all
        .iter()
        .map(|b| BatchRow {
            id: b.id,
            number: b.number(),
            course: course_title(&courses, b.course_id),
            dates: format!("{} – {}", b.start_date, b.end_date),
            pattern: pattern_text(&lang, b),
            seats: seats_text(&lang, b),
            status: status_text(&lang, b.status),
            status_kind: status_kind(b.status),
        })
        .collect();
    let course_options: Vec<[String; 2]> = courses
        .iter()
        .filter(|c| c.active)
        .map(|c| [c.id.to_string(), c.title()])
        .collect();
    let course_selected = course_options
        .first()
        .map(|option| option[0].clone())
        .unwrap_or_default();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        batches: rows,
        course_options,
        course_selected,
        can_manage: user.has_permission(permissions::BATCHES_MANAGE),
    }))
}

/// The form that plans a batch.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The course's id.
    #[serde(default)]
    pub course: String,
    /// The seats.
    #[serde(default)]
    pub capacity: String,
    /// The first day.
    #[serde(default)]
    pub start_date: String,
    /// The last day.
    #[serde(default)]
    pub end_date: String,
    /// The weekdays, `1,3`.
    #[serde(default)]
    pub weekdays: String,
    /// The start time, `18:00`.
    #[serde(default)]
    pub start_time: String,
    /// The length of a session in minutes.
    #[serde(default)]
    pub duration: String,
    /// The tutor's resource number.
    #[serde(default)]
    pub tutor: String,
    /// The tutor's employee number.
    #[serde(default)]
    pub employee: String,
    /// The room's resource number.
    #[serde(default)]
    pub room: String,
    /// The persons the room holds.
    #[serde(default)]
    pub room_limit: String,
}

/// `POST /edtech/batches`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::BATCHES_MANAGE)?;
    let course = parse_number(&form.course)?;
    let capacity = parse_number(&form.capacity)?;
    let duration = parse_number(&form.duration)?;
    let dates = (parse_date(&form.start_date)?, parse_date(&form.end_date)?);
    let weekdays = parse_weekdays(&form.weekdays)?;
    let mut new = NewBatch::new(course, capacity, dates, &weekdays, &form.start_time, duration);
    new.tutor_resource_id = parse_optional_id(&form.tutor)?;
    new.tutor_employee_id = parse_optional_id(&form.employee)?;
    new.room_resource_id = parse_optional_id(&form.room)?;
    new.room_limit = parse_optional_id(&form.room_limit)?;
    let batch = batches::create(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "batches.created")),
        renox::HxRedirect(format!("/edtech/batches/{}", batch.id)),
    )
        .into_response())
}

/// What the page of one batch needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/batch.html")]
pub(crate) struct ShowPage {
    tx: Texts,
    id: i64,
    number: String,
    course: String,
    status: String,
    status_kind: &'static str,
    dates: String,
    pattern: String,
    seats: String,
    free_seats: i64,
    tutor: String,
    room: String,
    sessions: Vec<SessionRow>,
    tutor_history: Vec<TutorRow>,
    waiting: Vec<WaitingRow>,
    from_date: String,
    can_open: bool,
    can_change_tutor: bool,
    can_wait: bool,
    can_unwait: bool,
    show_waiting: bool,
}

/// One entry of the waiting list of the page.
#[derive(Serialize, Debug, Clone)]
pub struct WaitingRow {
    /// The primary key.
    pub id: i64,
    /// The student's name.
    pub student: String,
    /// The payer's name.
    pub payer: String,
    /// When the entry was added.
    pub since: String,
}

/// One tutor assignment of the page.
#[derive(Serialize, Debug, Clone)]
pub struct TutorRow {
    /// The tutor.
    pub tutor: String,
    /// What it covers: the batch, or one session.
    pub scope: String,
    /// The first day.
    pub from: String,
    /// The last day, or a dash while it holds.
    pub to: String,
}

/// A resource as the page reads it: `Resource 12`, or a dash.
fn resource_text(lang: &Lang, id: Option<i64>) -> String {
    id.map_or_else(
        || text(lang, "none"),
        |id| format!("{} {id}", text(lang, "batches.resource")),
    )
}

/// `GET /edtech/batches/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::BATCHES_VIEW)?;
    let batch = Batch::find_or_404(&state.db, id).await?;
    let course = Course::find_or_404(&state.db, batch.course_id).await?;
    let number = batch.number();
    let planned = Session::of_batch(&state.db, batch.id).await?;
    let rows = planned.iter().map(|s| row_of(&lang, s, &number)).collect();
    let history = tutors::history(&state.db, batch.id)
        .await?
        .into_iter()
        .map(|a| TutorRow {
            tutor: resource_text(&lang, Some(a.tutor_resource_id)),
            scope: match a.session_id {
                Some(session) => format!("{} {session}", text(&lang, "tutors.session")),
                None => text(&lang, "tutors.batch"),
            },
            from: a.valid_from.to_string(),
            to: a
                .valid_to
                .map_or_else(|| text(&lang, "none"), |day| day.to_string()),
        })
        .collect();
    let mut waiting_rows = Vec::new();
    for entry in BatchWaiting::of_batch(&state.db, batch.id).await? {
        waiting_rows.push(WaitingRow {
            id: entry.id,
            student: party_name(&state, &lang.locale, entry.student_party_id).await,
            payer: party_name(&state, &lang.locale, entry.payer_party_id).await,
            since: entry
                .created_at
                .map_or_else(|| text(&lang, "none"), dubai_text),
        });
    }
    let manage_enrolments = user.has_permission(permissions::ENROLMENTS_MANAGE);
    Ok(View::typed(ShowPage {
        tx: texts::all(&lang),
        id: batch.id,
        number: batch.number(),
        course: course.title(),
        status: status_text(&lang, batch.status),
        status_kind: status_kind(batch.status),
        dates: format!("{} – {}", batch.start_date, batch.end_date),
        pattern: pattern_text(&lang, &batch),
        seats: seats_text(&lang, &batch),
        free_seats: seats::free(&batch),
        tutor: resource_text(&lang, batch.tutor_resource_id),
        room: resource_text(&lang, batch.room_resource_id),
        sessions: rows,
        tutor_history: history,
        waiting: waiting_rows,
        can_wait: batch.status == BatchStatus::Open && manage_enrolments,
        can_unwait: manage_enrolments,
        show_waiting: user.has_permission(permissions::ENROLMENTS_VIEW),
        from_date: today().to_string(),
        can_open: batch.status == BatchStatus::Planned
            && user.has_permission(permissions::BATCHES_MANAGE),
        can_change_tutor: batch.status.is_live()
            && user.has_permission(permissions::BATCHES_MANAGE),
    }))
}

/// `POST /edtech/batches/{id}/open`.
async fn open(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    batches::open(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "batches.opened")),
        renox::HxRefresh,
    )
        .into_response())
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
    /// The first day the new tutor teaches.
    #[serde(default)]
    pub from: String,
}

/// `POST /edtech/batches/{id}/tutor`.
async fn tutor(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<TutorForm>,
) -> Result<Response> {
    let resource = parse_number(&form.tutor)?;
    let employee = parse_optional_id(&form.employee)?;
    let from = if form.from.trim().is_empty() {
        tutors::default_from()
    } else {
        parse_date(&form.from)?
    };
    tutors::change_batch_tutor(&state, &user, id, resource, employee, from).await?;
    Ok((
        Toast::success(text(&lang, "tutors.changed")),
        renox::HxRefresh,
    )
        .into_response())
}

/// The form that puts a student on the waiting list.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct WaitingForm {
    /// The student (party).
    #[serde(default)]
    pub student: String,
    /// The payer (party), when not the student.
    #[serde(default)]
    pub payer: String,
}

/// `POST /edtech/batches/{id}/waiting`.
async fn join_waiting(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<WaitingForm>,
) -> Result<Response> {
    require(&user, permissions::ENROLMENTS_MANAGE)?;
    let student = party_of(&state, &form.student).await?;
    let payer = optional_party_of(&state, &form.payer).await?;
    waiting::join(&state, &user, id, student, payer).await?;
    Ok((
        Toast::success(text(&lang, "waiting.added")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /edtech/waiting/{id}/remove`.
async fn remove_waiting(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<Response> {
    waiting::remove(&state, &user, id).await?;
    Ok((
        Toast::success(text(&lang, "waiting.removed")),
        renox::HxRefresh,
    )
        .into_response())
}
