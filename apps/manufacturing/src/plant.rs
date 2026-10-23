//! The plant and the production line (07 Manufacturing §3.2, §6): the scopes
//! roles are assigned in, and the line carries the hourly rate (labour and
//! overhead, `MFG.CST.01`) and the locations of the store the materials come
//! from and the finished goods go to.

use renox::auth::permissions::Scope;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::LOCATION_SUBJECT_TYPE;

use crate::db::refused;
use crate::permissions;
use crate::texts::english;

/// The subject type of a plant.
pub const PLANT_SUBJECT_TYPE: &str = "manufacturing.plant";
/// The subject type of a production line.
pub const LINE_SUBJECT_TYPE: &str = "manufacturing.line";

/// A plant.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_plants")]
pub struct Plant {
    /// The primary key.
    pub id: i64,
    /// The name.
    pub name: String,
    /// Whether work orders can still be made for it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Plant {
    /// The scope roles are assigned in.
    pub fn scope(&self) -> Scope {
        permissions::plant_scope(self.id)
    }
}

/// A production line of a plant.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "manufacturing_lines", index(plant_id))]
pub struct Line {
    /// The primary key.
    pub id: i64,
    /// The plant it belongs to.
    pub plant_id: i64,
    /// The code, `L1`.
    pub code: String,
    /// The name.
    pub name: String,
    /// The hourly rate of labour and overhead, in fils.
    pub hourly_rate: i64,
    /// The store the materials are issued from, a wl-inventory location.
    pub store_location_id: Option<i64>,
    /// The finished goods location the output goes to.
    pub finished_location_id: Option<i64>,
    /// Whether work can still be scheduled on it.
    pub active: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Line {
    /// The scope supervisors and operators are assigned in.
    pub fn scope(&self) -> Scope {
        permissions::line_scope(self.id)
    }

    /// The store as a subject.
    pub fn store(&self) -> Option<Subject> {
        self.store_location_id
            .map(|id| Subject::new(LOCATION_SUBJECT_TYPE, id))
    }

    /// The finished goods location as a subject.
    pub fn finished_goods(&self) -> Option<Subject> {
        self.finished_location_id
            .map(|id| Subject::new(LOCATION_SUBJECT_TYPE, id))
    }
}

/// `by` adds a plant.
///
/// # Errors
///
/// `Forbidden` without `plants.manage`; a 422 for a blank name.
pub async fn create_plant(state: &AppState, by: &AuthUser, name: &str) -> Result<Plant> {
    if !by.has_permission(permissions::PLANTS_MANAGE) {
        return Err(Error::Forbidden);
    }
    if name.trim().is_empty() {
        return Err(refused(english("manufacturing.problems.plant_name", &[])));
    }
    Plant::create(
        &state.db,
        Plant {
            name: name.trim().to_owned(),
            active: true,
            ..Plant::default()
        },
    )
    .await
}

/// What a new line is made of.
#[derive(Debug, Clone, Default)]
pub struct NewLine {
    /// The plant.
    pub plant_id: i64,
    /// The code.
    pub code: String,
    /// The name.
    pub name: String,
    /// The hourly rate in fils.
    pub hourly_rate: i64,
    /// The store.
    pub store_location_id: Option<i64>,
    /// The finished goods location.
    pub finished_location_id: Option<i64>,
}

/// `by` adds a production line.
///
/// # Errors
///
/// `Forbidden` without `plants.manage` for the plant; a 422 for an unknown
/// plant, a blank code or name, a negative rate or a code in use.
pub async fn create_line(state: &AppState, by: &AuthUser, new: NewLine) -> Result<Line> {
    let plant = Plant::find(&state.db, new.plant_id)
        .await?
        .ok_or_else(|| refused(english("manufacturing.problems.plant_unknown", &[])))?;
    if !by.has_permission_in(permissions::PLANTS_MANAGE, &plant.scope()) {
        return Err(Error::Forbidden);
    }
    if new.code.trim().is_empty() || new.name.trim().is_empty() || new.hourly_rate < 0 {
        return Err(refused(english("manufacturing.problems.line_fields", &[])));
    }
    let taken = Line::query()
        .where_(Line::CODE.eq(new.code.trim()))
        .exists(&state.db)
        .await?;
    if taken {
        return Err(refused(english("manufacturing.problems.line_code", &[])));
    }
    Line::create(
        &state.db,
        Line {
            plant_id: plant.id,
            code: new.code.trim().to_owned(),
            name: new.name.trim().to_owned(),
            hourly_rate: new.hourly_rate,
            store_location_id: new.store_location_id,
            finished_location_id: new.finished_location_id,
            active: true,
            ..Line::default()
        },
    )
    .await
}
