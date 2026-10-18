#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

mod texts;

use renox::prelude::*;

/// The module's permissions (wl-documents §6).
pub mod permissions {
    /// See documents of records in scope (non-sensitive classes).
    pub const VIEW: &str = "documents.view";
    /// Attach documents (accepted directly for kinds without review).
    pub const UPLOAD: &str = "documents.upload";
    /// Accept or reject documents in `pending_review`.
    pub const REVIEW: &str = "documents.review";
    /// Archive documents, edit document fields, manage holds.
    pub const MANAGE: &str = "documents.manage";
    /// Edit the client's document kinds, their blocks and reminder offsets.
    pub const KINDS_MANAGE: &str = "document_kinds.manage";
}

/// The documents module. The host adds it with
/// `.module(wl_documents::WlDocuments)` when `client.toml` enables `documents`.
pub struct WlDocuments;

impl Module for WlDocuments {
    fn name(&self) -> &'static str {
        "wl-documents"
    }
}
