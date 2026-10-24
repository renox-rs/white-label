#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod absences;
pub mod attendance;
pub mod batch;
pub mod batch_machine;
pub mod batches;
pub mod blocked;
pub mod certificate;
pub mod certificate_machine;
pub mod certificates;
pub mod chargeback;
pub mod chargebacks;
pub mod completion;
pub mod corrections;
pub mod course;
pub mod courses;
pub mod credits;
pub mod dashboard;
mod db;
pub mod defaults;
pub mod desk;
pub mod documents;
pub mod eligibility;
pub mod enrolment;
pub mod enrolment_machine;
pub mod enrolments;
pub mod exit;
pub mod fees;
pub mod flags;
pub mod frozen;
pub mod lapse;
pub mod linked;
pub mod make_up;
pub mod money_defaults;
pub mod notices;
pub mod pages;
pub mod permissions;
pub mod permits;
pub mod plan;
pub mod reasons;
pub mod refund;
pub mod roster;
pub mod search;
pub mod seats;
pub mod seed;
pub mod session;
pub mod session_machine;
pub mod sessions;
pub mod settlement;
mod texts;
mod texts_alert;
mod texts_attendance;
mod texts_chargeback;
mod texts_desk;
mod texts_exit;
mod texts_hold;
mod texts_lapse;
mod texts_more;
mod texts_permit;
mod texts_reissue;
mod texts_seats;
mod texts_tutor;
pub mod transfer;
pub mod tutor_alert;
pub mod tutor_alerts;
pub mod tutor_assignment;
pub mod tutor_bookings;
pub mod tutors;
pub mod waiting;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use attendance::{Attendance, Mark};
pub use batch::{Batch, BatchStatus};
pub use certificate::{Certificate, CertificateStatus};
pub use chargeback::Chargeback;
pub use course::{Course, DeliveryMode};
pub use enrolment::{
    Enrolment, EnrolmentFreeze, EnrolmentKind, EnrolmentStatus, FreezeCause, Instalment,
    InstalmentStatus,
};
pub use make_up::MakeUpRight;
pub use session::{Session, SessionStatus};
pub use tutor_alert::TutorAlert;
pub use tutor_assignment::TutorAssignment;
pub use waiting::BatchWaiting;

/// The templates of the pages, under the names the pages ask for. An app
/// replaces one by putting a file of that name in its own views.
const VIEWS: &[(&str, &str)] = &[
    (
        "edtech/courses.html",
        include_str!("../views/edtech/courses.html"),
    ),
    (
        "edtech/batches.html",
        include_str!("../views/edtech/batches.html"),
    ),
    (
        "edtech/batch.html",
        include_str!("../views/edtech/batch.html"),
    ),
    (
        "edtech/sessions.html",
        include_str!("../views/edtech/sessions.html"),
    ),
    (
        "edtech/session.html",
        include_str!("../views/edtech/session.html"),
    ),
    (
        "edtech/enrolments.html",
        include_str!("../views/edtech/enrolments.html"),
    ),
    (
        "edtech/enrolment.html",
        include_str!("../views/edtech/enrolment.html"),
    ),
    (
        "edtech/collect.html",
        include_str!("../views/edtech/collect.html"),
    ),
    (
        "edtech/certificates.html",
        include_str!("../views/edtech/certificates.html"),
    ),
    (
        "edtech/certificate.html",
        include_str!("../views/edtech/certificate.html"),
    ),
    (
        "edtech/certificate_print.html",
        include_str!("../views/edtech/certificate_print.html"),
    ),
    (
        "edtech/verify.html",
        include_str!("../views/edtech/verify.html"),
    ),
];

/// Dirhams as text from fils: `AED 1,250.00`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let magnitude = fils.unsigned_abs();
    let whole = (magnitude / 100).to_string();
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}AED {grouped}.{:02}", magnitude % 100)
}

/// The EdTech module. Add it to the app with `.module(edtech::EdTech)`.
pub struct EdTech;

