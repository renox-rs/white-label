//! The figures of the home dashboard that belong to the vertical: units
//! available and leased, leases ending within 90 days, and the buildings
//! flagged non-compliant. Each is `None` when the user holds the permission
//! in no building, so the dashboard hides the tile; with a role given per
//! building (01 Property §3.2) the count covers those buildings only.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use serde::Serialize;

use crate::building::Building;
use crate::compliance::BuildingFlag;
use crate::lease::{Lease, LeaseStatus};
use crate::permissions;
use crate::unit::{Unit, UnitStatus};

/// The days ahead a lease's end counts as "expiring".
pub const EXPIRY_DAYS: u64 = 90;

/// A lease that ends soon.
#[derive(Debug, Clone, Serialize)]
pub struct ExpiringLease {
    /// The lease's id (`/property/leases/{id}`).
    pub id: i64,
    /// Its number with the version.
    pub number: String,
    /// The last day of the term.
    pub end_date: NaiveDate,
    /// The days left from today.
    pub days_left: i64,
}

/// What the dashboard shows of the vertical.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Figures {
    /// Units that can be leased now (`units.view`).
    pub units_available: Option<u64>,
    /// Units with an active lease (`units.view`).
    pub units_leased: Option<u64>,
    /// Leases that end within [`EXPIRY_DAYS`] days (`leases.view`).
    pub leases_expiring: Option<u64>,
    /// The soonest of them, at most five.
    pub expiring: Vec<ExpiringLease>,
    /// Buildings with an open compliance flag (`compliance.view`).
    pub buildings_flagged: Option<u64>,
}

/// The leases that still run and can end: the statuses a lease passes
/// through between its start and its end.
const RUNNING: [LeaseStatus; 3] = [
    LeaseStatus::Active,
    LeaseStatus::RenewalDue,
    LeaseStatus::NoticeGiven,
];

/// The figures `user` may see, as of `today`.
///
/// # Errors
///
/// The database's errors.
pub async fn figures(state: &AppState, user: &AuthUser, today: NaiveDate) -> Result<Figures> {
    let mut out = Figures::default();
    let units = user.scopes_with::<Building>(permissions::UNITS_VIEW);
    if !units.is_empty() {
        let count = |status: UnitStatus| {
            units
                .apply(Unit::query(), &["building_id"])
                .where_(Unit::STATUS.eq(status))
                .count(&state.db)
        };
        out.units_available = Some(count(UnitStatus::Available).await?);
        out.units_leased = Some(count(UnitStatus::Occupied).await?);
    }
    let leases = user.scopes_with::<Building>(permissions::LEASES_VIEW);
    if !leases.is_empty() {
        let until = today + Days::new(EXPIRY_DAYS);
        let rows = leases
            .apply(Lease::query(), &["building_id"])
            .where_(Lease::STATUS.is_in(RUNNING))
            .where_(Lease::END_DATE.gte(today))
            .where_(Lease::END_DATE.lte(until))
            .order_by(Lease::END_DATE)
            .order_by(Lease::ID)
            .get(&state.db)
            .await?;
        out.leases_expiring = Some(rows.len() as u64);
        out.expiring = rows
            .iter()
            .take(5)
            .map(|lease| ExpiringLease {
                id: lease.id,
                number: lease.number_with_version(),
                end_date: lease.end_date,
                days_left: (lease.end_date - today).num_days(),
            })
            .collect();
    }
    let flagged = user.scopes_with::<Building>(permissions::COMPLIANCE_VIEW);
    if !flagged.is_empty() {
        let mut buildings: Vec<i64> = BuildingFlag::open(&state.db)
            .await?
            .into_iter()
            .map(|flag| flag.building_id)
            .filter(|id| flagged.contains(id))
            .collect();
        buildings.sort_unstable();
        buildings.dedup();
        out.buildings_flagged = Some(buildings.len() as u64);
    }
    Ok(out)
}
