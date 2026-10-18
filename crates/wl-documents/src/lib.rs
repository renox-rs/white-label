#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod documents;
pub mod expiry;
pub mod flags;
pub mod guard;
pub mod kinds;
pub mod machine;
pub mod overrides;
pub mod pages;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use documents::{
    Attach, AttachReason, AttachRefused, Document, DocumentSource, DocumentStatus, DocumentVersion,
    Documents, NewDocument,
};
pub use expiry::{AffectedItem, ExpiredLog, ReminderLog, RenewalTask};
pub use flags::DocumentFlag;
pub use guard::DocumentsGuard;
pub use kinds::{
    BlockChangeRefused, BlockSeed, DocumentKind, KindBlock, KindSeed, RefusalReason, Sensitivity,
    add_block, blocks_for, register_kind, remove_block, set_block_mode,
};
pub use overrides::{BlockOverride, BlockOverrideStatus};

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
    /// Approve a Block override in the record's scope (never one's own).
    pub const BLOCK_OVERRIDE: &str = "documents.block.override";
}

/// The templates, compiled in. An app replaces one with a file of the same
/// name under its views directory.
const VIEWS: &[(&str, &str)] = &[
    (
        "wl-documents/review_queue.html",
        include_str!("../views/wl-documents/review_queue.html"),
    ),
    (
        "wl-documents/flags.html",
        include_str!("../views/wl-documents/flags.html"),
    ),
    (
        "wl-documents/kinds.html",
        include_str!("../views/wl-documents/kinds.html"),
    ),
    (
        "wl-documents/kind.html",
        include_str!("../views/wl-documents/kind.html"),
    ),
    (
        "wl-documents/overrides.html",
        include_str!("../views/wl-documents/overrides.html"),
    ),
    (
        "wl-documents/override.html",
        include_str!("../views/wl-documents/override.html"),
    ),
    (
        "wl-documents/override_form.html",
        include_str!("../views/wl-documents/override_form.html"),
    ),
];

/// The documents module. The host adds it with
/// `.module(wl_documents::WlDocuments)` when `client.toml` enables `documents`.
pub struct WlDocuments;

impl Module for WlDocuments {
    fn name(&self) -> &'static str {
        "wl-documents"
    }

    fn routes(&self) -> Routes {
        pages::routes().merge(overrides::web::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        app.model::<DocumentKind>()
            .model::<KindBlock>()
            .model::<Document>()
            .model::<DocumentVersion>()
            .model::<DocumentFlag>()
            .model::<AffectedItem>()
            .model::<ReminderLog>()
            .model::<ExpiredLog>()
            .model::<RenewalTask>()
            .model::<BlockOverride>()
            .view_data::<pages::ReviewQueuePage>()
            .view_data::<pages::FlagsPage>()
            .view_data::<pages::KindsPage>()
            .view_data::<pages::KindPage>()
            .view_data::<overrides::web::OverridesPage>()
            .view_data::<overrides::web::OverrideFormPage>()
            .view_data::<overrides::web::OverridePage>();
        // `DOC.RUL.01` to `DOC.RUL.03`: the `documents.expiring` and
        // `documents.expired` types, the daily `documents:expiry` task
        // (01:00 Asia/Dubai) and its command.
        expiry::run::register_type();
        expiry::run::schedule(app);
        // Lifting the block: an accepted renewal closes the renewal task.
        expiry::renewal::register(app);
        // Block overrides: the machine, the approval policy and the listener
        // that refuses an override whose approval ended without a grant.
        overrides::register(app);
        // `rnx state:diagram Document` and the admin panel find the machine.
        renox::state_machine::register::<Document>();
        // The standard guard `document_valid` other modules call through
        // `wl_kernel::documents::guard`.
        wl_kernel::documents::provide_guard(app, DocumentsGuard);
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
        // The review queue in the app group and the kinds in the admin group
        // of the staff sidebar; the labels are the app's `nav.documents_review`
        // and `nav.document_kinds` texts.
        app.nav(
            "app",
            NavLink::route("wl_documents.review", "nav.documents_review")
                .icon("inbox")
                .group("nav.app")
                .order(35)
                .permission(permissions::REVIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_documents.flags", "nav.document_flags")
                .icon("flag")
                .group("nav.app")
                .order(36)
                .permission(permissions::VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_documents.overrides", "nav.document_overrides")
                .icon("shield")
                .group("nav.app")
                .order(37),
        );
        app.nav(
            "app",
            NavLink::route("wl_documents.kinds", "nav.document_kinds")
                .icon("folder")
                .group("nav.admin")
                .order(45)
                .permission(permissions::KINDS_MANAGE),
        );
        app.templates(|env| {
            for (name, source) in VIEWS {
                // The app's own file of that name wins.
                if env.get_template(name).is_err() {
                    renox::view::add_template(env, *name, *source)
                        .expect("a built-in template compiles");
                }
            }
        });
    }
}
