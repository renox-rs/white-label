//! The construction records in the global search and the *Related* panel
//! (BL-AGT-12, decision 0006): a QS finds an IPA by its number, a project
//! by its code, a variation by its number, a guarantee by the bank's
//! guarantee number, each with its related records.
//!
//! Every document carries the project's scope (`construction.project`), so
//! a holder of `projects.view` in one project finds only that project's
//! records (02 Construction §3.2). A record is indexed in the transaction
//! that writes it ([`wl_kernel::search::index_in`]); a transition run with
//! `run` writes it again once committed ([`listen`]).

use renox::auth::permissions::Scope;
use renox::db::Transaction;
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::guarantee::{self, Guarantee, GuaranteeDocument};
use crate::ipa::{self, Ipa};
use crate::permissions;
use crate::project::{self, Project};
use crate::retention::{self, Retention};
use crate::variation::{self, Variation};

/// The subject type of a deduction line of a certificate (no page of its
/// own: shown on the IPA).
const DEDUCTION_SUBJECT_TYPE: &str = crate::deduction::SUBJECT_TYPE;

/// The permission to see parties, as the parties module names it.
const PARTIES_VIEW: &str = "parties.view";

/// The permission to see invoices, as the invoicing module names it.
const INVOICES_VIEW: &str = "invoices.view";

/// The scope of the records of project `project_id`.
fn scope_of(project_id: i64) -> Scope {
    Scope::new(project::SUBJECT_TYPE, project_id)
}

/// The project `id`, read in `tx`.
async fn project_in(tx: &mut Transaction, id: i64) -> Result<Option<Project>> {
    Project::unscoped().where_eq("id", id).first(&mut *tx).await
}

/// The number an IPA is known by: `<project code>-IPA-<n>`.
pub fn ipa_number(project_code: &str, number: i64) -> String {
    format!("{project_code}-IPA-{number}")
}

async fn project_document(_tx: &mut Transaction, project: &Project) -> Result<SearchDocument> {
    Ok(SearchDocument::new(&project.code)
        .secondary(&project.name)
        .keyword(&project.emirate)
        .identifier(kinds::NUMBER, &project.code)
        .status(project.status.as_str())
        .scope(scope_of(project.id)))
}

async fn ipa_document(tx: &mut Transaction, ipa: &Ipa) -> Result<SearchDocument> {
    let project = project_in(tx, ipa.project_id).await?.unwrap_or_default();
    let number = ipa_number(&project.code, ipa.number);
    let mut document = SearchDocument::new(&number)
        .secondary(&project.name)
        .identifier(kinds::NUMBER, &number)
        .identifier(kinds::NUMBER, format!("IPA-{}", ipa.number))
        .status(ipa.status.as_str())
        .scope(scope_of(ipa.project_id));
    if ipa.revision > 1 {
        document = document.keyword(format!("rev {}", ipa.revision));
    }
    for reference in [&ipa.certificate_ref, &ipa.late_certificate_ref]
        .into_iter()
        .flatten()
    {
        document = document.identifier(kinds::NUMBER, reference);
    }
    Ok(document)
}

async fn variation_document(tx: &mut Transaction, variation: &Variation) -> Result<SearchDocument> {
    let project = project_in(tx, variation.project_id)
        .await?
        .unwrap_or_default();
    let mut document = SearchDocument::new(format!("{} {}", project.code, variation.code))
        .secondary(&variation.description)
        .identifier(kinds::NUMBER, &variation.code)
        .identifier(
            kinds::NUMBER,
            format!("{}-{}", project.code, variation.code),
        )
        .keyword(&project.name)
        .status(variation.status.as_str())
        .scope(scope_of(variation.project_id));
    if let Some(reference) = &variation.client_reference {
        document = document.identifier(kinds::NUMBER, reference);
    }
    if let Some(instruction) = &variation.site_instruction {
        document = document.identifier(kinds::NUMBER, instruction);
    }
    Ok(document)
}

async fn guarantee_document(tx: &mut Transaction, guarantee: &Guarantee) -> Result<SearchDocument> {
    let project = project_in(tx, guarantee.project_id)
        .await?
        .unwrap_or_default();
    // The bank's guarantee number is the reference of its documents (the
    // issue, each extension and reduction); the issue's names it.
    let documents = GuaranteeDocument::query()
        .where_eq("guarantee_id", guarantee.id)
        .order_by("id")
        .get(&mut *tx)
        .await?;
    let first = documents
        .iter()
        .map(|document| document.reference.trim())
        .find(|reference| !reference.is_empty());
    let kind = guarantee.kind.as_str().replace('_', " ");
    let title = match first {
        Some(reference) => format!("{reference} · {kind}"),
        None => format!("{} {kind} guarantee", project.code),
    };
    let mut document = SearchDocument::new(title)
        .secondary(format!("{} · {}", guarantee.bank, project.name))
        .keyword(&project.code)
        .status(guarantee.status.as_str())
        .scope(scope_of(guarantee.project_id));
    for reference in &documents {
        document = document.identifier(kinds::NUMBER, reference.reference.trim());
    }
    Ok(document)
}

