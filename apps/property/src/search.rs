//! Buildings, units and leases in the global search, and the relations of
//! 01 Property in the Related panel (BL-AGT-04, `PLT.SRC.01`, `PLT.REL.01`).
//!
//! | Type | Title | Line | Identifiers |
//! |---|---|---|---|
//! | `property.building` | name | code · community | code |
//! | `property.unit` | `JVT-A / 1204` | building · current tenant | unit no. (with and without the building code) |
//! | `property.lease` | `L-000012 v2` | tenant · unit | lease no., Ejari no. |
//!
//! Every document is in its building's scope (`property_buildings`), the
//! scope roles are assigned in, so a property manager of one building finds
//! that building's units and leases only. The tenant's name is the title of
//! the party's own search document.
//!
//! **Relations.** This app declares the relations of its own types and the
//! ones from the shared modules' records to a lease or a unit, which name
//! them by `Subject` (an invoice's source, a cheque's or a deposit's
//! agreement, a ticket's subject): a lease's invoices, cheques, deposits and
//! tickets; a unit's current lease, its cheques and open tickets; a
//! payment's, cheque's, deposit's or refund's lease; a ticket's unit; a
//! party's leases. A cheque pays the charges of a lease, whose invoices are
//! the invoices it pays: the app's `invoices` relation of a cheque covers
//! them and the invoices a cheque is linked to directly, and replaces the
//! payments module's (the vertical is mounted last, `host::mount_vertical`).
//! The loaders read the shared modules' tables through those subject
//! columns only.

use renox::db::{DbValue, Transaction, sql};
use renox::prelude::*;
use renox::state_machine::Transitioned;
use wl_kernel::Subject;
use wl_kernel::cheques::CHEQUE_SUBJECT_TYPE;
use wl_kernel::invoicing::INVOICE_SUBJECT_TYPE;
use wl_kernel::parties::PARTY_SUBJECT_TYPE;
use wl_kernel::payments::{DEPOSIT_SUBJECT_TYPE, PAYMENT_SUBJECT_TYPE, REFUND_SUBJECT_TYPE};
use wl_kernel::related::Relation;
use wl_kernel::search::{SearchDocument, SearchDocumentRow, Searchable, kinds};
use wl_kernel::tickets::TICKET_SUBJECT_TYPE;

use crate::building::{BUILDING_SUBJECT_TYPE, Building};
use crate::charge::CHARGE_SUBJECT_TYPE;
use crate::lease::{LEASE_SUBJECT_TYPE, Lease, LeaseStatus};
use crate::permissions;
use crate::unit::{UNIT_SUBJECT_TYPE, Unit};

/// The permissions of the shared modules' records, as they name them.
const PARTIES_VIEW: &str = "parties.view";
const INVOICES_VIEW: &str = "invoices.view";
const CHEQUES_VIEW: &str = "cheques.view";
const DEPOSITS_VIEW: &str = "deposits.view";
const TICKETS_VIEW: &str = "tickets.view";

/// The states of a ticket that is no longer open (wl-ticketing's
/// `resolved`, `closed` and `cancelled`).
const TICKET_DONE: &str = "'resolved', 'closed', 'cancelled'";

/// Declares the three types for search and the relations, and keeps the
/// documents current: a lease's and its unit's after each transition of the
/// lease (the unit's current tenant follows), a unit's after its own.
pub(crate) fn register(app: &mut Registry) {
    Searchable::of::<Building>(BUILDING_SUBJECT_TYPE)
        .permission(permissions::UNITS_VIEW)
        .label("Buildings")
        .link("/property/buildings/{id}")
        .document(building_document)
        .register();
    // A unit has no page of its own: a result opens its related records
    // (the kernel's page), where its lease, cheques and tickets are.
    Searchable::of::<Unit>(UNIT_SUBJECT_TYPE)
        .after(PARTY_SUBJECT_TYPE)
        .permission(permissions::UNITS_VIEW)
        .label("Units")
        .document_in(|tx, unit| Box::pin(unit_document(tx, unit)))
        .register();
    Searchable::of::<Lease>(LEASE_SUBJECT_TYPE)
        .after(PARTY_SUBJECT_TYPE)
        .permission(permissions::LEASES_VIEW)
        .label("Leases")
        .link("/property/leases/{id}")
        .document_in(|tx, lease| Box::pin(lease_document(tx, lease)))
        .register();
    relations();
    app.listen(|event: Transitioned<Lease>, state: AppState| async move {
        let mut subjects = vec![Subject::new(LEASE_SUBJECT_TYPE, event.key)];
        if let Some(lease) = Lease::find(&state.db, event.key).await? {
            subjects.push(Subject::new(UNIT_SUBJECT_TYPE, lease.unit_id));
        }
        index(&state.db, &subjects).await
    });
    app.listen(|event: Transitioned<Unit>, state: AppState| async move {
        index(&state.db, &[Subject::new(UNIT_SUBJECT_TYPE, event.key)]).await
    });
}

