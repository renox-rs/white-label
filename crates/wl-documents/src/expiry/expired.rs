//! `DOC.RUL.03`: a blocking document has expired (wl-documents §5.2, #49).
//!
//! On the first run after the expiry date ([`due`]) the rule collects the
//! records the document's `blocks` guard ([`collect_blocked`], A2): those
//! already running (`FutureWork::open_for`) and those planned after the
//! expiry date (`FutureWork::planned_from`). Then, per document:
//!
//! 1. one transaction writes the [`ExpiredLog`] row (unique per document, so
//!    the event is never emitted twice and a missed night is caught up) and
//!    the open [`RenewalTask`] row;
//! 2. after the commit, [`DocumentExpired`] is emitted;
//! 3. each recipient gets one `documents.expired` message ([`recipients`],
//!    A5, A6): the responsible role, the kind's `also_notify` roles and, for
//!    a compliance-critical kind, the business owner;
//! 4. the renewal task is opened ([`renewal::open`], A7).
//!
//! A document that fails is reported with `AutomationFailed` and the run
//! moves on (A10). Nothing is written before the transaction, so a provider
//! failure leaves no log row and the next night tries again.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;

use renox::auth::permissions::Scope;
use renox::chrono::{Days, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::serde_json::{Value, json};
use wl_kernel::documents::{
    AffectedGroup, BlockedGroup, DocumentExpired, PlannedRecord, future_work,
};
use wl_kernel::notify::{self, Category, NotificationTypeDef, NotifyChannel, Urgency};
use wl_kernel::{AutomationFailed, COMPANY_ADMIN_ROLE, ClientConfig, Subject};

use super::logs::{ExpiredLog, RenewalTask};
use super::responsible::{self, holders_in};
use super::{renewal, run};
use crate::documents::{Document, DocumentStatus};
use crate::kinds::{DocumentKind, KindBlock};
use crate::texts::english;

/// The rule id.
pub const RULE: &str = "DOC.RUL.03";

/// The notification type of the message.
pub const NOTIFICATION_TYPE: &str = "documents.expired";

/// What the rule did in one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) struct Outcome {
    /// Documents picked up.
    pub scanned: usize,
    /// `DocumentExpired` events emitted.
    pub events: usize,
    /// Messages queued.
    pub messages: usize,
    /// Renewal tasks opened (with or without a ticket).
    pub tasks: usize,
    /// Documents or messages that failed.
    pub failed: usize,
}

/// The notification type `documents.expired`: a reminder that goes out at
/// once.
pub fn notification_type() -> NotificationTypeDef {
    let subject = english("wl_documents.expired.subject", &[]);
    let body = english("wl_documents.expired.body", &[]);
    NotificationTypeDef::new(NOTIFICATION_TYPE, "documents")
        .group("documents")
        .category(Category::Reminder)
        .urgency(Urgency::Urgent)
        .template(NotifyChannel::Email, "en", Some(&subject), &body)
}

/// Registers the notification type with the kernel registry.
pub(crate) fn register_type() {
    notify::register_type(notification_type());
}

/// The accepted documents whose expiry date is before `today` and whose
/// kind has `blocks` or is compliance-critical (A3), oldest id first, with the kind. Documents that
/// already have an [`ExpiredLog`] row are left out by [`handle`].
async fn due(
    state: &AppState,
    kinds: &BTreeMap<i64, DocumentKind>,
    today: NaiveDate,
) -> Result<Vec<(Document, KindBlocks)>> {
    let db = &state.db;
    let documents = Document::query()
        .where_(Document::STATUS.eq(DocumentStatus::Accepted))
        .where_(Document::EXPIRY_DATE.is_not_null())
        .order_by(Document::ID)
        .get(db)
        .await?;
    let mut due = Vec::new();
    for document in documents {
        if document.expiry_date.is_none_or(|expiry| expiry >= today) {
            continue;
        }
        let Some(kind) = kinds.get(&document.kind_id) else {
            continue;
        };
        let blocks = kind.blocks(db).await?;
        // A kind that blocks nothing is still watched when it is
        // compliance-critical: its expiry alerts the owner (06 health card).
        if (blocks.is_empty() && !kind.compliance_critical)
            || ExpiredLog::is_logged(db, document.id).await?
        {
            continue;
        }
        due.push((
            document,
            KindBlocks {
                kind: kind.clone(),
                blocks,
            },
        ));
    }
    Ok(due)
}

