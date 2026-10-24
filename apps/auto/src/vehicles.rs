//! The vehicle services (08 `AUT.VEH.01`): registering a vehicle, changing
//! its details and recording the odometer.
//!
//! Decisions: a plate is an emirate, a code of up to three letters or digits
//! and one to six digits (`Dubai A 12345`); the VIN is the 17 characters of
//! ISO 3779 (no I, O or Q) and is unique when given; the odometer only goes
//! up, so a reading below the last one is refused; the owner is a party, and
//! a fleet vehicle names its fleet contract besides (the fleet contracts come
//! with their entity).

use renox::chrono::Datelike;
use renox::db::now;
use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::parties::directory;

use crate::db::refused;
use crate::permissions;
use crate::texts::english;
use crate::vehicle::{Emirate, Vehicle};

/// A vehicle to register or to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewVehicle {
    /// The emirate of the plate.
    pub emirate: Emirate,
    /// The plate code.
    pub code: String,
    /// The plate number.
    pub number: String,
    /// The VIN, when known.
    pub vin: Option<String>,
    /// The make.
    pub make: String,
    /// The model.
    pub model_name: String,
    /// The model year.
    pub model_year: Option<i64>,
    /// The engine.
    pub engine: Option<String>,
    /// The colour.
    pub colour: Option<String>,
    /// The owner party.
    pub owner_party_id: i64,
    /// The fleet contract, when the vehicle has one.
    pub fleet_contract_id: Option<i64>,
    /// The odometer reading in kilometres.
    pub odometer_km: i64,
}

impl NewVehicle {
    /// A vehicle `make` `model_name` with this plate, owned by party `owner`.
    pub fn new(
        emirate: Emirate,
        code: &str,
        number: &str,
        make: &str,
        model_name: &str,
        owner_party_id: i64,
    ) -> Self {
        NewVehicle {
            emirate,
            code: code.to_owned(),
            number: number.to_owned(),
            vin: None,
            make: make.to_owned(),
            model_name: model_name.to_owned(),
            model_year: None,
            engine: None,
            colour: None,
            owner_party_id,
            fleet_contract_id: None,
            odometer_km: 0,
        }
    }

    /// With this VIN.
    #[must_use]
    pub fn vin(mut self, vin: &str) -> Self {
        self.vin = Some(vin.to_owned());
        self
    }

    /// Of this model year.
    #[must_use]
    pub fn year(mut self, year: i64) -> Self {
        self.model_year = Some(year);
        self
    }

    /// With this engine.
    #[must_use]
    pub fn engine(mut self, engine: &str) -> Self {
        self.engine = Some(engine.to_owned());
        self
    }

    /// In this colour.
    #[must_use]
    pub fn colour(mut self, colour: &str) -> Self {
        self.colour = Some(colour.to_owned());
        self
    }

    /// Serviced under fleet contract `id`.
    #[must_use]
    pub fn fleet(mut self, id: i64) -> Self {
        self.fleet_contract_id = Some(id);
        self
    }

    /// Reading `km` on the odometer.
    #[must_use]
    pub fn odometer(mut self, km: i64) -> Self {
        self.odometer_km = km;
        self
    }
}

/// The text of a trimmed field, `None` when blank.
fn optional(text: &Option<String>) -> Option<String> {
    text.as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
}

/// The plate code in capitals: one to three letters or digits.
///
/// # Errors
///
/// A 422 for anything else.
pub fn clean_plate_code(code: &str) -> Result<String> {
    let code = code.trim().to_uppercase();
    if code.is_empty()
        || code.chars().count() > 3
        || !code.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(refused(english("auto.vehicles.problem.plate_code", &[])));
    }
    Ok(code)
}

/// The plate number: one to six digits.
///
/// # Errors
///
/// A 422 for anything else.
pub fn clean_plate_number(number: &str) -> Result<String> {
    let number = number.trim();
    if number.is_empty() || number.len() > 6 || !number.chars().all(|c| c.is_ascii_digit()) {
        return Err(refused(english("auto.vehicles.problem.plate_number", &[])));
    }
    let digits = number.trim_start_matches('0');
    Ok(if digits.is_empty() {
        "0".to_owned()
    } else {
        digits.to_owned()
    })
}

/// The VIN in capitals: 17 letters and digits without I, O and Q; `None`
/// when blank.
///
/// # Errors
///
/// A 422 for anything else.
pub fn clean_vin(vin: &Option<String>) -> Result<Option<String>> {
    let Some(vin) = optional(vin) else {
        return Ok(None);
    };
    let vin = vin.to_uppercase();
    let valid = vin.len() == 17
        && vin
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && !matches!(c, 'I' | 'O' | 'Q'));
    if !valid {
        return Err(refused(english("auto.vehicles.problem.vin", &[])));
    }
    Ok(Some(vin))
}

