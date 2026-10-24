//! Who may see which search documents (BL-AGT-02, docs/ACCESS-CONTROL.md
//! layers 3 and 4): the permission of each registered type, and where the
//! user holds it, everywhere or in some scopes ([`Reach`]). The search page,
//! the related records panel and, later, the agents' tools read the same
//! [`Access`], so a record hidden from one is hidden from all of them.
//!
//! An `Access` is loaded once per request with at most two queries (the
//! roles and the user's role assignments), whatever the number of types or
//! results, and then answers in memory.
//!
//! **Scope.** A document carries its record's scope (`scope_type`,
//! `scope_id`). A global holder of the type's permission sees every
//! document of the type; a scoped holder sees the documents of the scopes
//! they hold it in, and never a global (scope-less) document.
//!
//! **Legal entities.** A user reaches a legal entity only through a scope
//! linked to it (`ScopeEntity`) or through a global role: there is no
//! separate user-to-entity grant in the kernel, so the scope filter is also
//! the legal-entity filter.
//!
//! **Record rules.** No staff role of this repository restricts records to
//! their owner or assignee (`OWN`, `ASG`) yet; the documents carry no owner.
//! When a module needs one, its documents gain the owner and this type the
//! check, in the same statement. External users (`OWN` through their party)
//! search in the portal (BL-AGT-05), not here.

use std::collections::{BTreeMap, BTreeSet};

use renox::auth::permissions::{self, Scope};
use renox::prelude::*;

use super::registry::{self, Searchable};
use crate::scopes::Reach;

/// What one user may see: for each permission the kernel was asked about,
/// where the user holds it. A permission missing from it is held nowhere.
#[derive(Debug, Clone, Default)]
pub struct Access {
    user_id: i64,
    reach: BTreeMap<String, Reach>,
}

impl Access {
    /// The access of the signed-in `user`, for the permissions of every
    /// registered searchable type and relation. A permission `user.allows`
    /// (a global role, `gate_before`) is held everywhere; any other is
    /// looked up in the user's scoped role assignments.
    pub async fn of(db: &Db, user: &AuthUser) -> Result<Self> {
        let wanted = wanted_permissions();
        Self::load(db, user, &wanted, |permission| user.allows(permission)).await
    }

    /// The access of `user` read from the database alone (no request: a
    /// job, a command, an agent's tool, a test): global role assignments
    /// count everywhere, scoped ones in their scope.
    pub async fn of_user(db: &Db, user: &User) -> Result<Self> {
        let wanted = wanted_permissions();
        Self::load(db, user, &wanted, |_| false).await
    }

    /// An access built by hand, for code that already knows the answer.
    pub fn with(user_id: i64, reach: impl IntoIterator<Item = (String, Reach)>) -> Self {
        Access {
            user_id,
            reach: reach.into_iter().collect(),
        }
    }

    async fn load(
        db: &Db,
        user: &User,
        wanted: &BTreeSet<String>,
        global: impl Fn(&str) -> bool,
    ) -> Result<Self> {
        let mut reach = BTreeMap::new();
        let mut pending: Vec<&String> = Vec::new();
        for permission in wanted {
            if global(permission) {
                reach.insert(permission.clone(), Reach::everything());
            } else {
                pending.push(permission);
            }
        }
        if !pending.is_empty() {
            let roles: BTreeMap<String, Vec<String>> =
                permissions::roles(db).await?.into_iter().collect();
            let assignments = user.assignments(db).await?;
            for permission in pending {
                let mut everywhere = false;
                let mut scopes: Vec<Scope> = Vec::new();
                for assignment in assignments.iter().filter(|a| a.is_active()) {
                    let grants = roles
                        .get(&assignment.role)
                        .is_some_and(|list| list.iter().any(|p| p == permission));
                    if !grants {
                        continue;
                    }
                    if assignment.scope.is_global() {
                        everywhere = true;
                        break;
                    }
                    if !scopes.contains(&assignment.scope) {
                        scopes.push(assignment.scope.clone());
                    }
                }
                if everywhere {
                    reach.insert(permission.clone(), Reach::everything());
                } else if !scopes.is_empty() {
                    reach.insert(permission.clone(), Reach::only(scopes));
                }
            }
        }
        Ok(Access {
            user_id: user.id,
            reach,
        })
    }

    /// The user's id.
    pub fn user_id(&self) -> i64 {
        self.user_id
    }

    /// Where the user holds `permission`; `None` when nowhere.
    pub fn reach(&self, permission: &str) -> Option<&Reach> {
        self.reach.get(permission)
    }

    /// Whether the user holds `permission` anywhere.
    pub fn holds(&self, permission: &str) -> bool {
        self.reach.contains_key(permission)
    }

    /// Whether the user holds `permission` everywhere (a global role).
    pub fn holds_everywhere(&self, permission: &str) -> bool {
        self.reach(permission).is_some_and(Reach::is_all)
    }

    /// Whether the user may see a record needing `permission` whose scope is
    /// `scope_type` / `scope_id` (`""` / `""` for a global record, which is
    /// for global holders only).
    pub fn sees(&self, permission: &str, scope_type: &str, scope_id: &str) -> bool {
        self.reach(permission)
            .is_some_and(|reach| covers(reach, scope_type, scope_id))
    }

    /// The searchable types the user may search, sorted by subject type.
    pub fn searchable_types(&self) -> Vec<Searchable> {
        registry::registered_types()
            .into_iter()
            .filter(|searchable| self.holds(searchable.permission()))
            .collect()
    }
}

/// Whether a record of scope `scope_type` / `scope_id` is inside `reach`.
pub(crate) fn covers(reach: &Reach, scope_type: &str, scope_id: &str) -> bool {
    reach.is_all()
        || (!scope_type.is_empty()
            && reach
                .scopes()
                .iter()
                .any(|scope| scope.kind() == scope_type && scope.id() == scope_id))
}

/// Whether `user` may search at all: they hold the permission of some
/// registered type, globally or in a scope. In memory (no query), for the
/// top bar's box.
pub fn can_search(user: &AuthUser) -> bool {
    registry::registered_types().iter().any(|searchable| {
        user.allows(searchable.permission())
            || user.has_permission_anywhere(searchable.permission())
    })
}

/// The permissions of every registered searchable type and relation.
fn wanted_permissions() -> BTreeSet<String> {
    let mut wanted: BTreeSet<String> = registry::registered_types()
        .iter()
        .map(|searchable| searchable.permission().to_owned())
        .collect();
    wanted.extend(crate::related::permissions());
    wanted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scoped_reach_covers_its_scopes_and_never_a_global_record() {
        let reach = Reach::only(vec![Scope::new("buildings", 1)]);
        assert!(covers(&reach, "buildings", "1"));
        assert!(!covers(&reach, "buildings", "2"));
        assert!(!covers(&reach, "", ""));
        assert!(covers(&Reach::everything(), "", ""));
        let access = Access::with(7, [("invoices.view".to_owned(), reach)]);
        assert!(access.sees("invoices.view", "buildings", "1"));
        assert!(!access.sees("invoices.view", "buildings", "2"));
        assert!(!access.sees("parties.view", "buildings", "1"));
        assert!(!access.holds_everywhere("invoices.view"));
    }
}
