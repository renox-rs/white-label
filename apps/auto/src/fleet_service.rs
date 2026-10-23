//! The fleet service plan: a service that falls due for a vehicle under a
//! fleet contract, by mileage or by months (08 §4.5 step 1, `AUT.FLT.01`,
//! `WRK.PPM.01`). When it is due it is booked through wl-scheduling
//! ([`crate::fleet_services`]); the visit that delivers it moves the plan on.

use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use serde::{Deserialize, Serialize};

/// A planned service of a vehicle.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "auto_fleet_services",
    index(contract_id),
    index(vehicle_id),
    index(booking_id)
)]
pub struct FleetService {
    /// The primary key.
    pub id: i64,
    /// The contract.
    #[model(references = "auto_fleet_contracts")]
    pub contract_id: i64,
    /// The vehicle.
    #[model(references = "auto_vehicles")]
    pub vehicle_id: i64,
    /// The code of the service type that is booked (`periodic_service`).
    pub service_code: String,
    /// The mileage between two services, in kilometres.
    pub interval_km: Option<i64>,
    /// The months between two services.
    pub interval_months: Option<i64>,
    /// The day of the last service.
    pub last_service_date: Option<NaiveDate>,
    /// The odometer reading of the last service.
    pub last_service_km: Option<i64>,
    /// The day the next service falls due.
    pub next_due_date: Option<NaiveDate>,
    /// The odometer reading the next service falls due at.
    pub next_due_km: Option<i64>,
    /// The booking made for the service that fell due.
    pub booking_id: Option<i64>,
    /// The day the booking was asked for.
    pub requested_for_date: Option<NaiveDate>,
    /// When the booking was asked for.
    pub requested_at: Option<DateTime>,
    /// Whether the plan is in use.
    pub active: bool,
    /// The user who made it.
    pub created_by: Option<i64>,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// `date` plus `months`, on the same day of the month or the last day of a
/// shorter month.
pub fn add_months(date: NaiveDate, months: i64) -> NaiveDate {
    use renox::chrono::Months;
    let months = u32::try_from(months.max(0)).unwrap_or(0);
    date.checked_add_months(Months::new(months)).unwrap_or(date)
}

impl FleetService {
    /// The day and the reading the service next falls due at, from the last
    /// service (or `since` for a plan that has not had one yet) and the
    /// intervals.
    pub fn next_due(&self, since: NaiveDate) -> (Option<NaiveDate>, Option<i64>) {
        let date = self
            .interval_months
            .map(|months| add_months(self.last_service_date.unwrap_or(since), months));
        let km = self
            .interval_km
            .map(|km| self.last_service_km.unwrap_or(0) + km);
        (date, km)
    }

    /// Whether the service is due on `today`: within `lead_days` of its due
    /// day, or within `km_lead` kilometres of its due reading at `odometer_km`.
    pub fn is_due(&self, today: NaiveDate, lead_days: i64, odometer_km: i64, km_lead: i64) -> bool {
        let by_date = self
            .next_due_date
            .is_some_and(|due| due - today <= renox::chrono::TimeDelta::days(lead_days));
        let by_km = self
            .next_due_km
            .is_some_and(|due| odometer_km + km_lead >= due);
        by_date || by_km
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(month: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, d).unwrap()
    }

    #[test]
    fn months_are_added_to_the_same_day_or_the_last_of_a_short_month() {
        assert_eq!(add_months(day(1, 15), 6), day(7, 15));
        assert_eq!(
            add_months(day(8, 31), 6),
            NaiveDate::from_ymd_opt(2027, 2, 28).unwrap()
        );
        assert_eq!(add_months(day(1, 1), 0), day(1, 1));
    }

    #[test]
    fn the_next_service_follows_the_last_by_the_intervals() {
        let plan = FleetService {
            interval_km: Some(10_000),
            interval_months: Some(6),
            last_service_date: Some(day(1, 10)),
            last_service_km: Some(50_000),
            ..FleetService::default()
        };
        assert_eq!(plan.next_due(day(1, 1)), (Some(day(7, 10)), Some(60_000)));
        let fresh = FleetService {
            interval_months: Some(3),
            ..FleetService::default()
        };
        assert_eq!(fresh.next_due(day(2, 1)), (Some(day(5, 1)), None));
    }

    #[test]
    fn a_service_is_due_within_its_lead_by_date_or_by_mileage() {
        let plan = FleetService {
            next_due_date: Some(day(11, 10)),
            next_due_km: Some(60_000),
            ..FleetService::default()
        };
        assert!(!plan.is_due(day(11, 1), 7, 55_000, 500));
        assert!(plan.is_due(day(11, 3), 7, 55_000, 500), "seven days before");
        assert!(plan.is_due(day(11, 1), 7, 59_600, 500), "within 500 km");
        let by_km_only = FleetService {
            next_due_km: Some(60_000),
            ..FleetService::default()
        };
        assert!(!by_km_only.is_due(day(11, 1), 7, 50_000, 500));
    }
}
