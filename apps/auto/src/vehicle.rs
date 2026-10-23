//! The vehicle: a customer's vehicle in the register (08 §6 *Vehicle*,
//! `AUT.VEH.01`).
//!
//! A vehicle has a UAE plate (an emirate, a code that is a letter or a
//! number, and a number: `Dubai A 12345`), a VIN, a make, a model and a year,
//! the latest odometer reading, and an owner party or a fleet contract. It is
//! checked in on a job card ([`crate::job_card`]); the service history is the
//! job cards of the vehicle, and the next service date and mileage feed the
//! reminder `AUT.RUL.01`.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

pub use crate::seed::VEHICLE_SUBJECT_TYPE;

/// The emirate of a plate.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Emirate {
    /// Abu Dhabi.
    AbuDhabi,
    /// Dubai.
    #[default]
    Dubai,
    /// Sharjah.
    Sharjah,
    /// Ajman.
    Ajman,
    /// Umm Al Quwain.
    UmmAlQuwain,
    /// Ras Al Khaimah.
    RasAlKhaimah,
    /// Fujairah.
    Fujairah,
}

impl Emirate {
    /// Every emirate, in the order a plate list offers them.
    pub const FORM_ORDER: [Emirate; 7] = [
        Emirate::Dubai,
        Emirate::AbuDhabi,
        Emirate::Sharjah,
        Emirate::Ajman,
        Emirate::UmmAlQuwain,
        Emirate::RasAlKhaimah,
        Emirate::Fujairah,
    ];

    /// The name on a plate, `Abu Dhabi`.
    pub fn label(self) -> &'static str {
        match self {
            Emirate::AbuDhabi => "Abu Dhabi",
            Emirate::Dubai => "Dubai",
            Emirate::Sharjah => "Sharjah",
            Emirate::Ajman => "Ajman",
            Emirate::UmmAlQuwain => "Umm Al Quwain",
            Emirate::RasAlKhaimah => "Ras Al Khaimah",
            Emirate::Fujairah => "Fujairah",
        }
    }

    /// The emirate written as `text`: its stored name (`abu_dhabi`) or its
    /// plate name (`Abu Dhabi`), in any case.
    pub fn parse(text: &str) -> Option<Emirate> {
        let wanted = text.trim();
        Emirate::FORM_ORDER.into_iter().find(|emirate| {
            emirate.as_str().eq_ignore_ascii_case(wanted)
                || emirate.label().eq_ignore_ascii_case(wanted)
        })
    }
}

/// A vehicle.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "auto_vehicles",
    unique(plate_emirate, plate_code, plate_number),
    unique(vin),
    index(owner_party_id),
    index(fleet_contract_id)
)]
pub struct Vehicle {
    /// The primary key.
    pub id: i64,
    /// The emirate of the plate.
    pub plate_emirate: Emirate,
    /// The code of the plate: a letter or a number (`A`, `22`).
    pub plate_code: String,
    /// The number of the plate (`12345`).
    pub plate_number: String,
    /// The VIN, 17 capital letters and digits; unique when given.
    pub vin: Option<String>,
    /// The make (`Toyota`).
    pub make: String,
    /// The model (`Hilux`).
    pub model_name: String,
    /// The year of the model.
    pub model_year: Option<i64>,
    /// The engine (`2.7 L petrol`).
    pub engine: Option<String>,
    /// The colour.
    pub colour: Option<String>,
    /// The owner (a party): the person, or the company that runs the fleet.
    pub owner_party_id: i64,
    /// The fleet contract the vehicle is serviced under, when it has one.
    pub fleet_contract_id: Option<i64>,
    /// The latest odometer reading in kilometres.
    pub odometer_km: i64,
    /// When that reading was taken.
    pub odometer_at: Option<DateTime>,
    /// The date the next service falls due (`AUT.JOB.04`).
    pub next_service_date: Option<NaiveDate>,
    /// The odometer reading the next service falls due at.
    pub next_service_odometer: Option<i64>,
    /// Whether the vehicle is still serviced here.
    pub active: bool,
    /// The user who registered it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Vehicle {
    /// The plate as people write it: `Dubai A 12345`.
    pub fn plate(&self) -> String {
        format!(
            "{} {} {}",
            self.plate_emirate.label(),
            self.plate_code,
            self.plate_number
        )
    }

    /// The vehicle for people: the plate and the make and model.
    pub fn title(&self) -> String {
        format!("{} · {} {}", self.plate(), self.make, self.model_name)
    }

    /// The vehicle as a [`Subject`]: `auto.vehicle:7`.
    pub fn subject(&self) -> Subject {
        Subject::new(VEHICLE_SUBJECT_TYPE, self.id)
    }

    /// Whether the vehicle is serviced under a fleet contract.
    pub fn is_fleet(&self) -> bool {
        self.fleet_contract_id.is_some()
    }

    /// The vehicle with this plate.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn by_plate(
        db: &Db,
        emirate: Emirate,
        code: &str,
        number: &str,
    ) -> Result<Option<Vehicle>> {
        Vehicle::query()
            .where_(Vehicle::PLATE_EMIRATE.eq(emirate))
            .where_(Vehicle::PLATE_CODE.eq(code.trim().to_uppercase()))
            .where_(Vehicle::PLATE_NUMBER.eq(number.trim()))
            .first(db)
            .await
    }

    /// The vehicle with this VIN.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn by_vin(db: &Db, vin: &str) -> Result<Option<Vehicle>> {
        Vehicle::query()
            .where_(Vehicle::VIN.eq(Some(vin.trim().to_uppercase())))
            .first(db)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plate_reads_emirate_code_number() {
        let vehicle = Vehicle {
            id: 4,
            plate_emirate: Emirate::AbuDhabi,
            plate_code: "12".to_owned(),
            plate_number: "34567".to_owned(),
            make: "Toyota".to_owned(),
            model_name: "Hilux".to_owned(),
            ..Vehicle::default()
        };
        assert_eq!(vehicle.plate(), "Abu Dhabi 12 34567");
        assert_eq!(vehicle.title(), "Abu Dhabi 12 34567 · Toyota Hilux");
        assert_eq!(vehicle.subject().to_string(), "auto.vehicle:4");
    }

    #[test]
    fn an_emirate_is_read_by_its_stored_name_or_its_plate_name() {
        assert_eq!(Emirate::parse("dubai"), Some(Emirate::Dubai));
        assert_eq!(Emirate::parse(" Abu Dhabi "), Some(Emirate::AbuDhabi));
        assert_eq!(Emirate::parse("umm_al_quwain"), Some(Emirate::UmmAlQuwain));
        assert_eq!(Emirate::parse("Riyadh"), None);
        for emirate in Emirate::FORM_ORDER {
            assert_eq!(Emirate::parse(emirate.as_str()), Some(emirate));
        }
    }

    #[test]
    fn a_vehicle_with_a_fleet_contract_is_a_fleet_vehicle() {
        let mut vehicle = Vehicle::default();
        assert!(!vehicle.is_fleet());
        vehicle.fleet_contract_id = Some(3);
        assert!(vehicle.is_fleet());
    }
}
