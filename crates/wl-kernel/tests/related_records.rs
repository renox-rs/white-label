//! Related records of any record (BL-AGT-03, `PLT.REL.01`): the *Related*
//! panel of an invoice lists its lines, their products (depth 2 through an
//! `expand` relation), the customer, the allocated payments, the documents
//! and the notes, each with its count; a group the user may not see is
//! absent; hidden records are neither listed nor counted; the panel's query
//! count does not grow with the rows; `related(...)` gives the same groups.
//!
//! Every subject is test-only: invoices, lines, items, parties and payments
//! of this file, and documents and notes attached by `Subject`.

use renox::auth::Permissions;
use renox::auth::permissions::{self, Scope};
use renox::db::{Migration, capture_queries, sql};
use renox::prelude::*;
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::related::{self, PANEL_DEPTH, Relation};
use wl_kernel::search::{self, Access, SearchDocument, Searchable, kinds};
use wl_kernel::{ClientConfig, Subject};

const INVOICE: &str = "invoicing.invoice";
const LINE: &str = "invoicing.invoice_line";
const ITEM: &str = "inventory.item";
const PARTY: &str = "parties.party";
const PAYMENT: &str = "payments.payment";
const DOCUMENT: &str = "documents.document";
const NOTE: &str = "notes.note";
const BUILDINGS: &str = "buildings";

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_rr_invoices")]
struct Invoice {
    id: i64,
    number: String,
    party_id: i64,
    building_id: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_rr_items")]
struct Item {
    id: i64,
    name: String,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_rr_parties")]
struct Party {
    id: i64,
    name: String,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_rr_payments")]
struct Payment {
    id: i64,
    number: String,
    voided: bool,
    building_id: i64,
}

const UP: &str = r#"
CREATE TABLE "test_rr_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    party_id INTEGER NOT NULL,
    building_id INTEGER NOT NULL
);
CREATE TABLE "test_rr_lines" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL,
    item_id INTEGER
);
CREATE TABLE "test_rr_items" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL
);
CREATE TABLE "test_rr_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL
);
CREATE TABLE "test_rr_payments" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    voided INTEGER NOT NULL DEFAULT 0,
    building_id INTEGER NOT NULL
);
CREATE TABLE "test_rr_allocations" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL,
    payment_id INTEGER NOT NULL
);
CREATE TABLE "test_rr_documents" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL
);
CREATE TABLE "test_rr_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_type TEXT NOT NULL,
    subject_id INTEGER NOT NULL
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_rr_notes";
DROP TABLE "test_rr_documents";
DROP TABLE "test_rr_allocations";
DROP TABLE "test_rr_payments";
DROP TABLE "test_rr_parties";
DROP TABLE "test_rr_items";
DROP TABLE "test_rr_lines";
DROP TABLE "test_rr_invoices";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261101000197_create_test_related_subjects",
    UP,
    Some(DOWN),
)];

struct Subjects;

impl Module for Subjects {
    fn name(&self) -> &'static str {
        "test-related-subjects"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Invoice>()
            .model::<Item>()
            .model::<Party>()
            .model::<Payment>();
        Searchable::of::<Invoice>(INVOICE)
            .permission("invoices.view")
            .label("Invoices")
            .link("/invoices/{id}")
            .document(|invoice: &Invoice| {
                SearchDocument::new(&invoice.number)
                    .identifier(kinds::NUMBER, &invoice.number)
                    .scope(Scope::new(BUILDINGS, invoice.building_id))
            })
            .register();
        Searchable::of::<Item>(ITEM)
            .permission("inventory.view")
            .document(|item: &Item| SearchDocument::new(&item.name))
            .register();
        Searchable::of::<Party>(PARTY)
            .permission("parties.view")
            .document(|party: &Party| SearchDocument::new(&party.name))
            .register();
        Searchable::of::<Payment>(PAYMENT)
            .permission("payments.view")
            .link("/payments/{id}")
            .document(|payment: &Payment| {
                SearchDocument::new(&payment.number)
                    .voided(payment.voided)
                    .scope(Scope::new(BUILDINGS, payment.building_id))
            })
            .register();

