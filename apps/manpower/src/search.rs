//! The agency's records in the global search and the *Related* panel
//! (BL-AGT-13, decision 0006): an account manager finds a client request by
//! its number, a deployment by its number, a client contract by its
//! reference and a site by its name, each with its related records.
//!
//! Every document carries the client company's scope (`manpower.client`),
//! as the agency's permissions do (09 §3.2), so a holder in one client finds
//! that client's records only. **No worker's identity** is indexed: a
//! deployment's document names the role and the site, never the worker's
//! passport, Emirates ID, visa or labour card numbers, and the worker's
//! documents are no part of it.

use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::contract::{self, ClientContract, ContractStatus};
use crate::deployment::{self, Deployment, DeploymentStatus};
use crate::permissions::{self, client_scope};
use crate::request::{self, ManpowerRequest, RequestStatus};
use crate::site::{self, Site};

const PARTIES_VIEW: &str = "parties.view";

/// The name of site `id`, read in `tx`.
async fn site_name_in(tx: &mut Transaction, id: i64) -> Result<String> {
    Ok(Site::unscoped()
        .where_eq("id", id)
        .first(&mut *tx)
        .await?
        .map(|site| site.name)
        .unwrap_or_default())
}

/// The reference of contract `id`, read in `tx`.
async fn contract_reference_in(tx: &mut Transaction, id: i64) -> Result<String> {
    Ok(ClientContract::unscoped()
        .where_eq("id", id)
        .first(&mut *tx)
        .await?
        .map(|contract| contract.reference)
        .unwrap_or_default())
}

async fn request_document(
    tx: &mut Transaction,
    request: &ManpowerRequest,
) -> Result<SearchDocument> {
    let site = site_name_in(tx, request.site_id).await?;
    let reference = contract_reference_in(tx, request.client_contract_id).await?;
    let number = request.number();
    let mut document =
        SearchDocument::new(format!("{number} · {}", request.role.replace('_', " ")))
            .secondary(format!(
                "{site} · {} from {}",
                request.headcount, request.start_date
            ))
            .keyword(&reference)
            .identifier(kinds::NUMBER, &number)
            .status(request.status.as_str())
            .voided(request.status == RequestStatus::Cancelled)
            .scope(client_scope(request.client_party_id));
    if let Some(reference) = &request.cause_ref {
        document = document.identifier(kinds::NUMBER, reference);
    }
    Ok(document)
}

async fn deployment_document(
    tx: &mut Transaction,
    deployment: &Deployment,
) -> Result<SearchDocument> {
    let site = site_name_in(tx, deployment.site_id).await?;
    let reference = contract_reference_in(tx, deployment.client_contract_id).await?;
    let number = deployment.number();
    Ok(
        SearchDocument::new(format!("{number} · {}", deployment.role.replace('_', " ")))
            .secondary(format!("{site} · from {}", deployment.start_date))
            .keyword(&reference)
            .identifier(kinds::NUMBER, &number)
            .status(deployment.status.as_str())
            .voided(deployment.status == DeploymentStatus::Cancelled)
            .scope(client_scope(deployment.client_party_id)),
    )
}

fn contract_document(contract: &ClientContract) -> SearchDocument {
    SearchDocument::new(&contract.reference)
        .secondary(format!("{} to {}", contract.start_date, contract.end_date))
        .identifier(kinds::NUMBER, &contract.reference)
        .status(contract.status.as_str())
        .voided(contract.status == ContractStatus::Cancelled)
        .scope(client_scope(contract.client_party_id))
}

fn site_document(site: &Site) -> SearchDocument {
    let mut document = SearchDocument::new(&site.name)
        .status(if site.active { "active" } else { "inactive" })
        .scope(client_scope(site.client_party_id));
    if let Some(address) = &site.address {
        document = document.secondary(address);
    }
    document
}

