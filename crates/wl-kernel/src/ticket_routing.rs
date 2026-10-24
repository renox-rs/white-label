//! Routing a new ticket to a role in its subject's scope (wl-ticketing §8
//! `TicketRouter`, §5.1 point 2; wl-invoicing §3 *Invoice owner*).
//!
//! A ticket category names a default assignee role. When a ticket is opened
//! without an assignee, `wl-ticketing` asks the [`TicketRouter`] registered
//! for its category ([`register_ticket_router`]); without one, the default is
//! the category's role in the subject's scope ([`scopes::scope_of`]). The
//! answer is a [`RoleAssignee`]: the ticket is `assigned`, owned by the role,
//! and shows in the queue of every holder of the role in that scope until
//! one of them takes it (a reassignment to that user, which keeps the SLA).
//!
//! The kernel's [`Assignee`](crate::tickets::Assignee) stays a user or a
//! vendor party: a role is not a vendor (no vendor checks, no vendor notice)
//! and not a user (no field "my jobs"). The decided third variant
//! `Assignee::Role` is kept for when the kernel contract can change; until
//! then a role assignment is this type, stored by `wl-ticketing`.
//!
//! A role is either a role key (holders of the role there) or, for a role
//! defined by a permission like the receivables role (the holders of
//! `invoices.issue` in the invoice's scope), a permission.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role_in};
use renox::prelude::*;
use serde::{Deserialize, Serialize};

use crate::scopes;
use crate::tickets::OpenTicket;

/// A boxed future of the routing contract.
pub type RouteFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A role in a scope that owns a ticket until a holder takes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoleAssignee {
    /// The role's key, e.g. `receivables`.
    pub role: String,
    /// The role's name for people, e.g. `Receivables`.
    pub label: String,
    /// The permission that defines the holders, when the role is one
    /// (`invoices.issue` for the receivables role); `None` for a role key.
    pub permission: Option<String>,
    /// The scope's kind (its table; empty for global).
    pub scope_kind: String,
    /// The scope's key as text (empty for global).
    pub scope_id: String,
}

impl RoleAssignee {
    /// The holders of role `role` in `scope`.
    pub fn role(role: &str, label: &str, scope: &Scope) -> Self {
        RoleAssignee {
            role: role.to_owned(),
            label: label.to_owned(),
            permission: None,
            scope_kind: scope.kind().to_owned(),
            scope_id: scope.id().to_owned(),
        }
    }

    /// The role `role` whose holders are those who may do `permission` in
    /// `scope`.
    pub fn by_permission(role: &str, label: &str, permission: &str, scope: &Scope) -> Self {
        RoleAssignee {
            permission: Some(permission.to_owned()),
            ..RoleAssignee::role(role, label, scope)
        }
    }

    /// The scope.
    pub fn scope(&self) -> Scope {
        if self.scope_kind.is_empty() && self.scope_id.is_empty() {
            Scope::global()
        } else {
            Scope::new(&self.scope_kind, &self.scope_id)
        }
    }

    /// One text for the role and scope, to find the tickets of one queue:
    /// `receivables|wl_legal_entities|1`.
    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.role, self.scope_kind, self.scope_id)
    }

    /// The holders now, by id, once each.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn holders(&self, db: &Db) -> Result<Vec<User>> {
        let scope = self.scope();
        let mut users = match &self.permission {
            Some(permission) => users_with_permission_in(db, permission, &scope).await?,
            None => users_with_role_in(db, &self.role, &scope).await?,
        };
        users.sort_by_key(|user| user.id);
        users.dedup_by_key(|user| user.id);
        Ok(users)
    }

    /// Whether user `id` holds the role in its scope now.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn is_held_by(&self, db: &Db, id: i64) -> Result<bool> {
        Ok(self.holders(db).await?.iter().any(|user| user.id == id))
    }

    /// The name for people: `Receivables (ACME Properties LLC)`, or the
    /// label alone in the global scope.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn display(&self, db: &Db) -> Result<String> {
        let scope = self.scope();
        if scope.is_global() {
            return Ok(self.label.clone());
        }
        Ok(format!(
            "{} ({})",
            self.label,
            scopes::label_of(db, &scope).await?
        ))
    }
}

/// The name of role key `role` for people when nothing else names it:
/// `site_supervisor` → `Site supervisor`.
pub fn role_label(role: &str) -> String {
    let text = role.replace(['_', '.'], " ");
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Chooses the role a new ticket of one category goes to, from its subject
/// and scope (wl-ticketing §8).
pub trait TicketRouter: Send + Sync + 'static {
    /// The role `ticket` goes to, or `None` to leave it `open` for the
    /// coordinator.
    fn route<'a>(
        &'a self,
        state: &'a AppState,
        ticket: &'a OpenTicket,
    ) -> RouteFuture<'a, Option<RoleAssignee>>;
}

static ROUTERS: RwLock<BTreeMap<String, Arc<dyn TicketRouter>>> = RwLock::new(BTreeMap::new());

/// Adds `router` for the tickets of `category_key` to the process-wide
/// registry; registering the same category again replaces it.
pub fn register_ticket_router(category_key: &str, router: impl TicketRouter) {
    ROUTERS
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(category_key.to_owned(), Arc::new(router));
}

/// The router of `category_key`, if registered.
pub fn ticket_router(category_key: &str) -> Option<Arc<dyn TicketRouter>> {
    ROUTERS
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(category_key)
        .cloned()
}

/// The default route: the category's role `role` in the scope of the
/// ticket's subject.
///
/// # Errors
///
/// The database's errors.
pub async fn default_route(db: &Db, role: &str, ticket: &OpenTicket) -> Result<RoleAssignee> {
    let scope = scopes::scope_of(db, &ticket.subject).await?;
    Ok(RoleAssignee::role(role, &role_label(role), &scope))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_assignee_keys_its_role_and_scope() {
        let role = RoleAssignee::by_permission(
            "receivables",
            "Receivables",
            "invoices.issue",
            &Scope::new("wl_legal_entities", 1),
        );
        assert_eq!(role.key(), "receivables|wl_legal_entities|1");
        assert_eq!(role.scope(), Scope::new("wl_legal_entities", 1));
        let global = RoleAssignee::role("hr", "HR", &Scope::global());
        assert!(global.scope().is_global());
        assert_eq!(global.key(), "hr||");
    }

    #[test]
    fn a_role_key_reads_in_words() {
        assert_eq!(role_label("site_supervisor"), "Site supervisor");
        assert_eq!(role_label("hr"), "Hr");
    }
}
