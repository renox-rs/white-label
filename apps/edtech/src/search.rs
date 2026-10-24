//! The centre's records in the global search and the *Related* panel
//! (BL-AGT-13, decision 0006): a counsellor finds a student's enrolment by
//! its number or the student's name, a coordinator a batch by its number
//! and a course by its code, the centre manager a certificate by its
//! number, each with its related records.
//!
//! **No student identity number** is indexed: the documents carry the
//! student's name, never an Emirates ID, a passport or a phone. The
//! certificate's public verification code stays out of the index: the
//! public verification page answers through its own route
//! (`/verify/certificate/{code}`), never through this one, which needs
//! `certificates.view`.
//!
//! A batch, an enrolment and a certificate carry their campus's scope when
//! they have one, as the centre's lists do.

use renox::auth::permissions::Scope;
use renox::db::{Dialect, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::batch::{BATCH_SUBJECT_TYPE, Batch, BatchStatus};
use crate::certificate::{CERTIFICATE_SUBJECT_TYPE, Certificate, CertificateStatus};
use crate::course::Course;
use crate::enrolment::{ENROLMENT_SUBJECT_TYPE, Enrolment};
use crate::permissions;
use crate::seed::COURSE_SUBJECT_TYPE;

const PARTIES_VIEW: &str = "parties.view";

/// The scope of a campus, global without one.
fn campus_scope(kind: Option<&String>, id: Option<i64>) -> Scope {
    match (kind, id) {
        (Some(kind), Some(id)) => Scope::new(kind, id),
        _ => Scope::global(),
    }
}

/// Whether table `name` exists, read in `tx`.
async fn table_exists(tx: &mut Transaction, name: &str) -> Result<bool> {
    let statement = match tx.dialect() {
        Dialect::Sqlite => "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?",
        Dialect::Postgres => {
            "SELECT COUNT(*) FROM pg_tables WHERE schemaname = current_schema() AND tablename = ?"
        }
    };
    Ok(sql(statement).bind(name).scalar::<i64>(&mut *tx).await? > 0)
}

/// The display name of party `id`, read in `tx`; empty when unknown.
async fn name_in(tx: &mut Transaction, id: i64) -> Result<String> {
    if !table_exists(tx, "wl_parties_parties").await? {
        return Ok(String::new());
    }
    Ok(
        sql("SELECT display_name FROM wl_parties_parties WHERE id = ?")
            .bind(id)
            .scalar_optional::<String>(&mut *tx)
            .await?
            .unwrap_or_default(),
    )
}

/// The course of batch `id` (code and name), read in `tx`.
async fn course_of_batch_in(tx: &mut Transaction, batch: Option<i64>) -> Result<Option<Course>> {
    let Some(batch) = batch else {
        return Ok(None);
    };
    let Some(batch) = Batch::unscoped()
        .where_eq("id", batch)
        .first(&mut *tx)
        .await?
    else {
        return Ok(None);
    };
    Course::unscoped()
        .where_eq("id", batch.course_id)
        .first(&mut *tx)
        .await
}

fn course_document(course: &Course) -> SearchDocument {
    let mut document = SearchDocument::new(format!("{} · {}", course.code, course.name_en))
        .secondary(&course.programme)
        .identifier(kinds::NUMBER, &course.code)
        .status(if course.active { "active" } else { "inactive" });
    if let Some(name) = &course.name_ar {
        document = document.keyword(name);
    }
    if let Some(approval) = &course.approval_no {
        document = document.identifier(kinds::NUMBER, approval);
    }
    document
}

async fn batch_document(tx: &mut Transaction, batch: &Batch) -> Result<SearchDocument> {
    let course = Course::unscoped()
        .where_eq("id", batch.course_id)
        .first(&mut *tx)
        .await?
        .unwrap_or_default();
    let number = batch.number();
    Ok(
        SearchDocument::new(format!("{number} · {}", course.name_en))
            .secondary(format!("{} from {}", course.code, batch.start_date))
            .keyword(&course.code)
            .identifier(kinds::NUMBER, &number)
            .status(batch.status.as_str())
            .voided(batch.status == BatchStatus::Cancelled)
            .scope(campus_scope(batch.campus_type.as_ref(), batch.campus_id)),
    )
}

async fn enrolment_document(tx: &mut Transaction, enrolment: &Enrolment) -> Result<SearchDocument> {
    let student = name_in(tx, enrolment.student_party_id).await?;
    let course = course_of_batch_in(tx, enrolment.batch_id).await?;
    let what = course
        .as_ref()
        .map(|course| course.name_en.clone())
        .or_else(|| enrolment.item_name.clone())
        .unwrap_or_default();
    let number = enrolment.number();
    let mut document = SearchDocument::new(format!("{number} · {student}"))
        .secondary(what)
        .identifier(kinds::NUMBER, &number)
        .status(enrolment.status.as_str())
        .scope(campus_scope(
            enrolment.campus_type.as_ref(),
            enrolment.campus_id,
        ));
    if let Some(course) = course {
        document = document.keyword(course.code);
    }
    Ok(document)
}

async fn certificate_document(
    tx: &mut Transaction,
    certificate: &Certificate,
) -> Result<SearchDocument> {
    let enrolment = Enrolment::unscoped()
        .where_eq("id", certificate.enrolment_id)
        .first(&mut *tx)
        .await?;
    let title = certificate
        .number
        .clone()
        .unwrap_or_else(|| format!("Draft certificate · {}", certificate.student_name));
    let mut document = SearchDocument::new(title)
        .secondary(format!(
            "{} · {}",
            certificate.student_name, certificate.course_title
        ))
        .status(certificate.status.as_str())
        .voided(certificate.status == CertificateStatus::Revoked)
        .scope(match &enrolment {
            Some(enrolment) => campus_scope(enrolment.campus_type.as_ref(), enrolment.campus_id),
            None => Scope::global(),
        });
    if let Some(number) = &certificate.number {
        document = document.identifier(kinds::NUMBER, number);
    }
    if let Some(reference) = &certificate.attestation_ref {
        document = document.identifier(kinds::NUMBER, reference);
    }
    Ok(document)
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Course>(COURSE_SUBJECT_TYPE)
        .permission(permissions::COURSES_VIEW)
        .label("Courses")
        .document(course_document)
        .register();
    Searchable::of::<Batch>(BATCH_SUBJECT_TYPE)
        .permission(permissions::BATCHES_VIEW)
        .label("Batches")
        .link("/edtech/batches/{id}")
        .document_in(|tx, batch| Box::pin(batch_document(tx, batch)))
        .register();
    Searchable::of::<Enrolment>(ENROLMENT_SUBJECT_TYPE)
        .permission(permissions::ENROLMENTS_VIEW)
        .label("Enrolments")
        .link("/edtech/enrolments/{id}")
        .document_in(|tx, enrolment| Box::pin(enrolment_document(tx, enrolment)))
        .register();
    Searchable::of::<Certificate>(CERTIFICATE_SUBJECT_TYPE)
        .permission(permissions::CERTIFICATES_VIEW)
        .label("Certificates")
        .link("/edtech/certificates/{id}")
        .document_in(|tx, certificate| Box::pin(certificate_document(tx, certificate)))
        .register();

    // A course: its batches.
    Relation::has_many(
        COURSE_SUBJECT_TYPE,
        "batches",
        BATCH_SUBJECT_TYPE,
        "edtech_batches",
        "course_id",
    )
    .label("Batches")
    .register();

    // A batch: its course and its enrolments.
    Relation::belongs_to(
        BATCH_SUBJECT_TYPE,
        "course",
        COURSE_SUBJECT_TYPE,
        "edtech_batches",
        "course_id",
    )
    .label("Course")
    .register();
    Relation::has_many(
        BATCH_SUBJECT_TYPE,
        "enrolments",
        ENROLMENT_SUBJECT_TYPE,
        "edtech_enrolments",
        "batch_id",
    )
    .label("Enrolments")
    .register();

    // An enrolment: its batch, its certificates, the student and the payer.
    Relation::belongs_to(
        ENROLMENT_SUBJECT_TYPE,
        "batch",
        BATCH_SUBJECT_TYPE,
        "edtech_enrolments",
        "batch_id",
    )
    .label("Batch")
    .register();
    Relation::has_many(
        ENROLMENT_SUBJECT_TYPE,
        "certificates",
        CERTIFICATE_SUBJECT_TYPE,
        "edtech_certificates",
        "enrolment_id",
    )
    .label("Certificates")
    .register();
    Relation::belongs_to(
        ENROLMENT_SUBJECT_TYPE,
        "student",
        PARTY_SUBJECT_TYPE,
        "edtech_enrolments",
        "student_party_id",
    )
    .label("Student")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::belongs_to(
        ENROLMENT_SUBJECT_TYPE,
        "payer",
        PARTY_SUBJECT_TYPE,
        "edtech_enrolments",
        "payer_party_id",
    )
    .label("Payer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A certificate: its enrolment and the certificates it replaced or that
    // replaced it.
    Relation::belongs_to(
        CERTIFICATE_SUBJECT_TYPE,
        "enrolment",
        ENROLMENT_SUBJECT_TYPE,
        "edtech_certificates",
        "enrolment_id",
    )
    .label("Enrolment")
    .register();
    Relation::belongs_to(
        CERTIFICATE_SUBJECT_TYPE,
        "supersedes",
        CERTIFICATE_SUBJECT_TYPE,
        "edtech_certificates",
        "supersedes_id",
    )
    .label("Replaces")
    .register();
    Relation::belongs_to(
        CERTIFICATE_SUBJECT_TYPE,
        "superseded_by",
        CERTIFICATE_SUBJECT_TYPE,
        "edtech_certificates",
        "superseded_by_id",
    )
    .label("Replaced by")
    .register();
}

/// Writes the document of `subject` again in a transaction of its own, for
/// a record saved outside one.
///
/// # Errors
///
/// A database error.
pub async fn index(db: &Db, subject: &Subject) -> Result {
    let mut tx = db.begin().await?;
    index_in(&mut tx, subject).await?;
    tx.commit().await?;
    Ok(())
}

/// Writes the document of a record of `M` again once a transition run with
/// `run` has committed (its status changed).
fn reindex_after<M>(app: &mut Registry, subject_type: &'static str)
where
    M: StateMachine<Key = i64> + Send + Sync + 'static,
{
    app.listen(move |event: Transitioned<M>, state: AppState| async move {
        index(&state.db, &Subject::new(subject_type, event.key)).await
    });
}

/// Keeps the documents current after transitions.
pub(crate) fn listen(app: &mut Registry) {
    reindex_after::<Batch>(app, BATCH_SUBJECT_TYPE);
    reindex_after::<Enrolment>(app, ENROLMENT_SUBJECT_TYPE);
    reindex_after::<Certificate>(app, CERTIFICATE_SUBJECT_TYPE);
}
