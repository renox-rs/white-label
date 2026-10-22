//! The document guards of the batch `open` and the certificate `issue` (10 §9
//! *Documents that block work*, §7.1 *Batch* `open`, *Certificate* `issue`,
//! BL-EDU-07, `DOC.ACL.02`).
//!
//! Which documents these transitions need is not decided here: it is the
//! `blocks` of the document kinds the client seeds (`host/src/app/edtech_seed.rs`):
//!
//! | Kind (subject) | Blocks | Override |
//! |---|---|---|
//! | Centre permit (the legal entity of the campus) | `batch.open`, `certificate.issue` (always) | never |
//! | Course approval (the course, where the course needs one) | `batch.open`; `certificate.issue` when the certificate states the approval | never |
//!
//! This module registers the two [`BlockSubjects`] providers that map a batch
//! or a certificate to the subjects whose documents must be valid, and the
//! checks the services run before the transition ([`check_batch_open`],
//! [`check_certificate_issue`]): a refusal is a 422 that names every expired or
//! missing document, with no override (the kinds' override mode is *never*).
//! Without `wl-documents` nothing is guarded.
//!
//! - **`batch.open`** is judged on the **last session date** of the batch: the
//!   permit and the approval must cover the whole batch. A course that needs no
//!   approval has no approval check.
//! - **`certificate.issue`** is judged on the day it is issued; for a reissued
//!   draft on the enrolment's completion date, so a correct certificate earned
//!   under a valid approval can be reissued after the approval expired.
//!   The course approval counts only for a certificate that states it (the
//!   entry's condition on `states_approval`); the centre permit always counts.
//!
//! The tutor's documents (`police_clearance`, `tutor_permit`) block the same
//! `batch.open` through the same provider; it names no employee until
//! BL-EDU-08 adds the batch's tutor to [`batch_candidates`].

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, guard, register_block_subjects,
};
use wl_kernel::{LegalEntity, Subject};

use crate::batch::Batch;
use crate::certificate::Certificate;
use crate::certificate_machine::DocumentsVerdict;
use crate::course::Course;
use crate::enrolment::Enrolment;

/// The guarded transition `open` of a batch.
pub const GUARDED_BATCH_OPEN: &str = "batch.open";

/// The guarded transition `issue` of a certificate.
pub const GUARDED_CERTIFICATE_ISSUE: &str = "certificate.issue";

/// The subject type of a legal entity: what a centre permit is attached to.
pub const LEGAL_ENTITY_SUBJECT_TYPE: &str = "kernel.legal_entity";

/// The field the course approval's entry on `certificate.issue` reads: whether
/// the certificate states the approval (`true` or `false`).
pub const CONDITION_STATES_APPROVAL: &str = "states_approval";

/// The legal entity that issues for the campus `campus`, as the subject a centre
/// permit is attached to; the default entity without a campus.
///
/// # Errors
///
/// The database's errors.
pub async fn permit_subject(db: &Db, campus: Option<&Subject>) -> Result<Subject> {
    let scope = campus.map(|campus| (campus.subject_type.as_str(), campus.subject_id));
    let entity = LegalEntity::for_scope(db, scope).await?;
    Ok(Subject::new(LEGAL_ENTITY_SUBJECT_TYPE, entity.id))
}

/// The subjects a batch's documents are attached to, before the kind picks its
/// own: the legal entity of its campus (the centre permit) and its course when
/// the course needs the regulator's approval.
///
/// # Errors
///
/// The database's errors.
pub async fn batch_candidates(db: &Db, batch: &Batch) -> Result<Vec<Subject>> {
    let mut found = vec![permit_subject(db, batch.campus().as_ref()).await?];
    if let Some(course) = Course::find(db, batch.course_id).await?
        && course.approval_required
    {
        found.push(course.subject());
    }
    Ok(found)
}

/// Keeps of `candidates` the subjects `entry`'s kind is attached to. A kind the
/// guard does not know leaves every subject to check.
async fn for_kind(
    state: &AppState,
    entry: &BlockEntry,
    candidates: Vec<Subject>,
) -> Result<Vec<Subject>> {
    let types = guard(state)
        .kind_subject_types(&state.db, &entry.kind)
        .await?;
    if types.is_empty() {
        return Ok(candidates);
    }
    Ok(candidates
        .into_iter()
        .filter(|subject| types.contains(&subject.subject_type))
        .collect())
}

/// The name of `subject` in a refusal: the centre's legal name, or the course.
async fn label_of(state: &AppState, subject: &Subject) -> Result<Option<String>> {
    if subject.subject_type == LEGAL_ENTITY_SUBJECT_TYPE {
        return Ok(LegalEntity::find(&state.db, subject.subject_id)
            .await?
            .map(|entity| entity.name_en));
    }
    if subject.subject_type == crate::course::COURSE_SUBJECT_TYPE {
        return Ok(Course::find(&state.db, subject.subject_id)
            .await?
            .map(|course| course.title()));
    }
    Ok(None)
}

