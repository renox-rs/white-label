#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod appointment;
pub mod appointments;
mod db;
pub mod pages;
pub mod permissions;
pub mod policies;
pub mod policy;
pub mod reasons;
pub mod seed;
mod texts;
pub mod visit;
pub mod visit_machine;
pub mod visits;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use appointment::Appointment;
pub use policy::{EligibilityCheck, InsurancePolicy};
pub use visit::{Visit, VisitStatus};

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory (`resources/views/clinics/policies.html`).
const VIEWS: &[(&str, &str)] = &[(
    "clinics/policies.html",
    include_str!("../views/clinics/policies.html"),
)];

/// Dirhams as text from fils: `AED 450.00`.
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

/// The Clinics module. Add it to the app with `.module(clinics::Clinics)`.
pub struct Clinics;

impl Module for Clinics {
    fn name(&self) -> &'static str {
        "clinics"
    }

    fn routes(&self) -> Routes {
        pages::routes()
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.view_data::<pages::policies::IndexPage>();
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
        app.model::<Appointment>()
            .model::<Visit>()
            .model::<InsurancePolicy>()
            .model::<EligibilityCheck>();
        visit_machine::register();
        // The clinic's record follows the booking (`BookingBooked`,
        // `BookingRescheduled`, `BookingCancelled`).
        appointments::register(app);
        // The check-in opens the visit, which follows the booking.
        visits::register(app);
        // In the app group of the staff sidebar; the label is the host's
        // `nav.insurance_policies` text.
        app.nav(
            "app",
            NavLink::route("clinics.policies", "nav.insurance_policies")
                .icon("shield-check")
                .group("nav.app")
                .order(70)
                .permission(permissions::POLICIES_VIEW),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_reads_as_dirhams_with_grouped_thousands() {
        assert_eq!(money_text(45_000), "AED 450.00");
        assert_eq!(money_text(5), "AED 0.05");
        assert_eq!(money_text(-123_456_789), "-AED 1,234,567.89");
        assert_eq!(money_text(0), "AED 0.00");
    }
}
