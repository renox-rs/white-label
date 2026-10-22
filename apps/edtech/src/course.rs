//! The course: what is taught (10 §6 *Course*, `EDU.CRS.01`, `EDU.CRS.02`).
//!
//! A course belongs to a programme, has a level, modules, hours,
//! prerequisites and a delivery mode, and records the regulator approval
//! where the emirate asks for one (in Dubai every programme needs KHDA
//! approval). The approval is also a document of the course
//! (`seed::COURSE_APPROVAL_KIND`) that blocks `batch.open`; the number kept
//! here is what a certificate prints when it states the approval. A course
//! marked *for minors* asks its tutors for a police clearance on every session
//! date (`EDU.ACL.14`).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

pub use crate::seed::COURSE_SUBJECT_TYPE;

/// How a course is delivered.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DeliveryMode {
    /// In a classroom.
    #[default]
    InPerson,
    /// Online (links are pasted per session in the MVP).
    Online,
    /// Both.
    Hybrid,
}

impl DeliveryMode {
    /// The stored name: `in_person`, `online` or `hybrid`.
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryMode::InPerson => "in_person",
            DeliveryMode::Online => "online",
            DeliveryMode::Hybrid => "hybrid",
        }
    }

    /// The mode stored as `text`.
    pub fn parse(text: &str) -> Option<DeliveryMode> {
        [
            DeliveryMode::InPerson,
            DeliveryMode::Online,
            DeliveryMode::Hybrid,
        ]
        .into_iter()
        .find(|mode| mode.as_str() == text.trim())
    }
}

/// A course.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "edtech_courses", unique(code), index(active))]
pub struct Course {
    /// The primary key.
    pub id: i64,
    /// The code the centre knows it by (`IELTS-A`); unique.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The programme it belongs to (`English language`).
    pub programme: String,
    /// The level (`B2`).
    pub level: Option<String>,
    /// The modules, as the centre writes them.
    pub modules: Option<String>,
    /// The hours of teaching.
    pub hours: i64,
    /// What a student must have first.
    pub prerequisites: Option<String>,
    /// How it is delivered.
    pub delivery: DeliveryMode,
    /// Whether minors enrol (the tutors need a police clearance).
    pub for_minors: bool,
    /// Whether the regulator must approve the course.
    pub approval_required: bool,
    /// The regulator's approval number, when recorded.
    pub approval_no: Option<String>,
    /// Whether new batches may be planned.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Course {
    /// The course `code`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn by_code(db: &Db, code: &str) -> Result<Option<Course>> {
        Course::query()
            .where_(Course::CODE.eq(code.trim()))
            .first(db)
            .await
    }

    /// The course as a [`Subject`]: `edtech.course:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(COURSE_SUBJECT_TYPE, self.id)
    }

    /// The name for people: the English name.
    pub fn title(&self) -> String {
        format!("{} {}", self.code, self.name_en)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_delivery_mode_round_trips_through_its_name() {
        for mode in [
            DeliveryMode::InPerson,
            DeliveryMode::Online,
            DeliveryMode::Hybrid,
        ] {
            assert_eq!(DeliveryMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(DeliveryMode::parse("by post"), None);
    }

    #[test]
    fn a_course_is_a_subject() {
        let course = Course {
            id: 7,
            ..Course::default()
        };
        assert_eq!(course.subject().to_string(), "edtech.course:7");
    }
}
