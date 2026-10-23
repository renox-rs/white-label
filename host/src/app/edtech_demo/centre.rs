//! The centre itself: the tutors and rooms as wl-scheduling resources, the
//! centre permit (it ends in 45 days, so a batch that runs later cannot be
//! opened), the courses the academic coordinator defines at the courses page
//! and the regulator approvals on file for two of them.

use edtech::seed::{CENTRE_PERMIT_KIND, COURSE_APPROVAL_KIND, COURSE_SUBJECT_TYPE};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_scheduling::{Resource, ResourceType};

use super::super::demo_data::{at, day};
use super::{COORDINATOR, HIND, LAYLA, OMAR, RAVI, SARA, Seed};

/// A course: code, English name, Arabic name, programme, level, hours,
/// delivery, minors, regulator approval number.
pub(crate) type CoursePlan = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    i64,
    &'static str,
    bool,
    &'static str,
);

/// The courses: language, IT, exam preparation and kids.
pub(crate) const COURSES: &[CoursePlan] = &[
    ("IELTS-B2", "IELTS Preparation (Academic)", "التحضير لاختبار الآيلتس", "Exam preparation", "B2", 40, "in_person", false, "KHDA-DEMO-0101"),
    ("ENG-A2", "General English A2", "اللغة الإنجليزية العامة A2", "Language", "A2", 48, "in_person", false, ""),
    ("FR-A1", "French for Beginners A1", "الفرنسية للمبتدئين A1", "Language", "A1", 30, "in_person", false, ""),
    ("ARB-KIDS", "Arabic for Kids (6-10)", "العربية للأطفال (6-10)", "Kids", "", 24, "hybrid", true, ""),
    ("CODE-KIDS", "Young Coders (8-12)", "المبرمجون الصغار (8-12)", "Kids", "", 20, "in_person", true, "KHDA-DEMO-0207"),
    ("PY-101", "Python Programming Foundations", "أساسيات البرمجة بلغة بايثون", "IT", "Beginner", 36, "online", false, ""),
    ("XL-BIZ", "Excel for Business", "إكسل للأعمال", "IT", "Intermediate", 12, "in_person", false, ""),
    ("GMAT-P", "GMAT Intensive", "دورة الجي مات المكثفة", "Exam preparation", "Advanced", 30, "in_person", false, ""),
];

/// The rooms and their seats.
const ROOMS: &[(&str, i64)] = &[
    ("Room A", 14),
    ("Room B", 12),
    ("Computer lab", 16),
    ("Kids studio", 10),
    ("Seminar room", 20),
];

impl Seed {
    async fn resource(
        &self,
        name: &str,
        kind: ResourceType,
        employee: Option<i64>,
        capacity: i64,
    ) -> Result<i64> {
        let mut resource = Resource::new(name, kind);
        if let Some(employee) = employee {
            resource = resource.employee(employee).role("tutor");
        }
        resource.capacity = capacity;
        resource.save(self.db()).await?;
        Ok(resource.id)
    }

    /// The tutors and rooms, the centre permit, the courses and the approvals.
    pub(crate) async fn centre(&mut self) -> Result {
        for (email, name) in [
            (SARA, "Sara Whitfield"),
            (OMAR, "Omar Siddiqui"),
            (LAYLA, "Layla Hassan"),
            (RAVI, "Ravi Menon"),
            (HIND, "Hind Al Shamsi"),
        ] {
            let employee = self.employee_of(email);
            let id = self
                .resource(name, ResourceType::Person, Some(employee), 1)
                .await?;
            self.tutors.push((email, id));
        }
        for (name, seats) in ROOMS {
            let id = self.resource(name, ResourceType::Room, None, *seats).await?;
            self.rooms.insert(name, id);
        }
        // The centre permit of the regulator ends in 45 days: the batches that
        // run beyond it cannot be opened until the centre manager renews it.
        self.document(
            Subject::new("kernel.legal_entity", self.entity),
            CENTRE_PERMIT_KIND,
            "KHDA-CP-DEMO-5521",
            day(45),
            "Knowledge and Human Development Authority",
        )
        .await?;
        for plan in COURSES {
            let (code, en, ar, programme, level, hours, delivery, minors, approval) = *plan;
            let hours = hours.to_string();
            let mut form = vec![
                ("code", code),
                ("name_en", en),
                ("name_ar", ar),
                ("programme", programme),
                ("level", level),
                ("hours", hours.as_str()),
                ("delivery", delivery),
            ];
            if minors {
                form.push(("for_minors", "on"));
            }
            if !approval.is_empty() {
                form.push(("approval_no", approval));
            }
            at!(self, day(-120), {
                self.driver.post(COORDINATOR, "/edtech/courses", &form).await
            })?;
            let course = edtech::Course::by_code(self.db(), code)
                .await?
                .ok_or_else(|| renox::anyhow::anyhow!("demo seed: the course {code}"))?;
            self.courses.insert(code, course.id);
            if !approval.is_empty() {
                self.document(
                    Subject::new(COURSE_SUBJECT_TYPE, course.id),
                    COURSE_APPROVAL_KIND,
                    approval,
                    day(300),
                    "Knowledge and Human Development Authority",
                )
                .await?;
            }
        }
        Ok(())
    }
}
