//! Leads, sales orders, counter sales, sales returns and packages in the
//! search index and the *Related* panel (BL-AGT-11, `PLT.SRC.01`,
//! `PLT.REL.01`).
//!
//! Identifiers: the order, sale and return numbers, an order's customer PO
//! and external (channel) number. The customer's name is the title of the
//! party's search document (the parties module writes it). No amount is
//! put in a document. An order's *Related* panel shows its customer,
//! invoice, returns and (declared by those modules from their own tables)
//! its deliveries and dispatches.
//!
//! The documents are written in the transaction that creates the record and
//! by every transition of the five machines ([`reindex`]); the demo rows
//! the seed inserts come in with the rebuild the demo seeds end with.

use renox::auth::permissions::Scope;
use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::{StateMachine, Transition};
use wl_kernel::Subject;
use wl_kernel::invoicing::{CREDIT_NOTE_SUBJECT_TYPE, INVOICE_SUBJECT_TYPE};
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::related::{LoadFuture, Relation};
use wl_kernel::sales::{
    COUNTER_SALE_SUBJECT_TYPE, LEAD_SUBJECT_TYPE, PACKAGE_SUBJECT_TYPE, SALES_ORDER_SUBJECT_TYPE,
    SALES_RETURN_SUBJECT_TYPE, TILL_SESSION_SUBJECT_TYPE,
};
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, index_in, kinds};

use crate::counter_sale::CounterSale;
use crate::lead::Lead;
use crate::package::Package;
use crate::permissions;
use crate::sales_order::SalesOrder;
use crate::sales_return::SalesReturn;

/// The subject type of a package movement (reached through its package).
pub const PACKAGE_MOVEMENT_SUBJECT_TYPE: &str = "sales.package_movement";

/// The permission to see parties, as the parties module names it.
const PARTIES_VIEW: &str = "parties.view";
/// The permission to see invoices and credit notes, as the invoicing
/// module names it.
const INVOICES_VIEW: &str = "invoices.view";

/// Registers the searchable types and their relations.
pub(crate) fn register() {
    Searchable::of::<Lead>(LEAD_SUBJECT_TYPE)
        .permission(permissions::LEADS_VIEW)
        .label("Leads")
        .document_in(|tx, lead| Box::pin(lead_document(tx, lead)))
        .register();
    Searchable::of::<SalesOrder>(SALES_ORDER_SUBJECT_TYPE)
        .permission(permissions::SALES_ORDERS_VIEW)
        .label("Sales orders")
        .link("/sales-orders/{id}")
        .document_in(|tx, order| Box::pin(order_document(tx, order)))
        .register();
    Searchable::of::<CounterSale>(COUNTER_SALE_SUBJECT_TYPE)
        .permission(permissions::POS_SELL)
        .label("Counter sales")
        .link("/counter-sales/{id}")
        .document_in(|tx, sale| Box::pin(counter_sale_document(tx, sale)))
        .register();
    Searchable::of::<SalesReturn>(SALES_RETURN_SUBJECT_TYPE)
        .permission(permissions::RETURNS_REQUEST)
        .label("Sales returns")
        .link("/sales-returns/{id}")
        .document_in(|tx, ret| Box::pin(return_document(tx, ret)))
        .register();
    Searchable::of::<Package>(PACKAGE_SUBJECT_TYPE)
        .permission(permissions::PACKAGES_VIEW)
        .label("Packages")
        .document_in(|tx, package| Box::pin(package_document(tx, package)))
        .register();
    register_relations();
}