/// A kind with its `blocks`.
pub(super) struct KindBlocks {
    kind: DocumentKind,
    blocks: Vec<KindBlock>,
}

/// Runs `DOC.RUL.03` for `today`.
pub(super) async fn run_rule(
    state: &AppState,
    kinds: &BTreeMap<i64, DocumentKind>,
    today: NaiveDate,
) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    for (document, with) in due(state, kinds, today).await? {
        outcome.scanned += 1;
        match handle(state, &document, &with, today).await {
            Ok(done) => {
                outcome.events += usize::from(done.event);
                outcome.messages += done.messages;
                outcome.tasks += usize::from(done.task);
                outcome.failed += done.failed;
            }
            Err(error) => {
                eprintln!(
                    "documents:expiry: {RULE} for document {} failed: {error}",
                    document.id
                );
                outcome.failed += 1;
                fail(state, &document, &with.kind, &error.to_string()).await;
            }
        }
    }
    Ok(outcome)
}

/// Emits `AutomationFailed` for `document` (A10); a failure to emit is
/// printed.
async fn fail(state: &AppState, document: &Document, kind: &DocumentKind, error: &str) {
    let failed = AutomationFailed {
        rule: RULE.to_owned(),
        subject: renewal::document_subject(document.id),
        error: error.to_owned(),
        compliance_critical: kind.compliance_critical,
    };
    if let Err(emit) = state.emit(failed).await {
        eprintln!(
            "documents:expiry: AutomationFailed for document {} not emitted: {emit}",
            document.id
        );
    }
}

/// What one document gave.
#[derive(Default)]
struct Done {
    event: bool,
    messages: usize,
    task: bool,
    failed: usize,
}

/// Handles one expired document.
async fn handle(
    state: &AppState,
    document: &Document,
    with: &KindBlocks,
    today: NaiveDate,
) -> Result<Done> {
    let Some(expires_on) = document.expiry_date else {
        return Ok(Done::default());
    };
    let db = &state.db;
    let kind = &with.kind;
    let subject = document.subject();
    let blocked = collect_blocked(state, kind, &with.blocks, &subject, expires_on).await?;
    let items = affected(&blocked);

    // One transaction: the log row and the task row.
    let mut tx = db.begin().await?;
    if !ExpiredLog::insert_new(&mut tx, document.id, today).await? {
        return Ok(Done::default());
    }
    let task = RenewalTask {
        subject_type: subject.subject_type.clone(),
        subject_id: subject.subject_id,
        kind_id: kind.id,
        expired_document_id: document.id,
        opened_at: now(),
        ..RenewalTask::default()
    }
    .insert_new(&mut tx)
    .await?;
    tx.commit().await?;

    // After the commit: the event, the messages, the task.
    state
        .emit(DocumentExpired {
            document: document.document_id(),
            subject: subject.clone(),
            kind: kind.key.clone(),
            expires_on,
            blocks: with.blocks.iter().map(|b| b.to_entry(kind)).collect(),
            blocked,
        })
        .await?;
    let mut done = Done {
        event: true,
        ..Done::default()
    };

    let label = run::subject_label(state, &with.blocks, &subject).await;
    let recipients = recipients(state, kind, &subject, &items).await?;
    let mut last_error = None;
    for recipient in recipients {
        let data = message_data(kind.name("en"), &label, expires_on, &recipient.groups);
        match run::send_message(state, NOTIFICATION_TYPE, &subject, &label, &recipient, data).await
        {
            Ok(count) => done.messages += count,
            Err(error) => {
                eprintln!(
                    "documents:expiry: message for document {} to user {} failed: {error}",
                    document.id, recipient.user_id
                );
                done.failed += 1;
                last_error = Some(error.to_string());
            }
        }
    }
    if task {
        renewal::open(state, document, kind, &label, expires_on, today, &items).await?;
        done.task = true;
    }
    if let Some(error) = last_error {
        fail(state, document, kind, &error).await;
    }
    Ok(done)
}

