//! Tasks for a role in a building (01 Property §7.2).
//!
//! The kernel's [`role_tasks::open`] assigns a task to a holder of the role
//! everywhere, or to the company admin. In this vertical roles are assigned
//! per building (§3.2), so a task for "the property manager of the lease"
//! goes to a holder of the role in the lease's building first; only when
//! nobody holds it there does it fall back to the kernel's choice.

use renox::auth::permissions::{Scope, users_with_role_in};
use renox::chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::documents::DUBAI_OFFSET_SECONDS;
use wl_kernel::role_tasks::{self, RoleTask};
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TicketRef, task_creator};

/// 09:00 in Asia/Dubai on `day`, as an instant: the due time of a ticket whose
/// due date is a day.
pub(crate) fn dubai_morning(day: NaiveDate) -> Option<DateTime> {
    let local = day.and_hms_opt(9, 0, 0)?;
    let zone = FixedOffset::east_opt(DUBAI_OFFSET_SECONDS)?;
    Some(
        zone.from_local_datetime(&local)
            .single()?
            .with_timezone(&Utc),
    )
}

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
    open_in(state, scope, task, TASK_CATEGORY).await
}

/// Like [`open`], in the ticket category `category` (a category of the
/// vertical, such as *legal case* or *move-out inspection*). A category the
/// ticketing module does not know yet (the vertical's starter data seeds it)
/// falls back to the staff task category, so the work is never lost.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub(crate) async fn open_in(
    state: &AppState,
    scope: &Scope,
    task: RoleTask,
    category: &str,
) -> Result<Option<TicketRef>> {
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &task.source_key).await? {
        return Ok(Some(found));
    }
    let mut ticket = OpenTicket::new(task.subject.clone(), category, &task.title)
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
    match creator.open(state, ticket.clone()).await {
        Err(error) if category != TASK_CATEGORY => {
            eprintln!("property: no ticket category `{category}` ({error:?}); opened as a task");
            ticket.category_key = TASK_CATEGORY.to_owned();
            creator.open(state, ticket).await
        }
        other => other,
    }
}
