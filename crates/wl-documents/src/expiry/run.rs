//! The nightly expiry run (wl-documents §5.2, `DOC.RUL.01` and `DOC.RUL.02`,
//! #48 A3-A10).
//!
//! [`run`] looks at every `accepted` document with an expiry date that has
//! not passed:
//!
//! - **Offset reminder** (`DOC.RUL.01`, A6): an offset of the kind's
//!   `reminder_offsets` is reached when the days left are at most the offset
//!   and it is not in the [`ReminderLog`] yet. Only the nearest reached
//!   offset is sent; the larger ones are logged as passed.
//! - **Look-ahead** (`DOC.RUL.02`, A1, A4): inside the kind's
//!   `look_ahead_days`, each `blocks` entry whose transition has a
//!   [`FutureWork`](wl_kernel::documents::FutureWork) provider is asked for
//!   the planned records dated after the expiry date (`from` = expiry + 1).
//!   Records without an [`AffectedItem`] row are new.
//!
//! When there is an offset or a new item, the document is handled in one
//! transaction (the log rows are written), and after the commit one merged
//! [`DocumentExpiring`] is emitted (with all the current affected records)
//! and each responsible user gets one `documents.expiring` message through
//! the kernel [`Notifier`](wl_kernel::notify::Notifier) (A7, A9). Without
//! either, nothing is emitted or sent, so a second run on the same night
//! does nothing.
//!
//! `DOC.RUL.03` follows in the same run ([`expired`](super::expired)): the
//! documents that expired before `today` emit `DocumentExpired`, tell the
//! responsible staff and the owner, and open the renewal task.
//!
//! A failure on one document is printed to stderr and counted in
//! [`Report::failed`]; the run moves on (A10). A message that cannot be
//! queued is also counted there: its log rows stay, so it is not retried.

use std::collections::BTreeMap;
use std::fmt::Display;

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::Subject;
use wl_kernel::documents::{
    AffectedGroup, DocumentExpiring, PlannedRecord, block_subjects, default_subject_label,
    future_work,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, NotifyRequest};

use super::logs::{AffectedItem, ReminderLog};
use super::responsible::{self, Recipient};
use crate::documents::{Document, DocumentStatus};
use crate::kinds::{DocumentKind, KindBlock};
use crate::texts::english;

/// The notification type of the message.
pub const NOTIFICATION_TYPE: &str = "documents.expiring";

/// The daily scheduled task, and the command, that run [`run`].
pub const TASK: &str = "documents:expiry";

/// When [`TASK`] runs, in Asia/Dubai.
pub const TASK_AT: &str = "01:00";

/// The time zone of [`TASK`].
pub const TASK_ZONE: &str = "Asia/Dubai";

/// What a run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Documents that were scanned.
    pub scanned: usize,
    /// `DocumentExpiring` events emitted.
    pub events: usize,
    /// Messages queued through the notifier.
    pub messages: usize,
    /// Documents or messages that failed.
    pub failed: usize,
    /// Expired documents picked up by `DOC.RUL.03`.
    pub expired: usize,
    /// `DocumentExpired` events emitted.
    pub expired_events: usize,
    /// Renewal tasks opened.
    pub tasks: usize,
}

/// The offsets to handle for a document with `days_left` days to go:
/// `(send, passed)`. `send` is the nearest reached offset not in `logged`;
/// `passed` are the larger reached offsets, logged without a send. A
/// negative offset is ignored.
pub fn reached_offsets(offsets: &[i64], days_left: i64, logged: &[i64]) -> (Option<i64>, Vec<i64>) {
    let mut reached: Vec<i64> = offsets
        .iter()
        .copied()
        .filter(|offset| *offset >= 0 && days_left <= *offset && !logged.contains(offset))
        .collect();
    reached.sort_unstable();
    reached.dedup();
    if reached.is_empty() {
        return (None, Vec::new());
    }
    let send = reached.remove(0);
    (Some(send), reached)
}

