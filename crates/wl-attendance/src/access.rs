//! Who may act at a work location: a permission held in the scope of the
//! location (the project, site or branch it is in), or globally (wl-attendance
//! §6 *Permissions*, `HRM.ACL.01`).
//!
//! Decision: `HRM.ACL.01` also limits a supervisor to the crews and people
//! assigned to their location. Crews and assignments are not built yet, so
//! only the scope of the location is checked.

use renox::auth::permissions::{Scope, users_with_permission_in};
use renox::prelude::*;
use wl_kernel::Subject;

use crate::location::WorkLocation;

/// The scope a location is in: the one the app registered for the type of
/// its `Subject` (`wl_kernel::scopes`), else the subject's own type and id;
/// global for a location with no scope.
pub async fn scope_of(db: &Db, location: &WorkLocation) -> Result<Scope> {
    scope_of_subject(db, location.scope().as_ref()).await
}

/// The scope of a context (project, deployment...): the one the app
/// registered for the type of its `Subject`, else the subject's own type and
/// id; global for none.
pub async fn scope_of_subject(db: &Db, subject: Option<&Subject>) -> Result<Scope> {
    let Some(subject) = subject else {
        return Ok(Scope::global());
    };
    let registered = wl_kernel::scopes::scope_of(db, subject).await?;
    Ok(if registered.is_global() {
        Scope::new(&subject.subject_type, subject.subject_id)
    } else {
        registered
    })
}

/// Whether user `user` holds `permission` in the scope of `context` (a
/// timesheet's context), or globally for none.
pub async fn holds_in_context(
    state: &AppState,
    user: i64,
    permission: &str,
    context: Option<&Subject>,
) -> Result<bool> {
    let scope = scope_of_subject(&state.db, context).await?;
    Ok(users_with_permission_in(&state.db, permission, &scope)
        .await?
        .iter()
        .any(|u| u.id == user))
}

/// Whether user `user` holds `permission` at `location`.
pub async fn holds(
    state: &AppState,
    user: i64,
    permission: &str,
    location: &WorkLocation,
) -> Result<bool> {
    let scope = scope_of(&state.db, location).await?;
    Ok(users_with_permission_in(&state.db, permission, &scope)
        .await?
        .iter()
        .any(|u| u.id == user))
}
