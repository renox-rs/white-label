//! The course pages (10 `EDU.CRS.01`, `EDU.CRS.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/courses` | `edtech.courses` | `courses.view` |
//! | `POST /edtech/courses` | `edtech.course_create` | `courses.manage` |

use renox::grid::{Column, Grid, GridPage, GridRequest};
use renox::prelude::*;
use renox::serde_json::json;
use serde::{Deserialize, Serialize};

use super::{Texts, invalid, parse_number, require, text, ticked};
use crate::course::{Course, DeliveryMode};
use crate::courses::{self, NewCourse};
use crate::permissions;
use crate::texts;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .get("/edtech/courses", index)
        .name("edtech.courses")
        .post("/edtech/courses", create)
        .name("edtech.course_create")
        .require_auth()
}

/// The delivery mode as staff read it.
pub(crate) fn delivery_text(lang: &Lang, mode: DeliveryMode) -> String {
    text(lang, &format!("courses.delivery.{}", mode.as_str()))
}

/// The courses grid: search on the code, name and programme, filters by
/// delivery, programme and approval, and the export for who manages courses.
/// The approval badge is drawn by the template (`custom`).
pub(crate) fn grid(lang: &Lang, can_export: bool) -> Grid {
    let delivery: Vec<(String, String)> = [
        DeliveryMode::InPerson,
        DeliveryMode::Online,
        DeliveryMode::Hybrid,
    ]
    .into_iter()
    .map(|mode| (mode.as_str().to_owned(), delivery_text(lang, mode)))
    .collect();
    // The name in the page's language, the English one when there is none.
    let name = if lang.locale.starts_with("ar") {
        Column::text("name", &text(lang, "courses.name"))
            .computed("COALESCE({T}.\"name_ar\", {T}.\"name_en\")")
    } else {
        Column::text("name_en", &text(lang, "courses.name"))
    };
    let grid = Grid::new("edtech-courses")
        .title(&text(lang, "courses.title"))
        .per_page(25)
        .remember()
        .advanced_filter()
        .column(
            Column::text("code", &text(lang, "courses.code"))
                .frozen()
                .mobile()
                .searchable()
                .copyable(),
        )
        .column(name.mobile().searchable())
        .column(
            Column::text("programme", &text(lang, "courses.programme"))
                .mobile()
                .searchable(),
        )
        .column(Column::text("level", &text(lang, "courses.level")).hidden())
        .column(Column::number("hours", &text(lang, "courses.hours")).hidden())
        .column(Column::select(
            "delivery",
            &text(lang, "courses.delivery"),
            delivery,
        ))
        .column(Column::bool("for_minors", &text(lang, "courses.for_minors")).hidden())
        .column(Column::text("approval_no", &text(lang, "courses.approval_no")).hidden())
        .column(Column::custom("approval", &text(lang, "courses.approval")).mobile())
        .sort_by("programme,code")
        .cards_on_mobile()
        .empty_state(&text(lang, "courses.empty"), None);
    if can_export { grid.exports() } else { grid }
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/courses.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    courses: GridPage<Course>,
    delivery_options: Vec<[String; 2]>,
    can_manage: bool,
}

/// The approval as the list reads it.
fn approval_of(lang: &Lang, course: &Course) -> (String, &'static str) {
    match (&course.approval_no, course.approval_required) {
        (Some(number), _) => (number.clone(), "success"),
        (None, true) => (text(lang, "courses.approval_missing"), "warning"),
        (None, false) => (text(lang, "courses.approval_not_needed"), "info"),
    }
}

/// `GET /edtech/courses`, or its export (`?export=`, with `courses.manage`).
async fn index(user: AuthUser, lang: Lang, request: GridRequest) -> Result<Response> {
    require(&user, permissions::COURSES_VIEW)?;
    let can_export = user.has_permission(permissions::COURSES_MANAGE);
    let grid = grid(&lang, can_export);
    if request.param("export").is_some() && !can_export {
        return Err(Error::Forbidden);
    }
    let query = Course::query();
    if let Some(file) = grid.export(query.clone(), &request).await? {
        return Ok(file);
    }
    let page = grid.page(query, &request).await?.extend(|c| {
        let (approval, approval_kind) = approval_of(&lang, c);
        json!({ "approval": approval, "approval_kind": approval_kind })
    });
    let delivery_options = [
        DeliveryMode::InPerson,
        DeliveryMode::Online,
        DeliveryMode::Hybrid,
    ]
    .into_iter()
    .map(|mode| [mode.as_str().to_owned(), delivery_text(&lang, mode)])
    .collect();
    Ok(View::typed(IndexPage {
        tx: texts::all(&lang),
        courses: page,
        delivery_options,
        can_manage: user.has_permission(permissions::COURSES_MANAGE),
    })
    .into_response())
}

/// The form that defines a course.
#[derive(Deserialize, Serialize, Debug, Default)]
pub struct CreateForm {
    /// The code.
    #[serde(default)]
    pub code: String,
    /// The name in English.
    #[serde(default)]
    pub name_en: String,
    /// The name in Arabic.
    #[serde(default)]
    pub name_ar: String,
    /// The programme.
    #[serde(default)]
    pub programme: String,
    /// The level.
    #[serde(default)]
    pub level: String,
    /// The hours.
    #[serde(default)]
    pub hours: String,
    /// The delivery mode (`in_person`, `online`, `hybrid`).
    #[serde(default)]
    pub delivery: String,
    /// Ticked when minors enrol.
    #[serde(default)]
    pub for_minors: String,
    /// Ticked when the regulator must approve the course.
    #[serde(default)]
    pub approval_required: String,
    /// The regulator's approval number.
    #[serde(default)]
    pub approval_no: String,
}

/// `POST /edtech/courses`.
async fn create(
    State(state): State<AppState>,
    user: AuthUser,
    lang: Lang,
    Form(form): Form<CreateForm>,
) -> Result<Response> {
    require(&user, permissions::COURSES_MANAGE)?;
    let hours = if form.hours.trim().is_empty() {
        0
    } else {
        parse_number(&form.hours)?
    };
    let delivery = DeliveryMode::parse(&form.delivery)
        .ok_or_else(|| invalid("delivery: in_person, online or hybrid"))?;
    let mut new =
        NewCourse::new(&form.code, &form.name_en, &form.programme, hours).delivery(delivery);
    new.name_ar = Some(form.name_ar.clone());
    new.level = Some(form.level.clone());
    new.for_minors = ticked(&form.for_minors);
    new.approval_required = ticked(&form.approval_required) || !form.approval_no.trim().is_empty();
    new.approval_no = Some(form.approval_no.clone());
    courses::create(&state, &user, new).await?;
    Ok((
        Toast::success(text(&lang, "courses.created")),
        renox::HxRefresh,
    )
        .into_response())
}
