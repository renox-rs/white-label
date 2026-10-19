//! A task for a role (wl-kernel §5.14 step 4, §5.15): a ticket through the
//! [`TaskCreator`](crate::tickets::TaskCreator) assigned to one holder of
//! the role, or of the company admin role when nobody holds it. The kernel
//! uses it for the reconciliation task of a recovered connection and for the
//! owner's task when a compliance-critical rule fails.
//!
//! Without the `ticketing` module no ticket is opened ([`NoTickets`](crate::NoTickets))
//! and [`open`] answers `None`.

use renox::auth::permissions::users_with_role;
use renox::db::DateTime;
use renox::prelude::*;

use crate::scopes::COMPANY_ADMIN_ROLE;
use crate::subject::Subject;
use crate::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY, TicketRef};

/// The user a task for `role` goes to: the first holder of the role, else
/// the first company admin; `None` when nobody holds either.
pub async fn assignee(db: &Db, role: &str) -> Result<Option<i64>> {
    for role in [role, COMPANY_ADMIN_ROLE] {
        if let Some(user) = users_with_role(db, role).await?.first() {
            return Ok(Some(user.id));
        }
    }
    Ok(None)
}

/// A task to open for a role.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RoleTask {
    /// The record the task is about.
    pub subject: Subject,
    /// The role responsible.
    pub role: String,
    /// The title.
    pub title: String,
    /// The description.
    pub description: String,
    /// A key the task can be found by while it is open.
    pub source_key: String,
    /// When it is due, if it has a due time.
    pub due_at: Option<DateTime>,
}

impl RoleTask {
    /// A task about `subject` for `role`, found by `source_key` while open.
    pub fn new(subject: Subject, role: &str, title: &str, source_key: &str) -> Self {
        RoleTask {
            subject,
            role: role.to_owned(),
            title: title.to_owned(),
            description: String::new(),
            source_key: source_key.to_owned(),
            due_at: None,
        }
    }

    /// With this description.
    pub fn description(mut self, description: &str) -> Self {
        self.description = description.to_owned();
        self
    }

    /// Due at `due_at`.
    pub fn due_at(mut self, due_at: DateTime) -> Self {
        self.due_at = Some(due_at);
        self
    }
}

/// Opens the task, or returns the one still open under the same source key.
/// `None` when ticketing is off.
pub async fn open(state: &AppState, task: RoleTask) -> Result<Option<TicketRef>> {
    let creator = tickets::task_creator(state);
    if let Some(found) = creator.find_open(state, &task.source_key).await? {
        return Ok(Some(found));
    }
    let mut ticket = OpenTicket::new(task.subject.clone(), TASK_CATEGORY, &task.title)
        .description(&task.description)
        .source_key(&task.source_key);
    if let Some(due) = task.due_at {
        ticket = ticket.due_at(due);
    }
    if let Some(user) = assignee(&state.db, &task.role).await? {
        ticket = ticket.assign_to(Assignee::user(user));
    }
    creator.open(state, ticket).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_names_its_subject_role_and_key() {
        let due: DateTime = "2026-11-03T06:00:00Z".parse().unwrap();
        let task = RoleTask::new(
            Subject::new("kernel.integration_connection", 3),
            "accountant",
            "Reconcile gateway.acme",
            "kernel.reconcile:gateway.acme:1",
        )
        .description("TX-889")
        .due_at(due);
        assert_eq!(task.role, "accountant");
        assert_eq!(task.description, "TX-889");
        assert_eq!(task.due_at, Some(due));
        assert_eq!(task.source_key, "kernel.reconcile:gateway.acme:1");
    }
}
