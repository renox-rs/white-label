//! The clinic's records in the global search and the *Related* panel
//! (BL-AGT-12, decision 0006): a receptionist finds a patient by the file
//! number or the Emirates ID, a visit by its number or the patient's file
//! number, a billing coordinator a claim or a prior authorisation by its
//! number or the payer's reference, each with its related records.
//!
//! **Health data is never indexed** (05 Clinics, `CLN.ACL.02`, decision
//! 0006): no document holds a diagnosis, a clinical note, a result, the
//! visit reason the booker wrote, the service given or anything of class
//! `health`. A patient's document holds the name, the file number and the
//! next appointment only; the Emirates ID is stored only as a hash
//! (`kinds::EMIRATES_ID`) and matches on the exact number. Agent access to
//! 05 stays locked (BL-AGT-06): nothing here opens it.
//!
//! A patient is a party with the role `patient` (wl-parties): the type
//! `clinics.patient` reads the party rows (the file number is the party's
//! code, plus the MRN its patient role carries) and leaves every other
//! party blank, so a supplier or a payer is never a patient result. The
//! Emirates ID is the number of the patient's *Patient Emirates ID*
//! document (wl-documents), and the next appointment the patient's next
//! booking (wl-scheduling), both read in the indexing transaction when
//! those modules' tables exist. Decision: a patient's document is written
//! when one of their visits, claims or authorisations is, and by
//! `search:rebuild`; a patient registered in wl-parties is found once
//! indexed (the parties module's own document finds the party meanwhile).

use renox::db::{DateTime, Dialect, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use serde::{Deserialize, Serialize};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, Searchable, Sensitivity, index_in, kinds};

use crate::authorisation::{AUTHORISATION_SUBJECT_TYPE, PriorAuthorisation};
use crate::claim::{CLAIM_SUBJECT_TYPE, Claim, ClaimStatus};
use crate::permissions;
use crate::policy::POLICY_SUBJECT_TYPE;
use crate::visit::{VISIT_SUBJECT_TYPE, Visit};

/// The subject type of a patient's file.
pub const PATIENT_SUBJECT_TYPE: &str = "clinics.patient";

/// The role a patient's party holds.
const PATIENT_ROLE: &str = "patient";

const PARTIES_VIEW: &str = "parties.view";

