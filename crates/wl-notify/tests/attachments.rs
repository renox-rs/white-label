//! BL-NTF-10 (#464): stored documents sent as e-mail attachments through the
//! kernel notifier: logged on the message, read at send time (so a message
//! held by quiet hours sends the stored version), replaced by a signed link
//! above 10 MB, never on SMS, and nothing for an opted-out recipient.

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::document_store::{
    DocumentStore, StoreFuture, StoredFile, WithAttachments, attachments_in,
};
use wl_kernel::documents::DocumentId;
use wl_kernel::notify::{self, NotificationTypeDef, NotifyChannel, NotifyRecipient, NotifyRequest};
use wl_notify::send::ATTACHMENT_MISSING;
use wl_notify::{MessageStatus, SuppressionReason, release, suppressions};

use support::{Booted, MONA, SARA, SARA_EMAIL, SARA_MOBILE, boot_providers, dubai};

/// The type of the owner statement notice.
const PUBLISHED: &str = "owner_statement.published";
/// The statement PDF of 180 KB.
const STATEMENT_PDF: DocumentId = DocumentId(501);
/// A scan of 12 MB.
const LARGE_SCAN: DocumentId = DocumentId(502);
/// A document whose file is gone.
const GONE: DocumentId = DocumentId(503);
/// The name the owner sees.
const FILE_NAME: &str = "Owner statement 2026-10.pdf";

fn register_type() {
    notify::register_type(
        NotificationTypeDef::new(PUBLISHED, "property")
            .channels(&[NotifyChannel::Email, NotifyChannel::Sms])
            .template(
                NotifyChannel::Email,
                "en",
                Some("Your owner statement for {{ period }}"),
                "Your statement for {{ period }} is attached.",
            )
            .template(
                NotifyChannel::Email,
                "ar",
                Some("كشف حساب المالك {{ period }}"),
                "مرفق كشف حسابك عن {{ period }}.",
            )
            .template(
                NotifyChannel::Sms,
                "en",
                None,
                "Your statement for {{ period }} is ready.",
            )
            .template(
                NotifyChannel::Sms,
                "ar",
                None,
                "كشف حسابك عن {{ period }} جاهز.",
            ),
    );
}

/// The stored files the test store knows, by document.
#[derive(Clone, Default)]
struct TestStore(Arc<Mutex<BTreeMap<i64, StoredFile>>>);

impl TestStore {
    fn put(&self, document: DocumentId, size: usize) {
        self.0.lock().unwrap().insert(
            document.0,
            StoredFile {
                document,
                subject: Subject::new("property.owner_statement", 3),
                file_name: format!("stored-{}.pdf", document.0),
                mime_type: "application/pdf".into(),
                bytes: vec![b'%'; size],
            },
        );
    }
}

impl DocumentStore for TestStore {
    fn read<'a>(
        &'a self,
        _state: &'a AppState,
        document: DocumentId,
    ) -> StoreFuture<'a, Option<StoredFile>> {
        let found = self.0.lock().unwrap().get(&document.0).cloned();
        Box::pin(async move { Ok(found) })
    }

    fn download_link(
        &self,
        _state: &AppState,
        document: DocumentId,
        _ttl: Duration,
    ) -> Result<Option<String>> {
        Ok(Some(format!(
            "https://acme.example/documents/signed/{document}/file?signature=abc"
        )))
    }
}

async fn boot() -> (Booted, TestStore) {
    register_type();
    let store = TestStore::default();
    store.put(STATEMENT_PDF, 180 * 1024);
    store.put(LARGE_SCAN, 12 * 1024 * 1024);
    let provided = store.clone();
    let t = boot_providers(None, |m| m, move |app| {
        let store: Arc<dyn DocumentStore> = Arc::new(provided);
        app.provide(store)
    })
    .await;
    (t, store)
}

async fn request(t: &Booted, to: NotifyRecipient, document: DocumentId) -> Vec<notify::MessageId> {
    let request = NotifyRequest::new(PUBLISHED, Subject::new("property.owner_statement", 3))
        .to(to)
        .data(json!({ "period": "2026-10" }))
        .attach_document(document, FILE_NAME);
    let state = t.app.state();
    t.app
        .at_travelled_time(notify::notifier(state).notify(state, request))
        .await
        .expect("the notifier queues the message")
}

/// Done when 1: one e-mail with the file named "Owner statement
/// 2026-10.pdf", and the message log lists it.
#[renox::test]
async fn a_stored_pdf_is_sent_as_an_attachment_and_logged() {
    let (t, _store) = boot().await;
    let ids = request(&t, NotifyRecipient::Party { id: SARA }, STATEMENT_PDF).await;
    t.run_jobs().await;

    let message = t.message(ids[0]).await;
    assert_eq!(message.channel, NotifyChannel::Email);
    assert_eq!(message.status, MessageStatus::Sent);
    assert_eq!(message.attachment_names.as_deref(), Some(FILE_NAME));
    assert_eq!(attachments_in(&message.data.0).len(), 1, "the reference is stored");

    let sent = t.email.sent();
    assert_eq!(sent.len(), 1, "one e-mail");
    assert_eq!(sent[0].address, SARA_EMAIL);
    assert_eq!(sent[0].attachments.len(), 1);
    assert_eq!(sent[0].attachments[0].file_name, FILE_NAME);
    assert_eq!(sent[0].attachments[0].content_type, "application/pdf");
    assert_eq!(sent[0].attachments[0].bytes.len(), 180 * 1024);
    assert!(!sent[0].body.contains("https://"), "no link below 10 MB");

    // The log lists it.
    let admin = t.user("Lina", &[wl_notify::permissions::LOG_VIEW]).await;
    t.app.acting_as(&admin);
    t.app
        .get("/notification-log")
        .await
        .assert_ok()
        .assert_see(FILE_NAME);
}