fn register_relations() {
    // The customer of each sale, and the invoice where the record holds it.
    for (from, table, column) in [
        (LEAD_SUBJECT_TYPE, "wl_sales_leads", "party_id"),
        (
            SALES_ORDER_SUBJECT_TYPE,
            "wl_sales_sales_orders",
            "party_id",
        ),
        (
            COUNTER_SALE_SUBJECT_TYPE,
            "wl_sales_counter_sales",
            "party_id",
        ),
        (
            SALES_RETURN_SUBJECT_TYPE,
            "wl_sales_sales_returns",
            "party_id",
        ),
        (PACKAGE_SUBJECT_TYPE, "wl_sales_packages", "holder_party_id"),
    ] {
        Relation::belongs_to(from, "customer", PARTY_SUBJECT_TYPE, table, column)
            .label("Customer")
            .permission(PARTIES_VIEW)
            .link("/parties/{id}")
            .register();
    }
    for (from, table) in [
        (SALES_ORDER_SUBJECT_TYPE, "wl_sales_sales_orders"),
        (COUNTER_SALE_SUBJECT_TYPE, "wl_sales_counter_sales"),
        (PACKAGE_SUBJECT_TYPE, "wl_sales_packages"),
    ] {
        Relation::belongs_to(from, "invoice", INVOICE_SUBJECT_TYPE, table, "invoice_id")
            .label("Invoice")
            .permission(INVOICES_VIEW)
            .link("/invoices/{id}")
            .register();
    }

    // order and counter sale → their returns; a return → the sale.
    for sale in [SALES_ORDER_SUBJECT_TYPE, COUNTER_SALE_SUBJECT_TYPE] {
        Relation::new(sale, "returns", SALES_RETURN_SUBJECT_TYPE)
            .label("Returns")
            .load(move |db, ids| {
                pairs_of(
                    db,
                    ids,
                    "SELECT sale_id AS from_id, id AS to_id FROM wl_sales_sales_returns \
                     WHERE sale_type = ? AND sale_id IN ",
                    vec![DbValue::Text(sale.to_owned())],
                    (sale, SALES_RETURN_SUBJECT_TYPE),
                )
            })
            .register();
    }
    for (sale, name, label) in [
        (SALES_ORDER_SUBJECT_TYPE, "order", "Sales order"),
        (COUNTER_SALE_SUBJECT_TYPE, "counter_sale", "Counter sale"),
    ] {
        Relation::new(SALES_RETURN_SUBJECT_TYPE, name, sale)
            .label(label)
            .load(move |db, ids| {
                pairs_of(
                    db,
                    ids,
                    "SELECT id AS from_id, sale_id AS to_id FROM wl_sales_sales_returns \
                     WHERE sale_type = ? AND id IN ",
                    vec![DbValue::Text(sale.to_owned())],
                    (SALES_RETURN_SUBJECT_TYPE, sale),
                )
            })
            .register();
    }
    Relation::belongs_to(
        SALES_RETURN_SUBJECT_TYPE,
        "credit_note",
        CREDIT_NOTE_SUBJECT_TYPE,
        "wl_sales_sales_returns",
        "credit_note_id",
    )
    .label("Credit note")
    .permission(INVOICES_VIEW)
    .link("/credit-notes/{id}")
    .register();

    // A counter sale's till session; a lead's order once won; a package's
    // movements.
    Relation::belongs_to(
        COUNTER_SALE_SUBJECT_TYPE,
        "till_session",
        TILL_SESSION_SUBJECT_TYPE,
        "wl_sales_counter_sales",
        "till_session_id",
    )
    .label("Till session")
    .permission(permissions::POS_SELL)
    .link("/till-sessions/{id}")
    .register();
    Relation::new(LEAD_SUBJECT_TYPE, "order", SALES_ORDER_SUBJECT_TYPE)
        .label("Sales order")
        .load(|db, ids| {
            pairs_of(
                db,
                ids,
                "SELECT id AS from_id, converted_id AS to_id FROM wl_sales_leads \
                 WHERE converted_type = ? AND id IN ",
                vec![DbValue::Text(SALES_ORDER_SUBJECT_TYPE.to_owned())],
                (LEAD_SUBJECT_TYPE, SALES_ORDER_SUBJECT_TYPE),
            )
        })
        .register();
    Relation::has_many(
        PACKAGE_SUBJECT_TYPE,
        "movements",
        PACKAGE_MOVEMENT_SUBJECT_TYPE,
        "wl_sales_package_movements",
        "package_id",
    )
    .label("Credits used")
    .permission(permissions::PACKAGES_VIEW)
    .register();
}

/// Runs `statement` (ending in `IN `) with `values` then one placeholder per
/// id, reading `from_id` and `to_id` as records of the two `types`.
fn pairs_of<'a>(
    db: &'a Db,
    ids: &'a [i64],
    statement: &'static str,
    values: Vec<DbValue>,
    types: (&'static str, &'static str),
) -> LoadFuture<'a> {
    Box::pin(async move {
        let placeholders = vec!["?"; ids.len()].join(", ");
        let mut bound = values;
        bound.extend(ids.iter().map(|id| DbValue::Integer(*id)));
        let rows = sql(format!("{statement}({placeholders})"))
            .bind_all(bound)
            .fetch_all(db)
            .await?;
        let mut pairs = Vec::with_capacity(rows.len());
        for row in &rows {
            let from: Option<i64> = row.try_get("from_id")?;
            let to: Option<i64> = row.try_get("to_id")?;
            if let (Some(from), Some(to)) = (from, to) {
                pairs.push((Subject::new(types.0, from), Subject::new(types.1, to)));
            }
        }
        Ok(pairs)
    })
}

/// The party's name: the title of its search document, if any.
async fn party_name(tx: &mut Transaction, party: Option<i64>) -> Result<String> {
    let Some(party) = party else {
        return Ok(String::new());
    };
    Ok(SearchDocumentRow::query()
        .where_eq("subject_type", PARTY_SUBJECT_TYPE)
        .where_eq("subject_id", party)
        .first(&mut *tx)
        .await?
        .map(|row| row.title)
        .unwrap_or_default())
}

