//! The course pages (10 `EDU.CRS.01`, `EDU.CRS.02`).
//!
//! | Route | Name | Needs |
//! |---|---|---|
//! | `GET /edtech/courses` | `edtech.courses` | `courses.view` |
//! | `POST /edtech/courses` | `edtech.course_create` | `courses.manage` |

use renox::prelude::*;
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

/// One course of the list.
#[derive(Serialize, Debug, Clone)]
pub struct CourseRow {
    /// The primary key.
    pub id: i64,
    /// The code.
    pub code: String,
    /// The name in the page's language.
    pub name: String,
    /// The programme.
    pub programme: String,
    /// The level, or a dash.
    pub level: String,
    /// The hours.
    pub hours: i64,
    /// How it is delivered.
    pub delivery: String,
    /// The regulator approval: the number, `Not recorded` or `Not needed`.
    pub approval: String,
    /// The badge kind of the approval.
    pub approval_kind: &'static str,
    /// Whether minors enrol.
    pub for_minors: String,
}

/// What the list page needs.
#[derive(Serialize, ViewData)]
#[view(template = "edtech/courses.html")]
pub(crate) struct IndexPage {
    tx: Texts,
    courses: Vec<CourseRow>,
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

/// `GET /edtech/courses`.
async fn index(State(state): State<AppState>, user: AuthUser, lang: Lang) -> Result<View> {
    require(&user, permissions::COURSES_VIEW)?;
    let all = Course::query()
        .order_by(Course::PROGRAMME)
        .order_by(Course::CODE)
        .get(&state.db)
        .await?;
    let dash = text(&lang, "none");
    let arabic = lang.locale.starts_with("ar");
    let rows = all
        .iter()
        .map(|c| {
            let (approval, approval_kind) = approval_of(&lang, c);
            CourseRow {
                id: c.id,
                code: c.code.clone(),
                name: if arabic {
                    c.name_ar.clone().unwrap_or_else(|| c.name_en.clone())
                } else {
                    c.name_en.clone()
                },
                programme: c.programme.clone(),
                level: c.level.clone().unwrap_or_else(|| dash.clone()),
                hours: c.hours,
                delivery: delivery_text(&lang, c.delivery),
                approval,
                approval_kind,
                for_minors: text(
                    &lang,
                    if c.for_minors {
                        "courses.yes"
                    } else {
                        "courses.no"
                    },
                ),
            }
        })
        .collect();
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
        courses: rows,
        delivery_options,
        can_manage: user.has_permission(permissions::COURSES_MANAGE),
    }))
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
