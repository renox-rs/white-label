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
//! | `POST /edtech/batches/{id}/cancel` | `edtech.batch_cancel` | `batches.manage` |
//! | `POST /edtech/batches/{id}/terminate` | `edtech.batch_terminate` | `batches.manage` |
//!
//! The page of a batch lists its sessions (see [`super::sessions`]).
//!
//! The list shows each batch with its dates, its weekly pattern, its tutor and
//! room, its seats and its status in words. The transitions are the
//! machine's: the pages only read the form and call [`crate::batches`].

use std::collections::BTreeMap;

use renox::grid::{Column, Grid, GridPage, GridRequest, Summary};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};
use wl_kernel::approvals::{self, ApprovalCard};
use wl_kernel::documents::{dubai_text, today};
use wl_kernel::scheduling::{ResourceChoice, ResourceChoiceKind};

use super::sessions::{SessionRow, row_of};
use super::{
    Texts, employee_of, optional_party_of, parse_date, parse_number, parse_optional_id,
    parse_weekdays, party_of, require, resource_choices, resource_options, text,
};
use super::{names_of, resource_text};
use crate::batch::{Batch, BatchStatus};
use crate::batches::{self, NewBatch};
use crate::course::Course;
use crate::enrolment::Enrolment;
use crate::enrolments::party_name;
use crate::exit::ExitChoice;
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
        .post("/edtech/batches/{id}/cancel", cancel)
        .name("edtech.batch_cancel")
        .post("/edtech/batches/{id}/terminate", terminate)
        .name("edtech.batch_terminate")
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

