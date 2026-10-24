//! The logistics records in the global search and the *Related* panel
//! (BL-AGT-12, decision 0006): a dispatcher finds a shipment by its number,
//! the client's reference or a parcel's barcode, a documentation officer a
//! freight job by its number, AWB / B/L or container number, a cashier a COD
//! remittance by its number, each with its related records.
//!
//! A claim is a ticket of the category *claim* (wl-ticketing): its number
//! finds the shipment it is about, and the shipment's panel links the ticket;
//! the ticket itself becomes a result once the ticketing module registers
//! its type (BL-AGT-04).
//!
//! The recipient's phone is personal: stored only as a hash
//! (`kinds::PHONE`), matched on the exact number. A container number used on
//! several jobs over time finds them all, the latest first (the index orders
//! equal matches by their last change).

use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::payments::PAYMENT_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};
use wl_kernel::tickets::TICKET_SUBJECT_TYPE;

use crate::freight::{self, FreightContainer, FreightJob};
use crate::permissions;
use crate::remittance::{self, Remittance};
use crate::shipment::{self, Parcel, Shipment, ShipmentStatus};

const PARTIES_VIEW: &str = "parties.view";
const INVOICES_VIEW: &str = "invoices.view";
const PAYMENTS_VIEW: &str = "payments.view";
const TICKETS_VIEW: &str = "tickets.view";

/// The subject type of a freight container (a row of the job).
pub const CONTAINER_SUBJECT_TYPE: &str = "logistics.freight_container";

async fn shipment_document(tx: &mut Transaction, shipment: &Shipment) -> Result<SearchDocument> {
    let parcels = Parcel::query()
        .where_eq("shipment_id", shipment.id)
        .order_by("id")
        .get(&mut *tx)
        .await?;
    let mut document = SearchDocument::new(&shipment.number)
        .secondary(&shipment.area)
        .identifier(kinds::NUMBER, &shipment.number)
        .identifier(kinds::PHONE, &shipment.recipient_phone)
        .status(shipment.status.as_str())
        .voided(shipment.status == ShipmentStatus::Voided);
    for reference in [&shipment.client_reference, &shipment.claim_ticket_number]
        .into_iter()
        .flatten()
    {
        document = document.identifier(kinds::NUMBER, reference);
    }
    for parcel in &parcels {
        document = document.identifier(kinds::BARCODE, &parcel.barcode);
    }
    Ok(document)
}

async fn job_document(tx: &mut Transaction, job: &FreightJob) -> Result<SearchDocument> {
    let containers = FreightContainer::query()
        .where_eq("job_id", job.id)
        .order_by("id")
        .get(&mut *tx)
        .await?;
    let mut document = SearchDocument::new(&job.number)
        .secondary(format!("{} → {}", job.origin, job.destination))
        .keyword(&job.description)
        .keyword(&job.mode)
        .identifier(kinds::NUMBER, &job.number)
        .status(job.status.as_str());
    if let Some(waybill) = &job.bl_awb {
        document = document.identifier(kinds::TRACKING, waybill);
    }
    for reference in [&job.declaration_no, &job.delivery_order_ref]
        .into_iter()
        .flatten()
    {
        document = document.identifier(kinds::NUMBER, reference);
    }
    for container in &containers {
        document = document.identifier(kinds::TRACKING, &container.number);
    }
    Ok(document)
}

fn remittance_document(remittance: &Remittance) -> SearchDocument {
    let mut document = SearchDocument::new(&remittance.number)
        .secondary(remittance.period_end.date_naive().to_string())
        .identifier(kinds::NUMBER, &remittance.number)
        .status(remittance.status.as_str());
    for reference in [&remittance.payment_reference, &remittance.payment_number]
        .into_iter()
        .flatten()
    {
        document = document.identifier(kinds::NUMBER, reference);
    }
    document
}

