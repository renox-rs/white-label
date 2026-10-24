//! Reading stored documents from another module, and documents attached to
//! a notification (wl-notify §3 *Message*, §9 "Email with templates and
//! attachments"; wl-documents §6).
//!
//! - [`DocumentStore`] reads the current file of a stored document and signs
//!   a download link for it. `wl-documents` provides it with
//!   [`provide_document_store`]; without the module, [`document_store`]
//!   gives [`NoDocumentStore`], which knows no document.
//! - A [`NotifyRequest`] carries [`NotifyAttachment`]s (a document id and the
//!   file name to show) through [`WithAttachments::attach_document`]. They
//!   travel in the request's data under [`ATTACHMENTS_KEY`], so the message
//!   stores them with the rest of its data and the file is read at send
//!   time: a message deferred by quiet hours sends the stored version.
//!   [`attachments_in`] reads them back. Only e-mail carries files; an SMS or
//!   WhatsApp message of the same request carries none.
//!
//! The caller passes only documents of the request's subject: the notifier
//! does not check what the recipient's party may see.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use renox::prelude::*;
use renox::serde_json::Value;
use serde::{Deserialize, Serialize};

use crate::documents::DocumentId;
use crate::notify::NotifyRequest;
use crate::subject::Subject;

/// A boxed future of the document store.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// The key of the request data that holds the attachments.
pub const ATTACHMENTS_KEY: &str = "_attachments";

/// Above this total size (10 MB) the e-mail carries a signed download link
/// instead of the files.
pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// How long the download link of a file too large to attach works: 30 days.
pub const ATTACHMENT_LINK_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// The current file of a stored document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// The document.
    pub document: DocumentId,
    /// The record it is attached to.
    pub subject: Subject,
    /// The file name it was stored with.
    pub file_name: String,
    /// The MIME type.
    pub mime_type: String,
    /// The content.
    pub bytes: Vec<u8>,
}

/// Reads stored documents (`wl-documents`).
pub trait DocumentStore: Send + Sync + 'static {
    /// The current file of `document`, or `None` when the document or its
    /// file is gone.
    fn read<'a>(
        &'a self,
        state: &'a AppState,
        document: DocumentId,
    ) -> StoreFuture<'a, Option<StoredFile>>;

    /// A signed link that downloads the current file of `document` without
    /// a login, valid for `ttl`; `None` when links are not available.
    fn download_link(
        &self,
        state: &AppState,
        document: DocumentId,
        ttl: Duration,
    ) -> Result<Option<String>>;
}

/// The store when the `documents` module is off: knows no document.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoDocumentStore;

impl DocumentStore for NoDocumentStore {
    fn read<'a>(
        &'a self,
        _state: &'a AppState,
        _document: DocumentId,
    ) -> StoreFuture<'a, Option<StoredFile>> {
        Box::pin(async { Ok(None) })
    }

    fn download_link(
        &self,
        _state: &AppState,
        _document: DocumentId,
        _ttl: Duration,
    ) -> Result<Option<String>> {
        Ok(None)
    }
}

/// Makes `store` the app's [`DocumentStore`]; `wl-documents` calls it in
/// `register`.
pub fn provide_document_store(app: &mut Registry, store: impl DocumentStore) {
    let store: Arc<dyn DocumentStore> = Arc::new(store);
    app.provide(store);
}

/// The app's [`DocumentStore`], or [`NoDocumentStore`].
pub fn document_store(state: &AppState) -> Arc<dyn DocumentStore> {
    match state.provided::<Arc<dyn DocumentStore>>() {
        Some(store) => Arc::clone(&store),
        None => Arc::new(NoDocumentStore),
    }
}

/// A stored document attached to a notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyAttachment {
    /// The document whose current file is sent.
    pub document: DocumentId,
    /// The file name the recipient sees, e.g. `Owner statement 2026-10.pdf`.
    pub file_name: String,
}

/// Attaching documents to a [`NotifyRequest`].
pub trait WithAttachments: Sized {
    /// Also attaches the current file of `document`, named `file_name`.
    #[must_use]
    fn attach_document(self, document: DocumentId, file_name: &str) -> Self;
}

impl WithAttachments for NotifyRequest {
    fn attach_document(mut self, document: DocumentId, file_name: &str) -> Self {
        let mut list = attachments_in(&self.data);
        list.push(NotifyAttachment {
            document,
            file_name: file_name.to_owned(),
        });
        if !self.data.is_object() {
            self.data = Value::Object(Default::default());
        }
        if let Value::Object(map) = &mut self.data {
            map.insert(
                ATTACHMENTS_KEY.to_owned(),
                renox::serde_json::to_value(&list).unwrap_or(Value::Null),
            );
        }
        self
    }
}

/// The attachments a request's (or a message's) data carries.
pub fn attachments_in(data: &Value) -> Vec<NotifyAttachment> {
    data.get(ATTACHMENTS_KEY)
        .and_then(|list| renox::serde_json::from_value(list.clone()).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_travel_in_the_data_with_the_template_variables() {
        let request = NotifyRequest::new(
            "owner_statement.published",
            Subject::new("property.owner_statement", 3),
        )
        .data(json!({ "period": "2026-10" }))
        .attach_document(DocumentId(7), "Owner statement 2026-10.pdf");
        assert_eq!(request.data["period"], "2026-10");
        assert_eq!(
            attachments_in(&request.data),
            vec![NotifyAttachment {
                document: DocumentId(7),
                file_name: "Owner statement 2026-10.pdf".into(),
            }]
        );
        assert!(attachments_in(&json!({})).is_empty());
    }
}
