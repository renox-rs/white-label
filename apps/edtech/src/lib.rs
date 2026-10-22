#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod batch;
pub mod batch_machine;
pub mod batches;
pub mod course;
pub mod courses;
mod db;
pub mod pages;
pub mod permissions;
pub mod plan;
pub mod reasons;
pub mod seats;
pub mod seed;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use batch::{Batch, BatchStatus};
pub use course::{Course, DeliveryMode};

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
            .view_data::<pages::batches::ShowPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Course>().model::<Batch>();
        // The batch: its machine, its start when the first session date comes.
        batches::register(app);
        // In the app group of the staff sidebar; the labels are the host's
        // `nav.courses` and `nav.batches` texts.
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
