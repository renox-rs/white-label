//! Who may see and write the records of a client company (09 Manpower §3.2,
//! §3.3 `MPW.ACL.01`): staff by the permission in the client company, and the
//! client's own users by the portal access the company granted them.

use renox::prelude::*;
use wl_kernel::PartyId;
use wl_kernel::documents::today;
use wl_kernel::portal::portal_users;

use crate::contract::ClientContract;
use crate::permissions;

/// Whether `by` may write requests of `client`: staff holding `permission`
/// in the client company, or the client approver of the company in the
/// portal (`MPW.ACL.01`).
pub(crate) async fn may_write(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    permission: &str,
    portal_too: bool,
) -> Result<bool> {
    if by.has_permission_in(permission, &permissions::client_scope(client.0)) {
        return Ok(true);
    }
    if !portal_too {
        return Ok(false);
    }
    let acting = portal_users(state)
        .acting_for(&state.db, by.id, client, today())
        .await?;
    Ok(acting.is_some_and(|user| user.has_any_role(permissions::CLIENT_APPROVER_PORTAL_ROLES)))
}

/// Whether `by` may see the records of `client`: staff holding one of
/// `permissions` in the client company, or any user the client has granted
/// access in the portal (`MPW.ACL.01`: only their own company).
pub(crate) async fn may_see(
    state: &AppState,
    by: &AuthUser,
    client: PartyId,
    permissions: &[&str],
) -> Result<bool> {
    let scope = permissions::client_scope(client.0);
    if permissions
        .iter()
        .any(|permission| by.has_permission_in(permission, &scope))
    {
        return Ok(true);
    }
    Ok(portal_users(state)
        .acting_for(&state.db, by.id, client, today())
        .await?
        .is_some())
}

/// Whether the user holds a portal access for any client company the agency
/// serves: a client user whose company has no worker on site yet gets an
/// empty list, not a refusal.
pub(crate) async fn may_see_any(state: &AppState, by: &AuthUser) -> Result<bool> {
    let mut seen = Vec::new();
    for contract in ClientContract::query().get(&state.db).await? {
        if seen.contains(&contract.client_party_id) {
            continue;
        }
        seen.push(contract.client_party_id);
        let acting = portal_users(state)
            .acting_for(&state.db, by.id, contract.client(), today())
            .await?;
        if acting.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}
