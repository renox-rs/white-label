//! The vertical's static tables (10 §7.1 *Booking*, §7.2, §9). The host turns
//! them into the document kinds, the service types and the ticket categories
//! it installs (`host/src/app/edtech_seed.rs`); the app depends on no crate
//! that owns those tables.

/// The centre permit of the emirate's regulator (KHDA in Dubai): a document
/// of the legal entity (§2, §9). It blocks `batch.open` and
/// `certificate.issue`, never overridden.
pub const CENTRE_PERMIT_KIND: &str = "centre_permit";

/// The subject type of a course (`edtech.course:7`): what a course approval is
/// attached to.
pub const COURSE_SUBJECT_TYPE: &str = "edtech.course";

/// The approval of a course by the regulator, where the emirate asks for one
/// (in Dubai every programme needs KHDA approval): a document of the course
/// (`EDU.CRS.02`).
pub const COURSE_APPROVAL_KIND: &str = "course_approval";

/// The good-conduct (police clearance) certificate of a tutor who works with
/// minors: best practice, not a legal block (§9).
pub const POLICE_CLEARANCE_KIND: &str = "police_clearance";

/// The MoHRE private tutor permit or the freelance visa of a freelance tutor
/// (§2, §9).
pub const TUTOR_PERMIT_KIND: &str = "tutor_permit";

/// The Emirates ID or passport of a student, kept by the front desk
/// (`PTY.KYC.01`); the image is sensitive when the student is a minor
/// (`EDU.ACL.09`).
pub const STUDENT_ID_KIND: &str = "student_emirates_id";

/// The guardian's signed consent to a minor's data and photos
/// (`DOC.SIG.01`, `PTY.CNS.01`, §9).
pub const GUARDIAN_CONSENT_KIND: &str = "guardian_consent";

/// The kinds of qualification no service type of the centre starts with (the
/// trial class and the placement test are not tutor-guarded). The one-to-one
/// session asks [`ONE_TO_ONE_QUALIFICATIONS`] (BL-EDU-17).
pub const REQUIRED_QUALIFICATIONS: &str = "";

/// The code of the one-to-one service type.
pub const ONE_TO_ONE_SERVICE: &str = "one_to_one";

/// The kinds a tutor must hold to take a one-to-one booking, as wl-scheduling
/// stores them on the service type (`required_qualifications`, `SCH.ACL.04`):
/// the police clearance. wl-scheduling asks the same kinds whoever the student
/// is, so an adult student's one-to-one session asks for it too (the spec asks
/// it only for a minor: `$S/f10/DEFERRED.md`, #452). The private tutor permit
/// or freelance visa is asked of freelance tutors only, by
/// [`crate::tutor_bookings`].
pub const ONE_TO_ONE_QUALIFICATIONS: &str = POLICE_CLEARANCE_KIND;

/// The days before the expiry of a tutor document that `DOC.RUL.01` reminds.
pub const REMINDER_DAYS: &[i64] = &[90, 60, 30, 7];

/// The look-ahead of the tutor kinds in days (`DOC.RUL.02`, §9).
pub const TUTOR_LOOK_AHEAD_DAYS: i64 = 30;

/// The look-ahead of the centre permit and the course approvals in days
/// (`EDU.RUL.13`, §9).
pub const CENTRE_LOOK_AHEAD_DAYS: i64 = 60;

/// The days before the expiry of a student's ID that the front desk is
/// reminded.
pub const STUDENT_ID_REMINDER_DAYS: &[i64] = &[90, 30, 7];

/// `SCH.RUL.06`: hours before the start at which an open conflict goes to the
/// centre manager (§7.1 *Booking*).
pub const ESCALATION_HOURS: i64 = 2;

/// `SCH.RUL.05`: days ahead that bookings are checked for eligibility.
pub const BOOKING_LOOK_AHEAD_DAYS: i64 = 30;

/// `SCH.RUL.01`: hours before a trial or one-to-one booking that the party is
/// reminded.
pub const REMINDER_HOURS: &[i64] = &[24, 2];

/// A service type the centre starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceSeed {
    /// The stable code (`trial_class`).
    pub code: &'static str,
    /// The name in English.
    pub name_en: &'static str,
    /// The name in Arabic.
    pub name_ar: &'static str,
    /// How long it takes.
    pub minutes: i64,
    /// The qualification kinds the person who delivers it must hold,
    /// comma separated (`SCH.ACL.04`).
    pub qualifications: &'static str,
}

/// The service types the host installs where the database has none of that
/// code (§4.1, §7.1 *Booking*): the trial class, the placement test and the
/// one-to-one session. Check-in is not used in 10; prices, deposits (none by
/// default) and the catalogue items are the centre's own.
pub const SERVICE_TYPES: &[ServiceSeed] = &[
    ServiceSeed {
        code: "trial_class",
        name_en: "Trial class",
        name_ar: "حصة تجريبية",
        minutes: 60,
        qualifications: REQUIRED_QUALIFICATIONS,
    },
    ServiceSeed {
        code: "placement_test",
        name_en: "Placement test",
        name_ar: "اختبار تحديد المستوى",
        minutes: 45,
        qualifications: REQUIRED_QUALIFICATIONS,
    },
    ServiceSeed {
        code: ONE_TO_ONE_SERVICE,
        name_en: "One-to-one session",
        name_ar: "حصة فردية",
        minutes: 60,
        qualifications: ONE_TO_ONE_QUALIFICATIONS,
    },
];

/// A ticket category the centre starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategorySpec {
    /// The stable key.
    pub key: &'static str,
    /// The name in English.
    pub name_en: &'static str,
    /// The name in Arabic.
    pub name_ar: &'static str,
    /// The role a ticket of the category goes to.
    pub assignee_role: Option<&'static str>,
}

/// The ticket categories of 10: complaints from students and guardians, the
/// classroom and equipment, and questions about fees and refunds.
pub const CATEGORIES: &[CategorySpec] = &[
    CategorySpec {
        key: "student_complaint",
        name_en: "Student or guardian complaint",
        name_ar: "شكوى طالب أو ولي أمر",
        assignee_role: Some(crate::permissions::CENTRE_MANAGER_ROLE),
    },
    CategorySpec {
        key: "classroom_issue",
        name_en: "Classroom or equipment",
        name_ar: "قاعة أو معدات",
        assignee_role: Some(crate::permissions::COORDINATOR_ROLE),
    },
    CategorySpec {
        key: "fee_query",
        name_en: "Fee or refund query",
        name_ar: "استفسار عن الرسوم أو الاسترداد",
        assignee_role: Some(crate::permissions::ACCOUNTANT_ROLE),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_distinct() {
        let mut keys: Vec<&str> = SERVICE_TYPES.iter().map(|s| s.code).collect();
        keys.extend(CATEGORIES.iter().map(|c| c.key));
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count);
    }

    #[test]
    fn the_centre_kinds_look_further_ahead_than_the_tutor_kinds() {
        assert_eq!(CENTRE_LOOK_AHEAD_DAYS, 60);
        assert_eq!(TUTOR_LOOK_AHEAD_DAYS, 30);
    }
}
