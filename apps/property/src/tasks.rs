//! Tasks for a role in a building (01 Property §7.2).
//!
//! The kernel's [`role_tasks::open`] assigns a task to a holder of the role
//! everywhere, or to the company admin. In this vertical roles are assigned
//! per building (§3.2), so a task for "the property manager of the lease"
//! goes to a holder of the role in the lease's building first; only when
//! nobody holds it there does it fall back to the kernel's choice.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::prelude::*;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TicketRef, task_creator};

/// Opens `task`, or returns the one still open under the same source key,
/// assigned to the first holder of its role in `scope`, else to whom
/// [`role_tasks::assignee`] picks. `None` when ticketing is off.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub(crate) async fn open(
    state: &AppState,
    scope: &Scope,
    task: RoleTask,
) -> Result<Option<TicketRef>> {
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &task.source_key).await? {
        return Ok(Some(found));
    }
    let mut ticket = OpenTicket::new(task.subject.clone(), TASK_CATEGORY, &task.title)
        .description(&task.description)
        .source_key(&task.source_key);
    if let Some(due) = task.due_at {
        ticket = ticket.due_at(due);
    }
    let holder = match users_with_role_in(&state.db, &task.role, scope)
        .await?
        .first()
    {
        Some(user) => Some(user.id),
        None => role_tasks::assignee(&state.db, &task.role).await?,
    };
    if let Some(user) = holder {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await
}