/// The search document of `building`.
pub fn building_document(building: &Building) -> SearchDocument {
    let line = [
        building.code.as_str(),
        building.community.as_deref().unwrap_or_default(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    SearchDocument::new(building.name_en.clone())
        .secondary(line)
        .keyword(building.name_ar.clone().unwrap_or_default())
        .identifier(kinds::NUMBER, building.code.clone())
        .scope(building.scope())
}

/// The search document of `unit`: its building and number, over the
/// building's name and the current tenant.
pub async fn unit_document(tx: &mut Transaction, unit: &Unit) -> Result<SearchDocument> {
    let building = Building::query()
        .where_eq("id", unit.building_id)
        .first(&mut *tx)
        .await?;
    let (code, name) = building
        .map(|b| (b.code, b.name_en))
        .unwrap_or_default();
    let running: Vec<String> = LeaseStatus::RUNNING
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    let current = Lease::query()
        .where_eq("unit_id", unit.id)
        .where_in("status", running)
        .order_by_desc("id")
        .first(&mut *tx)
        .await?;
    let tenant = match current {
        Some(lease) => party_name_in(tx, lease.tenant_party_id).await?,
        None => String::new(),
    };
    Ok(SearchDocument::new(unit_label(&code, &unit.number))
        .secondary(joined(&name, &tenant))
        .keyword(tenant.clone())
        .identifier(kinds::UNIT, unit.number.clone())
        .identifier(kinds::UNIT, format!("{code}{}", unit.number))
        .status(unit.status.as_str())
        .scope(unit.scope()))
}

/// The search document of `lease`: its number and version, over the tenant
/// and the unit.
pub async fn lease_document(tx: &mut Transaction, lease: &Lease) -> Result<SearchDocument> {
    let tenant = party_name_in(tx, lease.tenant_party_id).await?;
    let unit = Unit::query()
        .where_eq("id", lease.unit_id)
        .first(&mut *tx)
        .await?;
    let building = Building::query()
        .where_eq("id", lease.building_id)
        .first(&mut *tx)
        .await?;
    let (code, building_name) = building
        .map(|b| (b.code, b.name_en))
        .unwrap_or_default();
    let unit_text = unit
        .map(|u| unit_label(&code, &u.number))
        .unwrap_or_default();
    Ok(SearchDocument::new(lease.number_with_version())
        .secondary(joined(&tenant, &unit_text))
        .keyword(building_name)
        .identifier(kinds::NUMBER, lease.number())
        .identifier(kinds::EJARI, lease.ejari_number.clone().unwrap_or_default())
        .status(lease.status.as_str())
        .scope(lease.scope()))
}

/// `JVT-A / 1204`, as the documents pages name a unit.
fn unit_label(code: &str, number: &str) -> String {
    if code.is_empty() {
        number.to_owned()
    } else {
        format!("{code} / {number}")
    }
}

/// `a · b`, leaving out an empty part.
fn joined(a: &str, b: &str) -> String {
    [a, b]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The name party `id` is found by (the title of its search document), or
/// `""`.
async fn party_name_in(tx: &mut Transaction, id: i64) -> Result<String> {
    Ok(SearchDocumentRow::query()
        .where_eq("subject_type", PARTY_SUBJECT_TYPE)
        .where_eq("subject_id", id)
        .first(&mut *tx)
        .await?
        .map(|document| document.title)
        .unwrap_or_default())
}

/// The states of a lease that has the unit now, for SQL.
fn running_sql() -> String {
    LeaseStatus::RUNNING
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A relation from `from_type` to `to_type` read by `statement`, whose
/// `{ids}` stands for the ids of the records it starts from and whose `?`
/// before it take `values`.
fn by_sql(
    from_type: &'static str,
    name: &str,
    to_type: &'static str,
    statement: String,
    values: &'static [&'static str],
) -> wl_kernel::related::RelationBuilder {
    Relation::new(from_type, name, to_type).load(move |db, ids| {
        let statement = statement.clone();
        Box::pin(async move {
            let values = values
                .iter()
                .map(|v| DbValue::Text((*v).to_owned()))
                .collect();
            pairs(db, &statement, values, ids, from_type, to_type).await
        })
    })
}

/// Declares the relations (see the module's doc).
fn relations() {
    let running = running_sql();
    // property.building
    Relation::has_many(
        BUILDING_SUBJECT_TYPE,
        "units",
        UNIT_SUBJECT_TYPE,
        "property_units",
        "building_id",
    )
    .label("Units")
    .register();
    Relation::has_many(
        BUILDING_SUBJECT_TYPE,
        "leases",
        LEASE_SUBJECT_TYPE,
        "property_leases",
        "building_id",
    )
    .label("Leases")
    .register();
    by_sql(
        BUILDING_SUBJECT_TYPE,
        "open_tickets",
        TICKET_SUBJECT_TYPE,
        format!(
            "SELECT b.id AS from_id, t.id AS to_id FROM property_buildings b \
             JOIN wl_ticketing_tickets t ON (t.subject_type = ? AND t.subject_id = b.id) \
             OR (t.subject_type = ? AND t.subject_id IN \
                 (SELECT u.id FROM property_units u WHERE u.building_id = b.id)) \
             WHERE t.status NOT IN ({TICKET_DONE}) AND b.id IN ({{ids}}) ORDER BY t.id"
        ),
        &[BUILDING_SUBJECT_TYPE, UNIT_SUBJECT_TYPE],
    )
    .label("Open tickets")
    .permission(TICKETS_VIEW)
    .register();

    // property.unit
    Relation::belongs_to(
        UNIT_SUBJECT_TYPE,
        "building",
        BUILDING_SUBJECT_TYPE,
        "property_units",
        "building_id",
    )
    .label("Building")
    .register();
    by_sql(
        UNIT_SUBJECT_TYPE,
        "current_lease",
        LEASE_SUBJECT_TYPE,
        format!(
            "SELECT unit_id AS from_id, id AS to_id FROM property_leases \
             WHERE status IN ({running}) AND unit_id IN ({{ids}}) ORDER BY id DESC"
        ),
        &[],
    )
    .label("Current lease")
    .permission(permissions::LEASES_VIEW)
    .register();
    by_sql(
        UNIT_SUBJECT_TYPE,
        "cheques",
        CHEQUE_SUBJECT_TYPE,
        format!(
            "SELECT l.unit_id AS from_id, c.id AS to_id FROM property_leases l \
             JOIN wl_payments_cheques c ON c.agreement_type = ? AND c.agreement_id = l.root_id \
             WHERE l.status IN ({running}) AND l.unit_id IN ({{ids}}) \
             ORDER BY c.cheque_date, c.id"
        ),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Cheques of the current lease")
    .permission(CHEQUES_VIEW)
    .register();
    by_sql(
        UNIT_SUBJECT_TYPE,
        "open_tickets",
        TICKET_SUBJECT_TYPE,
        format!(
            "SELECT subject_id AS from_id, id AS to_id FROM wl_ticketing_tickets \
             WHERE subject_type = ? AND status NOT IN ({TICKET_DONE}) \
             AND subject_id IN ({{ids}}) ORDER BY id"
        ),
        &[UNIT_SUBJECT_TYPE],
    )
    .label("Open tickets")
    .permission(TICKETS_VIEW)
    .register();

    // property.lease
    Relation::belongs_to(
        LEASE_SUBJECT_TYPE,
        "tenant",
        PARTY_SUBJECT_TYPE,
        "property_leases",
        "tenant_party_id",
    )
    .label("Tenant")
    .permission(PARTIES_VIEW)
    .link("/parties/{id}")
    .register();
    Relation::belongs_to(
        LEASE_SUBJECT_TYPE,
        "unit",
        UNIT_SUBJECT_TYPE,
        "property_leases",
        "unit_id",
    )
    .label("Unit")
    .register();
    // The invoices of every version of the lease.
    by_sql(
        LEASE_SUBJECT_TYPE,
        "invoices",
        INVOICE_SUBJECT_TYPE,
        "SELECT l.id AS from_id, i.id AS to_id FROM property_leases l \
         JOIN property_leases v ON v.root_id = l.root_id \
         JOIN wl_invoicing_invoices i ON i.source_type = ? AND i.source_id = v.id \
         WHERE l.id IN ({ids}) ORDER BY i.id"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Invoices")
    .permission(INVOICES_VIEW)
    .link("/invoices/{id}")
    .register();
    // Cheques and deposits are held for the lease (its first version).
    by_sql(
        LEASE_SUBJECT_TYPE,
        "cheques",
        CHEQUE_SUBJECT_TYPE,
        "SELECT l.id AS from_id, c.id AS to_id FROM property_leases l \
         JOIN wl_payments_cheques c ON c.agreement_type = ? AND c.agreement_id = l.root_id \
         WHERE l.id IN ({ids}) ORDER BY c.cheque_date, c.id"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Cheques")
    .permission(CHEQUES_VIEW)
    .link("/cheques/{id}")
    .register();
    by_sql(
        LEASE_SUBJECT_TYPE,
        "deposits",
        DEPOSIT_SUBJECT_TYPE,
        "SELECT l.id AS from_id, d.id AS to_id FROM property_leases l \
         JOIN wl_payments_deposits d ON d.source_type = ? AND d.source_id = l.root_id \
         WHERE l.id IN ({ids}) ORDER BY d.id"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Deposits")
    .permission(DEPOSITS_VIEW)
    .link("/deposits/{id}")
    .register();
    // Tickets about the lease, and the tenant's tickets about its unit.
    by_sql(
        LEASE_SUBJECT_TYPE,
        "tickets",
        TICKET_SUBJECT_TYPE,
        "SELECT l.id AS from_id, t.id AS to_id FROM property_leases l \
         JOIN wl_ticketing_tickets t ON (t.subject_type = ? AND t.subject_id = l.id) \
         OR (t.subject_type = ? AND t.subject_id = l.unit_id \
             AND t.requester_party_id = l.tenant_party_id) \
         WHERE l.id IN ({ids}) ORDER BY t.id"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE, UNIT_SUBJECT_TYPE],
    )
    .label("Tickets")
    .permission(TICKETS_VIEW)
    .link("/tickets/{id}")
    .register();

    // From the shared modules' records.
    Relation::has_many(
        PARTY_SUBJECT_TYPE,
        "leases",
        LEASE_SUBJECT_TYPE,
        "property_leases",
        "tenant_party_id",
    )
    .label("Leases")
    .permission(permissions::LEASES_VIEW)
    .register();
    by_sql(
        INVOICE_SUBJECT_TYPE,
        "lease",
        LEASE_SUBJECT_TYPE,
        "SELECT id AS from_id, source_id AS to_id FROM wl_invoicing_invoices \
         WHERE source_type = ? AND id IN ({ids})"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Lease")
    .permission(permissions::LEASES_VIEW)
    .register();
    // A cheque's invoices: linked directly, or through the lease charges it
    // pays (replaces the payments module's relation of the same name).
    by_sql(
        CHEQUE_SUBJECT_TYPE,
        "invoices",
        INVOICE_SUBJECT_TYPE,
        "SELECT k.cheque_id AS from_id, \
             CASE WHEN k.subject_type = ? THEN k.subject_id ELSE ch.invoice_id END AS to_id \
         FROM wl_payments_cheque_links k \
         LEFT JOIN property_lease_charges ch ON k.subject_type = ? AND ch.id = k.subject_id \
         WHERE k.cheque_id IN ({ids}) ORDER BY k.position"
            .to_owned(),
        &[INVOICE_SUBJECT_TYPE, CHARGE_SUBJECT_TYPE],
    )
    .label("Invoices it pays")
    .permission(INVOICES_VIEW)
    .link("/invoices/{id}")
    .register();
    by_sql(
        CHEQUE_SUBJECT_TYPE,
        "lease",
        LEASE_SUBJECT_TYPE,
        "SELECT id AS from_id, agreement_id AS to_id FROM wl_payments_cheques \
         WHERE agreement_type = ? AND id IN ({ids})"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Lease")
    .permission(permissions::LEASES_VIEW)
    .register();
    by_sql(
        DEPOSIT_SUBJECT_TYPE,
        "lease",
        LEASE_SUBJECT_TYPE,
        "SELECT id AS from_id, source_id AS to_id FROM wl_payments_deposits \
         WHERE source_type = ? AND id IN ({ids})"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE],
    )
    .label("Lease")
    .permission(permissions::LEASES_VIEW)
    .register();
    // A payment's lease: prepaid for it, or the source of an invoice it is
    // allocated to.
    by_sql(
        PAYMENT_SUBJECT_TYPE,
        "lease",
        LEASE_SUBJECT_TYPE,
        "SELECT p.id AS from_id, p.prepaid_id AS to_id FROM wl_payments_payments p \
         WHERE p.prepaid_type = ? AND p.id IN ({ids}) \
         UNION \
         SELECT a.payment_id AS from_id, i.source_id AS to_id FROM wl_payments_allocations a \
         JOIN wl_invoicing_invoices i ON a.target_type = ? AND i.id = a.target_id \
         WHERE a.removed_at IS NULL AND i.source_type = ? AND a.payment_id IN ({ids})"
            .to_owned(),
        &[LEASE_SUBJECT_TYPE, INVOICE_SUBJECT_TYPE, LEASE_SUBJECT_TYPE],
    )
    .label("Lease")
    .permission(permissions::LEASES_VIEW)
    .register();
    // A refund's lease: the one its deposit was held for.
    by_sql(
        REFUND_SUBJECT_TYPE,
        "lease",
        LEASE_SUBJECT_TYPE,
        "SELECT r.id AS from_id, d.source_id AS to_id FROM wl_payments_refunds r \
         JOIN wl_payments_deposits d ON r.source_type = ? AND d.id = r.source_id \
         WHERE d.source_type = ? AND r.id IN ({ids})"
            .to_owned(),
        &[DEPOSIT_SUBJECT_TYPE, LEASE_SUBJECT_TYPE],
    )
    .label("Lease")
    .permission(permissions::LEASES_VIEW)
    .register();
    by_sql(
        TICKET_SUBJECT_TYPE,
        "unit",
        UNIT_SUBJECT_TYPE,
        "SELECT id AS from_id, subject_id AS to_id FROM wl_ticketing_tickets \
         WHERE subject_type = ? AND id IN ({ids})"
            .to_owned(),
        &[UNIT_SUBJECT_TYPE],
    )
    .label("Unit")
    .permission(permissions::UNITS_VIEW)
    .register();
}

/// Runs `statement`, each `{ids}` of which stands for one placeholder per
/// id, with `values` then the ids (once per `{ids}`, in order: a statement
/// with two `{ids}` takes its values before the first only), reading
/// `from_id` and `to_id` as records of `from_type` and `to_type`.
async fn pairs(
    db: &Db,
    statement: &str,
    values: Vec<DbValue>,
    ids: &[i64],
    from_type: &str,
    to_type: &str,
) -> Result<Vec<(Subject, Subject)>> {
    let placeholders = vec!["?"; ids.len()].join(", ");
    let ids_values: Vec<DbValue> = ids.iter().map(|id| DbValue::Integer(*id)).collect();
    // Split the values among the `{ids}`: the `?` before each one take the
    // next values, then the ids.
    let parts: Vec<&str> = statement.split("{ids}").collect();
    let mut bound: Vec<DbValue> = Vec::new();
    let mut values = values.into_iter();
    for (index, part) in parts.iter().enumerate() {
        for _ in 0..part.matches('?').count() {
            if let Some(value) = values.next() {
                bound.push(value);
            }
        }
        if index + 1 < parts.len() {
            bound.extend(ids_values.iter().cloned());
        }
    }
    let rows = sql(statement.replace("{ids}", &placeholders))
        .bind_all(bound)
        .fetch_all(db)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let from: Option<i64> = row.try_get("from_id")?;
        let to: Option<i64> = row.try_get("to_id")?;
        if let (Some(from), Some(to)) = (from, to) {
            out.push((Subject::new(from_type, from), Subject::new(to_type, to)));
        }
    }
    Ok(out)
}

/// Writes the documents of `subjects` again in their own transaction.
pub async fn index(db: &Db, subjects: &[Subject]) -> Result {
    let mut tx = db.begin().await?;
    for subject in subjects {
        wl_kernel::search::index_in(&mut tx, subject).await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Builds the index again for the demo and after an import: the parties
/// first (the other documents take the tenant's name from them), then the
/// buildings, units and leases, then every other registered type (whose
/// scopes and names read those documents). Returns the number of documents
/// of each type.
///
/// `search:rebuild` builds the types in name order, so `invoicing.*` come
/// before `parties.party`: on an empty index their customer names are
/// missing until the next change. This order avoids it.
pub async fn rebuild_in_order(db: &Db) -> Result<Vec<(String, u64)>> {
    let first = [
        PARTY_SUBJECT_TYPE,
        BUILDING_SUBJECT_TYPE,
        UNIT_SUBJECT_TYPE,
        LEASE_SUBJECT_TYPE,
    ];
    let mut counts = Vec::new();
    for subject_type in first {
        if wl_kernel::search::searchable_type(subject_type).is_some() {
            let count = wl_kernel::search::rebuild_type(db, subject_type).await?;
            counts.push((subject_type.to_owned(), count));
        }
    }
    for searchable in wl_kernel::search::registered_types() {
        let subject_type = searchable.subject_type();
        if first.contains(&subject_type) {
            continue;
        }
        let count = wl_kernel::search::rebuild_type(db, subject_type).await?;
        counts.push((subject_type.to_owned(), count));
    }
    Ok(counts)
}