/// The records `blocks` guard for the expired document, per guarded
/// transition in entry order (A2): the running ones from
/// `FutureWork::open_for`, the planned ones from `planned_from` (expiry + 1).
/// An entry without a provider, or without records, gives no group.
pub async fn collect_blocked(
    state: &AppState,
    kind: &DocumentKind,
    blocks: &[KindBlock],
    subject: &Subject,
    expires_on: NaiveDate,
) -> Result<Vec<BlockedGroup>> {
    let from = expires_on.checked_add_days(Days::new(1));
    let mut groups: Vec<BlockedGroup> = Vec::new();
    for block in blocks {
        let Some(provider) = future_work(&block.guarded) else {
            continue;
        };
        let entry = block.to_entry(kind);
        let open = provider.open_for(state, &entry, subject).await?;
        let planned: Vec<PlannedRecord> = match from {
            Some(from) => provider
                .planned_from(state, &entry, subject, from)
                .await?
                .into_iter()
                .filter(|record| record.date >= from)
                .collect(),
            None => Vec::new(),
        };
        if open.is_empty() && planned.is_empty() {
            continue;
        }
        groups.push(BlockedGroup {
            guarded: block.guarded.clone(),
            mode: block.mode,
            open,
            planned,
        });
    }
    Ok(groups)
}

/// The blocked records as the resolver's groups: per guarded transition,
/// the running records first, then the planned ones, each record once.
pub fn affected(blocked: &[BlockedGroup]) -> Vec<AffectedGroup> {
    let mut groups: Vec<AffectedGroup> = Vec::new();
    for group in blocked {
        let mut records: Vec<PlannedRecord> = Vec::new();
        for record in group.open.iter().chain(&group.planned) {
            if !records.iter().any(|r| r.record == record.record) {
                records.push(record.clone());
            }
        }
        merge(
            &mut groups,
            &[AffectedGroup {
                guarded: group.guarded.clone(),
                records,
            }],
        );
    }
    groups
}

/// Adds `more` to `groups`, merging by guarded transition and leaving out a
/// record already there.
fn merge(groups: &mut Vec<AffectedGroup>, more: &[AffectedGroup]) {
    for group in more {
        let at = match groups.iter().position(|g| g.guarded == group.guarded) {
            Some(at) => at,
            None => {
                groups.push(AffectedGroup {
                    guarded: group.guarded.clone(),
                    records: Vec::new(),
                });
                groups.len() - 1
            }
        };
        for record in &group.records {
            if !groups[at].records.iter().any(|r| r.record == record.record) {
                groups[at].records.push(record.clone());
            }
        }
    }
}

/// The business owner role of this deployment: `[roles] owner`, else the
/// company administrators (A6).
fn owner_role(state: &AppState) -> String {
    state
        .provided::<Arc<ClientConfig>>()
        .map(|config| config.owner_role().to_owned())
        .unwrap_or_else(|| COMPANY_ADMIN_ROLE.to_owned())
}

/// Who is told, one entry per user (A5): the holders of the kind's
/// responsible role and of its `also_notify` roles, each with the records
/// they may see; and for a compliance-critical kind the business owner's
/// global holders, with all of them. Ordered by user id.
pub async fn recipients(
    state: &AppState,
    kind: &DocumentKind,
    subject: &Subject,
    items: &[AffectedGroup],
) -> Result<Vec<responsible::Recipient>> {
    let mut roles = vec![responsible::role_for(kind.responsible_role.as_deref()).to_owned()];
    for role in &kind.also_notify.0 {
        if !roles.contains(role) {
            roles.push(role.clone());
        }
    }
    let mut users: BTreeMap<i64, Vec<AffectedGroup>> = BTreeMap::new();
    for role in &roles {
        for recipient in responsible::resolve(state, role, &kind.key, subject, items).await? {
            merge(
                users.entry(recipient.user_id).or_default(),
                &recipient.groups,
            );
        }
    }
    if kind.compliance_critical {
        let owner = owner_role(state);
        for user_id in holders_in(&state.db, &owner, &Scope::global()).await? {
            // The owner sees everything.
            users.insert(user_id, items.to_vec());
        }
    }
    Ok(users
        .into_iter()
        .map(|(user_id, groups)| responsible::Recipient { user_id, groups })
        .collect())
}

