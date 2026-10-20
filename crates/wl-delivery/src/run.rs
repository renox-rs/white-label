//! The Run (wl-delivery §3, §4.2): one driver's trip with ordered stops. Its
//! status moves only through the machine in [`crate::run_machine`].

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::delivery::RunId;
use wl_kernel::numbering::{Reset, Series};

/// The subject type of a vehicle: an asset of `wl-inventory` (`STK.AST.01`).
pub const VEHICLE_SUBJECT_TYPE: &str = "inventory.asset";

/// The subject type used for a driver without an employee record (the
/// `hr` module off): the user's account.
pub const DRIVER_USER_SUBJECT_TYPE: &str = "auth.user";

/// `RUN-{yyyy}{mm}-{0000}` (§7: `RUN-` + date + sequence), restarting every
/// month.
pub const RUN_SERIES: Series = Series {
    key: "delivery_run",
    pattern: "{prefix}-{yyyy}{mm}-{0000}",
    prefix: "RUN",
    reset: Reset::Monthly,
    gapless: false,
    per_entity: false,
};

/// Where a run is in its life (wl-delivery §4.2). Stored as `planned`,
/// `dispatched`, `returned_to_depot`, `closed` or `cancelled`.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RunStatus {
    /// Stops grouped; driver and vehicle assigned.
    #[default]
    Planned,
    /// Left the depot.
    Dispatched,
    /// Driver back, or the run was aborted; cash and undelivered items to
    /// hand over.
    ReturnedToDepot,
    /// Every stop has an outcome, cash counted, goods back in or reported
    /// missing.
    Closed,
    /// Not driven.
    Cancelled,
}

/// What the run is driven with (wl-delivery §3).
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum VehicleType {
    /// A van.
    #[default]
    Van,
    /// A truck.
    Truck,
    /// A motorbike.
    Motorbike,
    /// An on-foot rider: no vehicle.
    OnFoot,
}

impl VehicleType {
    /// The type stored as `key`.
    pub fn parse(key: &str) -> Option<VehicleType> {
        VehicleType::ALL.iter().copied().find(|v| v.as_str() == key.trim())
    }
}

/// A run. Dates are Asia/Dubai calendar dates; times are instants.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_delivery_runs",
    state_machine,
    unique(number),
    index(run_date),
    index(status)
)]
pub struct Run {
    /// The primary key.
    pub id: i64,
    /// The number from the `delivery_run` series.
    pub number: String,
    /// The day the run is driven.
    pub run_date: NaiveDate,
    /// The depot it leaves from.
    pub depot: String,
    /// The driver: a user with the driver role.
    pub driver_user_id: i64,
    /// What it is driven with.
    pub vehicle_type: VehicleType,
    /// The vehicle (an asset); none for an on-foot rider.
    pub vehicle_asset_id: Option<i64>,
    /// The estimated duration in minutes.
    pub estimated_minutes: Option<i64>,
    /// The planned start.
    pub planned_start: Option<DateTime>,
    /// Why it was aborted.
    pub abort_reason: Option<String>,
    /// The cash the depot counted when the driver handed it over, in fils.
    pub cash_counted_amount: Option<i64>,
    /// The user who counted it (never the driver).
    pub cash_counted_by: Option<i64>,
    /// When it was counted.
    pub cash_counted_at: Option<DateTime>,
    /// When the driver came back to the depot.
    pub returned_at: Option<DateTime>,
    /// When the run closed.
    pub closed_at: Option<DateTime>,
    /// The lifecycle state; changed only by a transition.
    pub status: RunStatus,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Run {
    /// The id as the kernel's [`RunId`].
    pub fn run_id(&self) -> RunId {
        RunId(self.id)
    }

    /// The run as a [`Subject`].
    pub fn subject(&self) -> Subject {
        self.run_id().subject()
    }

    /// The vehicle's document subject; none for an on-foot rider or a run
    /// without a vehicle.
    pub fn vehicle_subject(&self) -> Option<Subject> {
        if self.vehicle_type == VehicleType::OnFoot {
            return None;
        }
        self.vehicle_asset_id
            .map(|id| Subject::new(VEHICLE_SUBJECT_TYPE, id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_vehicle_types_are_stored_in_snake_case() {
        let statuses: Vec<_> = RunStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            statuses,
            ["planned", "dispatched", "returned_to_depot", "closed", "cancelled"]
        );
        assert_eq!(VehicleType::parse("on_foot"), Some(VehicleType::OnFoot));
        assert_eq!(VehicleType::parse("boat"), None);
    }

    #[test]
    fn an_on_foot_rider_has_no_vehicle_subject() {
        let mut run = Run {
            id: 4,
            vehicle_type: VehicleType::Van,
            vehicle_asset_id: Some(9),
            ..Run::default()
        };
        assert_eq!(
            run.vehicle_subject(),
            Some(Subject::new("inventory.asset", 9))
        );
        run.vehicle_type = VehicleType::OnFoot;
        assert_eq!(run.vehicle_subject(), None);
        assert_eq!(run.subject(), Subject::new("delivery.run", 4));
    }
}
