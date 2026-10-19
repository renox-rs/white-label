//! The versions of an SOP: [`Sops::upload_version`], [`Sops::accept_version`],
//! [`Sops::reject_version`] and [`Sops::withdraw`] (#52, part 3/6).
//!
//! A version is a document of the SOP kind. It moves through the SOP
//! transitions of the Document machine (`accept_sop`, `reject_sop`,
//! `archive_sop`), which the generic ones refuse for these kinds, so the
//! history shows which ran.

use renox::auth::permissions::Scope;
use renox::chrono::{Months, NaiveDate};
use renox::db::now;
use renox::prelude::*;
use renox::state_machine::{StateMachine, TransitionError};
use wl_kernel::documents::{DocumentId, DocumentRejected, SopId, SopPublished, SopWithdrawn};

use super::model::Sop;
use crate::acknowledgements::Acknowledgements;
use super::service::{Sops, require_manage};
use super::{KIND_SOP, SUBJECT_TYPE};
use crate::documents::{emit_accepted, emit_superseded, supersede_previous};
use crate::expiry::responsible::holders_in;
use crate::machine::{ACCEPT_SOP, ARCHIVE_SOP, REJECT_SOP, SopRunInput};
use crate::{Attach, Document, DocumentKind, DocumentStatus, Documents, NewDocument, permissions};

/// The review date of a version accepted on `accepted_on` (an Asia/Dubai
/// date): that date plus `months`, clamped to the end of the month.
pub fn review_date(accepted_on: NaiveDate, months: i64) -> NaiveDate {
    let months = u32::try_from(months).unwrap_or(0);
    accepted_on
        .checked_add_months(Months::new(months))
        .unwrap_or(accepted_on)
}

/// A refusal as a transition error.
pub(super) fn guard(message: &str) -> TransitionError {
    TransitionError::Guard(message.to_owned())
}