/// `date` as `15 Dec 2026`.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The template variables of the message to one recipient.
pub fn message_data(
    kind_name: &str,
    subject_label: &str,
    expires_on: NaiveDate,
    groups: &[AffectedGroup],
) -> Value {
    let expires = date_text(expires_on);
    let params: [(&str, &dyn Display); 3] = [
        ("kind", &kind_name),
        ("subject", &subject_label),
        ("expires", &expires),
    ];
    let mut lines = Vec::new();
    let mut items = Vec::new();
    for group in groups {
        for record in &group.records {
            let date = date_text(record.date);
            lines.push(english(
                "wl_documents.expired.item",
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
    let mut summary = vec![english("wl_documents.expired.message", &params)];
    if !items.is_empty() {
        summary.push(english("wl_documents.expired.items", &params));
    }
    json!({
        "kind": kind_name,
        "kind_name": kind_name,
        "subject_label": subject_label,
        "expires_on": expires,
        "summary": summary.join(" "),
        "items_text": lines.join("\n"),
        "items": items,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use renox::auth::permissions::define_role;
    use renox::testing::TestApp;
    use wl_kernel::BlockMode;
    use wl_kernel::documents::{BlockEntry, DocumentFuture, FutureWork, register_future_work};
    use wl_kernel::notify::{
        MessageId, Notifier, NotifyFuture, NotifyRecipient, NotifyRequest, provide_notifier,
    };
    use wl_kernel::tickets::{
        Assignee, OpenTicket, TaskCreator, TicketFuture, TicketId, TicketRef, provide_task_creator,
    };

    use super::*;
    use crate::WlDocuments;
    use crate::kinds::{KindSeed, install_seeds};

    fn date(text: &str) -> NaiveDate {
        text.parse().expect("a date")
    }

    fn record(id: i64, day: &str) -> PlannedRecord {
        PlannedRecord {
            record: Subject::new("expd_test.deployment", id),
            date: date(day),
            label: format!("Deployment {id}"),
            scope: None,
        }
    }

    #[test]
    fn the_type_is_an_urgent_reminder_with_an_english_text() {
        let def = notification_type();
        assert_eq!(def.key, "documents.expired");
        assert_eq!(def.category, Category::Reminder);
        assert_eq!(def.urgency, Urgency::Urgent);
        let template = def.template_for(NotifyChannel::Email, "en").unwrap();
        assert!(template.subject.as_deref().unwrap().contains("expired on"));
    }

    #[test]
    fn the_blocked_records_are_listed_once_per_transition() {
        let blocked = [
            BlockedGroup {
                guarded: "expd_test.activate".into(),
                mode: wl_kernel::BlockMode::Block,
                open: vec![record(1, "2026-11-01")],
                planned: vec![record(1, "2026-11-01"), record(2, "2026-12-10")],
            },
            BlockedGroup {
                guarded: "expd_test.resume".into(),
                mode: wl_kernel::BlockMode::Flag,
                open: Vec::new(),
                planned: vec![record(1, "2026-12-12")],
            },
        ];
        let groups = affected(&blocked);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].guarded, "expd_test.activate");
        assert_eq!(groups[0].records.len(), 2);
        assert_eq!(groups[1].records.len(), 1);
    }

    #[test]
    fn the_message_names_the_kind_the_date_and_the_records() {
        let groups = [AffectedGroup {
            guarded: "expd_test.activate".into(),
            records: vec![record(2, "2026-12-10")],
        }];
        let data = message_data("Work permit", "Ravi S.", date("2026-11-30"), &groups);
        let def = notification_type();
        let template = def.template_for(NotifyChannel::Email, "en").unwrap();
        assert_eq!(
            notify::fill(template.subject.as_deref().unwrap(), &data),
            "Work permit of Ravi S. expired on 30 Nov 2026"
        );
        let body = notify::fill(&template.body, &data);
        assert!(body.contains("expired on 30 Nov 2026"), "{body}");
        assert!(
            body.contains("- Deployment 2 on 10 Dec 2026 (expd_test.activate)"),
            "{body}"
        );
        let plain = message_data("Work permit", "Ravi S.", date("2026-11-30"), &[]);
        assert_eq!(plain["items_text"], "");
    }

    struct Deployments {
        fail: bool,
    }

    impl FutureWork for Deployments {
        fn planned_from<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            _subject: &'a Subject,
            _from: NaiveDate,
        ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(Error::Status(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "no plan".into(),
                    ));
                }
                // The second record is not dated after the expiry date.
                Ok(vec![record(2, "2026-12-10"), record(3, "2026-11-30")])
            })
        }

        fn open_for<'a>(
            &'a self,
            _state: &'a AppState,
            _entry: &'a BlockEntry,
            _subject: &'a Subject,
        ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
            Box::pin(async { Ok(vec![record(1, "2026-11-01")]) })
        }
    }

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

    #[derive(Clone, Default)]
    struct Tasks(Arc<Mutex<Vec<OpenTicket>>>);

    impl TaskCreator for Tasks {
        fn open<'a>(
            &'a self,
            _state: &'a AppState,
            ticket: OpenTicket,
        ) -> TicketFuture<'a, Option<TicketRef>> {
            Box::pin(async move {
                let mut opened = self.0.lock().unwrap();
                opened.push(ticket);
                Ok(Some(TicketRef {
                    id: TicketId(opened.len() as i64),
                    number: format!("TKT-2026-{:04}", opened.len()),
                }))
            })
        }
    }

    struct Recorders(Recording, Tasks);

    impl Module for Recorders {
        fn name(&self) -> &'static str {
            "expired-test-recorders"
        }

        fn register(&self, app: &mut Registry) {
            provide_notifier(app, self.0.clone());
            provide_task_creator(app, self.1.clone());
        }
    }

    async fn staff(app: &TestApp, name: &str, role: &str) -> i64 {
        let email = format!("{}@example.com", name.to_lowercase());
        let user = User::register(app.db(), name, &email, "password123")
            .await
            .unwrap();
        user.assign_role(app.db(), role).await.unwrap();
        user.id
    }

    async fn accepted(db: &Db, key: &str, subject_id: i64) -> Document {
        let kind = DocumentKind::by_key(db, key).await.unwrap().unwrap();
        let mut document = Document {
            kind_id: kind.id,
            subject_type: "expd_test.worker".into(),
            subject_id,
            status: DocumentStatus::Accepted,
            expiry_date: Some(date("2026-11-30")),
            ..Document::default()
        };
        document.save(db).await.unwrap();
        document
    }

    #[renox::test]
    async fn an_expired_permit_is_reported_once_with_its_records() {
        register_future_work("expd_test.activate", Deployments { fail: false });
        let recording = Recording::default();
        let tasks = Tasks::default();
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(renox::auth::Permissions)
                .module(WlDocuments)
                .module(Recorders(recording.clone(), tasks.clone())),
        )
        .await;
        let db = app.db();
        for role in ["expd_pro", "expd_hr", COMPANY_ADMIN_ROLE] {
            define_role(db, role, &[]).await.unwrap();
        }
        let pro = staff(&app, "Pia", "expd_pro").await;
        let hr = staff(&app, "Hal", "expd_hr").await;
        let admin = staff(&app, "Ada", COMPANY_ADMIN_ROLE).await;
        install_seeds(
            db,
            [
                &KindSeed::new("expd_permit", "Work permit")
                    .expires()
                    .legal_block()
                    .responsible_role("expd_pro")
                    .also_notify(&["expd_hr"])
                    .blocks("expd_test.activate", BlockMode::Block),
                &KindSeed::new("expd_card", "Site card")
                    .expires()
                    .responsible_role("expd_pro")
                    .blocks("expd_test.activate", BlockMode::Flag),
            ],
        )
        .await
        .unwrap();
        let permit = accepted(db, "expd_permit", 7).await;
        accepted(db, "expd_card", 8).await;

        // The expiry date itself: still valid.
        let report = run::run(app.state(), date("2026-11-30")).await.unwrap();
        assert_eq!((report.expired, report.expired_events), (0, 0));

        let report = run::run(app.state(), date("2026-12-01")).await.unwrap();
        assert_eq!(
            (report.expired, report.expired_events, report.tasks),
            (2, 2, 2)
        );
        assert_eq!(report.failed, 0);
        assert!(ExpiredLog::is_logged(db, permit.id).await.unwrap());

        {
            // The night of the expiry date sent the look-ahead message.
            let sent: Vec<NotifyRequest> = recording
                .0
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.type_key == NOTIFICATION_TYPE)
                .cloned()
                .collect();
            let mut to_permit: Vec<i64> = Vec::new();
            for request in sent.iter().filter(|r| r.subject.subject_id == 7) {
                assert_eq!(request.type_key, NOTIFICATION_TYPE);
                assert_eq!(request.data["items"].as_array().unwrap().len(), 2);
                for recipient in &request.recipients {
                    match recipient {
                        NotifyRecipient::User { id } => to_permit.push(*id),
                        other => panic!("unexpected recipient {other:?}"),
                    }
                }
            }
            to_permit.sort_unstable();
            let mut expected = vec![pro, hr, admin];
            expected.sort_unstable();
            assert_eq!(to_permit, expected);
            // The non-critical card tells neither HR nor the owner.
            assert_eq!(sent.iter().filter(|r| r.subject.subject_id == 8).count(), 1);
        }

        {
            let opened = tasks.0.lock().unwrap();
            assert_eq!(opened.len(), 2);
            assert_eq!(
                opened[0].source_key.as_deref(),
                Some("documents.renewal:expd_test.worker:7:expd_permit")
            );
            assert_eq!(opened[0].title, "Renew Work permit of expd_test.worker #7");
            assert_eq!(opened[0].assignee, Some(Assignee::user(pro)));
        }
        let row = RenewalTask::query().first(db).await.unwrap().unwrap();
        assert_eq!(row.ticket_number.as_deref(), Some("TKT-2026-0001"));

        // A second run, and a later night, add nothing.
        for night in ["2026-12-01", "2026-12-02"] {
            let report = run::run(app.state(), date(night)).await.unwrap();
            assert_eq!(
                (report.expired_events, report.messages, report.tasks),
                (0, 0, 0)
            );
        }
        assert_eq!(tasks.0.lock().unwrap().len(), 2);
    }

    #[renox::test]
    async fn a_failing_provider_reports_the_failure_and_the_next_night_retries() {
        register_future_work("expd_fail.activate", Deployments { fail: true });
        let app = TestApp::new(
            App::new()
                .module(renox::auth::Auth::new())
                .module(renox::auth::Permissions)
                .module(WlDocuments),
        )
        .await;
        let db = app.db();
        install_seeds(
            db,
            [&KindSeed::new("expd_fail_permit", "Work permit")
                .expires()
                .legal_block()
                .blocks("expd_fail.activate", BlockMode::Block)],
        )
        .await
        .unwrap();
        let document = accepted(db, "expd_fail_permit", 1).await;

        let report = run::run(app.state(), date("2026-12-01")).await.unwrap();
        assert_eq!(
            (report.expired, report.expired_events, report.failed),
            (1, 0, 1)
        );
        assert!(!ExpiredLog::is_logged(db, document.id).await.unwrap());
        assert!(RenewalTask::query().first(db).await.unwrap().is_none());
    }
}
