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
//! The tutor's documents (`police_clearance`, `tutor_permit`, BL-EDU-08) are
//! attached to the tutor's employee. The providers here map a batch or a session
//! to the tutor's employee subject for a kind the tutor needs: the police
//! clearance when minors are enrolled ([`crate::eligibility::minors_enrolled`]),
//! the permit or visa for a freelance tutor. They serve `batch.open`,
//! `batch.change_tutor`, `session.change_tutor`, `session.reschedule` and
//! `session.submit_attendance` (the flag of BL-EDU-11); the refusal of an
//! assignment itself is [`crate::eligibility`]'s, on every session date. A
//! `FutureWork` provider lists the tutor's sessions after the expiry date for
//! wl-documents' look-ahead.

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::documents::{
    BlockEntry, BlockSubjects, DocumentFuture, FutureWork, PlannedRecord, guard,
    register_block_subjects, register_future_work,
};
use wl_kernel::workforce::{EMPLOYEE_SUBJECT_TYPE, EmployeeId};
use wl_kernel::{LegalEntity, Subject};

use crate::batch::Batch;
use crate::certificate::Certificate;
use crate::certificate_machine::DocumentsVerdict;
use crate::course::Course;
use crate::eligibility;
use crate::enrolment::Enrolment;
use crate::seed::{POLICE_CLEARANCE_KIND, TUTOR_PERMIT_KIND};
use crate::session::{Session, SessionStatus};

/// The guarded transition `open` of a batch.
pub const GUARDED_BATCH_OPEN: &str = "batch.open";

/// The guarded transition `change_tutor` of a batch (BL-EDU-08).
pub const GUARDED_BATCH_CHANGE_TUTOR: &str = "batch.change_tutor";

/// The guarded transition `change_tutor` of a session (BL-EDU-08).
pub const GUARDED_SESSION_CHANGE_TUTOR: &str = "session.change_tutor";

/// The guarded transition `reschedule` of a session (BL-EDU-08).
pub const GUARDED_SESSION_RESCHEDULE: &str = "session.reschedule";

/// The guarded transition `submit_attendance` of a session: a `flag` mode entry,
/// the attendance is never refused (BL-EDU-11).
pub const GUARDED_SESSION_SUBMIT_ATTENDANCE: &str = "session.submit_attendance";

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
    if subject.subject_type == EMPLOYEE_SUBJECT_TYPE {
        return Ok(Some(
            eligibility::tutor_text(state, Some(subject.subject_id)).await,
        ));
    }
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
            if is_tutor_kind(&entry.kind) {
                return tutor_subjects(state, entry, batch.tutor_employee_id, &batch).await;
            }
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

/// Whether `kind` is one of the two documents of a tutor.
fn is_tutor_kind(kind: &str) -> bool {
    kind == POLICE_CLEARANCE_KIND || kind == TUTOR_PERMIT_KIND
}

/// The tutor's employee as the subject their document of `entry.kind` is
/// attached to, when `batch` needs that document of them: the police clearance
/// when minors are enrolled, the permit or visa for a freelance tutor. No
/// subject for a tutor without an employee, or when the batch does not need it.
async fn tutor_subjects(
    state: &AppState,
    entry: &BlockEntry,
    employee: Option<i64>,
    batch: &Batch,
) -> Result<Vec<Subject>> {
    let Some(employee) = employee else {
        return Ok(Vec::new());
    };
    let needed = if entry.kind == POLICE_CLEARANCE_KIND {
        eligibility::minors_enrolled(&state.db, batch).await?
    } else if entry.kind == TUTOR_PERMIT_KIND {
        eligibility::is_freelance(state, Some(employee)).await?
    } else {
        false
    };
    Ok(if needed {
        vec![EmployeeId(employee).subject()]
    } else {
        Vec::new()
    })
}

/// `batch.change_tutor`: the tutor the batch has now, for the kinds it needs.
struct BatchTutorSubjects;