/// `date` as `15 Dec 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The notification type `documents.expiring`.
pub fn notification_type() -> NotificationTypeDef {
    let subject = english("wl_documents.expiring.subject", &[]);
    let body = english("wl_documents.expiring.body", &[]);
    NotificationTypeDef::new(NOTIFICATION_TYPE, "documents")
        .group("documents")
        .category(Category::Reminder)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the notification type with the kernel registry.
pub(crate) fn register_type() {
    notify::register_type(notification_type());
    super::expired::register_type();
}

/// The template variables of the message to one recipient.
fn message_data(
    kind_name: &str,
    subject_label: &str,
    expires_on: NaiveDate,
    days_left: i64,
    offset: Option<i64>,
    groups: &[AffectedGroup],
) -> Value {
    let expires = date_text(expires_on);
    let params: [(&str, &dyn Display); 4] = [
        ("kind", &kind_name),
        ("subject", &subject_label),
        ("expires", &expires),
        ("left", &days_left),
    ];
    let mut summary = Vec::new();
    if offset.is_some() {
        summary.push(english("wl_documents.expiring.reminder", &params));
    }
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for group in groups {
        for record in &group.records {
            let date = date_text(record.date);
            lines.push(english(
                "wl_documents.expiring.item",
                &[
                    ("label", &record.label),
                    ("date", &date),
                    ("guarded", &group.guarded),
                ],
            ));
            items.push(json!({
                "label": record.label,
                "date": record.date.to_string(),
                "guarded": group.guarded,
            }));
        }
    }
    if !items.is_empty() {
        summary.push(english("wl_documents.expiring.items", &params));
    }
    json!({
        "kind": kind_name,
        "kind_name": kind_name,
        "subject_label": subject_label,
        "expires_on": expires,
        "days_left": days_left,
        "offset": offset,
        "summary": summary.join(" "),
        "items_text": lines.join("\n"),
        "items": items,
    })
}

/// Runs the nightly check for `today` (Asia/Dubai) and reports what it did.
pub async fn run(state: &AppState, today: NaiveDate) -> Result<Report> {
    let db = &state.db;
    let kinds: BTreeMap<i64, DocumentKind> = DocumentKind::ordered(db)
        .await?
        .into_iter()
        .map(|kind| (kind.id, kind))
        .collect();
    let documents = Document::query()
        .where_(Document::STATUS.eq(DocumentStatus::Accepted))
        .where_(Document::EXPIRY_DATE.is_not_null())
        .order_by(Document::ID)
        .get(db)
        .await?;
    let mut report = Report::default();
    for document in documents {
        let Some(kind) = kinds.get(&document.kind_id) else {
            continue;
        };
        report.scanned += 1;
        match handle(state, &document, kind, today).await {
            Ok(done) => {
                report.events += usize::from(done.event);
                report.messages += done.messages;
                report.failed += done.failed;
            }
            Err(error) => {
                eprintln!("documents:expiry: document {} failed: {error}", document.id);
                report.failed += 1;
            }
        }
    }
    // DOC.RUL.03: the documents that expired.
    let expired = super::expired::run_rule(state, &kinds, today).await?;
    report.expired += expired.scanned;
    report.expired_events += expired.events;
    report.messages += expired.messages;
    report.tasks += expired.tasks;
    report.failed += expired.failed;
    Ok(report)
}

/// What one document gave.
#[derive(Default)]
struct Done {
    event: bool,
    messages: usize,
    failed: usize,
}

/// Handles one document: the logs in a transaction, then the event and the
/// messages.
async fn handle(
    state: &AppState,
    document: &Document,
    kind: &DocumentKind,
    today: NaiveDate,
) -> Result<Done> {
    let Some(expires_on) = document.expiry_date else {
        return Ok(Done::default());
    };
    let days_left = (expires_on - today).num_days();
    if days_left < 0 {
        return Ok(Done::default());
    }
    let db = &state.db;
    let subject = document.subject();

    // DOC.RUL.01: the offset.
    let logged = ReminderLog::offsets_for(db, document.id).await?;
    let (send, passed) = reached_offsets(&kind.reminder_offsets.0, days_left, &logged);

    // DOC.RUL.02: the planned work after the expiry date.
    let blocks = if days_left <= kind.look_ahead_days {
        kind.blocks(db).await?
    } else {
        Vec::new()
    };
    let affected = affected_records(state, kind, &blocks, &subject, expires_on).await?;

    let mut new_groups: Vec<AffectedGroup> = Vec::new();
    let mut tx = db.begin().await?;
    for group in &affected {
        let mut fresh = Vec::new();
        for record in &group.records {
            let item = AffectedItem {
                document_id: document.id,
                guarded: group.guarded.clone(),
                record_type: record.record.subject_type.clone(),
                record_id: record.record.subject_id,
                planned_date: record.date,
                first_reported_on: today,
                ..AffectedItem::default()
            };
            if item.insert_new(&mut tx).await? {
                fresh.push(record.clone());
            }
        }
        if !fresh.is_empty() {
            new_groups.push(AffectedGroup {
                guarded: group.guarded.clone(),
                records: fresh,
            });
        }
    }
    let mut offset = None;
    if let Some(send) = send
        && ReminderLog::insert_new(&mut tx, document.id, send, today).await?
    {
        offset = Some(send);
    }
    for passed in passed {
        ReminderLog::insert_new(&mut tx, document.id, passed, today).await?;
    }
    tx.commit().await?;
    if offset.is_none() && new_groups.is_empty() {
        return Ok(Done::default());
    }

    // After the commit: the event (A7: one per document and night).
    state
        .emit(DocumentExpiring {
            document_id: document.document_id(),
            subject: subject.clone(),
            kind: kind.key.clone(),
            expires_on,
            days_left,
            offset: offset.and_then(|offset| u32::try_from(offset).ok()),
            affected,
        })
        .await?;

    // One message per recipient, listing only the new items.
    let role = responsible::role_of(db, kind, document).await?;
    let recipients = responsible::resolve(state, &role, &kind.key, &subject, &new_groups).await?;
    let label = subject_label(state, &blocks, &subject).await;
    let mut done = Done {
        event: true,
        ..Done::default()
    };
    for recipient in recipients {
        let data = message_data(
            kind.name("en"),
            &label,
            expires_on,
            days_left,
            offset,
            &recipient.groups,
        );
        match send_message(state, NOTIFICATION_TYPE, &subject, &label, &recipient, data).await {
            Ok(count) => done.messages += count,
            Err(error) => {
                eprintln!(
                    "documents:expiry: message for document {} to user {} failed: {error}",
                    document.id, recipient.user_id
                );
                done.failed += 1;
            }
        }
    }
    Ok(done)
}

/// The full current list of planned records after `expires_on`, grouped by
/// guarded transition. Entries without a provider are skipped; a record not
/// dated after the expiry date is left out.
async fn affected_records(
    state: &AppState,
    kind: &DocumentKind,
    blocks: &[KindBlock],
    subject: &Subject,
    expires_on: NaiveDate,
) -> Result<Vec<AffectedGroup>> {
    let Some(from) = expires_on.checked_add_days(Days::new(1)) else {
        return Ok(Vec::new());
    };
    let mut groups: Vec<AffectedGroup> = Vec::new();
    for block in blocks {
        let Some(provider) = future_work(&block.guarded) else {
            continue;
        };
        let entry = block.to_entry(kind);
        let records: Vec<PlannedRecord> = provider
            .planned_from(state, &entry, subject, from)
            .await?
            .into_iter()
            .filter(|record| record.date >= from)
            .collect();
        if records.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|g| g.guarded == block.guarded) {
            Some(group) => group.records.extend(records),
            None => groups.push(AffectedGroup {
                guarded: block.guarded.clone(),
                records,
            }),
        }
    }
    Ok(groups)
}