/// `batch.open`: the centre permit and, where the course needs one, its approval.
struct BatchOpenSubjects;

impl BlockSubjects for BatchOpenSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(batch) = Batch::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            for_kind(state, entry, batch_candidates(&state.db, &batch).await?).await
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(label_of(state, subject))
    }
}

/// The subjects a certificate's documents are attached to: the legal entity of
/// the campus of its enrolment and, when the certificate states the approval,
/// its course.
///
/// # Errors
///
/// The database's errors.
pub async fn certificate_candidates(db: &Db, certificate: &Certificate) -> Result<Vec<Subject>> {
    let enrolment = Enrolment::find(db, certificate.enrolment_id).await?;
    let campus = enrolment.as_ref().and_then(|e| {
        e.campus_type
            .clone()
            .zip(e.campus_id)
            .map(|(kind, id)| Subject::new(&kind, id))
    });
    let mut found = vec![permit_subject(db, campus.as_ref()).await?];
    if certificate.states_approval
        && let Some(enrolment) = &enrolment
        && let Some(batch_id) = enrolment.batch_id
        && let Some(batch) = Batch::find(db, batch_id).await?
        && let Some(course) = Course::find(db, batch.course_id).await?
    {
        found.push(course.subject());
    }
    Ok(found)
}

/// `certificate.issue`: the centre permit, and the course approval when the
/// certificate states it (the entry's condition on `states_approval`).
struct CertificateIssueSubjects;

impl BlockSubjects for CertificateIssueSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(certificate) = Certificate::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let states = if certificate.states_approval {
                "true"
            } else {
                "false"
            };
            if !entry.applies_to(Some(states)) {
                return Ok(Vec::new());
            }
            for_kind(
                state,
                entry,
                certificate_candidates(&state.db, &certificate).await?,
            )
            .await
        })
    }

    fn label<'a>(
        &'a self,
        state: &'a AppState,
        subject: &'a Subject,
    ) -> DocumentFuture<'a, Option<String>> {
        Box::pin(label_of(state, subject))
    }
}

/// Asks the guard whether record `id` may run `guarded` on `on`: a refusal is a
/// 422 naming each document that is expired or missing.
async fn check(
    state: &AppState,
    guarded: &str,
    id: i64,
    on: NaiveDate,
    actor: Option<i64>,
) -> Result<()> {
    match guard(state).check_as(state, guarded, id, on, actor).await? {
        Ok(_outcome) => Ok(()),
        Err(refusal) => Err(refusal.into_error()),
    }
}

/// `batch.open`: the permit and the approval are valid on `last_session`.
///
/// # Errors
///
/// A 422 naming what is expired or missing; the guard's own failures.
pub async fn check_batch_open(
    state: &AppState,
    batch: &Batch,
    last_session: NaiveDate,
    actor: Option<i64>,
) -> Result<()> {
    check(state, GUARDED_BATCH_OPEN, batch.id, last_session, actor).await
}

/// `certificate.issue`: the permit (and the approval, when stated) are valid on
/// `on`. The answer goes to the machine's guard `document_valid` in the run's
/// input.
///
/// # Errors
///
/// The guard's own failures; a refusal is the [`DocumentsVerdict`].
pub async fn check_certificate_issue(
    state: &AppState,
    certificate: &Certificate,
    on: NaiveDate,
    actor: Option<i64>,
) -> Result<DocumentsVerdict> {
    match guard(state)
        .check_as(state, GUARDED_CERTIFICATE_ISSUE, certificate.id, on, actor)
        .await?
    {
        Ok(_outcome) => Ok(DocumentsVerdict::Valid),
        Err(refusal) => Ok(DocumentsVerdict::Refused(refusal.message())),
    }
}

/// Registers the two providers.
pub(crate) fn register() {
    register_block_subjects(GUARDED_BATCH_OPEN, BatchOpenSubjects);
    register_block_subjects(GUARDED_CERTIFICATE_ISSUE, CertificateIssueSubjects);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guarded_transitions_are_entity_dot_transition() {
        assert_eq!(GUARDED_BATCH_OPEN, "batch.open");
        assert_eq!(GUARDED_CERTIFICATE_ISSUE, "certificate.issue");
    }

    #[test]
    fn registering_makes_both_transitions_guardable() {
        register();
        let guarded = wl_kernel::documents::guarded_transitions();
        assert!(guarded.iter().any(|g| g == GUARDED_BATCH_OPEN));
        assert!(guarded.iter().any(|g| g == GUARDED_CERTIFICATE_ISSUE));
    }
}
