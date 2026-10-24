//! The F1 modules and 01 Property registered for search and related records
//! (BL-AGT-04), on the Property demo company (`property:demo`), searched as
//! its real staff:
//!
//! - `property_manager.all@palmgrove.example` (the property manager of every
//!   building, a global role): parties, units, leases, invoices, payments,
//!   cheques, deposits, tickets and documents — the "finance" user of the
//!   search page's tests;
//! - `leasing_agent@palmgrove.example`: parties, units and leases, no
//!   `invoices.view` — the "viewer";
//! - `property_manager@palmgrove.example`: the property manager of the two
//!   towers only (a scoped role).
//!
//! It checks the Done-when points of the issue: a tenant's name finds the
//! party, the lease, the unit and the open invoices; a cheque number finds
//! the cheque, whose Related shows its lease and the invoices it pays; a
//! unit's Related shows the current lease, its cheques and the open tickets;
//! and every registered type has its document and its relations. The demo
//! ends with a full index.

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::Subject;
use wl_kernel::related::{self, PANEL_DEPTH, RelatedGroup};
use wl_kernel::search::{self, Access, SearchResults};

const FINANCE: &str = "property_manager.all@palmgrove.example";
const VIEWER: &str = "leasing_agent@palmgrove.example";
const TOWERS: &str = "property_manager@palmgrove.example";

/// The types this issue registers.
const TYPES: &[&str] = &[
    "parties.party",
    "invoicing.invoice",
    "invoicing.credit_note",
    "payments.payment",
    "payments.cheque",
    "payments.deposit",
    "payments.refund",
    "ticketing.ticket",
    "wl_documents.document",
    "property.building",
    "property.unit",
    "property.lease",
];

