//! Tasks for a role on a project (02 Construction §7.1, §7.2).
//!
//! The kernel's [`role_tasks::open`] assigns a task to a holder of the role
//! everywhere, or to the company admin. Roles are assigned per project
//! (§3.2), so a task goes to the user named first, else to a holder of the
//! role in the project's scope, else to whom the kernel picks. A task opened
//! twice under one source key is the same task.

use renox::auth::permissions::users_with_role_in;
use renox::chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::documents::DUBAI_OFFSET_SECONDS;
use wl_kernel::role_tasks;
use wl_kernel::tickets::{Assignee, OpenTicket, TASK_CATEGORY, TicketRef, task_creator};

use crate::project::Project;

/// 09:00 in Asia/Dubai on `day`, as an instant: the due time of a task whose
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

/// A task to open.
#[derive(Debug, Clone)]
pub(crate) struct Task {
    /// The record the task is about.
    pub subject: Subject,
    /// The role responsible.
    pub role: &'static str,
    /// A user it goes to before anyone else.
    pub assignee: Option<i64>,
    /// The key it is found by while open.
    pub source_key: String,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// The day it is due.
    pub due: Option<NaiveDate>,
}

/// Opens `task` for `project`, or returns the one still open under its key.
/// `None` when ticketing is off.
///
/// # Errors
///
/// The database's and ticketing's errors.
pub(crate) async fn open(
    state: &AppState,
    project: &Project,
    task: Task,
) -> Result<Option<TicketRef>> {
    let creator = task_creator(state);
    if let Some(found) = creator.find_open(state, &task.source_key).await? {
        return Ok(Some(found));
    }
    let mut ticket = OpenTicket::new(task.subject.clone(), TASK_CATEGORY, &task.title)
        .description(&task.description)
        .source_key(&task.source_key);
    if let Some(due) = task.due.and_then(dubai_morning) {
        ticket = ticket.due_at(due);
    }
    let holder = match task.assignee {
        Some(user) => Some(user),
        None => match users_with_role_in(&state.db, task.role, &project.scope())
            .await?
            .first()
        {
            Some(user) => Some(user.id),
            None => role_tasks::assignee(&state.db, task.role).await?,
        },
    };
    if let Some(user) = holder {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_due_day_is_nine_in_the_morning_in_dubai() {
        let day = NaiveDate::from_ymd_opt(2027, 5, 31).unwrap();
        let due = dubai_morning(day).unwrap();
        assert_eq!(due.to_rfc3339(), "2027-05-31T05:00:00+00:00");
    }
}
