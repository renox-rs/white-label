//! The kernel's [`DocumentStore`] (wl-documents §6): other modules read the
//! current file of a document (wl-notify attaches it to an e-mail) and sign
//! a download link for it, without depending on this crate.
//!
//! The signed link opens `GET /documents/signed/{id}/file` without a login;
//! the signature (with its expiry) is the credential, as for the portal's
//! signed links.

use std::time::Duration;

use renox::Download;
use renox::prelude::*;
use renox::signed::ValidSignature;
use wl_kernel::document_store::{DocumentStore, StoreFuture, StoredFile};
use wl_kernel::documents::DocumentId;

use wl_kernel::Subject;

use crate::documents::{Document, Documents, NewDocument};

/// The route name of the signed download.
pub const SIGNED_FILE_ROUTE: &str = "wl_documents.signed_file";

/// Reads documents for the kernel contract.
#[derive(Debug, Clone, Copy, Default)]
pub struct StoredDocuments;

impl DocumentStore for StoredDocuments {
    fn read<'a>(
        &'a self,
        state: &'a AppState,
        document: DocumentId,
    ) -> StoreFuture<'a, Option<StoredFile>> {
        Box::pin(async move {
            let Some(row) = Document::find(&state.db, document.0).await? else {
                return Ok(None);
            };
            let Some(version) = row.current_file(&state.db).await? else {
                return Ok(None);
            };
            let Some(bytes) = version.read(state).await? else {
                return Ok(None);
            };
            Ok(Some(StoredFile {
                document,
                subject: row.subject(),
                file_name: version.file_name,
                mime_type: version.mime_type,
                bytes: bytes.to_vec(),
            }))
        })
    }

    fn download_link(
        &self,
        state: &AppState,
        document: DocumentId,
        ttl: Duration,
    ) -> Result<Option<String>> {
        state
            .signed_url(SIGNED_FILE_ROUTE, &[&document.0], ttl)
            .map(Some)
    }

    fn attach<'a>(
        &'a self,
        state: &'a AppState,
        subject: Subject,
        kind: &'a str,
        file: Upload,
        channel: &'a str,
    ) -> StoreFuture<'a, Option<DocumentId>> {
        Box::pin(async move {
            let new = NewDocument::new(subject, kind, file).channel(channel);
            match Documents::attach(state, None, new).await? {
                Ok(document) => Ok(Some(document.document_id())),
                Err(refused) => Err(refused.into_error()),
            }
        })
    }

    fn documents_of<'a>(
        &'a self,
        state: &'a AppState,
        subject: Subject,
    ) -> StoreFuture<'a, Vec<(DocumentId, String)>> {
        Box::pin(async move {
            Ok(Documents::of_subject(&state.db, &subject)
                .await?
                .into_iter()
                .map(|d| (d.document_id(), d.title))
                .collect())
        })
    }
}

/// The signed download route (public: the signature is the credential).
pub fn routes() -> Routes {
    Routes::new()
        .get("/documents/signed/{id}/file", signed_file)
        .name(SIGNED_FILE_ROUTE)
}

/// `GET /documents/signed/{id}/file`: the current file, as a download, when
/// the signature is valid and not expired (403 otherwise).
async fn signed_file(
    _signature: ValidSignature,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Download> {
    let document = Document::find(&state.db, id)
        .await?
        .ok_or(Error::NotFound)?;
    let version = document
        .current_file(&state.db)
        .await?
        .ok_or(Error::NotFound)?;
    let bytes = version.read(&state).await?.ok_or(Error::NotFound)?;
    Ok(Download::bytes(
        version.file_name,
        version.mime_type,
        bytes.to_vec(),
    ))
}