/// The `(from, to)` pairs of `statement` (ending in `IN `, selecting
/// `from_id` and `to_id`) for `ids`.
fn pairs_of<'a>(
    db: &'a Db,
    ids: &'a [i64],
    statement: &'static str,
    from_type: &'static str,
    to_type: &'static str,
) -> LoadFuture<'a> {
    Box::pin(async move {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let rows = sql(format!("{statement}({placeholders})"))
            .bind_all(ids.iter().map(|id| DbValue::Integer(*id)))
            .fetch_all(db)
            .await?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in &rows {
            let from: i64 = row.try_get("from_id")?;
            let to: i64 = row.try_get("to_id")?;
            pairs.push((Subject::new(from_type, from), Subject::new(to_type, to)));
        }
        Ok(pairs)
    })
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<ManpowerRequest>(request::SUBJECT_TYPE)
        .permission(permissions::REQUESTS_VIEW)
        .label("Client requests")
        .link("/manpower/requests/{id}")
        .document_in(|tx, request| Box::pin(request_document(tx, request)))
        .register();
    Searchable::of::<Deployment>(deployment::SUBJECT_TYPE)
        .permission(permissions::DEPLOYMENTS_VIEW)
        .label("Deployments")
        .link("/manpower/deployments/{id}")
        .document_in(|tx, deployment| Box::pin(deployment_document(tx, deployment)))
        .register();
    Searchable::of::<ClientContract>(contract::SUBJECT_TYPE)
        .permission(permissions::CONTRACTS_VIEW)
        .label("Client contracts")
        .link("/manpower/contracts/{id}")
        .document(contract_document)
        .register();
    Searchable::of::<Site>(site::SUBJECT_TYPE)
        .permission(permissions::CONTRACTS_VIEW)
        .label("Sites")
        .document(site_document)
        .register();

    // A contract: its requests, deployments, the client's sites and client.
    Relation::has_many(
        contract::SUBJECT_TYPE,
        "requests",
        request::SUBJECT_TYPE,
        "manpower_requests",
        "client_contract_id",
    )
    .label("Client requests")
    .register();
    Relation::has_many(
        contract::SUBJECT_TYPE,
        "deployments",
        deployment::SUBJECT_TYPE,
        "manpower_deployments",
        "client_contract_id",
    )
    .label("Deployments")
    .register();
    Relation::new(contract::SUBJECT_TYPE, "sites", site::SUBJECT_TYPE)
        .label("Sites")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT c.id AS from_id, s.id AS to_id FROM manpower_client_contracts c \
                 JOIN manpower_sites s ON s.client_party_id = c.client_party_id WHERE c.id IN ",
                contract::SUBJECT_TYPE,
                site::SUBJECT_TYPE,
            )
        })
        .register();
    Relation::belongs_to(
        contract::SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "manpower_client_contracts",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A request: its contract, its site and the deployments that fill it.
    Relation::belongs_to(
        request::SUBJECT_TYPE,
        "contract",
        contract::SUBJECT_TYPE,
        "manpower_requests",
        "client_contract_id",
    )
    .label("Client contract")
    .register();
    Relation::belongs_to(
        request::SUBJECT_TYPE,
        "site",
        site::SUBJECT_TYPE,
        "manpower_requests",
        "site_id",
    )
    .label("Site")
    .register();
    Relation::has_many(
        request::SUBJECT_TYPE,
        "deployments",
        deployment::SUBJECT_TYPE,
        "manpower_deployments",
        "request_id",
    )
    .label("Deployments")
    .register();
    Relation::belongs_to(
        request::SUBJECT_TYPE,
        "replaces",
        deployment::SUBJECT_TYPE,
        "manpower_requests",
        "replaces_deployment_id",
    )
    .label("Deployment it covers")
    .register();

    // A deployment: its request, contract and site, and the one it covers.
    Relation::belongs_to(
        deployment::SUBJECT_TYPE,
        "request",
        request::SUBJECT_TYPE,
        "manpower_deployments",
        "request_id",
    )
    .label("Client request")
    .register();
    Relation::belongs_to(
        deployment::SUBJECT_TYPE,
        "contract",
        contract::SUBJECT_TYPE,
        "manpower_deployments",
        "client_contract_id",
    )
    .label("Client contract")
    .register();
    Relation::belongs_to(
        deployment::SUBJECT_TYPE,
        "site",
        site::SUBJECT_TYPE,
        "manpower_deployments",
        "site_id",
    )
    .label("Site")
    .register();
    Relation::belongs_to(
        deployment::SUBJECT_TYPE,
        "covers",
        deployment::SUBJECT_TYPE,
        "manpower_deployments",
        "covers_deployment_id",
    )
    .label("Deployment it covers")
    .register();

    // A site: its requests and deployments, and the client.
    Relation::has_many(
        site::SUBJECT_TYPE,
        "requests",
        request::SUBJECT_TYPE,
        "manpower_requests",
        "site_id",
    )
    .label("Client requests")
    .register();
    Relation::has_many(
        site::SUBJECT_TYPE,
        "deployments",
        deployment::SUBJECT_TYPE,
        "manpower_deployments",
        "site_id",
    )
    .label("Deployments")
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
    reindex_after::<ManpowerRequest>(app, request::SUBJECT_TYPE);
    reindex_after::<Deployment>(app, deployment::SUBJECT_TYPE);
    reindex_after::<ClientContract>(app, contract::SUBJECT_TYPE);
}