async fn demo() -> TestApp {
    let app = TestApp::new(host::app()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    let report = host::demo_data::seed(app.kernel()).await.unwrap();
    assert!(!report.already);
    assert!(
        report.made.get("search documents").copied().unwrap_or(0) > 0,
        "the demo ends with the index rebuilt: {:?}",
        report.made
    );
    app
}

async fn user(app: &TestApp, email: &str) -> User {
    User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"))
}

async fn access(app: &TestApp, email: &str) -> Access {
    Access::of_user(app.db(), &user(app, email).await)
        .await
        .unwrap()
}

/// The ids of the hits of `subject_type` in `results`.
fn hits(results: &SearchResults, subject_type: &str) -> Vec<i64> {
    results
        .groups
        .iter()
        .filter(|g| g.subject_type == subject_type)
        .flat_map(|g| g.hits.iter().map(|h| h.subject.subject_id))
        .collect()
}

/// The ids of the records of group `name` in `groups`.
fn group(groups: &[RelatedGroup], name: &str) -> Vec<i64> {
    groups
        .iter()
        .find(|g| g.name == name)
        .map(|g| g.records.iter().map(|r| r.subject.subject_id).collect())
        .unwrap_or_default()
}

async fn ids(app: &TestApp, statement: &str) -> Vec<i64> {
    renox::db::sql(statement)
        .fetch_all(app.db())
        .await
        .unwrap()
        .iter()
        .map(|row| row.try_get::<i64>("id").unwrap())
        .collect()
}

async fn one(app: &TestApp, statement: &str) -> i64 {
    *ids(app, statement)
        .await
        .first()
        .unwrap_or_else(|| panic!("the demo has a row for: {statement}"))
}

async fn text(app: &TestApp, statement: &str) -> String {
    renox::db::sql(statement)
        .scalar::<String>(app.db())
        .await
        .unwrap()
}

#[renox::test]
async fn the_demo_company_is_searchable_and_its_records_show_their_related_records() {
    let app = demo().await;
    let db = app.db();
    let finance = access(&app, FINANCE).await;
    let viewer = access(&app, VIEWER).await;
    let towers = access(&app, TOWERS).await;

    // --- A tenant's name finds the party, the lease, the unit and the open
    // invoices. An active lease whose unit has no other running lease and
    // whose tenant has an open invoice.
    let lease = one(
        &app,
        "SELECT l.id FROM property_leases l \
         WHERE l.status = 'active' \
         AND (SELECT count(*) FROM property_leases o WHERE o.unit_id = l.unit_id \
              AND o.status IN ('active', 'renewal_due', 'expired', 'notice_given')) = 1 \
         AND EXISTS (SELECT 1 FROM wl_invoicing_invoices i WHERE i.party_id = l.tenant_party_id \
              AND i.status IN ('issued', 'partially_paid', 'overdue')) \
         ORDER BY l.id",
    )
    .await;
    let tenant = one(
        &app,
        &format!("SELECT tenant_party_id AS id FROM property_leases WHERE id = {lease}"),
    )
    .await;
    let unit = one(
        &app,
        &format!("SELECT unit_id AS id FROM property_leases WHERE id = {lease}"),
    )
    .await;
    let name = text(
        &app,
        &format!("SELECT display_name FROM wl_parties_parties WHERE id = {tenant}"),
    )
    .await;
    let open_invoices = ids(
        &app,
        &format!(
            "SELECT id FROM wl_invoicing_invoices WHERE party_id = {tenant} \
             AND status IN ('issued', 'partially_paid', 'overdue') ORDER BY id"
        ),
    )
    .await;
    assert!(!open_invoices.is_empty());

    let found = search::search(db, &finance, &name, 200).await.unwrap();
    assert!(
        hits(&found, "parties.party").contains(&tenant),
        "{name}: the party; got {found:?}"
    );
    assert!(
        hits(&found, "property.lease").contains(&lease),
        "{name}: the lease"
    );
    assert!(
        hits(&found, "property.unit").contains(&unit),
        "{name}: the unit"
    );
    let invoices = hits(&found, "invoicing.invoice");
    for invoice in &open_invoices {
        assert!(
            invoices.contains(invoice),
            "{name}: the open invoice {invoice}; got {invoices:?}"
        );
    }

    // The viewer (no `invoices.view`) finds the party, the lease and the unit
    // but not one invoice, and the page does not show them.
    let seen = search::search(db, &viewer, &name, 200).await.unwrap();
    assert!(hits(&seen, "parties.party").contains(&tenant));
    assert!(hits(&seen, "property.lease").contains(&lease));
    assert!(hits(&seen, "invoicing.invoice").is_empty(), "{seen:?}");
    let number = text(
        &app,
        &format!(
            "SELECT number FROM wl_invoicing_invoices WHERE id = {}",
            open_invoices[0]
        ),
    )
    .await;
    app.acting_as(&user(&app, VIEWER).await);
    app.get(&format!("/search?q={}", name.replace(' ', "+")))
        .await
        .assert_ok()
        .assert_see("Leases")
        .assert_dont_see(&number);
    app.acting_as(&user(&app, FINANCE).await);
    app.get(&format!("/search?q={}", name.replace(' ', "+")))
        .await
        .assert_ok()
        .assert_see(&number);
    // The invoice number opens the invoice for finance.
    app.get(&format!("/search?q={}", number.to_lowercase()))
        .await
        .assert_redirect(&format!("/invoices/{}", open_invoices[0]));

    // The property manager of the towers finds the leases of the towers and
    // never one of another building; a party (global) is for global roles.
    let tower_lease = one(
        &app,
        "SELECT l.id FROM property_leases l JOIN property_buildings b ON b.id = l.building_id \
         WHERE b.code = 'JVT-A' AND l.status = 'active' ORDER BY l.id",
    )
    .await;
    let other_lease = one(
        &app,
        "SELECT l.id FROM property_leases l JOIN property_buildings b ON b.id = l.building_id \
         WHERE b.code = 'BBY' AND l.status = 'active' ORDER BY l.id",
    )
    .await;
    for (lease_id, visible) in [(tower_lease, true), (other_lease, false)] {
        let tenant_name = text(
            &app,
            &format!(
                "SELECT p.display_name FROM property_leases l \
                 JOIN wl_parties_parties p ON p.id = l.tenant_party_id WHERE l.id = {lease_id}"
            ),
        )
        .await;
        let got = search::search(db, &towers, &tenant_name, 200)
            .await
            .unwrap();
        assert_eq!(
            hits(&got, "property.lease").contains(&lease_id),
            visible,
            "the towers' manager and the lease {lease_id} of {tenant_name}"
        );
        assert!(hits(&got, "parties.party").is_empty());
    }

    // --- A cheque number finds the cheque; its Related shows its lease and
    // the invoices it pays.
    let cheque = one(
        &app,
        "SELECT c.id FROM wl_payments_cheques c \
         JOIN wl_payments_cheque_links k ON k.cheque_id = c.id AND k.subject_type = 'property.lease_charge' \
         JOIN property_lease_charges ch ON ch.id = k.subject_id \
         WHERE c.agreement_type = 'property.lease' AND ch.invoice_id IS NOT NULL ORDER BY c.id",
    )
    .await;
    let cheque_number = text(
        &app,
        &format!("SELECT number FROM wl_payments_cheques WHERE id = {cheque}"),
    )
    .await;
    let agreement = one(
        &app,
        &format!("SELECT agreement_id AS id FROM wl_payments_cheques WHERE id = {cheque}"),
    )
    .await;
    let paid = ids(
        &app,
        &format!(
            "SELECT ch.invoice_id AS id FROM wl_payments_cheque_links k \
             JOIN property_lease_charges ch ON ch.id = k.subject_id \
             WHERE k.cheque_id = {cheque} AND k.subject_type = 'property.lease_charge' \
             AND ch.invoice_id IS NOT NULL"
        ),
    )
    .await;
    let found = search::search(db, &finance, &cheque_number, 50)
        .await
        .unwrap();
    assert!(
        hits(&found, "payments.cheque").contains(&cheque),
        "the cheque {cheque_number}; got {found:?}"
    );
    let cheque_group = found
        .groups
        .iter()
        .find(|g| g.subject_type == "payments.cheque")
        .unwrap();
    let hit = cheque_group
        .hits
        .iter()
        .find(|h| h.subject.subject_id == cheque)
        .unwrap();
    let bank = text(
        &app,
        &format!("SELECT bank FROM wl_payments_cheques WHERE id = {cheque}"),
    )
    .await;
    assert!(hit.identifier_match);
    assert!(hit.secondary.contains(&bank), "the bank shows: {hit:?}");
    let groups = related::related(
        db,
        &finance,
        &Subject::new("payments.cheque", cheque),
        PANEL_DEPTH,
    )
    .await
    .unwrap();
    assert_eq!(group(&groups, "lease"), vec![agreement], "{groups:?}");
    let invoices = group(&groups, "invoices");
    for invoice in &paid {
        assert!(invoices.contains(invoice), "{invoice} in {invoices:?}");
    }
    app.get(&format!("/cheques/{cheque}"))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/payments.cheque:{cheque}/panel\""
        ));

    // --- A unit's Related: the current lease, its cheques and the open
    // tickets. A unit with a running lease that has cheques and an open
    // ticket.
    let unit = one(
        &app,
        "SELECT u.id FROM property_units u \
         WHERE EXISTS (SELECT 1 FROM property_leases l JOIN wl_payments_cheques c \
              ON c.agreement_type = 'property.lease' AND c.agreement_id = l.root_id \
              WHERE l.unit_id = u.id AND l.status IN ('active', 'renewal_due', 'expired', 'notice_given')) \
         AND EXISTS (SELECT 1 FROM wl_ticketing_tickets t WHERE t.subject_type = 'property.unit' \
              AND t.subject_id = u.id AND t.status NOT IN ('resolved', 'closed', 'cancelled')) \
         ORDER BY u.id",
    )
    .await;
    let current = ids(
        &app,
        &format!(
            "SELECT id FROM property_leases WHERE unit_id = {unit} \
             AND status IN ('active', 'renewal_due', 'expired', 'notice_given')"
        ),
    )
    .await;
    let cheques = ids(
        &app,
        &format!(
            "SELECT c.id FROM property_leases l JOIN wl_payments_cheques c \
             ON c.agreement_type = 'property.lease' AND c.agreement_id = l.root_id \
             WHERE l.unit_id = {unit} AND l.status IN ('active', 'renewal_due', 'expired', 'notice_given')"
        ),
    )
    .await;
    let open_tickets = ids(
        &app,
        &format!(
            "SELECT id FROM wl_ticketing_tickets WHERE subject_type = 'property.unit' \
             AND subject_id = {unit} AND status NOT IN ('resolved', 'closed', 'cancelled')"
        ),
    )
    .await;
    let closed_tickets = ids(
        &app,
        &format!(
            "SELECT id FROM wl_ticketing_tickets WHERE subject_type = 'property.unit' \
             AND subject_id = {unit} AND status IN ('resolved', 'closed', 'cancelled')"
        ),
    )
    .await;
    let groups = related::related(
        db,
        &finance,
        &Subject::new("property.unit", unit),
        PANEL_DEPTH,
    )
    .await
    .unwrap();
    let mut got = group(&groups, "current_lease");
    got.sort_unstable();
    assert_eq!(got, current, "{groups:?}");
    let shown = group(&groups, "cheques");
    for cheque in cheques.iter().take(related::ROWS_PER_GROUP) {
        assert!(shown.contains(cheque), "cheque {cheque} in {shown:?}");
    }
    let tickets = group(&groups, "open_tickets");
    for ticket in &open_tickets {
        assert!(tickets.contains(ticket), "ticket {ticket} in {tickets:?}");
    }
    for ticket in &closed_tickets {
        assert!(!tickets.contains(ticket), "closed ticket {ticket} shown");
    }
    app.get(&format!("/related/property.unit:{unit}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"current_lease\"")
        .assert_see("data-group=\"cheques\"")
        .assert_see("data-group=\"open_tickets\"");

    // --- Every registered type: a document for each record and its
    // relations.
    for subject_type in TYPES {
        assert!(
            search::searchable_type(subject_type).is_some(),
            "{subject_type} is registered"
        );
    }
    let table_of = |subject_type: &str| {
        search::searchable_type(subject_type)
            .unwrap()
            .table()
            .to_owned()
    };
    for subject_type in TYPES {
        let table = table_of(subject_type);
        let rows: i64 = renox::db::sql(format!("SELECT count(*) FROM {table}"))
            .scalar(db)
            .await
            .unwrap();
        let documents: i64 =
            renox::db::sql("SELECT count(*) FROM wl_search_documents WHERE subject_type = ?")
                .bind(*subject_type)
                .scalar(db)
                .await
                .unwrap();
        assert!(rows > 0, "the demo has {subject_type} records");
        assert_eq!(documents, rows, "{subject_type}: one document per record");
    }

    // The relations each type declares, on a record that has them.
    let checks: &[(&str, &str, &[&str])] = &[
        (
            "parties.party",
            "SELECT tenant_party_id AS id FROM property_leases WHERE status = 'active' ORDER BY id",
            &["invoices", "leases"],
        ),
        (
            "invoicing.invoice",
            "SELECT i.id FROM wl_invoicing_invoices i WHERE i.status = 'paid' \
             AND i.source_type = 'property.lease' ORDER BY i.id",
            &["lines", "customer", "payments", "lease"],
        ),
        (
            "invoicing.credit_note",
            "SELECT id FROM wl_invoicing_credit_notes ORDER BY id",
            &["invoice", "customer"],
        ),
        (
            "payments.payment",
            "SELECT a.payment_id AS id FROM wl_payments_allocations a \
             JOIN wl_invoicing_invoices i ON i.id = a.target_id \
             WHERE a.target_type = 'invoicing.invoice' AND a.removed_at IS NULL \
             AND i.source_type = 'property.lease' ORDER BY a.id",
            &["invoices", "party", "lease"],
        ),
        (
            "payments.cheque",
            "SELECT id FROM wl_payments_cheques WHERE agreement_type = 'property.lease' ORDER BY id",
            &["party", "lease"],
        ),
        (
            "payments.deposit",
            "SELECT id FROM wl_payments_deposits WHERE source_type = 'property.lease' ORDER BY id",
            &["party", "lease"],
        ),
        (
            "payments.refund",
            "SELECT id FROM wl_payments_refunds ORDER BY id",
            &["party"],
        ),
        (
            "ticketing.ticket",
            "SELECT id FROM wl_ticketing_tickets WHERE subject_type = 'property.unit' \
             AND requester_party_id IS NOT NULL ORDER BY id",
            &["unit", "party"],
        ),
        (
            "property.building",
            "SELECT id FROM property_buildings ORDER BY id",
            &["units", "leases"],
        ),
        (
            "property.lease",
            "SELECT l.id FROM property_leases l WHERE l.status = 'active' \
             AND EXISTS (SELECT 1 FROM wl_payments_deposits d WHERE d.source_type = 'property.lease' \
                 AND d.source_id = l.root_id) ORDER BY l.id",
            &["tenant", "unit", "invoices", "cheques", "deposits"],
        ),
    ];
    for (subject_type, statement, names) in checks {
        let id = one(&app, statement).await;
        let subject = Subject::new(subject_type, id);
        let document = search::document_of(db, &subject)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("{subject} has a document"));
        assert!(!document.title.is_empty(), "{subject}: a title");
        let groups = related::related(db, &finance, &subject, PANEL_DEPTH)
            .await
            .unwrap();
        for name in *names {
            assert!(
                !group(&groups, name).is_empty(),
                "{subject}: the relation `{name}` lists a record; got {groups:?}"
            );
        }
    }
    // A refund of a deposit names the deposit and its lease.
    let refunds = ids(
        &app,
        "SELECT r.id FROM wl_payments_refunds r JOIN wl_payments_deposits d \
         ON r.source_type = 'payments.deposit' AND d.id = r.source_id \
         WHERE d.source_type = 'property.lease' ORDER BY r.id",
    )
    .await;
    if let Some(refund) = refunds.first() {
        let groups = related::related(
            db,
            &finance,
            &Subject::new("payments.refund", *refund),
            PANEL_DEPTH,
        )
        .await
        .unwrap();
        assert!(!group(&groups, "deposit").is_empty(), "{groups:?}");
        assert!(!group(&groups, "lease").is_empty(), "{groups:?}");
    }
    // A document: found by its title, in its record's scope.
    let documents = ids(
        &app,
        "SELECT id FROM wl_documents_documents WHERE subject_type = 'property.lease' ORDER BY id",
    )
    .await;
    if let Some(document) = documents.first() {
        let row = search::document_of(db, &Subject::new("wl_documents.document", *document))
            .await
            .unwrap()
            .expect("the document is indexed");
        assert_eq!(row.scope_type, "property_buildings");
    }

    // --- Never indexed: an IBAN or a card number (a refund's payee details,
    // a payment's bank account and method details).
    let ibans: i64 = renox::db::sql(
        "SELECT count(*) FROM wl_search_documents \
         WHERE keywords LIKE '%AE07%' OR secondary LIKE '%AE07%' OR title LIKE '%AE07%'",
    )
    .scalar(db)
    .await
    .unwrap();
    assert_eq!(ibans, 0);
    let payee: Vec<String> = renox::db::sql(
        "SELECT payee_details FROM wl_payments_refunds WHERE payee_details IS NOT NULL",
    )
    .fetch_all(db)
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get::<String>("payee_details").unwrap())
    .collect();
    for details in payee.iter().filter(|d| d.len() > 6) {
        let leaked: i64 = renox::db::sql(
            "SELECT count(*) FROM wl_search_documents WHERE keywords LIKE ? OR secondary LIKE ? OR title LIKE ?",
        )
        .bind(format!("%{details}%"))
        .bind(format!("%{details}%"))
        .bind(format!("%{details}%"))
        .scalar(db)
        .await
        .unwrap();
        assert_eq!(leaked, 0, "{details} is not indexed");
    }
}
