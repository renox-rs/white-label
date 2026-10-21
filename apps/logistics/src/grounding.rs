//! A vehicle whose documents expired is grounded (03 Logistics §7.2
//! `LOG.RUL.10`, §9 *Document blocks*; BL-LOG-25).
//!
//! The rule runs daily (the host's `logistics_rules`, which reads the vehicles
//! of the open runs from wl-delivery and asks the document guard for the
//! documents of each): for a vehicle with a blocking vehicle document that has
//! expired, [`ground`] writes a [`VehicleGrounding`] and tells the fleet
//! managers and the dispatchers. Dispatch is refused by the document guard on
//! its own; the grounding is the record the fleet manager works from and the
//! notice that makes the expiry known the day it happens. A vehicle record
//! with a status is the inventory module's (`inventory.asset`, not built), so
//! "grounded" is this record, not a state of the vehicle.

use renox::auth::permissions::users_with_role;
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;

use crate::defaults::VEHICLE_SUBJECT;
use crate::notices::tell_user;
use crate::permissions::{DISPATCHER_ROLE, FLEET_MANAGER_ROLE};
use crate::texts::english;

/// A vehicle taken off the road for an expired document.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "logistics_vehicle_groundings",
    unique(vehicle_asset_id, document_kind, expired_on),
    index(vehicle_asset_id)
)]
pub struct VehicleGrounding {
    /// The primary key.
    pub id: i64,
    /// The vehicle (`inventory.asset`).
    pub vehicle_asset_id: i64,
    /// The document kind that expired.
    pub document_kind: String,
    /// The day the document expired.
    pub expired_on: NaiveDate,
    /// The day the rule grounded the vehicle.
    pub grounded_on: NaiveDate,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl VehicleGrounding {
    /// The vehicle as a [`Subject`].
    pub fn vehicle(&self) -> Subject {
        Subject::new(VEHICLE_SUBJECT, self.vehicle_asset_id)
    }
}

/// The groundings of the vehicle, the first first.
///
/// # Errors
///
/// Database errors.
pub async fn groundings_of(db: &Db, vehicle_asset_id: i64) -> Result<Vec<VehicleGrounding>> {
    VehicleGrounding::query()
        .where_(VehicleGrounding::VEHICLE_ASSET_ID.eq(vehicle_asset_id))
        .order_by(VehicleGrounding::ID)
        .get(db)
        .await
}

/// Grounds the vehicle `vehicle_asset_id` because its document `kind` expired
/// on `expired_on`, on `day`, and tells the fleet managers and the
/// dispatchers. Once for each vehicle, kind and expiry date: a second call
/// changes nothing. Returns whether it grounded the vehicle now.
///
/// # Errors
///
/// Database errors and the notifier's.
pub async fn ground(
    state: &AppState,
    vehicle_asset_id: i64,
    kind: &str,
    expired_on: NaiveDate,
    day: NaiveDate,
) -> Result<bool> {
    let known = VehicleGrounding::query()
        .where_(VehicleGrounding::VEHICLE_ASSET_ID.eq(vehicle_asset_id))
        .where_(VehicleGrounding::DOCUMENT_KIND.eq(kind))
        .where_(VehicleGrounding::EXPIRED_ON.eq(expired_on))
        .first(&state.db)
        .await?;
    if known.is_some() {
        return Ok(false);
    }
    let mut row = VehicleGrounding {
        vehicle_asset_id,
        document_kind: kind.to_owned(),
        expired_on,
        grounded_on: day,
        ..VehicleGrounding::default()
    };
    row.insert(&state.db).await?;
    let label = vehicle_asset_id.to_string();
    let word = kind.replace('_', " ");
    let title = english("logistics.grounding.title", &[("vehicle", &label)]);
    let body = english(
        "logistics.grounding.body",
        &[
            ("vehicle", &label),
            ("document", &word),
            ("expired", &expired_on),
        ],
    );
    for role in [FLEET_MANAGER_ROLE, DISPATCHER_ROLE] {
        for user in users_with_role(&state.db, role).await? {
            tell_user(state, row.vehicle(), &label, user.id, &title, &body).await?;
        }
    }
    Ok(true)
}