        Relation::has_many(INVOICE, "lines", LINE, "test_rr_lines", "invoice_id")
            .label("Lines")
            .permission("invoices.view")
            .expand()
            .register();
        Relation::belongs_to(LINE, "products", ITEM, "test_rr_lines", "item_id")
            .label("Products")
            .register();
        // The way back, declared by the "other" module: never followed from
        // the invoice.
        Relation::belongs_to(LINE, "invoice", INVOICE, "test_rr_lines", "invoice_id")
            .label("Invoice")
            .register();
        Relation::belongs_to(INVOICE, "customer", PARTY, "test_rr_invoices", "party_id")
            .label("Customer")
            .register();
        Relation::new(INVOICE, "payments", PAYMENT)
            .label("Payments")
            .load(|db, ids| {
                Box::pin(async move {
                    let placeholders = vec!["?"; ids.len()].join(", ");
                    let rows = sql(format!(
                        "SELECT invoice_id, payment_id FROM test_rr_allocations \
                         WHERE invoice_id IN ({placeholders}) ORDER BY id"
                    ))
                    .bind_all(ids.iter().map(|id| renox::db::DbValue::Integer(*id)))
                    .fetch_all(db)
                    .await?;
                    let mut pairs = Vec::new();
                    for row in &rows {
                        pairs.push((
                            Subject::new(INVOICE, row.try_get::<i64>("invoice_id")?),
                            Subject::new(PAYMENT, row.try_get::<i64>("payment_id")?),
                        ));
                    }
                    Ok(pairs)
                })
            })
            .register();
        Relation::attached("documents", DOCUMENT, "test_rr_documents")
            .label("Documents")
            .permission("documents.view")
            .register();
        Relation::attached("notes", NOTE, "test_rr_notes")
            .label("Notes")
            .permission("notes.view")
            .register();
    }
}

const CLIENT: &str = r#"
[client]
name = "Gulf Tower Management LLC"
app = "property"

[modules]
enabled = []

[[legal_entities]]
code = "PGP"
name_en = "Gulf Tower Management LLC"
default = true
"#;

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const ALL: &[&str] = &[
    "invoices.view",
    "inventory.view",
    "parties.view",
    "payments.view",
    "documents.view",
    "notes.view",
];

struct World {
    app: TestApp,
    _views: tempfile::TempDir,
    /// Everything, everywhere.
    clerk: User,
    /// Everything but `payments.view`.
    no_payments: User,
    /// Everything, in Tower 1 only.
    manager: User,
}

async fn world() -> World {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    let layout = dir.path().join("layouts/app.html");
    std::fs::create_dir_all(layout.parent().expect("a path")).expect("a directory");
    std::fs::write(layout, LAYOUT).expect("a layout");
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Subjects),
        config,
    )
    .expect("the kernel applies");
    let views = dir.path().to_path_buf();
    let app = TestApp::with_config(app, move |c| c.views_path = views.clone()).await;
    let db = app.db();
    permissions::define_role(db, "clerk", ALL).await.unwrap();
    let without: Vec<&str> = ALL
        .iter()
        .copied()
        .filter(|p| *p != "payments.view")
        .collect();
    permissions::define_role(db, "no_payments", &without)
        .await
        .unwrap();
    let user = |name: &'static str| async move {
        User::register(
            db,
            name,
            &format!("{name}@gulftower.example"),
            "password123",
        )
        .await
        .unwrap()
    };
    let clerk = user("clerk").await;
    clerk.assign_role(db, "clerk").await.unwrap();
    let no_payments = user("nopay").await;
    no_payments.assign_role(db, "no_payments").await.unwrap();
    let manager = user("manager").await;
    manager
        .assign_role_in(db, "clerk", &Scope::new(BUILDINGS, 1))
        .await
        .unwrap();
    World {
        app,
        _views: dir,
        clerk,
        no_payments,
        manager,
    }
}

impl World {
    async fn indexed<M>(&self, subject_type: &str, mut record: M) -> M
    where
        M: Model<Key = i64> + Send + Sync,
    {
        let mut tx = self.app.db().begin().await.unwrap();
        record.insert(&mut tx).await.unwrap();
        search::index_in(&mut tx, &Subject::new(subject_type, record.id()))
            .await
            .unwrap();
        tx.commit().await.unwrap();
        record
    }

    async fn run(&self, statement: &str, values: &[i64]) {
        values
            .iter()
            .fold(sql(statement), |q, v| q.bind(*v))
            .execute(self.app.db())
            .await
            .unwrap();
    }

