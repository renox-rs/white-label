//! The courses and programmes (10 `EDU.CRS.01`, `EDU.CRS.02`): defining one
//! and recording the regulator's approval.

use renox::prelude::*;

use crate::course::{Course, DeliveryMode};
use crate::db::refused;
use crate::permissions;
use crate::texts::english;

/// A course to define.
#[derive(Debug, Clone)]
pub struct NewCourse {
    /// The code the centre knows it by.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The programme.
    pub programme: String,
    /// The level.
    pub level: Option<String>,
    /// The hours of teaching.
    pub hours: i64,
    /// The prerequisites.
    pub prerequisites: Option<String>,
    /// How it is delivered.
    pub delivery: DeliveryMode,
    /// Whether minors enrol.
    pub for_minors: bool,
    /// Whether the regulator must approve it.
    pub approval_required: bool,
    /// The regulator's approval number.
    pub approval_no: Option<String>,
}

impl NewCourse {
    /// A course `code` named `name_en` in `programme`, taught `hours` hours,
    /// in person, for adults, needing no approval.
    pub fn new(code: &str, name_en: &str, programme: &str, hours: i64) -> Self {
        NewCourse {
            code: code.to_owned(),
            name_en: name_en.to_owned(),
            name_ar: None,
            programme: programme.to_owned(),
            level: None,
            hours,
            prerequisites: None,
            delivery: DeliveryMode::InPerson,
            for_minors: false,
            approval_required: false,
            approval_no: None,
        }
    }

    /// With the level.
    pub fn level(mut self, level: &str) -> Self {
        self.level = Some(level.to_owned());
        self
    }

    /// For minors.
    pub fn for_minors(mut self) -> Self {
        self.for_minors = true;
        self
    }

    /// Needing the regulator's approval, with its number when known.
    pub fn approval(mut self, number: Option<&str>) -> Self {
        self.approval_required = true;
        self.approval_no = number.map(str::to_owned);
        self
    }

    /// Delivered `delivery`.
    pub fn delivery(mut self, delivery: DeliveryMode) -> Self {
        self.delivery = delivery;
        self
    }
}

/// The text of `value`, trimmed; `None` when blank.
fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Defines a course.
///
/// # Errors
///
/// A 403 without `courses.manage`; a 422 without a code or a name, with
/// negative hours, or when the code is taken.
pub async fn create(state: &AppState, by: &AuthUser, new: NewCourse) -> Result<Course> {
    if !by.has_permission(permissions::COURSES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let code = new.code.trim().to_owned();
    let name = new.name_en.trim().to_owned();
    if code.is_empty() || name.is_empty() {
        return Err(refused(english("edtech.courses.problem.name", &[])));
    }
    if new.hours < 0 {
        return Err(refused(english("edtech.courses.problem.hours", &[])));
    }
    if Course::by_code(&state.db, &code).await?.is_some() {
        return Err(refused(english("edtech.courses.problem.code_taken", &[])));
    }
    let mut course = Course {
        code,
        name_en: name,
        name_ar: clean(&new.name_ar),
        programme: new.programme.trim().to_owned(),
        level: clean(&new.level),
        hours: new.hours,
        prerequisites: clean(&new.prerequisites),
        delivery: new.delivery,
        for_minors: new.for_minors,
        approval_required: new.approval_required,
        approval_no: clean(&new.approval_no),
        active: true,
        ..Course::default()
    };
    course.save(&state.db).await?;
    Ok(course)
}

/// Records the regulator's approval number of course `id` (`EDU.CRS.02`).
///
/// # Errors
///
/// A 403 without `courses.manage`; a 404 for an unknown course; a 422 without
/// a number.
pub async fn record_approval(
    state: &AppState,
    by: &AuthUser,
    id: i64,
    number: &str,
) -> Result<Course> {
    if !by.has_permission(permissions::COURSES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut course = Course::find_or_404(&state.db, id).await?;
    let number = number.trim();
    if number.is_empty() {
        return Err(refused(english("edtech.courses.problem.approval_no", &[])));
    }
    course.approval_required = true;
    course.approval_no = Some(number.to_owned());
    course.save(&state.db).await?;
    Ok(course)
}
