//! Deliveries and runs in the search index and the *Related* panel
//! (BL-AGT-11, `PLT.SRC.01`, `PLT.REL.01`).
//!
//! A delivery is found by its number, by its recipient's phone (hashed) and
//! by the **tracking numbers** of the record it delivers: the numbers and
//! tracking identifiers its source declared for search (a shipment's
//! `SHP-0001`, a sales order's `SO-2026-0014`, a dispatch's courier number),
//! read from the kernel's identifier table so this module needs no other
//! module's tables. A run is found by its number and by the numbers and
//! tracking numbers of the deliveries on it, so a tracking number finds the
//! delivery and the run that carries it.
//!
//! The documents are written in the transaction that writes the record:
//! the request ([`crate::request`]), a run's creation ([`crate::plan`]), a
//! hand-over that moves deliveries ([`crate::hand_over`]), and every
//! transition of both machines ([`reindex_delivery`], [`reindex_run`]),
//! which also re-index the runs a delivery joins or leaves.

use renox::db::{Transaction, sql};
use renox::prelude::*;
use renox::state_machine::Transition;
use wl_kernel::Subject;
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, SearchIdentifierRow, Searchable, index_in, kinds};

use crate::delivery::Delivery;
use crate::permissions;
use crate::run::Run;

/// The subject type of a delivery.
pub const DELIVERY_SUBJECT_TYPE: &str = "delivery.delivery";
/// The subject type of a run.
pub const RUN_SUBJECT_TYPE: &str = "delivery.run";
/// The subject type of a delivery attempt (no page of its own).
pub const ATTEMPT_SUBJECT_TYPE: &str = "delivery.attempt";
/// The subject type of a proof of delivery (no page of its own).
pub const PROOF_SUBJECT_TYPE: &str = "delivery.proof";

/// The sources a delivery is for that have a group of their own on the
/// delivery's panel: subject type, relation name, heading, page.
const SOURCES: &[(&str, &str, &str, &str)] = &[
    (
        "sales.sales_order",
        "order",
        "Sales order",
        "/sales-orders/{id}",
    ),
    (
        "inventory.dispatch",
        "dispatch",
        "Dispatch",
        "/dispatches/{id}",
    ),
    (
        "logistics.shipment",
        "shipment",
        "Shipment",
        "/shipments/{id}",
    ),
];

/// Registers the two searchable types and the relations of both.
pub(crate) fn register() {
    Searchable::of::<Delivery>(DELIVERY_SUBJECT_TYPE)
        .permission(permissions::VIEW)
        .label("Deliveries")
        .link("/deliveries/{id}")
        .document_in(|tx, delivery| Box::pin(delivery_document(tx, delivery)))
        .register();
    Searchable::of::<Run>(RUN_SUBJECT_TYPE)
        .permission(permissions::RUNS_VIEW)
        .label("Runs")
        .link("/runs/{id}")
        .document_in(|tx, run| Box::pin(run_document(tx, run)))
        .register();

    Relation::belongs_to(
        DELIVERY_SUBJECT_TYPE,
        "run",
        RUN_SUBJECT_TYPE,
        "wl_delivery_deliveries",
        "run_id",
    )
    .label("Run")
    .register();
    Relation::has_many(
        DELIVERY_SUBJECT_TYPE,
        "attempts",
        ATTEMPT_SUBJECT_TYPE,
        "wl_delivery_attempts",
        "delivery_id",
    )
    .label("Attempts")
    .permission(permissions::VIEW)
    .register();
    Relation::has_many(
        DELIVERY_SUBJECT_TYPE,
        "proofs",
        PROOF_SUBJECT_TYPE,
        "wl_delivery_proofs",
        "delivery_id",
    )
    .label("Proof of delivery and cash collected")
    .permission(permissions::POD_VIEW)
    .register();
    for &(source_type, name, label, link) in SOURCES {
        source_relation(source_type, name, label, link);
    }
    Relation::has_many(
        RUN_SUBJECT_TYPE,
        "deliveries",
        DELIVERY_SUBJECT_TYPE,
        "wl_delivery_deliveries",
        "run_id",
    )
    .label("Deliveries")
    .register();
}

/// delivery → its source of `source_type` (none when the delivery is for
/// another kind of record). The source's module registers it for search;
/// until then the group shows to holders of `deliveries.view`.
fn source_relation(source_type: &'static str, name: &str, label: &str, link: &str) {
    Relation::new(DELIVERY_SUBJECT_TYPE, name, source_type)
        .label(label)
        .permission(permissions::VIEW)
        .link(link)
        .load(move |db, ids| {
            Box::pin(async move {
                let placeholders = vec!["?"; ids.len()].join(", ");
                let rows = sql(format!(
                    "SELECT id, source_id FROM wl_delivery_deliveries \
                     WHERE source_type = ? AND id IN ({placeholders}) ORDER BY id"
                ))
                .bind(source_type)
                .bind_all(ids.iter().map(|id| renox::db::DbValue::Integer(*id)))
                .fetch_all(db)
                .await?;
                let mut pairs = Vec::with_capacity(rows.len());
                for row in &rows {
                    let delivery: i64 = row.try_get("id")?;
                    let source: i64 = row.try_get("source_id")?;
                    pairs.push((
                        Subject::new(DELIVERY_SUBJECT_TYPE, delivery),
                        Subject::new(source_type, source),
                    ));
                }
                Ok(pairs)
            })
        })
        .register();
}