    /// An issued invoice with `lines` lines over two products, `payments`
    /// allocated payments (the last one voided when `voided`) and one PDF.
    async fn invoice(&self, lines: usize, payments: usize, voided: bool) -> Invoice {
        let party = self
            .indexed(
                PARTY,
                Party {
                    name: "Mohammed Al Ali".into(),
                    ..Default::default()
                },
            )
            .await;
        let invoice = self
            .indexed(
                INVOICE,
                Invoice {
                    number: format!("INV-PGP-2026-{:04}", party.id),
                    party_id: party.id,
                    building_id: 1,
                    ..Default::default()
                },
            )
            .await;
        let mut items = Vec::new();
        for name in ["Chilled water", "Service charge"] {
            items.push(
                self.indexed(
                    ITEM,
                    Item {
                        name: name.into(),
                        ..Default::default()
                    },
                )
                .await,
            );
        }
        for n in 0..lines {
            let item = &items[usize::from(n + 1 == lines)];
            self.run(
                "INSERT INTO test_rr_lines (invoice_id, item_id) VALUES (?, ?)",
                &[invoice.id, item.id],
            )
            .await;
        }
        for n in 0..payments {
            let payment = self
                .indexed(
                    PAYMENT,
                    Payment {
                        number: format!("RCT-PGP-2026-{:04}-{n}", invoice.id),
                        voided: voided && n + 1 == payments,
                        building_id: 1,
                        ..Default::default()
                    },
                )
                .await;
            self.run(
                "INSERT INTO test_rr_allocations (invoice_id, payment_id) VALUES (?, ?)",
                &[invoice.id, payment.id],
            )
            .await;
        }
        sql("INSERT INTO test_rr_documents (subject_type, subject_id) VALUES (?, ?)")
            .bind(INVOICE)
            .bind(invoice.id)
            .execute(self.app.db())
            .await
            .unwrap();
        invoice
    }

    async fn panel(&self, user: &User, invoice: &Invoice) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app
            .get(&format!("/related/{INVOICE}:{}/panel", invoice.id))
            .await
    }
}

/// The `(group, count)` pairs the panel draws, in order.
fn groups_of(html: &str) -> Vec<(String, u64)> {
    let mut groups = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("data-group=\"") {
        rest = &rest[at + "data-group=\"".len()..];
        let name = rest[..rest.find('"').unwrap()].to_owned();
        let at = rest.find("data-count=\"").unwrap();
        let after = &rest[at + "data-count=\"".len()..];
        let count = after[..after.find('"').unwrap()].parse().unwrap();
        groups.push((name, count));
    }
    groups
}

#[renox::test]
async fn an_invoice_lists_its_lines_products_customer_payments_documents_and_notes() {
    let w = world().await;
    let invoice = w.invoice(3, 1, false).await;
    let page = w.panel(&w.clerk, &invoice).await;
    page.assert_ok();
    let groups = groups_of(&page.text());
    let expected: Vec<(String, u64)> = [
        ("lines", 3),
        ("customer", 1),
        ("payments", 1),
        ("documents", 1),
        ("notes", 0),
        ("products", 2),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_owned(), count))
    .collect();
    assert_eq!(groups, expected, "{}", page.text());
    page.assert_see("Chilled water");
    page.assert_see("Service charge");
    page.assert_see("Mohammed Al Ali");
    page.assert_see("/payments/");
    // The way back to the invoice is never a group of the invoice.
    page.assert_dont_see("data-group=\"invoice\"");

    // The page of its own shows the same panel under the invoice's number.
    w.app
        .get(&format!("/related/{INVOICE}:{}", invoice.id))
        .await
        .assert_ok()
        .assert_see(&invoice.number)
        .assert_see("data-group=\"lines\"");
}

#[renox::test]
async fn without_payments_view_the_payments_group_is_absent() {
    let w = world().await;
    let invoice = w.invoice(3, 1, false).await;
    let page = w.panel(&w.no_payments, &invoice).await;
    page.assert_ok();
    page.assert_dont_see("data-group=\"payments\"");
    page.assert_see("data-group=\"lines\" data-count=\"3\"");
    let access = Access::of_user(w.app.db(), &w.no_payments).await.unwrap();
    let groups = related::related(
        w.app.db(),
        &access,
        &Subject::new(INVOICE, invoice.id),
        PANEL_DEPTH,
    )
    .await
    .unwrap();
    assert!(groups.iter().all(|g| g.name != "payments"));
}