impl Module for EdTech {
    fn name(&self) -> &'static str {
        "edtech"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        // The records in the global search and the Related panel (BL-AGT-13).
        search::register();
        search::listen(app);
        app.view_data::<pages::courses::IndexPage>()
            .view_data::<pages::batches::IndexPage>()
            .view_data::<pages::batches::ShowPage>()
            .view_data::<pages::sessions::IndexPage>()
            .view_data::<pages::sessions::ShowPage>()
            .view_data::<pages::enrolments::IndexPage>()
            .view_data::<pages::enrolments::ShowPage>()
            .view_data::<pages::enrolments::CollectPage>()
            .view_data::<pages::certificates::IndexPage>()
            .view_data::<pages::certificates::ShowPage>()
            .view_data::<pages::certificates::PrintPage>()
            .view_data::<pages::certificates::VerifyPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Course>()
            .model::<Batch>()
            .model::<Session>()
            .model::<Enrolment>()
            .model::<EnrolmentFreeze>()
            .model::<Instalment>()
            .model::<Certificate>()
            .model::<TutorAssignment>()
            .model::<TutorAlert>()
            .model::<Attendance>()
            .model::<MakeUpRight>()
            .model::<BatchWaiting>()
            .model::<Chargeback>();
        // The batch and its sessions: the machines, the start of a batch on its
        // first session date, its completion after the last session, and the
        // blocks the sessions make in wl-scheduling.
        batches::register(app);
        // The credits the marks cost, taken from the packages after the commit.
        credits::register(app);
        // The enrolments: the machine, the fees and their daily run, the
        // first payment that activates one, and the certificate of one that
        // completes.
        enrolments::register(app);
        certificates::register(app);
        // The centre permit and the course approval block `batch.open` and
        // `certificate.issue`; their expiry tells the centre (`EDU.RUL.13`).
        documents::register();
        // The owner approves the one-use override of a police clearance block.
        defaults::override_policy().register();
        permits::register();
        permits::listen(app);
        notices::register_types();
        // A freelance tutor of a one-to-one booking holds the permit or visa.
        tutor_bookings::register(app);
        // A tutor who will not be eligible or available: the sessions flagged,
        // the coordinator's task and the centre manager told a day before
        // (`EDU.RUL.12`).
        tutor_alerts::register();
        tutor_alerts::listen(app);
        // A bill or a document that names a record shows `ENR-0007`, not
        // `edtech.enrolment:7`.
        register_labels();
        // In the app group of the staff sidebar; the labels are the host's
        // `nav.courses`, `nav.batches` and `nav.sessions` texts.
        app.nav(
            "app",
            NavLink::route("edtech.courses", "nav.courses")
                .icon("book-open")
                .group("nav.app")
                .order(80)
                .permission(permissions::COURSES_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("edtech.batches", "nav.batches")
                .icon("users")
                .group("nav.app")
                .order(81)
                .permission(permissions::BATCHES_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("edtech.sessions", "nav.sessions")
                .icon("calendar-days")
                .group("nav.app")
                .order(82)
                .permission(permissions::SESSIONS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("edtech.enrolments", "nav.enrolments")
                .icon("clipboard-list")
                .group("nav.app")
                .order(83)
                .permission(permissions::ENROLMENTS_VIEW),
        );
        app.nav(
            "app",
            NavLink::route("edtech.certificates", "nav.certificates")
                .icon("award")
                .group("nav.app")
                .order(84)
                .permission(permissions::CERTIFICATES_VIEW),
        );
    }
}

/// How the documents and bills of other modules name the records of the app:
/// the batch, the session, the enrolment, the certificate and the tutor
/// assignment (wl-documents `register_subject_label`), so that a document
/// kind's `blocks` entry on `batch.open`, `session.reschedule` or
/// `certificate.issue` shows `BAT-0007` and not `edtech.batch:7`. The
/// `BlockSubjects` providers themselves are BL-EDU-07's and BL-EDU-08's.
fn register_labels() {
    use wl_kernel::documents::register_subject_label;
    register_subject_label(batch::BATCH_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Batch::find(db, id).await?.map(|batch| batch.number())) })
    });
    register_subject_label(session::SESSION_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            let Some(found) = Session::find(db, id).await? else {
                return Ok(None);
            };
            let batch = Batch::find(db, found.batch_id)
                .await?
                .map_or_else(|| format!("#{}", found.batch_id), |batch| batch.number());
            Ok(Some(format!("{batch} · {}", found.number)))
        })
    });
    register_subject_label(enrolment::ENROLMENT_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(Enrolment::find(db, id)
                .await?
                .map(|enrolment| enrolment.number()))
        })
    });
    register_subject_label(certificate::CERTIFICATE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move {
            Ok(Certificate::find(db, id)
                .await?
                .map(|certificate| certificate.label()))
        })
    });
    register_subject_label(course::COURSE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Course::find(db, id).await?.map(|course| course.title())) })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_dirhams_with_grouped_thousands() {
        assert_eq!(money_text(125_000), "AED 1,250.00");
        assert_eq!(money_text(5), "AED 0.05");
        assert_eq!(money_text(-123_456_789), "-AED 1,234,567.89");
        assert_eq!(money_text(0), "AED 0.00");
    }
}