/// The numbers and tracking numbers `source` declared for search (not
/// hashed ones), as stored (normalised).
async fn source_tracking(tx: &mut Transaction, source: &Subject) -> Result<Vec<String>> {
    let rows = SearchIdentifierRow::query()
        .where_eq("subject_type", source.subject_type.as_str())
        .where_eq("subject_id", source.subject_id)
        .where_eq("hashed", false)
        .where_in(
            "kind",
            vec![kinds::NUMBER.to_owned(), kinds::TRACKING.to_owned()],
        )
        .order_by("id")
        .get(&mut *tx)
        .await?;
    Ok(rows.into_iter().map(|row| row.value).collect())
}

/// How a delivery reads in a search result.
async fn delivery_document(tx: &mut Transaction, delivery: &Delivery) -> Result<SearchDocument> {
    let source = Subject::new(&delivery.source_type, delivery.source_id);
    let mut document = SearchDocument::new(&delivery.number)
        .secondary(format!("{} · {}", delivery.recipient_name, delivery.area))
        .identifier(kinds::NUMBER, &delivery.number)
        .identifier(kinds::TRACKING, &delivery.number)
        .identifier(kinds::PHONE, &delivery.recipient_phone)
        .keyword(&delivery.address)
        .status(delivery.status.as_str());
    for tracking in source_tracking(tx, &source).await? {
        document = document.identifier(kinds::TRACKING, tracking);
    }
    Ok(document)
}

/// How a run reads in a search result: its deliveries' numbers and
/// tracking numbers find it too.
async fn run_document(tx: &mut Transaction, run: &Run) -> Result<SearchDocument> {
    let mut document = SearchDocument::new(&run.number)
        .secondary(format!("{} · {}", run.depot, run.run_date))
        .identifier(kinds::NUMBER, &run.number)
        .keyword(&run.depot)
        .status(run.status.as_str());
    let deliveries = Delivery::query()
        .where_(Delivery::RUN_ID.eq(run.id))
        .order_by(Delivery::ID)
        .get(&mut *tx)
        .await?;
    for delivery in &deliveries {
        document = document.identifier(kinds::TRACKING, &delivery.number);
        let source = Subject::new(&delivery.source_type, delivery.source_id);
        for tracking in source_tracking(tx, &source).await? {
            document = document.identifier(kinds::TRACKING, tracking);
        }
    }
    Ok(document)
}

/// Writes the document of delivery `id`, of the run it is on now and of
/// `previous_run` (the run it was on before the change), in `tx`.
pub(crate) async fn index_delivery_in(
    tx: &mut Transaction,
    id: i64,
    previous_run: Option<i64>,
) -> Result {
    index_in(tx, &Subject::new(DELIVERY_SUBJECT_TYPE, id)).await?;
    let current_run = Delivery::query()
        .where_(Delivery::ID.eq(id))
        .first(&mut *tx)
        .await?
        .and_then(|delivery| delivery.run_id);
    if let Some(run) = current_run {
        index_in(tx, &Subject::new(RUN_SUBJECT_TYPE, run)).await?;
    }
    if let Some(run) = previous_run.filter(|run| Some(*run) != current_run) {
        index_in(tx, &Subject::new(RUN_SUBJECT_TYPE, run)).await?;
    }
    Ok(())
}

/// Writes the document of run `id` in `tx`.
pub(crate) async fn index_run_in(tx: &mut Transaction, id: i64) -> Result {
    index_in(tx, &Subject::new(RUN_SUBJECT_TYPE, id)).await?;
    Ok(())
}

/// `transition`, writing the delivery's document (and its runs') after its
/// own effects, in its transaction.
pub(crate) fn reindex_delivery(transition: Transition<Delivery>) -> Transition<Delivery> {
    transition.effect(|delivery: &Delivery, _ctx, tx| {
        let id = delivery.id;
        let previous_run = delivery.run_id;
        Box::pin(async move { index_delivery_in(tx, id, previous_run).await })
    })
}

/// `transition`, writing the run's document after its own effects, in its
/// transaction.
pub(crate) fn reindex_run(transition: Transition<Run>) -> Transition<Run> {
    transition.effect(|run: &Run, _ctx, tx| {
        let id = run.id;
        Box::pin(async move { index_run_in(tx, id).await })
    })
}