/// Done when 2: in quiet hours the message waits, and the end of quiet
/// hours sends the attachment (read then, from the store).
#[renox::test]
async fn in_quiet_hours_the_attachment_is_sent_when_they_end() {
    let (t, store) = boot().await;
    t.clock.set("2026-10-19 22:15");
    let ids = request(&t, NotifyRecipient::Party { id: SARA }, STATEMENT_PDF).await;
    t.run_jobs().await;
    let held = t.message(ids[0]).await;
    assert_eq!(held.status, MessageStatus::Deferred);
    assert!(t.email.sent().is_empty());

    // The file stored now is the one sent.
    store.put(STATEMENT_PDF, 181 * 1024);
    t.clock.set("2026-10-20 08:00");
    let released = release::run_due(t.app.state(), t.clock.now()).await.unwrap();
    assert_eq!(released, 1);
    t.run_jobs().await;
    assert_eq!(t.message(ids[0]).await.status, MessageStatus::Sent);
    let sent = t.email.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].attachments[0].file_name, FILE_NAME);
    assert_eq!(sent[0].attachments[0].bytes.len(), 181 * 1024);
    assert_eq!(held.scheduled_for, Some(dubai("2026-10-20 08:00")));
}

/// Done when 3: an attachment of 12 MB is replaced by a signed link, and the
/// message says so.
#[renox::test]
async fn an_attachment_over_10_mb_is_replaced_by_a_signed_link() {
    let (t, _store) = boot().await;
    let ids = request(&t, NotifyRecipient::Party { id: SARA }, LARGE_SCAN).await;
    t.run_jobs().await;
    let sent = t.email.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].attachments.is_empty(), "no file over 10 MB");
    assert!(sent[0].body.contains("too large to attach"), "{}", sent[0].body);
    assert!(
        sent[0]
            .body
            .contains(&format!("{FILE_NAME}: https://acme.example/documents/signed/{}/file", LARGE_SCAN.0)),
        "{}",
        sent[0].body
    );
    let message = t.message(ids[0]).await;
    assert_eq!(message.status, MessageStatus::Sent);
    assert_eq!(
        message.attachment_names.as_deref(),
        Some("Owner statement 2026-10.pdf (link)")
    );
    assert!(message.rendered_body.contains("within 30 days"));
}

/// Done when 4: an opted-out recipient gets nothing, as without an
/// attachment.
#[renox::test]
async fn an_opted_out_recipient_gets_nothing() {
    let (t, _store) = boot().await;
    let mut tx = t.db().begin().await.unwrap();
    for (channel, address) in [
        (NotifyChannel::Email, SARA_EMAIL),
        (NotifyChannel::Sms, SARA_MOBILE),
    ] {
        suppressions::add(&mut tx, channel, address, SuppressionReason::OptOut, None)
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();

    let ids = request(&t, NotifyRecipient::Party { id: SARA }, STATEMENT_PDF).await;
    t.run_jobs().await;
    assert!(t.email.sent().is_empty(), "no e-mail");
    assert!(t.sms.sent().is_empty(), "no SMS");
    for id in ids {
        assert_eq!(t.message(id).await.status, MessageStatus::Suppressed);
    }
}

/// Done when 5: an SMS of the same type carries no file.
#[renox::test]
async fn an_sms_of_the_same_type_carries_no_file() {
    let (t, _store) = boot().await;
    let ids = request(&t, NotifyRecipient::Party { id: MONA }, STATEMENT_PDF).await;
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.channel, NotifyChannel::Sms);
    assert_eq!(message.status, MessageStatus::Sent);
    assert!(message.attachments().is_empty());
    assert_eq!(message.attachment_names, None);
    let sent = t.sms.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].attachments.is_empty());
    assert!(t.email.sent().is_empty());
}

/// A document that cannot be read at send time fails the message with
/// *attachment missing*.
#[renox::test]
async fn a_missing_document_fails_the_message() {
    let (t, _store) = boot().await;
    let ids = request(&t, NotifyRecipient::Party { id: SARA }, GONE).await;
    t.run_jobs().await;
    let message = t.message(ids[0]).await;
    assert_eq!(message.last_error_code.as_deref(), Some(ATTACHMENT_MISSING));
    assert!(t.history(ids[0]).await.contains(&"fail".to_owned()));
    assert!(t.email.sent().is_empty(), "no e-mail without its file");
}
