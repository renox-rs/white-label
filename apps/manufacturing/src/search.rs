//! The manufacturing records in the global search and the *Related* panel
//! (BL-AGT-13, decision 0006): a planner finds a work order by its number or
//! the customer's PO number, QC an NCR by its number or the lot's, a
//! returned-goods inspection by the dispatch's number or the customer's PO
//! number, each with its related records.
//!
//! A work order and an NCR carry their plant's scope (`manufacturing.plant`),
//! so a holder of the plant only finds that plant's records. The customer's
//! PO number is the one of the sales order (wl-sales) the work order or the
//! returned dispatch fulfils, read in the indexing transaction when that
//! module's table exists.

use renox::auth::permissions::Scope;
use renox::db::{DbValue, Dialect, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::late_inspection::INSPECTION_SUBJECT_TYPE;
use crate::ncr::{self, Ncr, NcrStatus};
use crate::permissions::{self, PLANT_SCOPE_TYPE};
use crate::returns::ReturnInspection;
use crate::work_order::{self, WorkOrder, WorkOrderStatus};

const PARTIES_VIEW: &str = "parties.view";

/// The kind of identifier of a customer's purchase order number.
pub const CUSTOMER_PO: &str = "customer_po";

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

/// The number and customer PO of sales order `id`, when wl-sales is on.
async fn sales_order_in(
    tx: &mut Transaction,
    id: Option<i64>,
) -> Result<(Option<String>, Option<String>)> {
    let Some(id) = id else {
        return Ok((None, None));
    };
    if !table_exists(tx, "wl_sales_sales_orders").await? {
        return Ok((None, None));
    }
    let rows = sql("SELECT number, customer_po FROM wl_sales_sales_orders WHERE id = ?")
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
    match rows.first() {
        Some(row) => Ok((
            Some(row.try_get::<String>("number")?),
            row.try_get::<Option<String>>("customer_po")?,
        )),
        None => Ok((None, None)),
    }
}

async fn work_order_document(tx: &mut Transaction, order: &WorkOrder) -> Result<SearchDocument> {
    let (sales_number, po) = sales_order_in(tx, order.sales_order_id).await?;
    let mut document = SearchDocument::new(&order.number)
        .identifier(kinds::NUMBER, &order.number)
        .status(order.status.as_str())
        .voided(order.status == WorkOrderStatus::Cancelled)
        .scope(Scope::new(PLANT_SCOPE_TYPE, order.plant_id));
    if let Some(start) = order.planned_start {
        document = document.secondary(format!("Planned from {start}"));
    }
    if let Some(number) = sales_number {
        document = document.keyword(&number).identifier(kinds::NUMBER, number);
    }
    if let Some(po) = po {
        document = document.identifier(CUSTOMER_PO, po);
    }
    Ok(document)
}

fn ncr_document(ncr: &Ncr) -> SearchDocument {
    let mut document = SearchDocument::new(&ncr.number)
        .secondary(&ncr.defect)
        .identifier(kinds::NUMBER, &ncr.number)
        .status(ncr.status.as_str())
        .voided(ncr.status == NcrStatus::Cancelled)
        .scope(Scope::new(PLANT_SCOPE_TYPE, ncr.plant_id));
    if let Some(lot) = &ncr.lot_number {
        document = document.identifier(kinds::LOT, lot);
    }
    document
}

async fn inspection_document(
    tx: &mut Transaction,
    inspection: &ReturnInspection,
) -> Result<SearchDocument> {
    let (sales_number, po) = sales_order_in(tx, Some(inspection.order_id)).await?;
    let mut document = SearchDocument::new(format!("Return of {}", inspection.dispatch_number))
        .identifier(kinds::NUMBER, &inspection.dispatch_number)
        .status(&inspection.status);
    if let Some(reason) = &inspection.reason {
        document = document.secondary(reason);
    }
    if let Some(number) = sales_number {
        document = document.identifier(kinds::NUMBER, number);
    }
    if let Some(po) = po {
        document = document.identifier(CUSTOMER_PO, po);
    }
    Ok(document)
}

/// Declares the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<WorkOrder>(work_order::SUBJECT_TYPE)
        .permission(permissions::WORK_ORDERS_VIEW)
        .label("Work orders")
        .link("/manufacturing/work-orders/{id}")
        .document_in(|tx, order| Box::pin(work_order_document(tx, order)))
        .register();
    Searchable::of::<Ncr>(ncr::SUBJECT_TYPE)
        .permission(permissions::NCRS_VIEW)
        .label("NCRs")
        .link("/manufacturing/ncrs/{id}")
        .document(ncr_document)
        .register();
    Searchable::of::<ReturnInspection>(INSPECTION_SUBJECT_TYPE)
        .permission(permissions::INSPECTIONS_VIEW)
        .label("Return inspections")
        .document_in(|tx, inspection| Box::pin(inspection_document(tx, inspection)))
        .register();

    // A work order: the NCRs raised on it and the NCRs it reworks.
    Relation::new(work_order::SUBJECT_TYPE, "ncrs", ncr::SUBJECT_TYPE)
        .label("NCRs")
        .load(|db, ids| {
            Box::pin(async move {
                let placeholders = vec!["?"; ids.len()].join(", ");
                let mut values = vec![DbValue::Text(work_order::SUBJECT_TYPE.to_owned())];
                values.extend(ids.iter().map(|id| DbValue::Integer(*id)));
                let rows = sql(format!(
                    "SELECT source_id AS from_id, id AS to_id FROM manufacturing_ncrs \
                     WHERE source_type = ? AND source_id IN ({placeholders}) ORDER BY id"
                ))
                .bind_all(values)
                .fetch_all(db)
                .await?;
                let mut pairs = Vec::with_capacity(rows.len());
                for row in &rows {
                    let from: i64 = row.try_get("from_id")?;
                    let to: i64 = row.try_get("to_id")?;
                    pairs.push((
                        Subject::new(work_order::SUBJECT_TYPE, from),
                        Subject::new(ncr::SUBJECT_TYPE, to),
                    ));
                }
                Ok(pairs)
            })
        })
        .register();
    Relation::has_many(
        work_order::SUBJECT_TYPE,
        "reworked_ncrs",
        ncr::SUBJECT_TYPE,
        "manufacturing_ncrs",
        "rework_work_order_id",
    )
    .label("NCRs it reworks")
    .register();

    // An NCR: its rework order, its supplier and the inspections behind it.
    Relation::belongs_to(
        ncr::SUBJECT_TYPE,
        "rework_order",
        work_order::SUBJECT_TYPE,
        "manufacturing_ncrs",
        "rework_work_order_id",
    )
    .label("Rework order")
    .register();
    Relation::belongs_to(
        ncr::SUBJECT_TYPE,
        "supplier",
        PARTY_SUBJECT_TYPE,
        "manufacturing_ncrs",
        "supplier_party_id",
    )
    .label("Supplier")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::has_many(
        ncr::SUBJECT_TYPE,
        "inspections",
        INSPECTION_SUBJECT_TYPE,
        "manufacturing_return_inspections",
        "ncr_id",
    )
    .label("Return inspections")
    .register();

    // A return inspection: the NCR it raised.
    Relation::belongs_to(
        INSPECTION_SUBJECT_TYPE,
        "ncr",
        ncr::SUBJECT_TYPE,
        "manufacturing_return_inspections",
        "ncr_id",
    )
    .label("NCR")
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
    reindex_after::<WorkOrder>(app, work_order::SUBJECT_TYPE);
    reindex_after::<Ncr>(app, ncr::SUBJECT_TYPE);
}