/// A patient's file as the search reads it: the party row (read only; the
/// party belongs to wl-parties).
#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "wl_parties_parties")]
pub struct PatientFile {
    /// The party's id.
    pub id: i64,
    /// The party's code: the patient's file number.
    pub code: String,
    /// The patient's name.
    pub display_name: String,
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

/// The MRNs of party `id`'s patient roles; `None` when it is no patient.
async fn mrns_in(tx: &mut Transaction, id: i64) -> Result<Option<Vec<String>>> {
    let rows =
        sql("SELECT external_reference FROM wl_parties_roles WHERE party_id = ? AND role_key = ?")
            .bind(id)
            .bind(PATIENT_ROLE)
            .fetch_all(&mut *tx)
            .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut mrns = Vec::new();
    for row in &rows {
        if let Some(mrn) = row.try_get::<Option<String>>("external_reference")? {
            mrns.push(mrn);
        }
    }
    Ok(Some(mrns))
}

/// The numbers of the patient's Emirates ID documents.
async fn emirates_ids_in(tx: &mut Transaction, id: i64) -> Result<Vec<String>> {
    if !table_exists(tx, "wl_documents_documents").await? {
        return Ok(Vec::new());
    }
    let rows = sql("SELECT d.number AS number FROM wl_documents_documents d \
         JOIN wl_documents_kinds k ON k.id = d.kind_id \
         WHERE k.key = ? AND d.subject_type = ? AND d.subject_id = ? AND d.number IS NOT NULL")
    .bind(crate::seed::EMIRATES_ID_KIND)
    .bind(PARTY_SUBJECT_TYPE)
    .bind(id)
    .fetch_all(&mut *tx)
    .await?;
    let mut numbers = Vec::new();
    for row in &rows {
        numbers.push(row.try_get::<String>("number")?);
    }
    Ok(numbers)
}

/// When the patient's next booking starts, if any.
async fn next_appointment_in(tx: &mut Transaction, id: i64) -> Result<Option<DateTime>> {
    if !table_exists(tx, "wl_scheduling_bookings").await? {
        return Ok(None);
    }
    sql("SELECT MIN(starts_at) FROM wl_scheduling_bookings \
         WHERE party_id = ? AND starts_at >= ? AND status IN ('requested', 'booked', 'confirmed')")
    .bind(id)
    .bind(renox::db::now())
    .scalar::<Option<DateTime>>(&mut *tx)
    .await
    .map_err(Into::into)
}

/// The name and file number of patient `id`, read in `tx`.
async fn patient_in(tx: &mut Transaction, id: Option<i64>) -> Result<PatientFile> {
    let Some(id) = id else {
        return Ok(PatientFile::default());
    };
    Ok(PatientFile::unscoped()
        .where_eq("id", id)
        .first(&mut *tx)
        .await?
        .unwrap_or_default())
}

async fn patient_document(tx: &mut Transaction, file: &PatientFile) -> Result<SearchDocument> {
    let Some(mrns) = mrns_in(tx, file.id).await? else {
        // Not a patient: a blank document no query matches.
        return Ok(SearchDocument::new("").sensitivity(Sensitivity::Personal));
    };
    let next = next_appointment_in(tx, file.id).await?;
    let secondary = match next {
        Some(at) => format!(
            "File {} · next appointment {}",
            file.code,
            wl_kernel::documents::dubai_text(at)
        ),
        None => format!("File {}", file.code),
    };
    let mut document = SearchDocument::new(&file.display_name)
        .secondary(secondary)
        .identifier(kinds::NUMBER, &file.code)
        .sensitivity(Sensitivity::Personal);
    for mrn in mrns {
        document = document.identifier(kinds::NUMBER, mrn);
    }
    for number in emirates_ids_in(tx, file.id).await? {
        document = document.identifier(kinds::EMIRATES_ID, number);
    }
    Ok(document)
}

async fn visit_document(tx: &mut Transaction, visit: &Visit) -> Result<SearchDocument> {
    let patient = patient_in(tx, visit.patient_party_id).await?;
    let number = visit.number();
    Ok(
        SearchDocument::new(format!("{number} · {}", visit.visit_date))
            .secondary(&patient.display_name)
            .identifier(kinds::NUMBER, &number)
            .identifier(kinds::NUMBER, &patient.code)
            .status(visit.status.as_str())
            .sensitivity(Sensitivity::Personal),
    )
}

async fn claim_document(tx: &mut Transaction, claim: &Claim) -> Result<SearchDocument> {
    let patient = patient_in(tx, Some(claim.patient_party_id)).await?;
    let number = claim.number();
    let mut document = SearchDocument::new(&number)
        .secondary(format!("{} · {}", patient.display_name, claim.visit_date))
        .identifier(kinds::NUMBER, &number)
        .status(claim.status.as_str())
        .voided(claim.status == ClaimStatus::Voided)
        .sensitivity(Sensitivity::Personal);
    for reference in [
        &claim.payer_reference,
        &claim.submission_reference,
        &claim.last_remittance_reference,
    ]
    .into_iter()
    .flatten()
    {
        document = document.identifier(kinds::NUMBER, reference);
    }
    Ok(document)
}

async fn authorisation_document(
    tx: &mut Transaction,
    authorisation: &PriorAuthorisation,
) -> Result<SearchDocument> {
    let patient = patient_in(tx, Some(authorisation.patient_party_id)).await?;
    let number = authorisation.number();
    let mut document = SearchDocument::new(&number)
        .secondary(&patient.display_name)
        .identifier(kinds::NUMBER, &number)
        .status(authorisation.status.as_str())
        .sensitivity(Sensitivity::Personal);
    if let Some(reference) = &authorisation.reference {
        document = document.identifier(kinds::NUMBER, reference);
    }
    Ok(document)
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<PatientFile>(PATIENT_SUBJECT_TYPE)
        .permission(permissions::VISITS_VIEW)
        .label("Patients")
        .document_in(|tx, file| Box::pin(patient_document(tx, file)))
        .register();
    Searchable::of::<Visit>(VISIT_SUBJECT_TYPE)
        .permission(permissions::VISITS_VIEW)
        .label("Appointments")
        .link("/clinics/visits/{id}")
        .document_in(|tx, visit| Box::pin(visit_document(tx, visit)))
        .register();
    Searchable::of::<Claim>(CLAIM_SUBJECT_TYPE)
        .permission(permissions::CLAIMS_MANAGE)
        .label("Insurance claims")
        .link("/clinics/claims/{id}")
        .document_in(|tx, claim| Box::pin(claim_document(tx, claim)))
        .register();
    Searchable::of::<PriorAuthorisation>(AUTHORISATION_SUBJECT_TYPE)
        .permission(permissions::POLICIES_VIEW)
        .label("Prior authorisations")
        .link("/clinics/authorisations/{id}")
        .document_in(|tx, authorisation| Box::pin(authorisation_document(tx, authorisation)))
        .register();

    // A patient: the visits, claims and authorisations of the patient.
    Relation::has_many(
        PATIENT_SUBJECT_TYPE,
        "visits",
        VISIT_SUBJECT_TYPE,
        "clinics_visits",
        "patient_party_id",
    )
    .label("Appointments")
    .register();
    Relation::has_many(
        PATIENT_SUBJECT_TYPE,
        "claims",
        CLAIM_SUBJECT_TYPE,
        "clinics_claims",
        "patient_party_id",
    )
    .label("Insurance claims")
    .register();
    Relation::has_many(
        PATIENT_SUBJECT_TYPE,
        "authorisations",
        AUTHORISATION_SUBJECT_TYPE,
        "clinics_prior_authorisations",
        "patient_party_id",
    )
    .label("Prior authorisations")
    .register();

    // A visit: its patient, its claims and the authorisations it holds.
    Relation::belongs_to(
        VISIT_SUBJECT_TYPE,
        "patient",
        PATIENT_SUBJECT_TYPE,
        "clinics_visits",
        "patient_party_id",
    )
    .label("Patient")
    .register();
    Relation::has_many(
        VISIT_SUBJECT_TYPE,
        "claims",
        CLAIM_SUBJECT_TYPE,
        "clinics_claims",
        "visit_id",
    )
    .label("Insurance claims")
    .register();
    Relation::new(
        VISIT_SUBJECT_TYPE,
        "authorisations",
        AUTHORISATION_SUBJECT_TYPE,
    )
    .label("Prior authorisations")
    .load(|db, ids| {
        Box::pin(async move {
            let placeholders = vec!["?"; ids.len()].join(", ");
            let rows = sql(format!(
                "SELECT DISTINCT visit_id, authorisation_id FROM clinics_authorisation_holds \
                     WHERE visit_id IN ({placeholders})"
            ))
            .bind_all(ids.iter().map(|id| renox::db::DbValue::Integer(*id)))
            .fetch_all(db)
            .await?;
            let mut pairs = Vec::with_capacity(rows.len());
            for row in &rows {
                let visit: i64 = row.try_get("visit_id")?;
                let authorisation: i64 = row.try_get("authorisation_id")?;
                pairs.push((
                    Subject::new(VISIT_SUBJECT_TYPE, visit),
                    Subject::new(AUTHORISATION_SUBJECT_TYPE, authorisation),
                ));
            }
            Ok(pairs)
        })
    })
    .register();

    // A claim: its visit, its patient and its payer.
    Relation::belongs_to(
        CLAIM_SUBJECT_TYPE,
        "visit",
        VISIT_SUBJECT_TYPE,
        "clinics_claims",
        "visit_id",
    )
    .label("Appointment")
    .register();
    Relation::belongs_to(
        CLAIM_SUBJECT_TYPE,
        "patient",
        PATIENT_SUBJECT_TYPE,
        "clinics_claims",
        "patient_party_id",
    )
    .label("Patient")
    .register();
    Relation::belongs_to(
        CLAIM_SUBJECT_TYPE,
        "payer",
        PARTY_SUBJECT_TYPE,
        "clinics_claims",
        "payer_party_id",
    )
    .label("Payer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // An authorisation: its patient, its policy and the visits it holds.
    Relation::belongs_to(
        AUTHORISATION_SUBJECT_TYPE,
        "patient",
        PATIENT_SUBJECT_TYPE,
        "clinics_prior_authorisations",
        "patient_party_id",
    )
    .label("Patient")
    .register();
    Relation::belongs_to(
        AUTHORISATION_SUBJECT_TYPE,
        "policy",
        POLICY_SUBJECT_TYPE,
        "clinics_prior_authorisations",
        "policy_id",
    )
    .label("Insurance policy")
    .permission(permissions::POLICIES_VIEW)
    .register();
}

/// Writes the documents of `subject` and of its patient again in `tx`.
///
/// # Errors
///
/// A database error.
pub async fn index_with_patient_in(
    tx: &mut Transaction,
    subject: &Subject,
    patient: Option<i64>,
) -> Result {
    index_in(tx, subject).await?;
    if let Some(patient) = patient {
        index_in(tx, &Subject::new(PATIENT_SUBJECT_TYPE, patient)).await?;
    }
    Ok(())
}

/// [`index_with_patient_in`] in a transaction of its own, for a record
/// saved outside one.
///
/// # Errors
///
/// A database error.
pub async fn index_with_patient(db: &Db, subject: &Subject, patient: Option<i64>) -> Result {
    let mut tx = db.begin().await?;
    index_with_patient_in(&mut tx, subject, patient).await?;
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
        index_with_patient(&state.db, &Subject::new(subject_type, event.key), None).await
    });
}

/// Keeps the documents current after transitions.
pub(crate) fn listen(app: &mut Registry) {
    reindex_after::<Visit>(app, VISIT_SUBJECT_TYPE);
    reindex_after::<Claim>(app, CLAIM_SUBJECT_TYPE);
    reindex_after::<PriorAuthorisation>(app, AUTHORISATION_SUBJECT_TYPE);
}