/// The batches grid: search on the number and the course, filters by course,
/// tutor, room and status, sums of the seats, and the export for who manages
/// batches. The weekly pattern and the seats in words are drawn by the
/// template (`custom`).
pub(crate) fn grid(
    lang: &Lang,
    courses: &[Course],
    tutors: Vec<(String, String)>,
    rooms: Vec<(String, String)>,
    can_export: bool,
) -> Grid {
    let statuses: Vec<(String, String)> = [
        BatchStatus::Planned,
        BatchStatus::Open,
        BatchStatus::Running,
        BatchStatus::Completed,
        BatchStatus::Cancelled,
        BatchStatus::Terminated,
    ]
    .into_iter()
    .map(|status| (status.as_str().to_owned(), status_text(lang, status)))
    .collect();
    let course_options: Vec<(String, String)> = courses
        .iter()
        .map(|c| (c.id.to_string(), c.title()))
        .collect();
    let grid = Grid::new("edtech-batches")
        .title(&text(lang, "batches.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("number", &text(lang, "batches.number"))
                .computed(
                    "'BAT-' || CASE WHEN {T}.\"id\" < 10 THEN '000' WHEN {T}.\"id\" < 100 THEN '00' \
                     WHEN {T}.\"id\" < 1000 THEN '0' ELSE '' END || CAST({T}.\"id\" AS TEXT)",
                )
                .frozen()
                .mobile()
                .searchable()
                .link("/edtech/batches/{id}"),
        )
        .column(
            Column::select("course_id", &text(lang, "batches.course"), course_options).mobile(),
        )
        .column(
            Column::text("course", &text(lang, "batches.course"))
                .computed(
                    "(SELECT c.\"code\" || ' ' || c.\"name_en\" FROM \"edtech_courses\" c \
                     WHERE c.\"id\" = {T}.\"course_id\")",
                )
                .hidden()
                .searchable(),
        )
        .column(Column::date("start_date", &text(lang, "batches.start_date")))
        .column(Column::date("end_date", &text(lang, "batches.end_date")).hidden())
        .column(Column::custom("pattern", &text(lang, "batches.pattern")))
        .column(Column::select(
            "tutor_resource_id",
            &text(lang, "batches.tutor"),
            tutors,
        ))
        .column(Column::select(
            "room_resource_id",
            &text(lang, "batches.room"),
            rooms,
        ))
        .column(Column::custom("seats", &text(lang, "batches.seats")).mobile())
        .column(
            Column::number("capacity", &text(lang, "batches.capacity"))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::number("seats_taken", &text(lang, "batches.seats"))
                .hidden()
                .summary(Summary::Sum),
        )
        .column(
            Column::select("status", &text(lang, "fields.status"), statuses)
                .badges(&[
                    ("planned", "info"),
                    ("open", "info"),
                    ("running", "success"),
                    ("completed", "success"),
                    ("cancelled", "info"),
                    ("terminated", "danger"),
                ])
                .mobile(),
        )
        .sort_by("-id")
        .row_url("/edtech/batches/{id}")
        .cards_on_mobile()
        .empty_state(&text(lang, "batches.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/batches.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    batches: GridPage<Batch>,
    course_options: Vec<[String; 2]>,
    course_selected: String,
    tutor_options: Vec<[String; 2]>,
    room_options: Vec<[String; 2]>,
    can_manage: bool,
}

/// A resource as a grid option, `(id, name)`.
fn grid_options(choices: Vec<ResourceChoice>) -> Vec<(String, String)> {
    choices
        .into_iter()
        .map(|c| (c.id.to_string(), c.name))
        .collect()
}

/// `GET /edtech/batches`, or its export (`?export=`, with `batches.manage`).
async fn index(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    request: GridRequest,
) -> Result<Response> {
    require(&user, permissions::BATCHES_VIEW)?;
    let own = crate::sessions::assigned_scope(&state, &user).await?;
    let mine = match own {
        Some(employee) => Some(crate::sessions::assigned_batches(&state.db, employee).await?),
        None => None,
    };
    let courses = Course::query()
        .order_by(Course::CODE)
        .get(&state.db)
        .await?;
    let mut tutors = resource_choices(&state, ResourceChoiceKind::Person).await;
    // A tutor's tutor filter names only themselves, never the other tutors.
    if let Some(employee) = own {
        tutors.retain(|c| c.employee_id == Some(employee));
    }
    let rooms = resource_choices(&state, ResourceChoiceKind::Room).await;
    let can_export = user.has_permission(permissions::BATCHES_MANAGE);
    let grid = grid(
        &lang,
        &courses,
        grid_options(tutors),
        grid_options(rooms),
        can_export,
    );
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    // A tutor sees the batches they are assigned to (`EDU.ACL.03`).
    let query = match mine {
        Some(ids) => {
            Batch::query().where_(Batch::ID.is_in(if ids.is_empty() { vec![-1] } else { ids }))
        }
        None => Batch::query(),
    };
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?.extend(|b| {
        json!({
            "pattern": pattern_text(&lang, b),
            "seats": seats_text(&lang, b),
        })
    });
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
        batches: page,
        course_options,
        course_selected,
        tutor_options: resource_options(
            &state,
            ResourceChoiceKind::Person,
            Some(text(&lang, "none")),
        )
        .await,
        room_options: resource_options(&state, ResourceChoiceKind::Room, Some(text(&lang, "none")))
            .await,
        can_manage: user.has_permission(permissions::BATCHES_MANAGE),
    })
    .into_response())
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
    let mut new = NewBatch::new(
        course,
        capacity,
        dates,
        &weekdays,
        &form.start_time,
        duration,
    );
    new.tutor_resource_id = parse_optional_id(&form.tutor)?;
    new.tutor_employee_id = match (parse_optional_id(&form.employee)?, new.tutor_resource_id) {
        (Some(employee), _) => Some(employee),
        (None, Some(resource)) => employee_of(&state, resource).await,
        (None, None) => None,
    };
    new.room_resource_id = parse_optional_id(&form.room)?;
    new.room_limit = match (parse_optional_id(&form.room_limit)?, new.room_resource_id) {
        (Some(limit), _) => Some(limit),
        // The persons the room holds: its capacity, when it states one.
        (None, Some(room)) => resource_choices(&state, ResourceChoiceKind::Room)
            .await
            .into_iter()
            .find(|choice| choice.id == room)
            .map(|choice| choice.capacity)
            .filter(|capacity| *capacity > 1),
        (None, None) => None,
    };
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
    tutor_options: Vec<[String; 2]>,
    can_open: bool,
    can_change_tutor: bool,
    can_wait: bool,
    can_unwait: bool,
    show_waiting: bool,
    can_cancel: bool,
    can_terminate: bool,
    exit_rows: Vec<ExitRow>,
    exit_options: Vec<[String; 2]>,
    reason_options: Vec<[String; 2]>,
    pending: bool,
    approvals: Vec<ApprovalCard>,
}