/// The fields of a vehicle, cleaned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    /// The plate code in capitals.
    pub code: String,
    /// The plate number.
    pub number: String,
    /// The VIN in capitals.
    pub vin: Option<String>,
    /// The make.
    pub make: String,
    /// The model.
    pub model_name: String,
    /// The model year.
    pub model_year: Option<i64>,
    /// The engine.
    pub engine: Option<String>,
    /// The colour.
    pub colour: Option<String>,
}

/// Checks the fields of `new` and cleans them.
///
/// # Errors
///
/// A 422 naming the first field that is wrong.
pub fn clean(new: &NewVehicle) -> Result<Cleaned> {
    let code = clean_plate_code(&new.code)?;
    let number = clean_plate_number(&new.number)?;
    let vin = clean_vin(&new.vin)?;
    let make = new.make.trim().to_owned();
    if make.is_empty() {
        return Err(refused(english("auto.vehicles.problem.make", &[])));
    }
    let model_name = new.model_name.trim().to_owned();
    if model_name.is_empty() {
        return Err(refused(english("auto.vehicles.problem.model", &[])));
    }
    if let Some(year) = new.model_year
        && !(1950..=i64::from(today().year()) + 1).contains(&year)
    {
        return Err(refused(english("auto.vehicles.problem.year", &[])));
    }
    if new.odometer_km < 0 {
        return Err(refused(english(
            "auto.vehicles.problem.odometer_negative",
            &[],
        )));
    }
    Ok(Cleaned {
        code,
        number,
        vin,
        make,
        model_name,
        model_year: new.model_year,
        engine: optional(&new.engine),
        colour: optional(&new.colour),
    })
}

/// Refuses a plate or a VIN another vehicle than `except` already has.
async fn ensure_free(db: &Db, emirate: Emirate, cleaned: &Cleaned, except: Option<i64>) -> Result {
    let by_plate = Vehicle::by_plate(db, emirate, &cleaned.code, &cleaned.number).await?;
    if let Some(found) = by_plate
        && Some(found.id) != except
    {
        return Err(refused(english(
            "auto.vehicles.problem.plate_taken",
            &[("plate", &found.plate())],
        )));
    }
    if let Some(vin) = &cleaned.vin {
        let by_vin = Vehicle::by_vin(db, vin).await?;
        if let Some(found) = by_vin
            && Some(found.id) != except
        {
            return Err(refused(english(
                "auto.vehicles.problem.vin_taken",
                &[("vin", vin), ("plate", &found.plate())],
            )));
        }
    }
    Ok(())
}

/// Refuses an owner that is not a party.
async fn ensure_owner(state: &AppState, owner: i64) -> Result {
    let found = directory(state)
        .display_name(&state.db, PartyId(owner), "en")
        .await?;
    if found.is_none() {
        return Err(refused(english("auto.vehicles.problem.owner", &[])));
    }
    Ok(())
}

