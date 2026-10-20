//! The records of a building, for the lists: what a user who holds a
//! permission in some buildings sees of a module-wide list (invoices,
//! cheques, deposits, tickets, parties, agreements; `wl_kernel::scopes`).
//! The other direction of the scope resolvers of `lib.rs`: from the
//! buildings to their units, leases, agreements and people.

use renox::auth::permissions::Scope;
use renox::prelude::*;
use wl_kernel::scopes::{register_derived_members, register_members};

use crate::agreement::{AGREEMENT_SUBJECT_TYPE, AgreementUnit, ManagementAgreement};
use crate::building::{BUILDING_SUBJECT_TYPE, Building};
use crate::lease::{LEASE_SUBJECT_TYPE, Lease};
use crate::unit::{UNIT_SUBJECT_TYPE, Unit};

/// The ids of the buildings among `scopes`.
pub fn building_ids(scopes: &[Scope]) -> Vec<i64> {
    scopes
        .iter()
        .filter_map(|scope| scope.key::<Building>())
        .collect()
}

/// The ids of the units of the buildings among `scopes`.
pub async fn unit_ids(db: &Db, scopes: &[Scope]) -> Result<Vec<i64>> {
    let buildings = building_ids(scopes);
    if buildings.is_empty() {
        return Ok(Vec::new());
    }
    Ok(Unit::query()
        .where_in(Unit::BUILDING_ID.name(), buildings)
        .get(db)
        .await?
        .into_iter()
        .map(|unit| unit.id)
        .collect())
}

/// The ids of the agreements with a unit in the buildings among `scopes`.
pub async fn agreement_ids(db: &Db, scopes: &[Scope]) -> Result<Vec<i64>> {
    let units = unit_ids(db, scopes).await?;
    if units.is_empty() {
        return Ok(Vec::new());
    }
    Ok(AgreementUnit::query()
        .where_in(AgreementUnit::UNIT_ID.name(), units)
        .get(db)
        .await?
        .into_iter()
        .map(|row| row.agreement_id)
        .collect())
}

/// Registers the buildings' members with the kernel.
pub fn register() {
    register_members(BUILDING_SUBJECT_TYPE, |_db, scopes| {
        Box::pin(async move { Ok(building_ids(scopes)) })
    });
    register_members(UNIT_SUBJECT_TYPE, |db, scopes| {
        Box::pin(async move { unit_ids(db, scopes).await })
    });
    register_members(LEASE_SUBJECT_TYPE, |db, scopes| {
        Box::pin(async move {
            let buildings = building_ids(scopes);
            if buildings.is_empty() {
                return Ok(Vec::new());
            }
            Ok(Lease::query()
                .where_in(Lease::BUILDING_ID.name(), buildings)
                .get(db)
                .await?
                .into_iter()
                .map(|lease| lease.id)
                .collect())
        })
    });
    register_members(AGREEMENT_SUBJECT_TYPE, |db, scopes| {
        Box::pin(async move { agreement_ids(db, scopes).await })
    });
    // The people of a building: its tenants and the owners of the units it
    // has under management.
    register_derived_members(wl_kernel::parties::PARTY_SUBJECT_TYPE, |db, scopes| {
        Box::pin(async move {
            let buildings = building_ids(scopes);
            let mut parties: Vec<i64> = Vec::new();
            if !buildings.is_empty() {
                parties.extend(
                    Lease::query()
                        .where_in(Lease::BUILDING_ID.name(), buildings)
                        .get(db)
                        .await?
                        .into_iter()
                        .map(|lease| lease.tenant_party_id),
                );
            }
            let agreements = agreement_ids(db, scopes).await?;
            if !agreements.is_empty() {
                parties.extend(
                    ManagementAgreement::query()
                        .where_in(ManagementAgreement::ID.name(), agreements)
                        .get(db)
                        .await?
                        .into_iter()
                        .map(|agreement| agreement.owner_party_id),
                );
            }
            Ok(parties)
        })
    });
}