/// One enrolment of the page that a `cancel` or `terminate` must say a choice
/// for.
#[derive(Serialize, Debug, Clone)]
pub struct ExitRow {
    /// The enrolment's number (`7`), which names the form field.
    pub id: i64,
    /// `ENR-0007 · Sara Ali`.
    pub label: String,
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
/// `GET /edtech/batches/{id}`.
async fn show(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
) -> Result<View> {
    require(&user, permissions::BATCHES_VIEW)?;
    let batch = Batch::find_or_404(&state.db, id).await?;
    if let Some(employee) = crate::sessions::assigned_scope(&state, &user).await?
        && !crate::sessions::assigned_batches(&state.db, employee)
            .await?
            .contains(&batch.id)
    {
        return Err(Error::Forbidden);
    }
    let course = Course::find_or_404(&state.db, batch.course_id).await?;
    let number = batch.number();
    let planned = Session::of_batch(&state.db, batch.id).await?;
    let assignments = tutors::history(&state.db, batch.id).await?;
    let names = names_of(
        &state,
        planned
            .iter()
            .flat_map(|s| [s.tutor_resource_id, s.room_resource_id])
            .chain(assignments.iter().map(|a| Some(a.tutor_resource_id)))
            .chain([batch.tutor_resource_id, batch.room_resource_id]),
    )
    .await;
    let rows = planned
        .iter()
        .map(|s| row_of(&lang, &names, s, &number))
        .collect();
    let history = assignments
        .into_iter()
        .map(|a| TutorRow {
            tutor: resource_text(&lang, &names, Some(a.tutor_resource_id)),
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
    let manage_batches = user.has_permission(permissions::BATCHES_MANAGE);
    let can_cancel =
        manage_batches && matches!(batch.status, BatchStatus::Planned | BatchStatus::Open);
    let can_terminate = manage_batches && batch.status == BatchStatus::Running;
    let mut exit_rows = Vec::new();
    let mut exit_options = Vec::new();
    if can_cancel || can_terminate {
        for e in Enrolment::seated_in(&state.db, batch.id).await? {
            exit_rows.push(ExitRow {
                id: e.id,
                label: format!(
                    "{} · {}",
                    e.number(),
                    party_name(&state, &lang.locale, e.student_party_id).await
                ),
            });
        }
        exit_options.push(["withdraw".to_owned(), text(&lang, "exit.withdraw")]);
        let others = Batch::query()
            .where_(Batch::COURSE_ID.eq(batch.course_id))
            .where_(Batch::STATUS.is_in([BatchStatus::Open, BatchStatus::Running]))
            .order_by(Batch::ID)
            .get(&state.db)
            .await?;
        for other in others.iter().filter(|o| o.id != batch.id) {
            exit_options.push([
                other.id.to_string(),
                format!(
                    "{} {} · {} {}",
                    text(&lang, "exit.transfer_to"),
                    other.number(),
                    seats::free(other),
                    text(&lang, "enrolments.seats_free")
                ),
            ]);
        }
    }
    let reason_options = batches::TERMINATION_REASONS
        .iter()
        .map(|(code, _)| {
            [
                (*code).to_owned(),
                text(&lang, &format!("batches.termination.{code}")),
            ]
        })
        .collect();
    let subject = batch.subject();
    let pending = !approvals::pending_for(&state.db, &subject)
        .await?
        .is_empty();
    let approval_cards = approvals::cards_for(&state.db, &subject, &user, &lang).await?;
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
        tutor: resource_text(&lang, &names, batch.tutor_resource_id),
        room: resource_text(&lang, &names, batch.room_resource_id),
        sessions: rows,
        tutor_history: history,
        waiting: waiting_rows,
        can_wait: batch.status == BatchStatus::Open && manage_enrolments,
        can_unwait: manage_enrolments,
        show_waiting: user.has_permission(permissions::ENROLMENTS_VIEW),
        can_cancel,
        can_terminate,
        exit_rows,
        exit_options,
        reason_options,
        pending,
        approvals: approval_cards,
        from_date: today().to_string(),
        tutor_options: resource_options(&state, ResourceChoiceKind::Person, None).await,
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
    let employee = match parse_optional_id(&form.employee)? {
        Some(employee) => Some(employee),
        None => employee_of(&state, resource).await,
    };
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

/// The choices of a form: a field `choice_<enrolment>` holds `withdraw` or the
/// number of the batch the student moves to.
fn choices_of(form: &BTreeMap<String, String>) -> std::result::Result<Vec<ExitChoice>, Error> {
    let mut choices = Vec::new();
    for (name, value) in form {
        let Some(id) = name.strip_prefix("choice_") else {
            continue;
        };
        let enrolment = parse_number(id)?;
        let value = value.trim();
        if value.is_empty() || value == "withdraw" {
            choices.push(ExitChoice::withdraw(enrolment));
        } else {
            choices.push(ExitChoice::transfer(enrolment, parse_number(value)?));
        }
    }
    Ok(choices)
}

/// `POST /edtech/batches/{id}/cancel`: a `reason` and a `choice_<enrolment>` per
/// enrolment.
async fn cancel(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let reason = form.get("reason").cloned().unwrap_or_default();
    batches::cancel(&state, &user, id, &reason, choices_of(&form)?).await?;
    Ok((
        Toast::success(text(&lang, "batches.cancelled")),
        renox::HxRefresh,
    )
        .into_response())
}

/// `POST /edtech/batches/{id}/terminate`: a `reason`, a `note` and a
/// `choice_<enrolment>` per enrolment.
async fn terminate(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Path(id): Path<i64>,
    Form(form): Form<BTreeMap<String, String>>,
) -> Result<Response> {
    let code = form.get("reason").cloned().unwrap_or_default();
    let note = form.get("note").cloned();
    let asked = batches::terminate(
        &state,
        &user,
        id,
        &code,
        note.as_deref(),
        choices_of(&form)?,
    )
    .await?;
    let key = if asked.approval_request.is_some() {
        "batches.termination_sent"
    } else {
        "batches.terminated"
    };
    Ok((Toast::success(text(&lang, key)), renox::HxRefresh).into_response())
}