/// The subject's label: the first `BlockSubjects::label` among the kind's
/// guarded transitions, else `<subject_type> #<id>`.
pub(super) async fn subject_label(
    state: &AppState,
    blocks: &[KindBlock],
    subject: &Subject,
) -> String {
    for block in blocks {
        if let Some(provider) = block_subjects(&block.guarded)
            && let Ok(Some(label)) = provider.label(state, subject).await
        {
            return label;
        }
    }
    default_subject_label(subject)
}

/// Queues the message to one recipient; returns how many messages went.
pub(super) async fn send_message(
    state: &AppState,
    type_key: &str,
    subject: &Subject,
    label: &str,
    recipient: &Recipient,
    data: Value,
) -> Result<usize> {
    let request = NotifyRequest::new(type_key, subject.clone())
        .to_user(recipient.user_id)
        .subject_label(label)
        .data(data);
    let ids = notify::notifier(state).notify(state, request).await?;
    // Without wl-notify no id comes back, but the bell was rung.
    Ok(ids.len().max(1))
}

/// The date of `--date YYYY-MM-DD`, else `today`.
pub fn date_arg(value: Option<&str>, today: NaiveDate) -> Result<NaiveDate> {
    match value {
        None => Ok(today),
        Some(text) => text.parse().map_err(|_| {
            Error::Status(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("--date must be YYYY-MM-DD, got `{text}`"),
            )
        }),
    }
}

