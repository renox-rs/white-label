#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod kinds;
mod texts;

use renox::db::Migration;
use renox::prelude::*;

pub use kinds::{
    BlockChangeRefused, BlockSeed, DocumentKind, KindBlock, KindSeed, RefusalReason, Sensitivity,
    add_block, blocks_for, register_kind, remove_block, set_block_mode,
};

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

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<DocumentKind>().model::<KindBlock>();
        // The kinds modules and verticals register are installed by this
        // module's command and seeder; both keep the client's changes and
        // lock legal blocks again.
        app.command(
            "documents:install",
            "Store the document kinds the modules register, and lock legal blocks again",
            |_args, state: AppState| async move {
                let created = kinds::install(&state.db).await?;
                println!("{created} document kinds created.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move { kinds::install(&state.db).await.map(|_| ()) });
    }
}
