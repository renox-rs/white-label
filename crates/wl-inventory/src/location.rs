//! The Location (wl-inventory §3): a place that holds stock. Only what
//! receiving needs is here: the code, the name, the type, the scope (the
//! project, building or branch the location is in, which decides who may
//! post a receipt there) and the negative stock policy.

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::inventory::LOCATION_SUBJECT_TYPE;

/// The code of the location stock is moved to when it goes back to a
/// supplier, until the supplier collects it.
pub const QUARANTINE_CODE: &str = "QUARANTINE";

/// What kind of place a location is.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LocationType {
    /// A warehouse.
    Warehouse,
    /// The central store of the company.
    CentralStore,
    /// A store.
    #[default]
    Store,
    /// A site.
    Site,
    /// A sales outlet.
    Outlet,
    /// A kitchen.
    Kitchen,
    /// A branch.
    Branch,
    /// A van.
    Van,
    /// Stock on its way between two locations.
    InTransit,
    /// Where goods wait for inspection.
    Quarantine,
    /// Where scrap goes.
    Scrap,
}

/// What a movement does when it would take the balance below zero.
#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NegativeStockPolicy {
    /// The movement is refused.
    #[default]
    Block,
    /// Automatic consumption of items that are not lot-tracked may go below
    /// zero.
    AllowConsumption,
}

/// A place that holds stock.
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(
    table = "wl_inventory_locations",
    unique(code),
    index(scope_type, scope_id)
)]
pub struct Location {
    /// The primary key.
    pub id: i64,
    /// The code, e.g. `STORE-1`.
    pub code: String,
    /// The name in English.
    pub name_en: String,
    /// The kind of place.
    pub location_type: LocationType,
    /// The location it is in.
    pub parent_id: Option<i64>,
    /// The type of the scope's `Subject`.
    pub scope_type: Option<String>,
    /// The id of the scope's `Subject`.
    pub scope_id: Option<i64>,
    /// What a movement below zero does here.
    pub negative_stock_policy: NegativeStockPolicy,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

impl Location {
    /// A location with `code` and `name`.
    pub fn new(code: &str, name: &str) -> Self {
        Location {
            code: code.trim().to_owned(),
            name_en: name.trim().to_owned(),
            ..Location::default()
        }
    }

    /// In `scope` (the project, building or branch).
    pub fn in_scope(mut self, scope: &Subject) -> Self {
        self.scope_type = Some(scope.subject_type.clone());
        self.scope_id = Some(scope.subject_id);
        self
    }

    /// The location as a [`Subject`]: `inventory.location:2`.
    pub fn subject(&self) -> Subject {
        Subject::new(LOCATION_SUBJECT_TYPE, self.id)
    }

    /// The scope subject (project, building, branch) the location is in.
    pub fn scope_subject(&self) -> Option<Subject> {
        match (&self.scope_type, self.scope_id) {
            (Some(kind), Some(id)) => Some(Subject::new(kind, id)),
            _ => None,
        }
    }

    /// The permission scope of the location: its project, building or
    /// branch, else global.
    pub fn scope(&self) -> Scope {
        scope_of(self.scope_type.as_deref(), self.scope_id)
    }
}

/// The quarantine location, made the first time stock is moved to it (the
/// one coded [`QUARANTINE_CODE`], or any location of the type).
///
/// # Errors
///
/// A database error.
pub(crate) async fn quarantine_location(tx: &mut Transaction) -> Result<i64> {
    if let Some(found) = Location::query()
        .where_(Location::LOCATION_TYPE.eq(LocationType::Quarantine))
        .order_by(Location::ID)
        .first(&mut *tx)
        .await?
    {
        return Ok(found.id);
    }
    let mut location = Location {
        code: QUARANTINE_CODE.to_owned(),
        name_en: "Quarantine".to_owned(),
        location_type: LocationType::Quarantine,
        ..Location::default()
    };
    location.insert(&mut *tx).await?;
    Ok(location.id)
}

/// The permission scope of a scope subject given by type and id; global when
/// either is missing.
pub fn scope_of(scope_type: Option<&str>, scope_id: Option<i64>) -> Scope {
    match (scope_type, scope_id) {
        (Some(kind), Some(id)) => Scope::new(kind, id),
        _ => Scope::global(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_location_has_the_scope_it_is_put_in() {
        let free = Location::new(" S1 ", " Store 1 ");
        assert_eq!(free.code, "S1");
        assert_eq!(free.scope(), Scope::global());
        assert_eq!(free.scope_subject(), None);
        let site =
            Location::new("S2", "Site store").in_scope(&Subject::new("construction.project", 4));
        assert_eq!(site.scope(), Scope::new("construction.project", 4));
        assert_eq!(
            site.scope_subject(),
            Some(Subject::new("construction.project", 4))
        );
        assert_eq!(site.location_type, LocationType::Store);
        assert_eq!(site.negative_stock_policy, NegativeStockPolicy::Block);
    }
}
