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
use wl_kernel::documents::{AffectedGroup, DUBAI_OFFSET_SECONDS, DocumentAccepted, Validity};
use wl_kernel::tickets::{self, Assignee, OpenTicket, TASK_CATEGORY, TicketId};
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

/// Lifts the block of an accepted renewal (#49 A9): when the guard answers
/// *yes* for the subject and kind on `today`, closes every open
/// [`RenewalTask`] in one transaction, recording the accepted document, and
/// after the commit completes the task tickets with "Renewed: <kind name>,
/// valid until <date>". Returns the number of rows closed. A renewal that is
/// already past its expiry date, a kind this module does not know and a
/// subject without an open task close nothing. Nothing is resumed here.
pub async fn lift(
    state: &AppState,
    accepted: &DocumentAccepted,
    today: NaiveDate,
) -> Result<usize> {
    let db = &state.db;
    let Some(kind) = DocumentKind::by_key(db, &accepted.kind).await? else {
        return Ok(0);
    };
    let subject = &accepted.subject;
    let validity = wl_kernel::documents::guard(state)
        .is_valid(db, subject, &accepted.kind, today)
        .await?;
    let Validity::Valid { expires_on, .. } = validity else {
        return Ok(0);
    };
    let open: Vec<RenewalTask> =
        RenewalTask::for_subject(db, &subject.subject_type, subject.subject_id, kind.id)
            .await?
            .into_iter()
            .filter(|row| row.closed_at.is_none())
            .collect();
    let mut tx = db.begin().await?;
    let mut closed = Vec::new();
    for mut row in open {
        row.close(&mut tx, accepted.document.0).await?;
        closed.push(row);
    }
    tx.commit().await?;

    // After the commit: the tickets.
    let note = note(kind.name("en"), expires_on.or(accepted.expires_on));
    let creator = tickets::task_creator(state);
    for row in &closed {
        if let Some(ticket) = row.ticket_id {
            creator.complete(state, TicketId(ticket), &note).await?;
        }
    }
    Ok(closed.len())
}

/// "Renewed: <kind name>, valid until <date>"; without an expiry date,
/// "Renewed: <kind name>, no expiry date".
fn note(kind_name: &str, expires_on: Option<NaiveDate>) -> String {
    match expires_on {
        Some(date) => english(
            "wl_documents.renewal.done",
            &[
                ("kind", &kind_name),
                ("expires", &date.format("%-d %b %Y").to_string()),
            ],
        ),
        None => english("wl_documents.renewal.done_open", &[("kind", &kind_name)]),
    }
}

/// Registers the `DocumentAccepted` listener that calls [`lift`].
pub(crate) fn register(app: &mut Registry) {
    app.listen(|event: DocumentAccepted, state: AppState| async move {
        lift(&state, &event, wl_kernel::documents::today())
            .await
            .map(|_| ())
    });
}

#[cfg(test)]
mod tests {
    use renox::testing::TestApp;

    use super::*;
    use crate::DocumentStatus;

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

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    #[test]
    fn the_note_names_the_kind_and_the_new_expiry() {
        assert_eq!(
            note("Work permit", Some(date("2028-11-30"))),
            "Renewed: Work permit, valid until 30 Nov 2028"
        );
        assert_eq!(
            note("Work permit", None),
            "Renewed: Work permit, no expiry date"
        );
    }

    /// A subject with an open renewal task and an accepted renewal that
    /// expires on `expiry`; the event for the renewal.
    async fn renewal_case(app: &TestApp, expiry: &str) -> (DocumentAccepted, i64, i64) {
        use crate::kinds::{KindSeed, install_seeds};
        let db = app.db();
        install_seeds(
            db,
            [&KindSeed::new("renewal_permit", "Work permit").expires()],
        )
        .await
        .expect("a kind");
        let kind = DocumentKind::by_key(db, "renewal_permit")
            .await
            .unwrap()
            .unwrap();
        let at: DateTime = "2026-12-05T08:00:00Z".parse().unwrap();
        let mut old = Document {
            kind_id: kind.id,
            subject_type: "hr.employee".into(),
            subject_id: 7,
            status: DocumentStatus::Accepted,
            accepted_at: Some("2025-12-01T08:00:00Z".parse().unwrap()),
            submitted_at: Some("2025-12-01T08:00:00Z".parse().unwrap()),
            expiry_date: Some(date("2026-11-30")),
            ..Document::default()
        };
        old.save(db).await.unwrap();
        let mut renewal = Document {
            expiry_date: Some(date(expiry)),
            status: DocumentStatus::Accepted,
            accepted_at: Some(at),
            submitted_at: Some(at),
            ..old.clone()
        };
        renewal.id = 0;
        renewal.save(db).await.unwrap();
        let row = RenewalTask {
            subject_type: "hr.employee".into(),
            subject_id: 7,
            kind_id: kind.id,
            expired_document_id: old.id,
            ticket_id: Some(11),
            opened_at: at,
            ..RenewalTask::default()
        };
        assert!(row.insert_new(db).await.unwrap());
        let event = DocumentAccepted {
            document: wl_kernel::documents::DocumentId(renewal.id),
            subject: Subject::new("hr.employee", 7),
            kind: "renewal_permit".into(),
            expires_on: Some(date(expiry)),
        };
        (event, kind.id, renewal.id)
    }

    #[renox::test]
    async fn an_accepted_renewal_closes_the_open_task() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let (event, kind_id, renewal) = renewal_case(&app, "2028-11-30").await;

        let closed = lift(app.state(), &event, date("2026-12-05")).await.unwrap();
        assert_eq!(closed, 1);
        let rows = RenewalTask::for_subject(app.db(), "hr.employee", 7, kind_id)
            .await
            .unwrap();
        assert!(rows[0].closed_at.is_some());
        assert_eq!(rows[0].renewal_document_id, Some(renewal));
        // Run again: nothing is open any more.
        assert_eq!(
            lift(app.state(), &event, date("2026-12-05")).await.unwrap(),
            0
        );
    }

    #[renox::test]
    async fn a_renewal_already_past_its_expiry_closes_nothing() {
        let app = TestApp::new(App::new().module(crate::WlDocuments)).await;
        let (event, kind_id, _) = renewal_case(&app, "2026-12-03").await;

        let closed = lift(app.state(), &event, date("2026-12-05")).await.unwrap();
        assert_eq!(closed, 0);
        assert!(
            RenewalTask::find_open(app.db(), "hr.employee", 7, kind_id)
                .await
                .unwrap()
                .is_some()
        );
    }
}
