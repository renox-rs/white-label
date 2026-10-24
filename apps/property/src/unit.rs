//! Units: the rentable spaces of a building, with their attributes and their
//! availability (`PRP.PRT.01`, `PRP.PRT.02`, 01 Property §6, §7.1 *Unit*).
//!
//! A unit's status moves only through the Unit machine
//! ([`crate::unit_machine`]); the lease machine drives it (`occupy` on
//! `activate` of a new lease, `mark_notice`, `vacate`).

use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::{Subject, VatCategory};

use crate::building::{Building, clean, refused};

/// The subject type of a unit.
pub const UNIT_SUBJECT_TYPE: &str = "property.unit";

/// What kind of space a unit is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnitType {
    /// An apartment.
    #[default]
    Apartment,
    /// A villa.
    Villa,
    /// An office.
    Office,
    /// A shop.
    Shop,
    /// A warehouse.
    Warehouse,
}

/// What a unit is used for; the VAT treatment of its rent follows it.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnitUse {
    /// Residential: rent is exempt from VAT.
    #[default]
    Residential,
    /// Commercial: rent is standard-rated at 5%.
    Commercial,
}

impl UnitUse {
    /// The VAT category of the rent of a unit with this use (§2: residential
    /// leases are exempt, commercial ones standard-rated).
    pub fn rent_vat(self) -> VatCategory {
        match self {
            UnitUse::Residential => VatCategory::Exempt,
            UnitUse::Commercial => VatCategory::Standard,
        }
    }
}

/// Where a unit stands (§7.1 *Unit*).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnitStatus {
    /// Can be leased.
    #[default]
    Available,
    /// Held for a prospect with a booking deposit (Later).
    Reserved,
    /// An active lease exists.
    Occupied,
    /// The tenant gave notice; the unit can be marketed.
    NoticeGiven,
    /// Not rentable until work is done.
    UnderMaintenance,
    /// Withdrawn by the owner or manager.
    OffMarket,
}

/// A rentable space.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "property_units",
    state_machine,
    unique(building_id, number),
    index(building_id, status)
)]
pub struct Unit {
    /// The primary key.
    pub id: i64,
    /// The building.
    #[model(references = "property_buildings")]
    pub building_id: i64,
    /// The floor, when recorded.
    pub floor_id: Option<i64>,
    /// The unit number inside the building (`1204`).
    pub number: String,
    /// The kind of space.
    pub unit_type: UnitType,
    /// Residential or commercial.
    pub usage: UnitUse,
    /// The size in square feet, in thousandths.
    pub area_sqft_milli: Option<i64>,
    /// The number of bedrooms.
    pub bedrooms: Option<i64>,
    /// The asking rent per year, in fils.
    pub asking_rent: Option<i64>,
    /// Free notes.
    pub notes: Option<String>,
    /// Where it stands; changed only by the machine.
    pub status: UnitStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Unit {
    /// The unit as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(UNIT_SUBJECT_TYPE, self.id)
    }

    /// The building's scope, where roles for this unit are looked for.
    pub fn scope(&self) -> renox::auth::permissions::Scope {
        Building::scope_of(self.building_id)
    }

    /// The units of a building, by number.
    pub async fn of_building(db: &Db, building_id: i64) -> Result<Vec<Unit>> {
        Unit::query()
            .where_(Unit::BUILDING_ID.eq(building_id))
            .order_by(Unit::NUMBER)
            .get(db)
            .await
    }
}

/// What a user enters for a new unit.
#[derive(Debug, Clone, Default)]
pub struct NewUnit {
    /// The building.
    pub building_id: i64,
    /// The floor, when recorded.
    pub floor_id: Option<i64>,
    /// The unit number.
    pub number: String,
    /// The kind of space.
    pub unit_type: UnitType,
    /// Residential or commercial.
    pub usage: UnitUse,
    /// The size in square feet, in thousandths.
    pub area_sqft_milli: Option<i64>,
    /// The number of bedrooms.
    pub bedrooms: Option<i64>,
    /// The asking rent per year, in fils.
    pub asking_rent: Option<i64>,
    /// Free notes.
    pub notes: Option<String>,
}

/// Creates a unit, `available`.
///
/// # Errors
///
/// 422 for a blank number, an unknown building, a number the building already
/// has, or a negative size or rent.
pub async fn create_unit(db: &Db, new: NewUnit) -> Result<Unit> {
    let number = new.number.trim().to_owned();
    if number.is_empty() {
        return Err(refused(crate::texts::english(
            "property.units.number_required",
            &[],
        )));
    }
    if Building::find(db, new.building_id).await?.is_none() {
        return Err(refused(crate::texts::english(
            "property.units.no_building",
            &[],
        )));
    }
    if [new.area_sqft_milli, new.asking_rent]
        .into_iter()
        .flatten()
        .any(|value| value < 0)
    {
        return Err(refused(crate::texts::english(
            "property.units.negative",
            &[],
        )));
    }
    let taken = Unit::query()
        .where_(Unit::BUILDING_ID.eq(new.building_id))
        .where_(Unit::NUMBER.eq(number.as_str()))
        .count(db)
        .await?;
    if taken > 0 {
        return Err(refused(crate::texts::english(
            "property.units.number_taken",
            &[],
        )));
    }
    let unit = Unit::create(
        db,
        Unit {
            building_id: new.building_id,
            floor_id: new.floor_id,
            number,
            unit_type: new.unit_type,
            usage: new.usage,
            area_sqft_milli: new.area_sqft_milli,
            bedrooms: new.bedrooms,
            asking_rent: new.asking_rent,
            notes: clean(new.notes),
            status: UnitStatus::Available,
            ..Default::default()
        },
    )
    .await?;
    // Findable from now on (BL-AGT-04).
    crate::search::index(db, &[unit.subject()]).await?;
    Ok(unit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rent_is_exempt_for_residential_and_standard_for_commercial() {
        assert_eq!(UnitUse::Residential.rent_vat(), VatCategory::Exempt);
        assert_eq!(UnitUse::Commercial.rent_vat(), VatCategory::Standard);
    }

    #[test]
    fn statuses_are_stored_in_snake_case() {
        assert_eq!(UnitStatus::NoticeGiven.as_str(), "notice_given");
        assert_eq!(UnitStatus::UnderMaintenance.as_str(), "under_maintenance");
        assert_eq!(UnitStatus::OffMarket.as_str(), "off_market");
        assert_eq!(UnitType::Warehouse.as_str(), "warehouse");
    }
}