/// The `(from, to)` pairs of the remittance lines: the shipment column
/// `from` and the remittance column `to`, or the reverse.
fn line_pairs<'a>(
    db: &'a Db,
    ids: &'a [i64],
    by: &'static str,
    other: &'static str,
    from_type: &'static str,
    to_type: &'static str,
) -> LoadFuture<'a> {
    Box::pin(async move {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let rows = sql(format!(
            "SELECT {by} AS from_id, {other} AS to_id FROM logistics_remittance_lines \
             WHERE {by} IN ({placeholders}) ORDER BY id"
        ))
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
    Searchable::of::<Shipment>(shipment::SUBJECT_TYPE)
        .permission(permissions::SHIPMENTS_VIEW)
        .label("Shipments")
        .link("/shipments/{id}")
        .document_in(|tx, shipment| Box::pin(shipment_document(tx, shipment)))
        .register();
    Searchable::of::<FreightJob>(freight::SUBJECT_TYPE)
        .permission(permissions::JOBS_VIEW)
        .label("Freight jobs")
        .link("/freight/{id}")
        .document_in(|tx, job| Box::pin(job_document(tx, job)))
        .register();
    Searchable::of::<Remittance>(remittance::SUBJECT_TYPE)
        .permission(permissions::REMITTANCES_VIEW)
        .label("COD remittances")
        .link("/remittances/{id}")
        .document(remittance_document)
        .register();

    // A shipment: its parcels, its remittances, the shipment it returns or
    // its return, its claim and review tickets, the claim invoice, its client.
    Relation::has_many(
        shipment::SUBJECT_TYPE,
        "parcels",
        shipment::PARCEL_SUBJECT_TYPE,
        "logistics_parcels",
        "shipment_id",
    )
    .label("Parcels")
    .permission(permissions::SHIPMENTS_VIEW)
    .register();
    Relation::new(
        shipment::SUBJECT_TYPE,
        "remittances",
        remittance::SUBJECT_TYPE,
    )
    .label("COD remittances")
    .load(|db, ids| {
        line_pairs(
            db,
            ids,
            "shipment_id",
            "remittance_id",
            shipment::SUBJECT_TYPE,
            remittance::SUBJECT_TYPE,
        )
    })
    .register();
    Relation::belongs_to(
        shipment::SUBJECT_TYPE,
        "return_of",
        shipment::SUBJECT_TYPE,
        "logistics_shipments",
        "return_of_id",
    )
    .label("Return of")
    .register();
    Relation::has_many(
        shipment::SUBJECT_TYPE,
        "returns",
        shipment::SUBJECT_TYPE,
        "logistics_shipments",
        "return_of_id",
    )
    .label("Return shipments")
    .register();
    Relation::belongs_to(
        shipment::SUBJECT_TYPE,
        "claim",
        TICKET_SUBJECT_TYPE,
        "logistics_shipments",
        "claim_ticket_id",
    )
    .label("Claim")
    .permission(TICKETS_VIEW)
    .link("/tickets/{id}")
    .register();
    Relation::belongs_to(
        shipment::SUBJECT_TYPE,
        "review",
        TICKET_SUBJECT_TYPE,
        "logistics_shipments",
        "review_ticket_id",
    )
    .label("Review")
    .permission(TICKETS_VIEW)
    .link("/tickets/{id}")
    .register();
    Relation::belongs_to(
        shipment::SUBJECT_TYPE,
        "claim_invoice",
        INVOICE_SUBJECT_TYPE,
        "logistics_shipments",
        "claim_invoice_id",
    )
    .label("Claim invoice")
    .permission(INVOICES_VIEW)
    .link("/invoices/{id}")
    .register();
    Relation::belongs_to(
        shipment::SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "logistics_shipments",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A freight job: its containers and its client.
    Relation::has_many(
        freight::SUBJECT_TYPE,
        "containers",
        CONTAINER_SUBJECT_TYPE,
        "logistics_freight_containers",
        "job_id",
    )
    .label("Containers")
    .permission(permissions::JOBS_VIEW)
    .register();
    Relation::belongs_to(
        freight::SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "logistics_freight_jobs",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A remittance: its shipments, its payment and its client.
    Relation::new(
        remittance::SUBJECT_TYPE,
        "shipments",
        shipment::SUBJECT_TYPE,
    )
    .label("Shipments")
    .load(|db, ids| {
        line_pairs(
            db,
            ids,
            "remittance_id",
            "shipment_id",
            remittance::SUBJECT_TYPE,
            shipment::SUBJECT_TYPE,
        )
    })
    .register();
    Relation::belongs_to(
        remittance::SUBJECT_TYPE,
        "payment",
        PAYMENT_SUBJECT_TYPE,
        "logistics_remittances",
        "payment_id",
    )
    .label("Payment")
    .permission(PAYMENTS_VIEW)
    .link("/payments/{id}")
    .register();
    Relation::belongs_to(
        remittance::SUBJECT_TYPE,
        "client",
        PARTY_SUBJECT_TYPE,
        "logistics_remittances",
        "client_party_id",
    )
    .label("Client")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
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
    reindex_after::<Shipment>(app, shipment::SUBJECT_TYPE);
    reindex_after::<FreightJob>(app, freight::SUBJECT_TYPE);
    reindex_after::<Remittance>(app, remittance::SUBJECT_TYPE);
}
