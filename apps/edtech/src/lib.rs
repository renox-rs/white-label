#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod batch;
pub mod batch_machine;
pub mod batches;
pub mod certificate;
pub mod certificate_machine;
pub mod certificates;
pub mod course;
pub mod courses;
mod db;
pub mod enrolment;
pub mod enrolment_machine;
pub mod enrolments;
pub mod fees;
pub mod pages;
pub mod permissions;
pub mod plan;
pub mod reasons;
pub mod refund;
pub mod seats;
pub mod seed;
pub mod session;
pub mod session_machine;
pub mod sessions;
mod texts;
mod texts_more;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use batch::{Batch, BatchStatus};
pub use certificate::{Certificate, CertificateStatus};
pub use course::{Course, DeliveryMode};
pub use enrolment::{
    Enrolment, EnrolmentFreeze, EnrolmentKind, EnrolmentStatus, FreezeCause, Instalment,
    InstalmentStatus,
};
pub use session::{Session, SessionStatus};

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
        "edtech/certificates.html",
        include_str!("../views/edtech/certificates.html"),
    ),
    (
        "edtech/certificate.html",
        include_str!("../views/edtech/certificate.html"),
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
        app.view_data::<pages::courses::IndexPage>()
            .view_data::<pages::batches::IndexPage>()
            .view_data::<pages::batches::ShowPage>()
            .view_data::<pages::sessions::IndexPage>()
            .view_data::<pages::sessions::ShowPage>()
            .view_data::<pages::enrolments::IndexPage>()
            .view_data::<pages::enrolments::ShowPage>()
            .view_data::<pages::certificates::IndexPage>()
            .view_data::<pages::certificates::ShowPage>()
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
            .model::<Certificate>();
        // The batch and its sessions: the machines, the start of a batch on its
        // first session date, its completion after the last session, and the
        // blocks the sessions make in wl-scheduling.
        batches::register(app);
        // The enrolments: the machine, the fees and their daily run, the
        // first payment that activates one, and the certificate of one that
        // completes.
        enrolments::register(app);
        certificates::register();
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
