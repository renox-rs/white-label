//! The translations of an SOP version (#52, part 4/6).
//!
//! A translation is a document of the SOP translation kind on the SOP, linked
//! to the version it translates by a [`SopTranslation`] row. It is accepted
//! and rejected like a version (the `accept_sop` and `reject_sop`
//! transitions, owner role needed) but emits no `SopPublished` and requests
//! nothing: only the ordinary `DocumentAccepted` / `DocumentRejected`. It is
//! superseded by a newer accepted translation in the same language, and with
//! the version it translates.

use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::DocumentRejected;

use super::model::{Sop, SopTranslation};
use super::service::{Sops, refused, require_manage};
use super::versions::{guard, holds_owner_role};
use super::{KIND_SOP, KIND_SOP_TRANSLATION, SUBJECT_TYPE};
use crate::documents::{Superseded, emit_accepted, emit_superseded};
use crate::machine::{ACCEPT_SOP, REJECT_SOP, SUPERSEDE, SopRunInput, SupersedeInput};
use crate::{Attach, Document, DocumentStatus, Documents, NewDocument, texts};

impl Sops {
    /// Attaches `file` as the `language` translation of the SOP version
    /// `version` (pending or accepted), pending review, uploaded by `by`
    /// (holding `sops.manage`). Emits `DocumentUploaded` only.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; 422 for a blank language or a
    /// document that is not a pending or accepted SOP version; as
    /// [`Documents::attach`] otherwise.
    pub async fn upload_translation(
        state: &AppState,
        by: &AuthUser,
        version: &Document,
        language: &str,
        file: Upload,
        note: Option<&str>,
    ) -> Attach<Document> {
        require_manage(by)?;
        let lang = state.current_lang();
        let language = language.trim().to_lowercase();
        if language.is_empty() {
            return Err(refused(texts::t(
                &lang,
                "wl_documents.sops.refused.blank_language",
            )));
        }
        let version_kind = version.kind(&state.db).await?;
        if version_kind.key != KIND_SOP
            || version.subject_type != SUBJECT_TYPE
            || !matches!(
                version.status,
                DocumentStatus::PendingReview | DocumentStatus::Accepted
            )
        {
            return Err(refused(texts::t(
                &lang,
                "wl_documents.sops.refused.translation_target",
            )));
        }
        let sop = Sop::find_or_404(&state.db, version.subject_id).await?;
        let mut new = NewDocument::new(sop.subject(), KIND_SOP_TRANSLATION, file)
            .title(&format!("{} {} ({language})", sop.code, sop.title_en));
        if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
            new = new.note(note);
        }
        let created = Documents::attach(state, Some(by), new).await?;
        if let Ok(document) = &created {
            SopTranslation::create(
                &state.db,
                SopTranslation {
                    document_id: document.id,
                    language,
                    version_document_id: version.id,
                    ..SopTranslation::default()
                },
            )
            .await?;
        }
        Ok(created)
    }

    /// The translation row of the translation document `document`.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn translation_of(db: &Db, document: i64) -> Result<Option<SopTranslation>> {
        SopTranslation::query()
            .where_(SopTranslation::DOCUMENT_ID.eq(document))
            .first(db)
            .await
    }

    /// The translations of the version document `version`, oldest first.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn translations(db: &Db, version: i64) -> Result<Vec<SopTranslation>> {
        SopTranslation::query()
            .where_(SopTranslation::VERSION_DOCUMENT_ID.eq(version))
            .order_by(SopTranslation::ID)
            .get(db)
            .await
    }

    /// `accept_sop` on the pending translation `document`: `by` (holding
    /// `sops.manage` and the SOP's owner role) accepts it when the version it
    /// translates is the SOP's current accepted one. Supersedes the accepted
    /// translations of the same language. Emits `DocumentAccepted` and
    /// `DocumentSuperseded`; no `SopPublished`.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; `Guard` when `document` is not an
    /// SOP translation, `by` lacks the owner role or the translated version
    /// is not the current one; `NotFrom` (not pending).
    pub async fn accept_translation(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
    ) -> std::result::Result<Document, TransitionError> {
        let (sop, link) = context(&state.db, document).await?;
        let current = sop.current_document_id == Some(link.version_document_id)
            && Document::find(&state.db, link.version_document_id)
                .await?
                .is_some_and(|v| v.status == DocumentStatus::Accepted);
        if !current {
            let lang = state.current_lang();
            return Err(guard(&texts::t(
                &lang,
                "wl_documents.sops.refused.translated_not_current",
            )));
        }
        let input = SopRunInput {
            sop_kind: true,
            owner_role: holds_owner_role(&state.db, by, &sop).await?,
            needs_expiry: false,
        };
        let mut tx = state.db.begin().await?;
        let outcome = document
            .transition(ACCEPT_SOP)
            .by(by)
            .input(&input)
            .run_in(&mut tx)
            .await?;
        let done = !outcome.is_pending();
        let superseded = if done {
            supersede_same_language(&mut tx, document, &link.language).await?
        } else {
            Vec::new()
        };
        tx.commit().await?;
        outcome.emit(state).await?;
        let accepted = Document::find_or_404(&state.db, document.id).await?;
        if done {
            let kind = accepted.kind(&state.db).await?;
            emit_accepted(state, &accepted, &kind).await?;
            emit_superseded(state, superseded, accepted.id).await?;
        }
        Ok(accepted)
    }

    /// `reject_sop` on the pending translation `document` for `reason`, by a
    /// holder of `sops.manage` and the SOP's owner role. Emits
    /// `DocumentRejected`.
    ///
    /// # Errors
    ///
    /// As [`Sops::accept_translation`]; `ReasonRequired` for a blank reason.
    pub async fn reject_translation(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        reason: &str,
    ) -> std::result::Result<Document, TransitionError> {
        let (sop, _) = context(&state.db, document).await?;
        let input = SopRunInput {
            sop_kind: true,
            owner_role: holds_owner_role(&state.db, by, &sop).await?,
            needs_expiry: false,
        };
        let outcome = document
            .transition(REJECT_SOP)
            .by(by)
            .input(&input)
            .reason(reason.trim())
            .run(state)
            .await?;
        let rejected = Document::find_or_404(&state.db, document.id).await?;
        if !outcome.is_pending() {
            state
                .emit(DocumentRejected {
                    document: rejected.document_id(),
                    subject: rejected.subject(),
                    kind: KIND_SOP_TRANSLATION.to_owned(),
                    reason: reason.trim().to_owned(),
                })
                .await?;
        }
        Ok(rejected)
    }
}

