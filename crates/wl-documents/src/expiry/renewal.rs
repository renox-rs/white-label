//! The renewal task of an expired document (wl-documents §5.2, #49 A7).
//!
//! [`open`] asks the kernel [`TaskCreator`] for one task in category `task`
//! for the responsible role: the source key is
//! `documents.renewal:{subject_type}:{subject_id}:{kind}`, the title
//! "Renew <kind name> of <subject label>" and the due date the run date. The
//! assignee is [`assignee`]. A task is never opened twice: the
//! [`RenewalTask`] row of the run's transaction is unique per open
//! subject and kind, and a ticket already open under the source key is
//! reused. Without ticketing the row stays without a ticket.

use std::collections::BTreeSet;

use renox::auth::permissions::Scope;
use renox::chrono::{FixedOffset, NaiveDate};
use renox::db::DateTime;
use renox::prelude::*;
use wl_kernel::documents::{AffectedGroup, DUBAI_OFFSET_SECONDS};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY};
use wl_kernel::{COMPANY_ADMIN_ROLE, Subject, scopes};

use super::logs::RenewalTask;
use super::responsible::{holders_in, role_for};
use crate::documents::Document;
use crate::kinds::DocumentKind;
use crate::texts::english;

/// The subject type of a document in kernel events and sources.
pub const DOCUMENT_SUBJECT_TYPE: &str = "wl_documents.document";

/// The document `id` as a [`Subject`].
pub fn document_subject(id: i64) -> Subject {
    Subject::new(DOCUMENT_SUBJECT_TYPE, id)
}

/// The key the task is found by while it is open.
pub fn source_key(subject: &Subject, kind_key: &str) -> String {
    format!(
        "documents.renewal:{}:{}:{}",
        subject.subject_type, subject.subject_id, kind_key
    )
}

/// The end of `date` in Asia/Dubai, as the task's due time.
pub fn due_at(date: NaiveDate) -> DateTime {
    let dubai = FixedOffset::east_opt(DUBAI_OFFSET_SECONDS).expect("UTC+4 is a valid offset");
    date.and_hms_opt(23, 59, 59)
        .and_then(|local| local.and_local_timezone(dubai).single())
        .map(|at| at.to_utc())
        .unwrap_or_default()
}

/// The user the task goes to: the lowest-id holder of `role` in the
/// subject's scope; else in the scopes of the blocked records; else
/// globally; else the company administrators.
pub async fn assignee(
    state: &AppState,
    role: &str,
    subject: &Subject,
    items: &[AffectedGroup],
) -> Result<Option<i64>> {
    let db = &state.db;
    let scope = scopes::scope_of(db, subject).await?;
    if !scope.is_global()
        && let Some(user) = holders_in(db, role, &scope).await?.first()
    {
        return Ok(Some(*user));
    }
    let mut record_scopes: BTreeSet<(String, String)> = BTreeSet::new();
    let mut found = BTreeSet::new();
    for record in items.iter().flat_map(|group| &group.records) {
        let Some(scope) = record.scope.as_ref().filter(|s| !s.is_global()) else {
            continue;
        };
        if record_scopes.insert((scope.kind().to_owned(), scope.id().to_owned())) {
            found.extend(holders_in(db, role, scope).await?);
        }
    }
    if let Some(user) = found.first() {
        return Ok(Some(*user));
    }
    for role in [role, COMPANY_ADMIN_ROLE] {
        if let Some(user) = holders_in(db, role, &Scope::global()).await?.first() {
            return Ok(Some(*user));
        }
    }
    Ok(None)
}

/// Opens the renewal task for the expired `document`, after the run's
/// transaction wrote its [`RenewalTask`] row, and records the ticket on the
/// row. Does nothing when ticketing is off.
pub async fn open(
    state: &AppState,
    document: &Document,
    kind: &DocumentKind,
    subject_label: &str,
    expires_on: NaiveDate,
    today: NaiveDate,
    items: &[AffectedGroup],
) -> Result<()> {
    let db = &state.db;
    let subject = document.subject();
    let creator = tickets::task_creator(state);
    let key = source_key(&subject, &kind.key);
    let ticket = match creator.find_open(state, &key).await? {
        Some(ticket) => Some(ticket),
        None => {
            let kind_name = kind.name("en");
            let expires = expires_on.format("%-d %b %Y").to_string();
            let title = english(
                "wl_documents.renewal.title",
                &[("kind", &kind_name), ("subject", &subject_label)],
            );
            let description = english(
                "wl_documents.renewal.description",
                &[
                    ("kind", &kind_name),
                    ("subject", &subject_label),
                    ("expires", &expires),
                ],
            );
            let role = role_for(kind.responsible_role.as_deref());
            let mut open = OpenTicket::new(subject.clone(), TASK_CATEGORY, &title)
                .description(&description)
                .source(document_subject(document.id))
                .source_key(&key)
                .due_at(due_at(today));
            if let Some(user) = assignee(state, role, &subject, items).await? {
                open = open.assign_to(Assignee::user(user));
            }
            creator.open(state, open).await?
        }
    };
    if let Some(ticket) = ticket
        && let Some(mut row) =
            RenewalTask::find_open(db, &subject.subject_type, subject.subject_id, kind.id).await?
    {
        row.ticket_id = Some(ticket.id.0);
        row.ticket_number = Some(ticket.number);
        row.save(db).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_source_key_names_the_subject_and_the_kind() {
        assert_eq!(
            source_key(&Subject::new("hr.employee", 7), "work_permit"),
            "documents.renewal:hr.employee:7:work_permit"
        );
    }

    #[test]
    fn the_task_is_due_at_the_end_of_the_run_day_in_dubai() {
        let date: NaiveDate = "2026-12-01".parse().unwrap();
        let due = due_at(date);
        assert_eq!(wl_kernel::documents::dubai_date(due), date);
        assert_eq!(due.to_rfc3339(), "2026-12-01T19:59:59+00:00");
    }

    #[test]
    fn a_document_is_a_subject() {
        assert_eq!(
            document_subject(3),
            Subject::new("wl_documents.document", 3)
        );
    }
}