async fn retention_document(tx: &mut Transaction, retention: &Retention) -> Result<SearchDocument> {
    let project = project_in(tx, retention.project_id)
        .await?
        .unwrap_or_default();
    Ok(SearchDocument::new(format!(
        "{} retention {}",
        project.code,
        retention.part.as_str()
    ))
    .secondary(&project.name)
    .keyword(&project.code)
    .status(retention.status.as_str())
    .scope(scope_of(retention.project_id)))
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Project>(project::SUBJECT_TYPE)
        .permission(permissions::PROJECTS_VIEW)
        .label("Projects")
        .document_in(|tx, project| Box::pin(project_document(tx, project)))
        .register();
    Searchable::of::<Ipa>(ipa::SUBJECT_TYPE)
        .permission(permissions::PROJECTS_VIEW)
        .label("IPAs")
        .link("/construction/ipas/{id}")
        .document_in(|tx, ipa| Box::pin(ipa_document(tx, ipa)))
        .register();
    Searchable::of::<Variation>(variation::SUBJECT_TYPE)
        .permission(permissions::PROJECTS_VIEW)
        .label("Variations")
        .document_in(|tx, variation| Box::pin(variation_document(tx, variation)))
        .register();
    Searchable::of::<Guarantee>(guarantee::SUBJECT_TYPE)
        .permission(permissions::PROJECTS_VIEW)
        .label("Guarantees")
        .document_in(|tx, guarantee| Box::pin(guarantee_document(tx, guarantee)))
        .register();
    Searchable::of::<Retention>(retention::SUBJECT_TYPE)
        .permission(permissions::PROJECTS_VIEW)
        .label("Retentions")
        .document_in(|tx, retention| Box::pin(retention_document(tx, retention)))
        .register();

    // A project: its IPAs, variations, guarantees, retention and client.
    Relation::has_many(
        project::SUBJECT_TYPE,
        "ipas",
        ipa::SUBJECT_TYPE,
        "construction_ipas",
        "project_id",
    )
    .label("IPAs")
    .register();
    Relation::has_many(
        project::SUBJECT_TYPE,
        "variations",
        variation::SUBJECT_TYPE,
        "construction_variations",
        "project_id",
    )
    .label("Variations")
    .register();
    Relation::has_many(
        project::SUBJECT_TYPE,
        "guarantees",
        guarantee::SUBJECT_TYPE,
        "construction_guarantees",
        "project_id",
    )
    .label("Guarantees")
    .register();
    Relation::has_many(
        project::SUBJECT_TYPE,
        "retentions",
        retention::SUBJECT_TYPE,
        "construction_retentions",
        "project_id",
    )
    .label("Retention")
    .register();
    Relation::belongs_to(
        project::SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "construction_projects",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // An IPA: its project, the IPA it replaced, its deductions, the
    // variations it carries and its invoice.
    Relation::belongs_to(
        ipa::SUBJECT_TYPE,
        "project",
        project::SUBJECT_TYPE,
        "construction_ipas",
        "project_id",
    )
    .label("Project")
    .register();
    Relation::belongs_to(
        ipa::SUBJECT_TYPE,
        "replaces",
        ipa::SUBJECT_TYPE,
        "construction_ipas",
        "replaces_id",
    )
    .label("Replaces")
    .register();
    Relation::has_many(
        ipa::SUBJECT_TYPE,
        "deductions",
        DEDUCTION_SUBJECT_TYPE,
        "construction_ipa_deductions",
        "ipa_id",
    )
    .label("Deductions")
    .permission(permissions::PROJECTS_VIEW)
    .link("/construction/ipas/{id}")
    .register();
    Relation::has_many(
        ipa::SUBJECT_TYPE,
        "ipa_variations",
        variation::SUBJECT_TYPE,
        "construction_variations",
        "ipa_id",
    )
    .label("Variations")
    .register();
    Relation::belongs_to(
        ipa::SUBJECT_TYPE,
        "invoice",
        INVOICE_SUBJECT_TYPE,
        "construction_ipas",
        "invoice_id",
    )
    .label("Invoice")
    .permission(INVOICES_VIEW)
    .link("/invoices/{id}")
    .register();

    // A variation: its project and the IPA that carries it.
    Relation::belongs_to(
        variation::SUBJECT_TYPE,
        "project",
        project::SUBJECT_TYPE,
        "construction_variations",
        "project_id",
    )
    .label("Project")
    .register();
    Relation::belongs_to(
        variation::SUBJECT_TYPE,
        "ipa",
        ipa::SUBJECT_TYPE,
        "construction_variations",
        "ipa_id",
    )
    .label("IPA")
    .register();

    // A guarantee: its project and the retention part it replaces.
    Relation::belongs_to(
        guarantee::SUBJECT_TYPE,
        "project",
        project::SUBJECT_TYPE,
        "construction_guarantees",
        "project_id",
    )
    .label("Project")
    .register();
    Relation::has_many(
        guarantee::SUBJECT_TYPE,
        "bonded_retentions",
        retention::SUBJECT_TYPE,
        "construction_retentions",
        "bond_id",
    )
    .label("Retention it replaces")
    .register();

    // A retention part: its project, its bond and its invoice.
    Relation::belongs_to(
        retention::SUBJECT_TYPE,
        "project",
        project::SUBJECT_TYPE,
        "construction_retentions",
        "project_id",
    )
    .label("Project")
    .register();
    Relation::belongs_to(
        retention::SUBJECT_TYPE,
        "bond",
        guarantee::SUBJECT_TYPE,
        "construction_retentions",
        "bond_id",
    )
    .label("Bond")
    .register();
    Relation::belongs_to(
        retention::SUBJECT_TYPE,
        "invoice",
        INVOICE_SUBJECT_TYPE,
        "construction_retentions",
        "invoice_id",
    )
    .label("Invoice")
    .permission(INVOICES_VIEW)
    .link("/invoices/{id}")
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
    reindex_after::<Project>(app, project::SUBJECT_TYPE);
    reindex_after::<Ipa>(app, ipa::SUBJECT_TYPE);
    reindex_after::<Variation>(app, variation::SUBJECT_TYPE);
    reindex_after::<Guarantee>(app, guarantee::SUBJECT_TYPE);
    reindex_after::<Retention>(app, retention::SUBJECT_TYPE);
}