impl BlockSubjects for BatchTutorSubjects {
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
            tutor_subjects(state, entry, batch.tutor_employee_id, &batch).await
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

/// `session.change_tutor`, `session.reschedule` and `session.submit_attendance`:
/// the session's tutor, for the kinds its batch needs.
struct SessionTutorSubjects;

impl BlockSubjects for SessionTutorSubjects {
    fn subjects<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        record_id: i64,
    ) -> DocumentFuture<'a, Vec<Subject>> {
        Box::pin(async move {
            let Some(session) = Session::find(&state.db, record_id).await? else {
                return Ok(Vec::new());
            };
            let Some(batch) = Batch::find(&state.db, session.batch_id).await? else {
                return Ok(Vec::new());
            };
            tutor_subjects(state, entry, session.tutor_employee_id, &batch).await
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

/// The `scheduled` sessions taught by the employee of `subject` on or after
/// `from` that need their document of `entry.kind`: what wl-documents lists as
/// the work an expired document leaves uncovered (`FutureWork`).
///
/// # Errors
///
/// The database's and the workforce's errors.
pub async fn sessions_from(
    state: &AppState,
    entry: &BlockEntry,
    subject: &Subject,
    from: NaiveDate,
) -> Result<Vec<PlannedRecord>> {
    if subject.subject_type != EMPLOYEE_SUBJECT_TYPE || !is_tutor_kind(&entry.kind) {
        return Ok(Vec::new());
    }
    let taught = Session::query()
        .where_(Session::TUTOR_EMPLOYEE_ID.eq(subject.subject_id))
        .where_(Session::STATUS.eq(SessionStatus::Scheduled))
        .where_(Session::SESSION_DATE.gte(from))
        .order_by(Session::STARTS_AT)
        .get(&state.db)
        .await?;
    let mut found = Vec::new();
    let mut needs: Vec<(i64, bool)> = Vec::new();
    for session in taught {
        let Some(batch) = Batch::find(&state.db, session.batch_id).await? else {
            continue;
        };
        let needed = match needs.iter().find(|(id, _)| *id == batch.id) {
            Some((_, needed)) => *needed,
            None => {
                let needed = !tutor_subjects(state, entry, Some(subject.subject_id), &batch)
                    .await?
                    .is_empty();
                needs.push((batch.id, needed));
                needed
            }
        };
        if needed {
            found.push(PlannedRecord {
                record: session.subject(),
                date: session.session_date,
                label: format!("{} · {}", batch.number(), session.number),
                // The campus of the batch: the coordinator of each campus where
                // the tutor has affected sessions is told, and only of those.
                scope: batch.campus().map(|campus| {
                    renox::auth::permissions::Scope::new(&campus.subject_type, campus.subject_id)
                }),
            });
        }
    }
    Ok(found)
}

/// The `FutureWork` provider of the sessions: the sessions of a tutor after the
/// expiry date of their document.
struct SessionWork;

impl FutureWork for SessionWork {
    fn planned_from<'a>(
        &'a self,
        state: &'a AppState,
        entry: &'a BlockEntry,
        subject: &'a Subject,
        from: NaiveDate,
    ) -> DocumentFuture<'a, Vec<PlannedRecord>> {
        Box::pin(sessions_from(state, entry, subject, from))
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

/// Registers the providers: the batch `open` and the certificate `issue`, the
/// tutor transitions of the batch and the session, and the look-ahead of the
/// sessions (listed once, under `session.change_tutor`, the remedy).
pub(crate) fn register() {
    register_block_subjects(GUARDED_BATCH_OPEN, BatchOpenSubjects);
    register_block_subjects(GUARDED_CERTIFICATE_ISSUE, CertificateIssueSubjects);
    register_block_subjects(GUARDED_BATCH_CHANGE_TUTOR, BatchTutorSubjects);
    register_block_subjects(GUARDED_SESSION_CHANGE_TUTOR, SessionTutorSubjects);
    register_block_subjects(GUARDED_SESSION_RESCHEDULE, SessionTutorSubjects);
    register_block_subjects(GUARDED_SESSION_SUBMIT_ATTENDANCE, SessionTutorSubjects);
    register_future_work(GUARDED_SESSION_CHANGE_TUTOR, SessionWork);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guarded_transitions_are_entity_dot_transition() {
        assert_eq!(GUARDED_BATCH_OPEN, "batch.open");
        assert_eq!(GUARDED_CERTIFICATE_ISSUE, "certificate.issue");
        assert_eq!(GUARDED_SESSION_RESCHEDULE, "session.reschedule");
    }

    #[test]
    fn the_two_documents_of_a_tutor_are_tutor_kinds() {
        assert!(is_tutor_kind("police_clearance") && is_tutor_kind("tutor_permit"));
        assert!(!is_tutor_kind("centre_permit"));
    }

    #[test]
    fn registering_makes_both_transitions_guardable() {
        register();
        let guarded = wl_kernel::documents::guarded_transitions();
        assert!(guarded.iter().any(|g| g == GUARDED_BATCH_OPEN));
        assert!(guarded.iter().any(|g| g == GUARDED_CERTIFICATE_ISSUE));
        for tutor_work in [
            GUARDED_BATCH_CHANGE_TUTOR,
            GUARDED_SESSION_CHANGE_TUTOR,
            GUARDED_SESSION_RESCHEDULE,
            GUARDED_SESSION_SUBMIT_ATTENDANCE,
        ] {
            assert!(guarded.iter().any(|g| g == tutor_work), "{tutor_work}");
        }
    }
}
