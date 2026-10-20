//! The portfolio's buildings and their floors (`PRP.PRT.01`, 01 Property §6).
//!
//! A building is the scope unit of the vertical (§3.2): roles are assigned
//! per building, and the building's issuing legal entity is the one linked to
//! it in the kernel (`ScopeEntity` with the subject type
//! [`BUILDING_SUBJECT_TYPE`], decision 0003).

use renox::auth::permissions::Scope;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

/// The subject type of a building, as the kernel's `ScopeEntity` and the
/// invoice drafts name it.
pub const BUILDING_SUBJECT_TYPE: &str = "property.building";

/// A property under management.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_buildings", unique(code))]
pub struct Building {
    /// The primary key.
    pub id: i64,
    /// The short code, upper case, unique (`TWR-A`).
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The emirate (`dubai`, `abu_dhabi`, ...).
    pub emirate: String,
    /// The community or area.
    pub community: Option<String>,
    /// The plot number.
    pub plot_number: Option<String>,
    /// The DEWA premise number (the ADDC account in Abu Dhabi).
    pub premise_number: Option<String>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Building {
    /// The building as a kernel subject.
    pub fn subject(&self) -> Subject {
        Subject::new(BUILDING_SUBJECT_TYPE, self.id)
    }

    /// The scope roles are assigned in for this building.
    pub fn scope(&self) -> Scope {
        Scope::of_id::<Building>(self.id)
    }

    /// The scope of the building `id`.
    pub fn scope_of(id: i64) -> Scope {
        Scope::of_id::<Building>(id)
    }

    /// The name to show: English, or Arabic when the language is `ar` and
    /// the building has an Arabic name.
    pub fn display_name(&self, locale: &str) -> String {
        match (&self.name_ar, locale) {
            (Some(ar), "ar") if !ar.trim().is_empty() => ar.clone(),
            _ => self.name_en.clone(),
        }
    }
}

/// A floor of a building.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "property_floors", unique(building_id, level))]
pub struct Floor {
    /// The primary key.
    pub id: i64,
    /// The building.
    #[model(references = "property_buildings")]
    pub building_id: i64,
    /// 0 is the ground floor, below 0 a basement.
    pub level: i64,
    /// The name (`Mezzanine`), else empty.
    pub name: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// What a user enters for a new building.
#[derive(Debug, Clone, Default)]
pub struct NewBuilding {
    /// The short code.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The name in Arabic.
    pub name_ar: Option<String>,
    /// The emirate; `dubai` when blank.
    pub emirate: String,
    /// The community.
    pub community: Option<String>,
    /// The plot number.
    pub plot_number: Option<String>,
    /// The premise number.
    pub premise_number: Option<String>,
}

/// A 422 with `message`.
pub(crate) fn refused(message: impl Into<String>) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message.into())
}

/// `text` trimmed, `None` when blank.
pub(crate) fn clean(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Creates a building.
///
/// # Errors
///
/// 422 for a blank code or name, or a code that is taken.
pub async fn create_building(db: &Db, new: NewBuilding) -> Result<Building> {
    let code = new.code.trim().to_uppercase();
    if code.is_empty() {
        return Err(refused(crate::texts::english(
            "property.buildings.code_required",
            &[],
        )));
    }
    if new.name_en.trim().is_empty() {
        return Err(refused(crate::texts::english(
            "property.buildings.name_required",
            &[],
        )));
    }
    let taken = Building::query()
        .where_(Building::CODE.eq(code.as_str()))
        .count(db)
        .await?;
    if taken > 0 {
        return Err(refused(crate::texts::english(
            "property.buildings.code_taken",
            &[],
        )));
    }
    let emirate = match new.emirate.trim() {
        "" => "dubai".to_owned(),
        other => other.to_lowercase(),
    };
    Building::create(
        db,
        Building {
            code,
            name_en: new.name_en.trim().to_owned(),
            name_ar: clean(new.name_ar),
            emirate,
            community: clean(new.community),
            plot_number: clean(new.plot_number),
            premise_number: clean(new.premise_number),
            ..Default::default()
        },
    )
    .await
}

/// Adds the floor `level` to `building_id`, or returns the one it has.
///
/// # Errors
///
/// A database error.
pub async fn ensure_floor(db: &Db, building_id: i64, level: i64, name: &str) -> Result<Floor> {
    if let Some(floor) = Floor::query()
        .where_(Floor::BUILDING_ID.eq(building_id))
        .where_(Floor::LEVEL.eq(level))
        .first(db)
        .await?
    {
        return Ok(floor);
    }
    Floor::create(
        db,
        Floor {
            building_id,
            level,
            name: name.trim().to_owned(),
            ..Default::default()
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arabic_name_is_used_only_for_arabic() {
        let building = Building {
            name_en: "Tower A".into(),
            name_ar: Some("برج أ".into()),
            ..Default::default()
        };
        assert_eq!(building.display_name("en"), "Tower A");
        assert_eq!(building.display_name("ar"), "برج أ");
        let plain = Building {
            name_en: "Tower B".into(),
            ..Default::default()
        };
        assert_eq!(plain.display_name("ar"), "Tower B");
    }

    #[test]
    fn a_building_is_a_scope_and_a_subject() {
        let building = Building {
            id: 7,
            ..Default::default()
        };
        assert_eq!(building.subject().to_string(), "property.building:7");
        assert_eq!(building.scope(), Building::scope_of(7));
    }
}
