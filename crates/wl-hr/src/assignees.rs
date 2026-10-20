//! Who a step is assigned to (wl-hr §3 *Onboarding*, *The employee's
//! manager*, issue #216).
//!
//! - `hr`: the holders of `onboarding.manage` in the employee's scopes, other
//!   than users who hold it only through the owner role; the owner role
//!   holds it as the second person for `waive` and `reopen`, and gets `hr`
//!   steps only when nobody else holds it.
//! - `manager`: the users who hold the manager role in one of the employee's
//!   scopes. With no such user the step goes to HR.
//! - `employee`: the employee's own account. An employee with no account has
//!   no assignee: the step is recorded in person (`HRM.ACL.10`).
//! - `role:<name>`: the holders of that role in the employee's scopes; with
//!   none, HR.
//!
//! An employee with no scope is looked for in the global scope.

use std::collections::BTreeSet;

use renox::auth::permissions::{self, Scope, users_with_permission_in, users_with_role_in};
use renox::prelude::*;
use wl_kernel::Subject;

use crate::config::config_of;
use crate::employee::Employee;
use crate::permissions::ONBOARDING_MANAGE;
use crate::template::{OWNER_EMPLOYEE, OWNER_HR, OWNER_MANAGER};

/// The scopes to look for people in: the employee's, else global.
fn scopes_of(scopes: &[Subject]) -> Vec<Scope> {
    if scopes.is_empty() {
        vec![Scope::global()]
    } else {
        scopes
            .iter()
            .map(|s| Scope::new(&s.subject_type, s.subject_id))
            .collect()
    }
}

/// The ids of the users holding `role` in any of `scopes`.
async fn holders_of_role(db: &Db, role: &str, scopes: &[Scope]) -> Result<BTreeSet<i64>> {
    let mut ids = BTreeSet::new();
    for scope in scopes {
        ids.extend(
            users_with_role_in(db, role, scope)
                .await?
                .iter()
                .map(|u| u.id),
        );
    }
    Ok(ids)
}

/// The `hr` assignees in `scopes`: the holders of `onboarding.manage`
/// without the ones who hold it only through the owner role, unless nobody
/// else does.
pub async fn hr_holders(state: &AppState, scopes: &[Subject]) -> Result<Vec<i64>> {
    let scopes = scopes_of(scopes);
    let owner_role = config_of(state).owner_role;
    let mut holders = BTreeSet::new();
    for scope in &scopes {
        holders.extend(
            users_with_permission_in(&state.db, ONBOARDING_MANAGE, scope)
                .await?
                .iter()
                .map(|u| u.id),
        );
    }
    // The other roles that grant the permission.
    let mut through_others = BTreeSet::new();
    for (role, granted) in permissions::roles(&state.db).await? {
        if role != owner_role && granted.iter().any(|p| p == ONBOARDING_MANAGE) {
            through_others.extend(holders_of_role(&state.db, &role, &scopes).await?);
        }
    }
    let others: Vec<i64> = holders.intersection(&through_others).copied().collect();
    Ok(if others.is_empty() {
        holders.into_iter().collect()
    } else {
        others
    })
}

/// The users `owner` names for `employee`, working in `scopes`: ordered by
/// id, without repeats.
pub async fn resolve(
    state: &AppState,
    employee: &Employee,
    scopes: &[Subject],
    owner: &str,
) -> Result<Vec<i64>> {
    let config = config_of(state);
    let looked = scopes_of(scopes);
    let named: Option<BTreeSet<i64>> = match owner {
        OWNER_EMPLOYEE => return Ok(employee.user_id.into_iter().collect()),
        OWNER_HR => return hr_holders(state, scopes).await,
        OWNER_MANAGER => Some(holders_of_role(&state.db, &config.manager_role, &looked).await?),
        other => match other.strip_prefix("role:") {
            Some(role) => Some(holders_of_role(&state.db, role.trim(), &looked).await?),
            None => None,
        },
    };
    match named {
        Some(ids) if !ids.is_empty() => Ok(ids.into_iter().collect()),
        _ => hr_holders(state, scopes).await,
    }
}