impl Sops {
    /// Attaches `file` as a new version of `sop`, pending review, uploaded
    /// by `by` (holding `sops.manage`). Refused as [`Documents::attach`] is.
    /// A withdrawn SOP takes new versions: accepting one makes it active.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; `NotFound` for an unknown SOP or
    /// when the SOP kinds are not installed; the database's or the disk's
    /// errors.
    pub async fn upload_version(
        state: &AppState,
        by: &AuthUser,
        sop: i64,
        file: Upload,
        note: Option<&str>,
    ) -> Attach<Document> {
        require_manage(by)?;
        let row = Sop::find_or_404(&state.db, sop).await?;
        let mut new = NewDocument::new(row.subject(), KIND_SOP, file)
            .title(&format!("{} {}", row.code, row.title_en));
        if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
            new = new.note(note);
        }
        Documents::attach(state, Some(by), new).await
    }

    /// The running number of the version `document` of its SOP: 1 for the
    /// first version uploaded, 2 for the next, and so on.
    ///
    /// # Errors
    ///
    /// The database's errors.
    pub async fn version_number(db: &Db, document: &Document) -> Result<i64> {
        let Some(kind) = DocumentKind::by_key(db, KIND_SOP).await? else {
            return Ok(1);
        };
        let versions = Document::query()
            .where_(Document::SUBJECT_TYPE.eq(document.subject_type.as_str()))
            .where_(Document::SUBJECT_ID.eq(document.subject_id))
            .where_(Document::KIND_ID.eq(kind.id))
            .get(db)
            .await?;
        let number = versions.iter().filter(|d| d.id <= document.id).count();
        Ok(i64::try_from(number).unwrap_or(i64::MAX))
    }

    /// `accept_sop`: `by` (holding `sops.manage` and the SOP's owner role)
    /// accepts the pending version `document`. In one transaction the review
    /// date is set to the accepted date plus the SOP's interval, the version
    /// is accepted, the previous accepted version is superseded and the SOP's
    /// current version becomes this one. Emits `DocumentAccepted`,
    /// `DocumentSuperseded` and `SopPublished` (with the required readers).
    /// The uploader may accept their own version.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; `Guard` when `document` is not an
    /// SOP version or `by` lacks the owner role; `NotFrom` (not pending).
    pub async fn accept_version(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
    ) -> std::result::Result<Document, TransitionError> {
        let sop = sop_of(&state.db, document).await?;
        let input = SopRunInput {
            sop_kind: true,
            owner_role: holds_owner_role(&state.db, by, &sop).await?,
            needs_expiry: true,
        };
        let review = review_date(
            wl_kernel::documents::dubai_date(now()),
            sop.review_interval_months,
        );
        let mut tx = state.db.begin().await?;
        // Before the run, so that the review date is there to be checked;
        // rolled back with the rest when the run is refused.
        Document::query()
            .where_(Document::ID.eq(document.id))
            .update(&mut tx, &[("expiry_date", &Some(review))])
            .await?;
        let fresh = Document::query()
            .where_(Document::ID.eq(document.id))
            .first(&mut tx)
            .await?
            .ok_or(Error::NotFound)?;
        let outcome = fresh
            .transition(ACCEPT_SOP)
            .by(by)
            .input(&input)
            .run_in(&mut tx)
            .await?;
        let done = !outcome.is_pending();
        let superseded = if done {
            Sop::query()
                .where_(Sop::ID.eq(sop.id))
                .update(
                    &mut tx,
                    &[
                        ("current_document_id", &Some(document.id)),
                        ("updated_at", &now()),
                    ],
                )
                .await?;
            let mut superseded = supersede_previous(&mut tx, &fresh).await?;
            // The translations of the replaced versions go with them.
            let replaced: Vec<i64> = superseded.iter().map(|(id, _)| *id).collect();
            superseded
                .extend(super::translations::supersede_translations(&mut tx, &replaced).await?);
            superseded
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
            state
                .emit(SopPublished {
                    sop: SopId(sop.id),
                    document: accepted.document_id(),
                    version: Sops::version_number(&state.db, &accepted).await?,
                    ack_due_days: sop.ack_due_days,
                    readers: Sops::readers(&state.db, sop.id).await?,
                })
                .await?;
        }
        Ok(accepted)
    }

    /// `reject_sop`: `by` (holding `sops.manage` and the SOP's owner role)
    /// rejects the pending version `document` for `reason`. The SOP stays on
    /// its current version (or `draft`). Emits `DocumentRejected`.
    ///
    /// # Errors
    ///
    /// As [`Sops::accept_version`]; `ReasonRequired` for a blank reason.
    pub async fn reject_version(
        state: &AppState,
        by: &AuthUser,
        document: &Document,
        reason: &str,
    ) -> std::result::Result<Document, TransitionError> {
        let sop = sop_of(&state.db, document).await?;
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
                    kind: KIND_SOP.to_owned(),
                    reason: reason.trim().to_owned(),
                })
                .await?;
        }
        Ok(rejected)
    }

    /// Withdraws `sop`: `archive_sop` on its current version by `by`
    /// (holding `sops.manage`; the owner role is not needed). The archived
    /// version stays the current one, so the SOP reads `withdrawn` until a
    /// new version is accepted. Cancels the pending acknowledgements and emits
    /// `SopWithdrawn` with their ids.
    ///
    /// # Errors
    ///
    /// `Forbidden` without `sops.manage`; `Guard` when the SOP has no
    /// accepted current version (draft or already withdrawn); `NotFound`
    /// for an unknown SOP.
    pub async fn withdraw(
        state: &AppState,
        by: &AuthUser,
        sop: i64,
        reason: Option<&str>,
    ) -> std::result::Result<Sop, TransitionError> {
        if !by.has_permission(permissions::SOPS_MANAGE) {
            return Err(TransitionError::Forbidden);
        }
        let row = Sop::find_or_404(&state.db, sop).await?;
        let current = match row.current_document_id {
            Some(id) => Document::find(&state.db, id).await?,
            None => None,
        };
        let Some(current) = current else {
            return Err(guard("The SOP has no published version to withdraw."));
        };
        if current.status != DocumentStatus::Accepted {
            return Err(guard(
                "The SOP is not active, so there is nothing to withdraw.",
            ));
        }
        let mut run = current.transition(ARCHIVE_SOP).by(by).input(&SopRunInput {
            sop_kind: true,
            owner_role: false,
            needs_expiry: false,
        });
        if let Some(reason) = reason.map(str::trim).filter(|r| !r.is_empty()) {
            run = run.reason(reason);
        }
        let outcome = run.run(state).await?;
        if !outcome.is_pending() {
            // `cancel`: the readers need not acknowledge a withdrawn SOP; the
            // acknowledged ones stay as history (`DOC.RUL.04`).
            let reason = crate::texts::english("wl_documents.acks.cancelled_withdrawn", &[]);
            let cancelled =
                Acknowledgements::cancel_for_sop(state, row.id, &reason).await?;
            state
                .emit(SopWithdrawn {
                    sop: SopId(row.id),
                    document: DocumentId(current.id),
                    version: Sops::version_number(&state.db, &current).await?,
                    cancelled_acknowledgements: cancelled,
                })
                .await?;
        }
        Ok(Sop::find_or_404(&state.db, row.id).await?)
    }
}

/// The SOP that the version `document` belongs to.
pub(super) async fn sop_of(
    db: &Db,
    document: &Document,
) -> std::result::Result<Sop, TransitionError> {
    let kind = document.kind(db).await?;
    if kind.key != KIND_SOP || document.subject_type != SUBJECT_TYPE {
        return Err(guard("The document is not an SOP version."));
    }
    Ok(Sop::find_or_404(db, document.subject_id).await?)
}

/// Whether `by` holds the SOP's owner role in the SOP's scope: its legal
/// entity, else global (global holders count everywhere).
pub(super) async fn holds_owner_role(db: &Db, by: &AuthUser, sop: &Sop) -> Result<bool> {
    let scope = if sop.legal_entity().is_some() {
        Scope::new(sop.legal_entity_type.as_str(), sop.legal_entity_id)
    } else {
        Scope::global()
    };
    Ok(holders_in(db, &sop.owner_role, &scope)
        .await?
        .contains(&by.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn the_review_date_is_the_accepted_date_plus_the_interval() {
        assert_eq!(review_date(date("2026-10-19"), 12), date("2027-10-19"));
        assert_eq!(review_date(date("2026-10-19"), 6), date("2027-04-19"));
    }

    #[test]
    fn the_review_date_is_clamped_to_the_end_of_the_month() {
        assert_eq!(review_date(date("2026-08-31"), 6), date("2027-02-28"));
        assert_eq!(review_date(date("2027-12-31"), 12), date("2028-12-31"));
        assert_eq!(review_date(date("2027-02-28"), 12), date("2028-02-28"));
        assert_eq!(review_date(date("2028-02-29"), 12), date("2029-02-28"));
    }
}
