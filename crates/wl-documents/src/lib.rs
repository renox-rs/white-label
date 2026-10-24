#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod acknowledgements;
pub mod documents;
pub mod expiry;
pub mod flags;
pub mod generation;
pub mod guard;
pub mod kinds;
pub mod machine;
pub mod overrides;
pub mod pages;
pub mod registrations;
pub mod search;
pub mod sops;
pub mod store;
pub mod subject_pages;
mod texts;

use renox::db::Migration;
use renox::nav::NavLink;
use renox::prelude::*;

pub use acknowledgements::{AckRefused, AckStatus, Acknowledgements, SopAcknowledgement, SopDesk};
pub use documents::{
    Attach, AttachReason, AttachRefused, CorrectionInput, Document, DocumentSource, DocumentStatus,
    DocumentVersion, Documents, FieldsUpdate, NewDocument,
};
pub use expiry::{AffectedItem, ExpiredLog, ReminderLog, RenewalTask};
pub use flags::DocumentFlag;
pub use generation::{
    BrandAssets, BuiltinPdf, DocumentTemplate, Generation, PaperSize, PdfRenderer, PdfRendering,
    RenderError, RenderJob, TemplateGenerator, TemplateSeed, TemplateWording, register_template,
};
pub use guard::DocumentsGuard;
pub use kinds::{
    BlockChangeRefused, BlockSeed, DocumentKind, KindBlock, KindSeed, RefusalReason, Sensitivity,
    add_block, blocks_for, register_kind, remove_block, set_block_mode, set_enabled,
};
pub use overrides::{BlockOverride, BlockOverrideStatus};
pub use sops::{Sop, SopInput, SopReader, SopStatus, SopTranslation, Sops};

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
    /// Manage SOPs: their readers, withdrawing, and (with the SOP's owner
    /// role) accepting or rejecting a version.
    pub const SOPS_MANAGE: &str = "sops.manage";
    /// Generate documents on demand (transitions generate as the system).
    pub const GENERATE: &str = "documents.generate";
    /// Edit templates (text, layout) within the branding rules.
    pub const TEMPLATES_MANAGE: &str = "document_templates.manage";
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
    (
        "wl-documents/document_edit.html",
        include_str!("../views/wl-documents/document_edit.html"),
    ),
    (
        "wl-documents/document_correction.html",
        include_str!("../views/wl-documents/document_correction.html"),
    ),
    (
        "wl-documents/subject.html",
        include_str!("../views/wl-documents/subject.html"),
    ),
    (
        "wl-documents/sops.html",
        include_str!("../views/wl-documents/sops.html"),
    ),
    (
        "wl-documents/sop.html",
        include_str!("../views/wl-documents/sop.html"),
    ),
    (
        "wl-documents/sop_form.html",
        include_str!("../views/wl-documents/sop_form.html"),
    ),
    (
        "wl-documents/acknowledgements.html",
        include_str!("../views/wl-documents/acknowledgements.html"),
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
        pages::routes()
            .merge(overrides::web::routes())
            .merge(subject_pages::routes())
            .merge(sops::web::routes())
            .merge(acknowledgements::web::routes())
            .merge(store::routes())
    }

    fn migrations(&self) -> &'static [Migration] {
        renox::migrations!()
    }

    fn register(&self, app: &mut Registry) {
        // The documents of any record, in its Related panel (BL-AGT-03): a
        // generic relation, attached by `subject_type` / `subject_id`.
        wl_kernel::related::Relation::attached(
            "documents",
            expiry::renewal::DOCUMENT_SUBJECT_TYPE,
            "wl_documents_documents",
        )
        .label("Documents")
        .permission(permissions::VIEW)
        .register();
        // Documents in the global search (BL-AGT-04).
        search::register(app);
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
            .model::<Sop>()
            .model::<SopReader>()
            .model::<SopTranslation>()
            .model::<DocumentTemplate>()
            .model::<Generation>()
            .view_data::<pages::ReviewQueuePage>()
            .view_data::<pages::FlagsPage>()
            .view_data::<pages::KindsPage>()
            .view_data::<pages::KindPage>()
            .view_data::<overrides::web::OverridesPage>()
            .view_data::<overrides::web::OverrideFormPage>()
            .view_data::<overrides::web::OverridePage>()
            .view_data::<sops::web::SopsPage>()
            .view_data::<sops::web::SopFormPage>()
            .view_data::<sops::web::SopPage>()
            .view_data::<acknowledgements::web::AcknowledgementsPage>();
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
        // The SOP and SOP translation kinds.
        sops::register();
        // `DOC.RUL.04`: SOP acknowledgements, their machine, the events that
        // ask and cancel, the daily run and the kernel contract wl-hr asks.
        acknowledgements::register(app);
        // The dashboard's "approvals waiting for you" links to the override.
        wl_kernel::approvals::register_subject_link(overrides::SUBJECT_TYPE, |id| {
            format!("/documents/overrides/{id}")
        });
        // `rnx state:diagram Document` and the admin panel find the machine.
        renox::state_machine::register::<Document>();
        // The standard guard `document_valid` other modules call through
        // `wl_kernel::documents::guard`.
        wl_kernel::documents::provide_guard(app, DocumentsGuard);
        // `DOC.GEN.01`: `Documents::generate` other modules call through
        // `wl_kernel::generation::generator`.
        wl_kernel::generation::provide_generator(app, TemplateGenerator);
        // The stored files other modules read (wl-notify's attachments) and
        // their signed download links.
        wl_kernel::document_store::provide_document_store(app, store::StoredDocuments);
        // The kinds modules and verticals register, and the kinds for
        // registrations and certifications every client gets (#54), are
        // installed by this module's command and seeder; both keep the
        // client's changes and lock legal blocks again.
        app.command(
            "documents:install",
            "Store the document kinds the modules register, and lock legal blocks again",
            |_args, state: AppState| async move {
                let created = registrations::install_all(&state).await?;
                println!("{created} document kinds created.");
                Ok(())
            },
        );
        app.seeder(|state: AppState| async move {
            registrations::install_all(&state).await.map(|_| ())
        });
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
                .icon("triangle-alert")
                .group("nav.app")
                .order(36)
                .permission(permissions::VIEW),
        );
        app.nav(
            "app",
            NavLink::route("wl_documents.overrides", "nav.document_overrides")
                .icon("shield")
                .group("nav.app")
                .order(37)
                // Shown to everyone signed in, but staff's: not to a portal user.
                .when(|user| !wl_kernel::portal::is_portal_only(user)),
        );
        app.nav(
            "app",
            NavLink::route("wl_documents.sops", "nav.document_sops")
                .icon("book-open")
                .group("nav.app")
                .order(38)
                .permission(permissions::VIEW),
        );
        app.nav(
            "app",
            NavLink::route(
                "wl_documents.acknowledgements",
                "nav.document_acknowledgements",
            )
            .icon("circle-check")
            .group("nav.app")
            .order(39)
            .when(|user| !wl_kernel::portal::is_portal_only(user)),
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