#[renox::test]
async fn a_hidden_payment_is_neither_listed_nor_counted_and_a_voided_one_is_marked() {
    let w = world().await;
    let invoice = w.invoice(1, 2, true).await;
    // The second payment moves to Tower 2, out of the manager's reach.
    sql("UPDATE test_rr_payments SET building_id = 2 WHERE voided = 0")
        .execute(w.app.db())
        .await
        .unwrap();
    let moved: Vec<i64> = sql("SELECT id FROM test_rr_payments WHERE voided = 0")
        .fetch_all(w.app.db())
        .await
        .unwrap()
        .iter()
        .map(|row| row.try_get::<i64>("id").unwrap())
        .collect();
    let mut tx = w.app.db().begin().await.unwrap();
    for id in moved {
        search::index_in(&mut tx, &Subject::new(PAYMENT, id))
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();

    let page = w.panel(&w.manager, &invoice).await;
    page.assert_ok();
    page.assert_see("data-group=\"payments\" data-count=\"1\"");
    page.assert_see(">Void<");
    let page = w.panel(&w.clerk, &invoice).await;
    page.assert_see("data-group=\"payments\" data-count=\"2\"");

    // A user who may not open the invoice gets a 404 for its panel.
    sql("UPDATE test_rr_invoices SET building_id = 2")
        .execute(w.app.db())
        .await
        .unwrap();
    let mut tx = w.app.db().begin().await.unwrap();
    search::index_in(&mut tx, &Subject::new(INVOICE, invoice.id))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    w.panel(&w.manager, &invoice).await.assert_not_found();
}

#[renox::test]
async fn a_deleted_target_is_skipped() {
    let w = world().await;
    let invoice = w.invoice(2, 1, false).await;
    // The payment's row and document go behind the panel's back.
    sql("DELETE FROM test_rr_payments")
        .execute(w.app.db())
        .await
        .unwrap();
    sql("DELETE FROM wl_search_documents WHERE subject_type = 'payments.payment'")
        .execute(w.app.db())
        .await
        .unwrap();
    let page = w.panel(&w.clerk, &invoice).await;
    page.assert_ok();
    page.assert_see("data-group=\"payments\" data-count=\"0\"");
}

#[renox::test]
async fn the_panel_runs_the_same_number_of_queries_whatever_the_number_of_rows() {
    let w = world().await;
    let small = w.invoice(3, 1, false).await;
    let large = w.invoice(30, 12, false).await;
    // Warm the session.
    w.panel(&w.clerk, &small).await.assert_ok();
    w.app.acting_as(&w.clerk);
    let (page, few) =
        capture_queries(w.app.get(&format!("/related/{INVOICE}:{}/panel", small.id))).await;
    page.assert_ok();
    let (page, many) =
        capture_queries(w.app.get(&format!("/related/{INVOICE}:{}/panel", large.id))).await;
    page.assert_ok();
    page.assert_see("data-group=\"lines\" data-count=\"30\"");
    page.assert_see("and 20 more");
    assert_eq!(few.len(), many.len(), "{few:#?}\n{many:#?}");
}

#[renox::test]
async fn related_returns_the_groups_the_panel_shows() {
    let w = world().await;
    let invoice = w.invoice(3, 1, false).await;
    let subject = Subject::new(INVOICE, invoice.id);
    for user in [&w.clerk, &w.no_payments, &w.manager] {
        let page = w.panel(user, &invoice).await;
        page.assert_ok();
        let access = Access::of_user(w.app.db(), user).await.unwrap();
        let groups = related::related(w.app.db(), &access, &subject, PANEL_DEPTH)
            .await
            .unwrap();
        let from_function: Vec<(String, u64)> =
            groups.iter().map(|g| (g.name.clone(), g.count)).collect();
        assert_eq!(groups_of(&page.text()), from_function, "{}", user.email);
        for record in groups.iter().flat_map(|g| g.records.iter()) {
            page.assert_see(&record.title);
        }
    }
    // Depth 1 stops at the invoice's own relations.
    let access = Access::of_user(w.app.db(), &w.clerk).await.unwrap();
    let shallow = related::related(w.app.db(), &access, &subject, 1)
        .await
        .unwrap();
    assert!(shallow.iter().all(|g| g.name != "products"));
    assert!(shallow.iter().any(|g| g.name == "lines"));
}