/// Registers a vehicle (`AUT.VEH.01`).
///
/// # Errors
///
/// A 403 without `vehicles.manage`; a 422 for a field that is wrong, an owner
/// that is not a party, or a plate or VIN registered already.
pub async fn create(state: &AppState, by: &AuthUser, new: NewVehicle) -> Result<Vehicle> {
    if !by.has_permission(permissions::VEHICLES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let cleaned = clean(&new)?;
    ensure_owner(state, new.owner_party_id).await?;
    ensure_free(&state.db, new.emirate, &cleaned, None).await?;
    let mut vehicle = Vehicle {
        plate_emirate: new.emirate,
        plate_code: cleaned.code,
        plate_number: cleaned.number,
        vin: cleaned.vin,
        make: cleaned.make,
        model_name: cleaned.model_name,
        model_year: cleaned.model_year,
        engine: cleaned.engine,
        colour: cleaned.colour,
        owner_party_id: new.owner_party_id,
        fleet_contract_id: new.fleet_contract_id,
        odometer_km: new.odometer_km,
        odometer_at: (new.odometer_km > 0).then(now),
        active: true,
        created_by: Some(by.id),
        ..Vehicle::default()
    };
    vehicle.save(&state.db).await?;
    Vehicle::find_or_404(&state.db, vehicle.id).await
}

/// Changes the details of vehicle `id` to those of `new`. The odometer only
/// goes up.
///
/// # Errors
///
/// A 403 without `vehicles.manage`; a 422 as for [`create`], or an odometer
/// below the last reading.
pub async fn update(state: &AppState, by: &AuthUser, id: i64, new: NewVehicle) -> Result<Vehicle> {
    if !by.has_permission(permissions::VEHICLES_MANAGE) {
        return Err(Error::Forbidden);
    }
    let mut vehicle = Vehicle::find_or_404(&state.db, id).await?;
    let cleaned = clean(&new)?;
    ensure_owner(state, new.owner_party_id).await?;
    ensure_free(&state.db, new.emirate, &cleaned, Some(vehicle.id)).await?;
    ensure_odometer(&vehicle, new.odometer_km)?;
    if new.odometer_km > vehicle.odometer_km {
        vehicle.odometer_km = new.odometer_km;
        vehicle.odometer_at = Some(now());
    }
    vehicle.plate_emirate = new.emirate;
    vehicle.plate_code = cleaned.code;
    vehicle.plate_number = cleaned.number;
    vehicle.vin = cleaned.vin;
    vehicle.make = cleaned.make;
    vehicle.model_name = cleaned.model_name;
    vehicle.model_year = cleaned.model_year;
    vehicle.engine = cleaned.engine;
    vehicle.colour = cleaned.colour;
    vehicle.owner_party_id = new.owner_party_id;
    vehicle.fleet_contract_id = new.fleet_contract_id;
    vehicle.save(&state.db).await?;
    Vehicle::find_or_404(&state.db, id).await
}

/// Refuses an odometer `km` below the vehicle's last reading.
///
/// # Errors
///
/// A 422 naming the last reading.
pub fn ensure_odometer(vehicle: &Vehicle, km: i64) -> Result {
    if km < vehicle.odometer_km {
        return Err(refused(english(
            "auto.vehicles.problem.odometer",
            &[("km", &vehicle.odometer_km)],
        )));
    }
    Ok(())
}

/// The vehicles matching `text` (a plate, a VIN, a make or a model, in any
/// case), newest first; every vehicle when `text` is blank.
///
/// # Errors
///
/// The database's errors.
pub async fn search(db: &Db, text: &str) -> Result<Vec<Vehicle>> {
    let all = Vehicle::query().order_by_desc(Vehicle::ID).get(db).await?;
    let wanted = text.trim().to_lowercase();
    if wanted.is_empty() {
        return Ok(all);
    }
    Ok(all
        .into_iter()
        .filter(|vehicle| {
            let haystack = format!(
                "{} {} {} {} {}",
                vehicle.plate(),
                vehicle.vin.clone().unwrap_or_default(),
                vehicle.make,
                vehicle.model_name,
                vehicle.plate_number
            )
            .to_lowercase();
            wanted
                .split_whitespace()
                .all(|word| haystack.contains(word))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> NewVehicle {
        NewVehicle::new(Emirate::Dubai, "a", "12345", "Toyota", "Hilux", 1)
    }

    #[test]
    fn the_plate_code_is_one_to_three_letters_or_digits_in_capitals() {
        assert_eq!(clean_plate_code(" a ").unwrap(), "A");
        assert_eq!(clean_plate_code("22").unwrap(), "22");
        assert!(clean_plate_code("").is_err());
        assert!(clean_plate_code("ABCD").is_err());
        assert!(clean_plate_code("A-1").is_err());
    }

    #[test]
    fn the_plate_number_is_one_to_six_digits() {
        assert_eq!(clean_plate_number("12345").unwrap(), "12345");
        assert_eq!(clean_plate_number("007").unwrap(), "7");
        assert_eq!(clean_plate_number("0").unwrap(), "0");
        assert!(clean_plate_number("").is_err());
        assert!(clean_plate_number("1234567").is_err());
        assert!(clean_plate_number("12a").is_err());
    }

    #[test]
    fn a_vin_is_seventeen_characters_without_i_o_and_q() {
        let good = Some("jthbk262582012345".to_owned());
        assert_eq!(
            clean_vin(&good).unwrap(),
            Some("JTHBK262582012345".to_owned())
        );
        assert_eq!(clean_vin(&Some("  ".to_owned())).unwrap(), None);
        assert_eq!(clean_vin(&None).unwrap(), None);
        assert!(clean_vin(&Some("JTHBK26258201234".to_owned())).is_err());
        assert!(clean_vin(&Some("JTHBK2625820123IO".to_owned())).is_err());
    }

    #[test]
    fn make_and_model_are_required_and_the_year_is_plausible() {
        assert!(clean(&base()).is_ok());
        let mut blank_make = base();
        blank_make.make = " ".to_owned();
        assert!(clean(&blank_make).is_err());
        let mut blank_model = base();
        blank_model.model_name = String::new();
        assert!(clean(&blank_model).is_err());
        assert!(clean(&base().year(1900)).is_err());
        assert!(clean(&base().year(2022)).is_ok());
        assert!(clean(&base().odometer(-1)).is_err());
    }

    #[test]
    fn the_odometer_only_goes_up() {
        let vehicle = Vehicle {
            odometer_km: 52_000,
            ..Vehicle::default()
        };
        assert!(ensure_odometer(&vehicle, 52_000).is_ok());
        assert!(ensure_odometer(&vehicle, 60_000).is_ok());
        assert!(ensure_odometer(&vehicle, 51_999).is_err());
    }
}