/// `parts` that are not empty, joined by " · ".
fn line(parts: &[&str]) -> String {
    parts
        .iter()
        .filter(|part| !part.trim().is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" · ")
}

/// How a lead reads in a search result: the contact, what they want.
async fn lead_document(tx: &mut Transaction, lead: &Lead) -> Result<SearchDocument> {
    let party = party_name(tx, lead.party_id).await?;
    let scope = match (&lead.scope_type, &lead.scope_id) {
        (Some(kind), Some(id)) if !kind.is_empty() => Scope::new(kind, id),
        _ => Scope::global(),
    };
    let mut document = SearchDocument::new(&lead.contact_name)
        .secondary(line(&[lead.interest.as_str(), party.as_str()]))
        .status(lead.status.as_str())
        .scope(scope)
        .legal_entity(lead.legal_entity_id);
    if !party.is_empty() && party != lead.contact_name {
        document = document.keyword(party);
    }
    Ok(document)
}

/// How a sales order reads in a search result.
async fn order_document(tx: &mut Transaction, order: &SalesOrder) -> Result<SearchDocument> {
    let customer = party_name(tx, Some(order.party_id)).await?;
    let po = order.customer_po.clone().unwrap_or_default();
    let mut document = SearchDocument::new(&order.number)
        .secondary(line(&[customer.as_str(), po.as_str()]))
        .identifier(kinds::NUMBER, &order.number)
        .status(order.status.as_str())
        .legal_entity(order.legal_entity_id);
    if let Some(po) = &order.customer_po {
        document = document.identifier(kinds::NUMBER, po);
    }
    if let Some(external) = &order.external_number {
        document = document.identifier(kinds::NUMBER, external);
    }
    Ok(document)
}

/// How a counter sale reads in a search result.
async fn counter_sale_document(tx: &mut Transaction, sale: &CounterSale) -> Result<SearchDocument> {
    let customer = party_name(tx, sale.party_id).await?;
    Ok(SearchDocument::new(&sale.number)
        .secondary(customer)
        .identifier(kinds::NUMBER, &sale.number)
        .status(sale.status.as_str())
        .legal_entity(sale.legal_entity_id))
}

/// How a sales return reads in a search result.
async fn return_document(tx: &mut Transaction, ret: &SalesReturn) -> Result<SearchDocument> {
    let customer = party_name(tx, ret.party_id).await?;
    Ok(SearchDocument::new(&ret.number)
        .secondary(line(&[customer.as_str(), ret.reason.as_str()]))
        .identifier(kinds::NUMBER, &ret.number)
        .status(ret.status.as_str()))
}

/// How a package reads in a search result: what it is, whose.
async fn package_document(tx: &mut Transaction, package: &Package) -> Result<SearchDocument> {
    let holder = party_name(tx, Some(package.holder_party_id)).await?;
    let valid = package
        .valid_to
        .map(|to| format!("valid to {to}"))
        .unwrap_or_default();
    Ok(SearchDocument::new(&package.item_name)
        .secondary(line(&[holder.as_str(), valid.as_str()]))
        .keyword(&package.item_code)
        .status(package.status.as_str()))
}

/// Writes the document of `subject_type` record `id` in `tx`.
pub(crate) async fn index(tx: &mut Transaction, subject_type: &str, id: i64) -> Result {
    index_in(tx, &Subject::new(subject_type, id)).await?;
    Ok(())
}

/// `transition`, writing the record's document after its own effects, in
/// its transaction.
fn reindex<M>(transition: Transition<M>, subject_type: &'static str) -> Transition<M>
where
    M: StateMachine + Model<Key = i64> + Send + Sync + 'static,
{
    transition.effect(move |record: &M, _ctx, tx| {
        let id = record.id();
        Box::pin(async move { index(tx, subject_type, id).await })
    })
}

/// A lead transition that keeps the lead's document current.
pub(crate) fn reindex_lead(transition: Transition<Lead>) -> Transition<Lead> {
    reindex(transition, LEAD_SUBJECT_TYPE)
}

/// A sales order transition that keeps the order's document current.
pub(crate) fn reindex_order(transition: Transition<SalesOrder>) -> Transition<SalesOrder> {
    reindex(transition, SALES_ORDER_SUBJECT_TYPE)
}

/// A counter sale transition that keeps the sale's document current.
pub(crate) fn reindex_counter_sale(transition: Transition<CounterSale>) -> Transition<CounterSale> {
    reindex(transition, COUNTER_SALE_SUBJECT_TYPE)
}

/// A sales return transition that keeps the return's document current.
pub(crate) fn reindex_return(transition: Transition<SalesReturn>) -> Transition<SalesReturn> {
    reindex(transition, SALES_RETURN_SUBJECT_TYPE)
}

/// A package transition that keeps the package's document current.
pub(crate) fn reindex_package(transition: Transition<Package>) -> Transition<Package> {
    reindex(transition, PACKAGE_SUBJECT_TYPE)
}
