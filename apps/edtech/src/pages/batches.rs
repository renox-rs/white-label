//! The batch pages (10 `EDU.BAT.01`, §7.1 *Batch*).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/batches` | `edtech.batches` | `batches.view` |
//! | `POST /edtech/batches` | `edtech.batch_create` | `batches.manage` |
//! | `GET /edtech/batches/{id}` | `edtech.batch` | `batches.view` |
//! | `POST /edtech/batches/{id}/open` | `edtech.batch_open` | `batches.manage` |
//!
//! The page of a batch lists its sessions (see [`super::sessions`]).
//!
//! The list shows each batch with its dates, its weekly pattern, its tutor and
//! room, its seats and its status in words. The transitions are the
//! machine's: the pages only read the form and call [`crate::batches`].

use renox::prelude::*;
use serde::{Deserialize, Serialize};

use super::sessions::{SessionRow, row_of};
use super::{
    Texts, parse_date, parse_number, parse_optional_id, parse_weekdays, require, text,
};
use crate::batch::{Batch, BatchStatus};
use crate::batches::{self, NewBatch};
use crate::course::Course;
use crate::permissions;
use crate::seats;
use crate::session::Session;
use crate::texts;

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
    can_open: bool,
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
        can_open: batch.status == BatchStatus::Planned
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
