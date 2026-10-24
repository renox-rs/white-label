//! The F&B records in the global search and the *Related* panel (BL-AGT-13,
//! decision 0006): an outlet manager finds an order by its own number or by
//! the aggregator's order number, a head chef a production batch by its
//! number, the accountant an aggregator payout by its number or the
//! aggregator's reference, each with its related records.
//!
//! An order and a batch carry their outlet's scope (`fnb.outlet`), so a
//! manager of one outlet finds that outlet's records only. The customer's
//! phone is personal: stored only as a hash. The aggregator's order number
//! is the external number of the order's sales order (wl-sales), read in
//! the indexing transaction when that module's table exists.

use renox::auth::permissions::Scope;
use renox::db::{DbValue, Dialect, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transitioned};
use wl_kernel::Subject;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::search::{SearchDocument, Searchable, index_in, kinds};

use crate::batch::{self, BatchStatus, ProductionBatch};
use crate::catalogue::OUTLET_SUBJECT_TYPE;
use crate::order::{self, FnbOrder};
use crate::payout::{self, PayoutStatement, PayoutStatus};
use crate::permissions;

const PARTIES_VIEW: &str = "parties.view";

/// The subject type of a stock lot (wl-inventory).
const LOT_SUBJECT_TYPE: &str = "inventory.lot";

/// The kind of identifier of an aggregator's order number.
pub const AGGREGATOR_ORDER: &str = "aggregator_order";

/// The scope of a record of outlet `outlet`, global without one.
fn scope_of(outlet: Option<i64>) -> Scope {
    outlet.map_or_else(Scope::global, |id| Scope::new(OUTLET_SUBJECT_TYPE, id))
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

async fn order_document(tx: &mut Transaction, order: &FnbOrder) -> Result<SearchDocument> {
    let mut document = SearchDocument::new(&order.number)
        .secondary(format!("{} · {}", order.channel, order.kind))
        .keyword(&order.area)
        .identifier(kinds::NUMBER, &order.number)
        .identifier(kinds::PHONE, &order.recipient_phone)
        .status(order.status.as_str())
        .scope(scope_of(order.outlet_id));
    if table_exists(tx, "wl_sales_sales_orders").await? {
        let rows = sql("SELECT number, external_number FROM wl_sales_sales_orders WHERE id = ?")
            .bind(order.sales_order_id)
            .fetch_all(&mut *tx)
            .await?;
        for row in &rows {
            document = document.identifier(kinds::NUMBER, row.try_get::<String>("number")?);
            if let Some(external) = row.try_get::<Option<String>>("external_number")? {
                document = document
                    .identifier(AGGREGATOR_ORDER, &external)
                    .keyword(external);
            }
        }
    }
    Ok(document)
}

fn batch_document(batch: &ProductionBatch) -> SearchDocument {
    let mut document = SearchDocument::new(&batch.number)
        .identifier(kinds::NUMBER, &batch.number)
        .status(batch.status.as_str())
        .voided(batch.status == BatchStatus::Discarded)
        .scope(scope_of(batch.outlet_id));
    if let Some(expiry) = batch.expiry_date {
        document = document.secondary(format!("Use by {expiry}"));
    }
    document
}

fn payout_document(statement: &PayoutStatement) -> SearchDocument {
    SearchDocument::new(&statement.number)
        .secondary(format!(
            "{} · {}",
            statement.aggregator, statement.reference
        ))
        .identifier(kinds::NUMBER, &statement.number)
        .identifier(kinds::NUMBER, &statement.reference)
        .status(statement.status.as_str())
        .voided(statement.status == PayoutStatus::Voided)
}

/// The `(from, to)` pairs of the payout lines, column `by` to column
/// `other`.
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
            "SELECT DISTINCT {by} AS from_id, {other} AS to_id FROM fnb_payout_lines \
             WHERE {other} IS NOT NULL AND {by} IN ({placeholders})"
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
    Searchable::of::<FnbOrder>(order::SUBJECT_TYPE)
        .permission(permissions::ORDERS_VIEW)
        .label("Orders")
        .link("/orders/{id}")
        .document_in(|tx, order| Box::pin(order_document(tx, order)))
        .register();
    Searchable::of::<ProductionBatch>(batch::SUBJECT_TYPE)
        .permission(permissions::PRODUCTION_VIEW)
        .label("Production batches")
        .link("/batches/{id}")
        .document(batch_document)
        .register();
    Searchable::of::<PayoutStatement>(payout::SUBJECT_TYPE)
        .permission(permissions::RECONCILIATION_VIEW)
        .label("Aggregator payouts")
        .link("/payouts/{id}")
        .document(payout_document)
        .register();

    // An order: the payouts that paid it and its customer.
    Relation::new(order::SUBJECT_TYPE, "payouts", payout::SUBJECT_TYPE)
        .label("Aggregator payouts")
        .load(|db, ids| {
            line_pairs(
                db,
                ids,
                "order_id",
                "statement_id",
                order::SUBJECT_TYPE,
                payout::SUBJECT_TYPE,
            )
        })
        .register();
    Relation::belongs_to(
        order::SUBJECT_TYPE,
        "customer",
        PARTY_SUBJECT_TYPE,
        "fnb_orders",
        "party_id",
    )
    .label("Customer")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();

    // A batch: the lot it made.
    Relation::belongs_to(
        batch::SUBJECT_TYPE,
        "lot",
        LOT_SUBJECT_TYPE,
        "fnb_batches",
        "lot_id",
    )
    .label("Lot")
    .permission(permissions::PRODUCTION_VIEW)
    .register();

    // A payout: the orders it pays.
    Relation::new(payout::SUBJECT_TYPE, "orders", order::SUBJECT_TYPE)
        .label("Orders")
        .load(|db, ids| {
            line_pairs(
                db,
                ids,
                "statement_id",
                "order_id",
                payout::SUBJECT_TYPE,
                order::SUBJECT_TYPE,
            )
        })
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
    reindex_after::<FnbOrder>(app, order::SUBJECT_TYPE);
    reindex_after::<ProductionBatch>(app, batch::SUBJECT_TYPE);
    reindex_after::<PayoutStatement>(app, payout::SUBJECT_TYPE);
}
