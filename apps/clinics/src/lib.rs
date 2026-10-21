#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod appointment;
pub mod appointments;
pub mod permissions;
pub mod reasons;
pub mod seed;
mod texts;

use renox::db::Migration;
use renox::prelude::*;

pub use appointment::Appointment;

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

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Appointment>();
        // The clinic's record follows the booking (`BookingBooked`,
        // `BookingRescheduled`, `BookingCancelled`).
        appointments::register(app);
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
