//! Asking for a block override (#50 A4): [`Documents::request_override`].

use renox::auth::permissions::Scope;
use renox::db::{DateTime, now};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::approvals::{self, Require, Required};
use wl_kernel::documents::{BlockEntry, BlockMode, OverrideMode, block_subjects, today};

use super::model::{BlockOverride, BlockOverrideStatus};
use super::{ACTION, SUBJECT_TYPE};
use crate::documents::Documents;
use crate::guard;
use crate::kinds::{self, DocumentKind};
use crate::texts;

/// A 422 with `message`.
fn refused(message: String) -> Error {
    Error::Status(StatusCode::UNPROCESSABLE_ENTITY, message)
}

/// Whether `existing` is still open: `requested`, or `approved`, unused and
/// still valid at `at`.
pub(crate) fn is_open(existing: &BlockOverride, at: DateTime) -> bool {
    existing.status == BlockOverrideStatus::Requested || existing.usable_at(at)
}

impl Documents {
    /// Asks for a *Block override* of `kind` on the guarded transition
    /// `guarded` of record `record_id`, for every subject whose document of
    /// that kind fails today (one override each, in the record's scope).
    /// Each goes through the `documents.block_override` approval; the
    /// approvers are told after the commit.
    ///
    /// Refused with 422 when the reason is blank; the kind is a legal block
    /// or its override mode is not `with_approval`; the guard does not
    /// refuse `guarded` on that record because of that kind; an override of
    /// the same kind, subject, transition and record is already open for
    /// `requester` for every failing subject; or the approval policy is off
    /// (an override never runs without a second person).
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown kind; the database's errors.
    pub async fn request_override(
        state: &AppState,
        requester: &User,
        kind: &str,
        guarded: &str,
        record_id: i64,
        reason: &str,
    ) -> Result<Vec<BlockOverride>> {
        let lang = state.current_lang();
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(refused(texts::t(
                &lang,
                "wl_documents.override.blank_reason",
            )));
        }
        let record = DocumentKind::by_key(&state.db, kind)
            .await?
            .ok_or(Error::NotFound)?;
        let name = record.name(&lang.locale).to_owned();
        if record.legal_block || record.override_mode != OverrideMode::WithApproval {
            return Err(refused(texts::t_with(
                &lang,
                "wl_documents.override.not_overridable",
                &[("kind", &name)],
            )));
        }

        let entries: Vec<BlockEntry> = kinds::blocks_for(&state.db, guarded)
            .await?
            .into_iter()
            .filter(|entry| entry.mode == BlockMode::Block && entry.kind == kind)
            .collect();
        let nothing = || {
            refused(texts::t_with(
                &lang,
                "wl_documents.override.nothing_to_override",
                &[("kind", &name), ("guarded", &guarded)],
            ))
        };
        let Some(entry) = entries.first().cloned() else {
            return Err(nothing());
        };
        let (failures, _) = guard::failing(state, guarded, entries, record_id, today()).await?;
        if failures.is_empty() {
            return Err(nothing());
        }

        // Skip the subjects that already have an open override.
        let at = now();
        let mut subjects: Vec<Subject> = Vec::new();
        for failure in failures {
            let open = BlockOverride::query()
                .where_(BlockOverride::KIND_ID.eq(record.id))
                .where_(BlockOverride::SUBJECT_TYPE.eq(failure.subject.subject_type.as_str()))
                .where_(BlockOverride::SUBJECT_ID.eq(failure.subject.subject_id))
                .where_(BlockOverride::GUARDED.eq(guarded))
                .where_(BlockOverride::RECORD_ID.eq(record_id))
                .where_(BlockOverride::REQUESTED_BY.eq(requester.id))
                .get(&state.db)
                .await?
                .iter()
                .any(|existing| is_open(existing, at));
            if !open && !subjects.contains(&failure.subject) {
                subjects.push(failure.subject);
            }
        }
        if subjects.is_empty() {
            return Err(refused(texts::t_with(
                &lang,
                "wl_documents.override.already_open",
                &[("kind", &name)],
            )));
        }

        let scope: Option<Scope> = match block_subjects(guarded) {
            Some(provider) => provider.scope(state, &entry, record_id).await?,
            None => None,
        };
        let (scope_type, scope_id) = match scope.filter(|s| !s.is_global()) {
            Some(scope) => (Some(scope.kind().to_owned()), Some(scope.id().to_owned())),
            None => (None, None),
        };

        let mut tx = state.db.begin().await?;
        let mut created = Vec::with_capacity(subjects.len());
        let mut requests = Vec::with_capacity(subjects.len());
        for subject in subjects {
            let mut row = BlockOverride::create(
                &mut tx,
                BlockOverride {
                    kind_id: record.id,
                    subject_type: subject.subject_type,
                    subject_id: subject.subject_id,
                    guarded: guarded.to_owned(),
                    record_id,
                    scope_type: scope_type.clone(),
                    scope_id: scope_id.clone(),
                    reason: reason.to_owned(),
                    requested_by: requester.id,
                    status: BlockOverrideStatus::Requested,
                    ..Default::default()
                },
            )
            .await?;
            let require = Require::new(
                ACTION,
                Subject::new(SUBJECT_TYPE, row.id),
                "grant",
                requester.id,
                "requested",
            );
            let request = match approvals::require_in(&mut tx, &require).await? {
                Required::Pending(request) => request,
                // The policy is off: an override never runs without a second
                // person. Dropping `tx` rolls the row back.
                Required::Allowed => {
                    return Err(refused(texts::t(
                        &lang,
                        "wl_documents.override.no_approval",
                    )));
                }
            };
            BlockOverride::query()
                .where_(BlockOverride::ID.eq(row.id))
                .update(&mut tx, &[("approval_request_id", &Some(request.id))])
                .await?;
            row.approval_request_id = Some(request.id);
            requests.push(request);
            created.push(row);
        }
        tx.commit().await?;
        for request in &requests {
            approvals::announce_request(state, request).await?;
        }
        Ok(created)
    }
}

#[cfg(test)]
mod tests {
    use renox::chrono::Duration;

    use super::*;

    #[test]
    fn a_requested_or_usable_override_is_open() {
        let at = now();
        let requested = BlockOverride::default();
        assert!(is_open(&requested, at));
        let approved = BlockOverride {
            status: BlockOverrideStatus::Approved,
            valid_until: Some(at + Duration::hours(1)),
            ..Default::default()
        };
        assert!(is_open(&approved, at));
        assert!(!is_open(&approved, at + Duration::hours(2)));
        let used = BlockOverride {
            status: BlockOverrideStatus::Used,
            ..approved
        };
        assert!(!is_open(&used, at));
        let refused = BlockOverride {
            status: BlockOverrideStatus::Refused,
            ..Default::default()
        };
        assert!(!is_open(&refused, at));
    }
}
