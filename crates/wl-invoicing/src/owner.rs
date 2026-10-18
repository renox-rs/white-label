//! Who looks after an invoice (wl-invoicing §3 *Invoice owner*): shared by
//! the dispute ticket (#67) and the dispute escalation (#68).

use renox::auth::permissions::{Scope, users_with_permission_in, users_with_role_in};
use renox::prelude::*;
use wl_kernel::LegalEntity;
use wl_kernel::scopes;

use crate::dispute::holds;
use crate::invoice::Invoice;
use crate::permissions;

/// The role a recipient is told as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerRole {
    /// The invoice's own owner.
    Owner,
    /// The owner is gone: every holder of `invoices.issue` in the scope.
    Receivables,
}

impl OwnerRole {
    /// The value stored in the escalation log.
    pub fn as_str(self) -> &'static str {
        match self {
            OwnerRole::Owner => "owner",
            OwnerRole::Receivables => "receivables",
        }
    }
}

/// The owner side of `invoice`: `owner_id` when that user still exists and
/// still holds `invoices.issue` in the invoice's scope, else every holder of
/// `invoices.issue` in that scope (sorted by id, once each).
pub async fn recipients(db: &Db, invoice: &Invoice) -> Result<(OwnerRole, Vec<User>)> {
    let scope = scopes::scope_of(db, &invoice.subject()).await?;
    if let Some(owner_id) = invoice.owner_id
        && let Some(owner) = User::find(db, owner_id).await?
        && holds(db, owner_id, permissions::ISSUE, &scope).await?
    {
        return Ok((OwnerRole::Owner, vec![owner]));
    }
    let mut users = users_with_permission_in(db, permissions::ISSUE, &scope).await?;
    users.sort_by_key(|user| user.id);
    users.dedup_by_key(|user| user.id);
    Ok((OwnerRole::Receivables, users))
}

/// The scope the finance manager role is looked up in: the invoice's legal
/// entity, or global when it has none.
pub fn entity_scope(invoice: &Invoice) -> Scope {
    match invoice.legal_entity_id {
        Some(id) => Scope::of_id::<LegalEntity>(id),
        None => Scope::global(),
    }
}

/// The holders of `role` for the invoice's entity (those given it for that
/// entity and the global holders), sorted by id, once each.
pub async fn finance_managers(db: &Db, role: &str, invoice: &Invoice) -> Result<Vec<User>> {
    let mut users = users_with_role_in(db, role, &entity_scope(invoice)).await?;
    users.sort_by_key(|user| user.id);
    users.dedup_by_key(|user| user.id);
    Ok(users)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roles_are_stored_by_name() {
        assert_eq!(OwnerRole::Owner.as_str(), "owner");
        assert_eq!(OwnerRole::Receivables.as_str(), "receivables");
    }

    #[test]
    fn an_invoice_without_an_entity_uses_the_global_scope() {
        let invoice = Invoice::default();
        assert_eq!(entity_scope(&invoice), Scope::global());
        let invoice = Invoice {
            legal_entity_id: Some(3),
            ..Invoice::default()
        };
        assert_eq!(entity_scope(&invoice), Scope::of_id::<LegalEntity>(3));
    }
}