/// Registers the daily [`TASK`] and the command of the same name.
pub(crate) fn schedule(app: &mut Registry) {
    app.schedule()
        .daily_at(TASK_AT, TASK, |state: AppState| async move {
            run(&state, wl_kernel::documents::today()).await.map(|_| ())
        })
        .timezone(TASK_ZONE);
    app.command(
        TASK,
        "Report documents whose reminder offsets are reached or that leave planned work \
         uncovered (--date YYYY-MM-DD checks another day)",
        |args, state: AppState| async move {
            let on = date_arg(args.value("--date"), wl_kernel::documents::today())?;
            let report = run(&state, on).await?;
            println!(
                "{} documents scanned, {} expiring, {} expired, {} tasks, {} messages, {} failed.",
                report.scanned,
                report.events,
                report.expired_events,
                report.tasks,
                report.messages,
                report.failed
            );
            Ok(())
        },
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use renox::auth::permissions::{Scope, define_role};
    use renox::testing::TestApp;
    use wl_kernel::notify::{MessageId, Notifier, NotifyFuture, provide_notifier};

    use super::*;
    use crate::WlDocuments;
    use crate::kinds::{KindSeed, install_seeds};

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    #[test]
    fn the_nearest_reached_offset_is_sent_and_larger_ones_are_passed() {
        let offsets = [90, 30, 7];
        assert_eq!(reached_offsets(&offsets, 100, &[]), (None, vec![]));
        assert_eq!(reached_offsets(&offsets, 90, &[]), (Some(90), vec![]));
        assert_eq!(reached_offsets(&offsets, 30, &[90]), (Some(30), vec![]));
        assert_eq!(reached_offsets(&offsets, 30, &[]), (Some(30), vec![90]));
        assert_eq!(reached_offsets(&offsets, 5, &[90]), (Some(7), vec![30]));
        assert_eq!(reached_offsets(&offsets, 5, &[90, 30, 7]), (None, vec![]));
        assert_eq!(reached_offsets(&offsets, 0, &[]), (Some(7), vec![30, 90]));
        assert_eq!(reached_offsets(&[-1], 0, &[]), (None, vec![]));
    }

    #[test]
    fn the_date_argument_defaults_to_today() {
        let today = date("2026-11-15");
        assert_eq!(date_arg(None, today).unwrap(), today);
        assert_eq!(
            date_arg(Some("2026-12-01"), today).unwrap(),
            date("2026-12-01")
        );
        assert!(date_arg(Some("tomorrow"), today).is_err());
    }

    #[test]
    fn the_type_is_a_reminder_with_an_english_text() {
        let def = notification_type();
        assert_eq!(def.key, "documents.expiring");
        assert_eq!(def.category, Category::Reminder);
        let template = def.template_for(NotifyChannel::Email, "en").unwrap();
        assert!(
            template
                .subject
                .as_deref()
                .unwrap()
                .contains("{{ kind_name }}")
        );
        assert!(template.body.contains("{{ items_text }}"));
    }

    fn visit(id: i64, day: &str) -> PlannedRecord {
        PlannedRecord {
            record: Subject::new("test.visit", id),
            date: date(day),
            label: format!("Visit {id}"),
            scope: None,
        }
    }

    #[test]
    fn the_message_has_the_reminder_line_and_the_new_items() {
        let groups = [AffectedGroup {
            guarded: "ticket.resolve".into(),
            records: vec![visit(1, "2026-12-20"), visit(2, "2027-01-10")],
        }];
        let data = message_data(
            "Civil Defence approval",
            "SafeFire LLC",
            date("2026-12-15"),
            30,
            Some(30),
            &groups,
        );
        let def = notification_type();
        let template = def.template_for(NotifyChannel::Email, "en").unwrap();
        assert_eq!(
            notify::fill(template.subject.as_deref().unwrap(), &data),
            "Civil Defence approval of SafeFire LLC expires on 15 Dec 2026"
        );
        let body = notify::fill(&template.body, &data);
        assert!(body.contains("30 day(s) left"), "{body}");
        assert!(
            body.contains("- Visit 1 on 20 Dec 2026 (ticket.resolve)"),
            "{body}"
        );
        assert!(body.contains("- Visit 2 on 10 Jan 2027"), "{body}");
        assert_eq!(data["items"].as_array().unwrap().len(), 2);

        // Items only: no reminder line.
        let items_only = message_data("K", "S", date("2026-12-15"), 29, None, &groups);
        assert!(!items_only["summary"].as_str().unwrap().contains("reminder"));
        assert_eq!(items_only["offset"], Value::Null);
        // Reminder only: no list.
        let plain = message_data("K", "S", date("2026-12-15"), 30, Some(30), &[]);
        assert_eq!(plain["items_text"], "");
    }

    /// Records every request instead of sending it.
    #[derive(Clone, Default)]
    struct Recording(Arc<Mutex<Vec<NotifyRequest>>>);

    impl Notifier for Recording {
        fn notify<'a>(
            &'a self,
            _state: &'a AppState,
            request: NotifyRequest,
        ) -> NotifyFuture<'a, Vec<MessageId>> {
            Box::pin(async move {
                self.0.lock().unwrap().push(request);
                Ok(vec![MessageId(1)])
            })
        }
    }

    struct Recorder(Recording);

    impl Module for Recorder {
        fn name(&self) -> &'static str {
            "run-test-recorder"
        }

        fn register(&self, app: &mut Registry) {
            provide_notifier(app, self.0.clone());
        }
    }

    #[renox::test]
    async fn a_reached_offset_is_sent_once_to_the_responsible_role() {
        let recording = Recording::default();
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(renox::auth::Permissions)
                .module(WlDocuments)
                .module(Recorder(recording.clone())),
        )
        .await;
        let db = app.db();
        define_role(db, "qc_inspector", &[]).await.unwrap();
        let plant1 = Scope::new("plants", 1);
        let user = User::register(db, "Pia", "pia@example.com", "password123")
            .await
            .unwrap();
        user.assign_role_in(db, "qc_inspector", &plant1)
            .await
            .unwrap();
        install_seeds(
            db,
            [&KindSeed::new("run_ecas", "ECAS certificate")
                .expires()
                .reminders(&[90, 30, 7])
                .responsible_role("qc_inspector")],
        )
        .await
        .unwrap();
        let kind = DocumentKind::by_key(db, "run_ecas").await.unwrap().unwrap();
        let mut document = Document {
            kind_id: kind.id,
            subject_type: "run_test.item".into(),
            subject_id: 1,
            status: DocumentStatus::Accepted,
            expiry_date: Some(date("2026-12-15")),
            ..Document::default()
        };
        document.save(db).await.unwrap();

        // 91 days left: nothing yet.
        let report = run(app.state(), date("2026-09-15")).await.unwrap();
        assert_eq!((report.scanned, report.events, report.messages), (1, 0, 0));
        // 30 days left: the 90 offset is passed, the 30 one is sent.
        let report = run(app.state(), date("2026-11-15")).await.unwrap();
        assert_eq!((report.events, report.messages, report.failed), (1, 1, 0));
        assert_eq!(
            ReminderLog::offsets_for(db, document.id).await.unwrap(),
            [30, 90]
        );
        // The second run that night sends nothing.
        let report = run(app.state(), date("2026-11-15")).await.unwrap();
        assert_eq!((report.events, report.messages), (0, 0));

        let sent = recording.0.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].type_key, NOTIFICATION_TYPE);
        assert_eq!(sent[0].data["offset"], 30);
        assert_eq!(sent[0].data["days_left"], 30);
        assert_eq!(sent[0].subject_label.as_deref(), Some("run_test.item #1"));
    }
}
