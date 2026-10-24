//! The search index (wl-kernel §5.8, `PLT.SRC.01`, BL-AGT-01, decision 0006):
//! one search document per findable record of every module, kept in two
//! tables so a search covers every module in one query.
//!
//! A module declares each findable entity once in its `register()`:
//!
//! ```ignore
//! Searchable::of::<Invoice>("invoicing.invoice")
//!     .permission("invoices.view")
//!     .portal_visible()
//!     .document(|invoice: &Invoice| {
//!         SearchDocument::new(&invoice.number)
//!             .secondary(&invoice.customer_name)
//!             .identifier(kinds::NUMBER, &invoice.number)
//!             .status(invoice.status.to_string())
//!             .legal_entity(invoice.legal_entity_id)
//!     })
//!     .register();
//! ```
//!
//! and keeps its documents current with [`index_in`] in the transaction that
//! changes the record (create, change, import), so a rolled back change
//! leaves no document. Transitions the kernel runs by name
//! ([`crate::SubjectMachine::run_in`]: approvals, `back_to_*`, `*_approved`)
//! and voids re-index the record themselves. [`rebuild`] (the command
//! `search:rebuild [--type <subject_type>]`) builds every document again from
//! the registered types; an import is undone with [`undo_import`], which
//! removes the documents of the rows it deletes.
//!
//! **Sensitive values** (`personal`, `health`, `financial` classes,
//! SECURITY.md §2) never reach the full-text columns: an identifier of those
//! classes is stored only as an HMAC of its normalised value under
//! `APP_KEY` and matches only on the exact value, and any word of the title,
//! secondary text or keywords equal to such a value is left out. Changing
//! `APP_KEY` changes the hashes: run `search:rebuild` after a rotation.
//!
//! The full-text index itself is Renox's (`#[model(search = …)]`): FTS5 on
//! SQLite, a `tsvector` column on PostgreSQL, over a copy of the text with
//! Arabic marks and spelling variants normalised (R22). The `forms` column
//! holds each word's Arabic–Latin match key (`renox::text::match_key`), so
//! "Mohammed" and "محمد" can find each other.

mod document;
mod index;
mod registry;

use renox::db::{DateTime, Migration};
use renox::prelude::*;
use serde::Serialize;

pub use document::{Identifier, SearchDocument, Sensitivity, kinds, normalize_identifier};
pub use index::{
    IdentifierMatch, document_of, forget_import_change, identifiers_of, index_in,
    lookup_identifier, rebuild, rebuild_type, remove_in, undo_import,
};
pub use registry::{Searchable, SearchableBuilder, registered_types};

/// The command that builds the index again.
pub const REBUILD_COMMAND: &str = "search:rebuild";

/// The name of the migration that adds the full-text index to
/// `wl_search_documents`; it runs after the one creating the table.
pub const INDEX_MIGRATION: &str = "20261101000100_index_wl_search_documents";

/// One record's searchable view, as stored. Written only through
/// [`index_in`], [`remove_in`] and [`rebuild`].
#[derive(Debug, Clone, Default, Serialize, Model)]
#[model(
    table = "wl_search_documents",
    unique(subject_type, subject_id),
    index(scope_type, scope_id),
    search = "title, secondary, keywords, forms",
    normalize = "arabic",
    search_language = "simple"
)]
pub struct SearchDocumentRow {
    /// The primary key.
    pub id: i64,
    /// The record's type, e.g. `invoicing.invoice`.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// What the record is called in a result, e.g. `INV-PGP-2026-0004`.
    pub title: String,
    /// The line under the title, e.g. the customer's name.
    pub secondary: String,
    /// More words the record is found by, separated by spaces.
    pub keywords: String,
    /// The Arabic–Latin match keys of the words above.
    pub forms: String,
    /// The record's status, shown as a badge.
    pub status: Option<String>,
    /// Whether the record was voided; it stays findable by its number.
    pub voided: bool,
    /// The permission a user needs to see the record's type.
    pub permission: String,
    /// Whether external users may find it in the portal.
    pub portal_visible: bool,
    /// The record's scope kind (a table), `""` when global.
    pub scope_type: String,
    /// The record's scope key, `""` when global.
    pub scope_id: String,
    /// The legal entity it belongs to, if any.
    pub legal_entity_id: Option<i64>,
    /// The record's data class: `normal`, `personal`, `health` or `financial`.
    pub sensitivity: String,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the document was last written.
    pub updated_at: Option<DateTime>,
}

/// A value that matches a record exactly, as stored.
#[derive(Debug, Clone, Default, Serialize, Model)]
#[model(
    table = "wl_search_identifiers",
    unique(kind, value, subject_type, subject_id),
    index(subject_type, subject_id)
)]
pub struct SearchIdentifierRow {
    /// The primary key.
    pub id: i64,
    /// The record's type.
    pub subject_type: String,
    /// The record's id.
    pub subject_id: i64,
    /// What the value is, one of [`kinds`].
    pub kind: String,
    /// The normalised value, or its HMAC when `hashed`.
    pub value: String,
    /// Whether `value` is a hash of a sensitive value.
    pub hashed: bool,
    /// When the row was created.
    pub created_at: Option<DateTime>,
    /// When the row was last changed.
    pub updated_at: Option<DateTime>,
}

/// The migration that adds the full-text index to `wl_search_documents`.
pub(crate) fn index_migration() -> Migration {
    renox::db::search::migration::<SearchDocumentRow>(INDEX_MIGRATION)
}

/// Registers the models and the `search:rebuild` command.
pub(crate) fn register(app: &mut Registry) {
    app.model::<SearchDocumentRow>()
        .model::<SearchIdentifierRow>();
    app.command(
        REBUILD_COMMAND,
        "Build the search index again from the registered types \
         (`--type <subject_type>` for one type)",
        |args, state: AppState| async move {
            let counts = match args.value("--type").map(str::to_owned) {
                Some(subject_type) => vec![(
                    subject_type.clone(),
                    rebuild_type(&state.db, &subject_type).await?,
                )],
                None => rebuild(&state.db).await?,
            };
            for (subject_type, count) in counts {
                println!("{subject_type}: {count} documents.");
            }
            Ok(())
        },
    );
}
