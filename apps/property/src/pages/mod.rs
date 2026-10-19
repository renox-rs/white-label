//! The back-office pages of the vertical: the portfolio (buildings and units)
//! and the leases. Every route needs a signed-in user; what a user may see
//! and do is decided per building, because roles are assigned per building
//! (§3.2) and a route guard counts global roles only.

pub mod agreements;
pub mod leases;
pub mod portfolio;
pub mod statements;

use std::collections::BTreeMap;

use renox::auth::permissions::Scopes;
use renox::prelude::*;

use crate::building::Building;
use crate::lease::LeaseStatus;
use crate::texts;
use crate::unit::UnitStatus;

/// The texts a template reads as `tx['portfolio.title']`.
pub(crate) type Texts = BTreeMap<&'static str, String>;

/// The routes of the pages.
pub fn routes() -> Routes {
    Routes::new()
        .merge(portfolio::routes())
        .merge(leases::routes())
        .merge(statements::routes())
        .merge(agreements::routes())
}

/// The package text `key` (without the `property.` prefix).
pub(crate) fn text(lang: &Lang, key: &str) -> String {
    texts::t(lang, &format!("property.{key}"))
}

/// The buildings in which the user holds `permission`; 403 when none.
pub(crate) fn buildings_with(user: &AuthUser, permission: &str) -> Result<Scopes<i64>> {
    let scopes = user.scopes_with::<Building>(permission);
    if scopes.is_empty() {
        return Err(Error::Forbidden);
    }
    Ok(scopes)
}

/// The badge kind of a lease status.
pub(crate) fn lease_kind(status: LeaseStatus) -> &'static str {
    match status {
        LeaseStatus::Active | LeaseStatus::Renewed => "success",
        LeaseStatus::PendingSignature | LeaseStatus::Draft => "info",
        LeaseStatus::RenewalDue | LeaseStatus::Expired | LeaseStatus::NoticeGiven => "warning",
        LeaseStatus::Amended | LeaseStatus::Terminated | LeaseStatus::Cancelled => "info",
    }
}

/// The badge kind of a unit status.
pub(crate) fn unit_kind(status: UnitStatus) -> &'static str {
    match status {
        UnitStatus::Occupied => "success",
        UnitStatus::Available | UnitStatus::Reserved => "info",
        UnitStatus::NoticeGiven | UnitStatus::UnderMaintenance => "warning",
        UnitStatus::OffMarket => "info",
    }
}

/// The name of the user `id`, else `#id`.
pub(crate) async fn user_name(db: &Db, id: i64) -> String {
    User::find(db, id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| format!("#{id}"), |u| u.name)
}
