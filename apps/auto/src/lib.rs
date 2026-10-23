#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod db;
pub mod pages;
pub mod permissions;
pub mod reasons;
pub mod seed;
mod texts;
mod texts_vehicles;
pub mod vehicle;
pub mod vehicles;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use vehicle::{Emirate, Vehicle};

/// The templates of the pages, under the names the pages ask for. An app
/// replaces one by putting a file of that name in its own views.
const VIEWS: &[(&str, &str)] = &[
    (
        "auto/vehicles.html",
        include_str!("../views/auto/vehicles.html"),
    ),
    (
        "auto/vehicle.html",
        include_str!("../views/auto/vehicle.html"),
    ),
];

/// A whole number with its thousands grouped: `52,000`.
pub fn number_text(number: i64) -> String {
    let sign = if number < 0 { "-" } else { "" };
    let digits = number.unsigned_abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    format!("{sign}{grouped}")
}

/// Dirhams as text from fils: `AED 1,250.00`.
pub fn money_text(fils: i64) -> String {
    let sign = if fils < 0 { "-" } else { "" };
    let magnitude = fils.unsigned_abs();
    let whole = i64::try_from(magnitude / 100).unwrap_or(i64::MAX);
    format!("{sign}AED {}.{:02}", number_text(whole), magnitude % 100)
}

/// The Auto module. Add it to the app with `.module(auto::Auto)`.
pub struct Auto;

impl Module for Auto {
    fn name(&self) -> &'static str {
        "auto"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::vehicles::IndexPage>()
            .view_data::<pages::vehicles::ShowPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Vehicle>();
        // A bill or a document that names a vehicle shows its plate, not
        // `auto.vehicle:7`.
        register_labels();
        // In the app group of the staff sidebar; the label is the host's
        // `nav.vehicles` text.
        app.nav(
            "app",
            NavLink::route("auto.vehicles", "nav.vehicles")
                .icon("truck")
                .group("nav.app")
                .order(80)
                .permission(permissions::VEHICLES_VIEW),
        );
    }
}

/// How the documents and bills of other modules name the records of the app
/// (wl-documents `register_subject_label`): the vehicle by its plate.
fn register_labels() {
    use wl_kernel::documents::register_subject_label;
    register_subject_label(vehicle::VEHICLE_SUBJECT_TYPE, |db, id| {
        Box::pin(async move { Ok(Vehicle::find(db, id).await?.map(|vehicle| vehicle.plate())) })
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

    #[test]
    fn numbers_group_their_thousands() {
        assert_eq!(number_text(52_000), "52,000");
        assert_eq!(number_text(999), "999");
        assert_eq!(number_text(-1_234_567), "-1,234,567");
    }
}