/// The SOP and the translation row of the translation document `document`.
async fn context(
    db: &Db,
    document: &Document,
) -> std::result::Result<(Sop, SopTranslation), TransitionError> {
    let kind = document.kind(db).await?;
    if kind.key != KIND_SOP_TRANSLATION || document.subject_type != SUBJECT_TYPE {
        return Err(guard("The document is not an SOP translation."));
    }
    let Some(link) = Sops::translation_of(db, document.id).await? else {
        return Err(guard("The translation is not linked to a version."));
    };
    Ok((Sop::find_or_404(db, document.subject_id).await?, link))
}

/// Supersedes, in `tx`, the other accepted translations of `accepted`'s SOP
/// in `language`.
async fn supersede_same_language(
    tx: &mut Transaction,
    accepted: &Document,
    language: &str,
) -> std::result::Result<Vec<Superseded>, TransitionError> {
    let rows = SopTranslation::query()
        .where_(SopTranslation::LANGUAGE.eq(language))
        .where_(SopTranslation::DOCUMENT_ID.ne(accepted.id))
        .get(&mut *tx)
        .await?;
    let ids: Vec<i64> = rows.iter().map(|r| r.document_id).collect();
    supersede_documents(tx, accepted, &ids).await
}

/// Supersedes, in `tx`, the accepted translations of the version documents
/// `versions` (replaced by a newer version), by the system.
pub(super) async fn supersede_translations(
    tx: &mut Transaction,
    versions: &[i64],
) -> std::result::Result<Vec<Superseded>, TransitionError> {
    let mut done = Vec::new();
    for version in versions {
        let rows = SopTranslation::query()
            .where_(SopTranslation::VERSION_DOCUMENT_ID.eq(*version))
            .get(&mut *tx)
            .await?;
        for row in rows {
            let Some(translation) = Document::find(&mut *tx, row.document_id).await? else {
                continue;
            };
            if translation.status != DocumentStatus::Accepted {
                continue;
            }
            // Replaced by the version that replaced the translated one.
            let replaced_by = Document::find(&mut *tx, *version)
                .await?
                .and_then(|v| v.superseded_by)
                .unwrap_or(*version);
            let outcome = translation
                .transition(SUPERSEDE)
                .by_system()
                .input(&SupersedeInput { replaced_by })
                .run_in(&mut *tx)
                .await?;
            done.push((translation.id, outcome));
        }
    }
    Ok(done)
}

/// Supersedes the accepted documents among `ids` other than `accepted`.
async fn supersede_documents(
    tx: &mut Transaction,
    accepted: &Document,
    ids: &[i64],
) -> std::result::Result<Vec<Superseded>, TransitionError> {
    let mut done = Vec::new();
    for id in ids {
        let Some(old) = Document::find(&mut *tx, *id).await? else {
            continue;
        };
        if old.status != DocumentStatus::Accepted
            || old.subject_type != accepted.subject_type
            || old.subject_id != accepted.subject_id
        {
            continue;
        }
        let outcome = old
            .transition(SUPERSEDE)
            .by_system()
            .input(&SupersedeInput {
                replaced_by: accepted.id,
            })
            .run_in(&mut *tx)
            .await?;
        done.push((old.id, outcome));
    }
    Ok(done)
}
