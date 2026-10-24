//! The search index (BL-AGT-01, wl-kernel §5.8): registered types, the
//! documents written in the transaction that changes a record, sensitive
//! identifiers hashed and kept out of the text, `search:rebuild`, voids and
//! an undone import.
//!
//! The subjects are a test-only invoice (with a state machine and the void
//! transitions) and a test-only party with an Arabic and a Latin name, an
//! Emirates ID and a phone.

use std::time::Duration;

use renox::auth::Permissions;
use renox::auth::permissions::Scope;
use renox::db::{Migration, sql};
use renox::import::Import;
use renox::prelude::*;
use renox::state_machine::{Machine, StateMachine, Transition};
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::machines::Run;
use wl_kernel::search::{
    self, SearchDocument, SearchDocumentRow, SearchIdentifierRow, Searchable, Sensitivity, kinds,
};
use wl_kernel::voids;
use wl_kernel::{ClientConfig, Subject, SubjectMachine, Voidable};

const INVOICE: &str = "invoicing.invoice";
const PARTY: &str = "parties.party";

#[derive(DbEnum, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
enum InvoiceStatus {
    #[default]
    Draft,
    Issued,
    Voided,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_invoices", state_machine)]
struct Invoice {
    id: i64,
    status: InvoiceStatus,
    number: String,
    customer: String,
    building_id: i64,
}

impl StateMachine for Invoice {
    type State = InvoiceStatus;
    const COLUMN: renox::db::Col<Invoice, InvoiceStatus> = Invoice::STATUS;

    fn machine() -> Machine<Self> {
        use InvoiceStatus::{Draft, Issued, Voided};
        voids::transitions::<Invoice, _>(INVOICE, [Issued])
            .into_iter()
            .fold(
                Machine::new()
                    .initial([Draft])
                    .state(Draft)
                    .state(Issued)
                    .state(Voided)
                    .terminal()
                    .transition(Transition::new("issue").from([Draft]).to(Issued).system()),
                Machine::transition,
            )
    }
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_parties")]
struct Party {
    id: i64,
    name_en: String,
    name_ar: String,
    emirates_id: String,
    phone: String,
}

/// One line of an import of parties.
#[derive(Deserialize, Validate)]
struct PartyLine {
    #[validate(required)]
    name_en: String,
    #[validate(required)]
    emirates_id: String,
}

const UP: &str = r#"
CREATE TABLE "test_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    status TEXT NOT NULL DEFAULT 'draft',
    number TEXT NOT NULL,
    customer TEXT NOT NULL,
    building_id INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE "test_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    emirates_id TEXT NOT NULL DEFAULT '',
    phone TEXT NOT NULL DEFAULT '',
    created_at TEXT,
    updated_at TEXT
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_parties";
DROP TABLE "test_invoices";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261101000199_create_test_search_subjects",
    UP,
    Some(DOWN),
)];

fn invoice_document(invoice: &Invoice) -> SearchDocument {
    SearchDocument::new(&invoice.number)
        .secondary(&invoice.customer)
        .identifier(kinds::NUMBER, &invoice.number)
        .status(invoice.status.to_string())
        .voided(invoice.status == InvoiceStatus::Voided)
        .scope(Scope::new("buildings", invoice.building_id))
}

fn party_document(party: &Party) -> SearchDocument {
    SearchDocument::new(&party.name_en)
        .secondary(&party.name_ar)
        // A careless module that also lists the ID as a keyword: the kernel
        // still keeps it out of the text.
        .keyword(&party.emirates_id)
        .identifier(kinds::EMIRATES_ID, &party.emirates_id)
        .identifier(kinds::PHONE, &party.phone)
        .sensitivity(Sensitivity::Personal)
}

struct Subjects;

impl Module for Subjects {
    fn name(&self) -> &'static str {
        "test-search-subjects"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Invoice>().model::<Party>();
        SubjectMachine::of::<Invoice>(INVOICE).register();
        Voidable::of::<Invoice>(INVOICE)
            .permission("invoices.void")
            .number("invoice", |invoice: &Invoice| Some(invoice.number.clone()))
            .register();
        Searchable::of::<Invoice>(INVOICE)
            .permission("invoices.view")
            .portal_visible()
            .document(invoice_document)
            .register();
        Searchable::of::<Party>(PARTY)
            .permission("parties.view")
            .document(party_document)
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

async fn app() -> TestApp {
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(
        App::new()
            .module(Auth::new())
            .module(Permissions)
            .module(Subjects),
        config,
    )
    .expect("the kernel applies");
    TestApp::new(app).await
}

/// A draft invoice, written without its document (as before the module
/// indexes it).
async fn draft(app: &TestApp, number: &str, customer: &str) -> Invoice {
    Invoice::create(
        app.db(),
        Invoice {
            number: number.into(),
            customer: customer.into(),
            building_id: 3,
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// A party with its document written in the same transaction.
async fn party(app: &TestApp, name_en: &str, name_ar: &str, emirates_id: &str) -> Party {
    let mut tx = app.db().begin().await.unwrap();
    let mut party = Party {
        name_en: name_en.into(),
        name_ar: name_ar.into(),
        emirates_id: emirates_id.into(),
        phone: "050 123 4567".into(),
        ..Default::default()
    };
    party.insert(&mut tx).await.unwrap();
    assert!(
        search::index_in(&mut tx, &Subject::new(PARTY, party.id))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    party
}

/// Everything stored for the index, without ids and times, in a stable order.
async fn snapshot(app: &TestApp) -> Vec<String> {
    let mut rows: Vec<String> = SearchDocumentRow::query()
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|d| {
            format!(
                "doc {} {} | {} | {} | {} | {} | {:?} | {} | {} | {} | {}:{} | {}",
                d.subject_type,
                d.subject_id,
                d.title,
                d.secondary,
                d.keywords,
                d.forms,
                d.status,
                d.voided,
                d.permission,
                d.portal_visible,
                d.scope_type,
                d.scope_id,
                d.sensitivity
            )
        })
        .collect();
    rows.extend(
        SearchIdentifierRow::query()
            .get(app.db())
            .await
            .unwrap()
            .into_iter()
            .map(|i| {
                format!(
                    "id {} {} {} {} {}",
                    i.subject_type, i.subject_id, i.kind, i.value, i.hashed
                )
            }),
    );
    rows.sort();
    rows
}

#[renox::test]
async fn issuing_an_invoice_writes_its_document_in_the_same_transaction() {
    let app = app().await;
    let invoice = draft(&app, "INV-PGP-2026-0004", "Mohammed Al Ali").await;
    let subject = Subject::new(INVOICE, invoice.id);
    let machine = wl_kernel::machines::get(INVOICE).expect("the machine is registered");

    // Rolled back: no document.
    let mut tx = app.db().begin().await.unwrap();
    let ran = machine
        .run_in(&mut tx, invoice.id, "issue", Run::new())
        .await
        .unwrap();
    assert_eq!(ran.to, "issued");
    tx.rollback().await.unwrap();
    assert!(
        search::document_of(app.db(), &subject)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        search::lookup_identifier(app.db(), "INV-PGP-2026-0004")
            .await
            .unwrap()
            .is_empty()
    );

    // Committed: one document, with the number as an identifier.
    let mut tx = app.db().begin().await.unwrap();
    let ran = machine
        .run_in(&mut tx, invoice.id, "issue", Run::new())
        .await
        .unwrap();
    assert_eq!(ran.to, "issued");
    tx.commit().await.unwrap();
    let document = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .expect("the issued invoice has a document");
    assert_eq!(document.title, "INV-PGP-2026-0004");
    assert_eq!(document.secondary, "Mohammed Al Ali");
    assert_eq!(document.status.as_deref(), Some("issued"));
    assert_eq!(document.permission, "invoices.view");
    assert!(document.portal_visible);
    assert_eq!(
        (document.scope_type.as_str(), document.scope_id.as_str()),
        ("buildings", "3")
    );
    assert_eq!(
        SearchDocumentRow::query()
            .where_eq("subject_type", INVOICE)
            .count(app.db())
            .await
            .unwrap(),
        1
    );
    let identifiers = search::identifiers_of(app.db(), &subject).await.unwrap();
    assert_eq!(identifiers.len(), 1);
    assert_eq!(identifiers[0].kind, kinds::NUMBER);
    assert_eq!(identifiers[0].value, "INVPGP20260004");
    assert!(!identifiers[0].hashed);

    // Found by its number however it is typed.
    for typed in ["INV-PGP-2026-0004", "inv pgp 2026 0004", "invpgp20260004"] {
        let found = search::lookup_identifier(app.db(), typed).await.unwrap();
        assert_eq!(found.len(), 1, "{typed}");
        assert_eq!(found[0].subject, subject);
        assert_eq!(found[0].kind, kinds::NUMBER);
    }
}

#[renox::test]
async fn a_party_is_found_by_its_arabic_and_its_latin_name() {
    let app = app().await;
    let party = party(&app, "Mohammed Al Ali", "محمد العلي", "784-1990-1234567-1").await;
    let document = search::document_of(app.db(), &Subject::new(PARTY, party.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.title, "Mohammed Al Ali");
    assert_eq!(document.secondary, "محمد العلي");
    assert!(
        document
            .forms
            .split(' ')
            .any(|key| key == renox::text::match_key("محمد"))
    );

    // The full-text index finds it by either form, and through the R22
    // normalisation by an Arabic spelling with marks (harakat).
    for words in ["Mohammed", "mohammed ali", "محمد", "العلي", "مُحَمَّد"] {
        let found = SearchDocumentRow::search(words)
            .where_eq("subject_type", PARTY)
            .get(app.db())
            .await
            .unwrap();
        assert_eq!(found.len(), 1, "{words}");
        assert_eq!(found[0].subject_id, party.id);
    }
    // The Latin spelling's match key is a word of the forms column, so a
    // Latin search can reach the Arabic-only record and back.
    let key = renox::text::match_key("Muhammad");
    let found = SearchDocumentRow::search(&key)
        .where_eq("subject_type", PARTY)
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
}

#[renox::test]
async fn an_emirates_id_is_stored_as_a_hash_and_matches_only_exactly() {
    let app = app().await;
    let party = party(&app, "Mohammed Al Ali", "محمد العلي", "784-1990-1234567-1").await;
    let subject = Subject::new(PARTY, party.id);

    let identifiers = search::identifiers_of(app.db(), &subject).await.unwrap();
    let eid = identifiers
        .iter()
        .find(|i| i.kind == kinds::EMIRATES_ID)
        .expect("the ID is an identifier");
    assert!(eid.hashed);
    assert!(!eid.value.contains("784"));
    assert!(!eid.value.contains("1234567"));
    let phone = identifiers.iter().find(|i| i.kind == kinds::PHONE).unwrap();
    assert!(phone.hashed);
    assert!(!phone.value.contains("501234567"));

    // Not in any full-text column, although the module listed it as a keyword.
    let document = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    for column in [
        &document.title,
        &document.secondary,
        &document.keywords,
        &document.forms,
    ] {
        assert!(!column.contains("784"), "{column}");
        assert!(!column.contains("1234567"), "{column}");
    }
    assert_eq!(document.sensitivity, "personal");
    let raw = sql("SELECT COUNT(*) FROM wl_search_documents_search WHERE wl_search_documents_search MATCH '784*'")
        .scalar::<i64>(app.db())
        .await
        .unwrap();
    assert_eq!(raw, 0, "the full-text index holds no digit of the ID");

    // The exact value finds it, typed any way; another ID does not.
    for typed in [
        "784-1990-1234567-1",
        "784199012345671",
        "٧٨٤-١٩٩٠-١٢٣٤٥٦٧-١",
    ] {
        let found = search::lookup_identifier(app.db(), typed).await.unwrap();
        assert_eq!(found.len(), 1, "{typed}");
        assert_eq!(found[0].subject, subject);
        assert_eq!(found[0].kind, kinds::EMIRATES_ID);
    }
    assert!(
        search::lookup_identifier(app.db(), "784-1990-1234567-2")
            .await
            .unwrap()
            .is_empty()
    );
    // The phone, typed the UAE way or in E.164.
    for typed in ["+971 50 123 4567", "0501234567"] {
        let found = search::lookup_identifier(app.db(), typed).await.unwrap();
        assert_eq!(found.len(), 1, "{typed}");
        assert_eq!(found[0].kind, kinds::PHONE);
    }
}

#[renox::test]
async fn two_records_with_the_same_identifier_both_match() {
    let app = app().await;
    let first = party(&app, "Ali Hassan", "", "784-1985-7654321-1").await;
    let second = party(&app, "Sara Hassan", "", "784-1987-7654321-2").await;
    // Both have the phone 050 123 4567.
    let found = search::lookup_identifier(app.db(), "050 123 4567")
        .await
        .unwrap();
    let ids: Vec<i64> = found.iter().map(|m| m.subject.subject_id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&first.id) && ids.contains(&second.id));
}

#[renox::test]
async fn rebuild_gives_the_same_index_as_the_writes_and_drops_deleted_records() {
    let app = app().await;
    let machine = wl_kernel::machines::get(INVOICE).unwrap();
    for (number, customer) in [
        ("INV-PGP-2026-0004", "Mohammed Al Ali"),
        ("INV-PGP-2026-0005", "Sara Hassan"),
        ("INV-PGP-2026-0006", "Gulf Tower Owners"),
    ] {
        let invoice = draft(&app, number, customer).await;
        let mut tx = app.db().begin().await.unwrap();
        let ran = machine
            .run_in(&mut tx, invoice.id, "issue", Run::new())
            .await
            .unwrap();
        assert_eq!(ran.to, "issued");
        tx.commit().await.unwrap();
    }
    party(&app, "Mohammed Al Ali", "محمد العلي", "784-1990-1234567-1").await;
    party(&app, "Sara Hassan", "سارة حسن", "784-1987-7654321-2").await;
    let written = snapshot(&app).await;
    assert_eq!(
        written.iter().filter(|row| row.starts_with("doc ")).count(),
        5
    );

    app.kernel()
        .call(search::REBUILD_COMMAND, Vec::<String>::new())
        .await
        .expect("search:rebuild runs");
    assert_eq!(snapshot(&app).await, written);

    // A record deleted behind the index's back loses its document on a
    // rebuild of its type; the other type is untouched.
    sql("DELETE FROM test_invoices WHERE number = 'INV-PGP-2026-0005'")
        .execute(app.db())
        .await
        .unwrap();
    app.kernel()
        .call(search::REBUILD_COMMAND, vec!["--type", INVOICE])
        .await
        .expect("search:rebuild --type runs");
    let rebuilt = snapshot(&app).await;
    assert_eq!(
        rebuilt.len(),
        written.len() - 2,
        "one document and one identifier"
    );
    assert!(!rebuilt.iter().any(|row| row.contains("INVPGP20260005")));
    assert_eq!(
        search::rebuild_type(app.db(), "nothing.here")
            .await
            .unwrap_err()
            .to_string(),
        renox::Error::BadRequest("`nothing.here` is not searchable".into()).to_string()
    );
}

#[renox::test]
async fn a_voided_invoice_stays_findable_by_its_number_marked_void() {
    let app = app().await;
    let invoice = draft(&app, "INV-PGP-2026-0009", "Sara Hassan").await;
    let machine = wl_kernel::machines::get(INVOICE).unwrap();
    let mut tx = app.db().begin().await.unwrap();
    let ran = machine
        .run_in(&mut tx, invoice.id, "issue", Run::new())
        .await
        .unwrap();
    assert_eq!(ran.to, "issued");
    tx.commit().await.unwrap();

    // `void_approved` runs the void's effect: the document is written again
    // in the same transaction, with the record already voided.
    let mut tx = app.db().begin().await.unwrap();
    let ran = machine
        .run_in(
            &mut tx,
            invoice.id,
            "void_approved",
            Run::new()
                .reason("entered twice")
                .meta(renox::serde_json::json!({ "requester_id": 1 })),
        )
        .await
        .unwrap();
    assert_eq!(ran.to, "voided");
    tx.commit().await.unwrap();

    let subject = Subject::new(INVOICE, invoice.id);
    let document = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    assert!(document.voided);
    assert_eq!(document.status.as_deref(), Some("voided"));
    let found = search::lookup_identifier(app.db(), "INV-PGP-2026-0009")
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].subject, subject);
}

#[renox::test]
async fn a_change_of_scope_moves_the_document_and_a_deleted_record_leaves() {
    let app = app().await;
    let mut invoice = draft(&app, "INV-PGP-2026-0010", "Sara Hassan").await;
    let subject = Subject::new(INVOICE, invoice.id);
    let mut tx = app.db().begin().await.unwrap();
    search::index_in(&mut tx, &subject).await.unwrap();
    tx.commit().await.unwrap();

    let mut tx = app.db().begin().await.unwrap();
    invoice.building_id = 8;
    invoice.save(&mut tx).await.unwrap();
    search::index_in(&mut tx, &subject).await.unwrap();
    tx.commit().await.unwrap();
    let document = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(document.scope_id, "8");
    assert_eq!(
        SearchDocumentRow::query()
            .where_eq("subject_type", INVOICE)
            .count(app.db())
            .await
            .unwrap(),
        1,
        "written again, not twice"
    );

    let mut tx = app.db().begin().await.unwrap();
    invoice.delete(&mut tx).await.unwrap();
    search::index_in(&mut tx, &subject).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        search::document_of(app.db(), &subject)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        search::identifiers_of(app.db(), &subject)
            .await
            .unwrap()
            .is_empty()
    );

    // A type nobody registered is not indexed.
    let mut tx = app.db().begin().await.unwrap();
    assert!(
        !search::index_in(&mut tx, &Subject::new("nothing.here", 1))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
}

#[renox::test]
async fn an_undone_import_takes_its_documents_with_it() {
    let app = app().await;
    let report = Import::csv(
        "name_en,emirates_id\n\
         Mohammed Al Ali,784-1990-1234567-1\n\
         Sara Hassan,784-1987-7654321-2\n",
    )
    .undoable(Duration::from_secs(3600))
    .run(app.state(), |tx, line: PartyLine| {
        Box::pin(async move {
            let mut party = Party {
                name_en: line.name_en,
                emirates_id: line.emirates_id,
                ..Default::default()
            };
            tx.insert(&mut party).await?;
            search::index_in(tx, &Subject::new(PARTY, party.id)).await?;
            Ok(())
        })
    })
    .await
    .unwrap();
    assert_eq!(report.imported, 2);
    let batch = report.batch_id.expect("an undoable import has a batch");
    assert_eq!(
        SearchDocumentRow::query()
            .where_eq("subject_type", PARTY)
            .count(app.db())
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        search::lookup_identifier(app.db(), "784-1987-7654321-2")
            .await
            .unwrap()
            .len(),
        1
    );

    let undone = search::undo_import(app.state(), batch).await.unwrap();
    assert_eq!(undone.deleted, 2);
    assert_eq!(
        SearchDocumentRow::query()
            .where_eq("subject_type", PARTY)
            .count(app.db())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        SearchIdentifierRow::query()
            .where_eq("subject_type", PARTY)
            .count(app.db())
            .await
            .unwrap(),
        0
    );
    assert!(
        search::lookup_identifier(app.db(), "784-1987-7654321-2")
            .await
            .unwrap()
            .is_empty()
    );
}

/// One line of an import that renames parties found by their English name.
#[derive(Deserialize, Validate)]
struct RenameLine {
    #[validate(required)]
    name_en: String,
    #[validate(required)]
    name_ar: String,
}

#[renox::test]
async fn an_undone_import_that_changed_a_record_gives_back_its_old_document() {
    let app = app().await;
    let party = party(&app, "Sara Hassan", "سارة حسن", "784-1987-7654321-2").await;
    let subject = Subject::new(PARTY, party.id);

    let report = Import::csv("name_en,name_ar\nSara Hassan,سارة الحسن\n")
        .undoable(Duration::from_secs(3600))
        .run(app.state(), |tx, line: RenameLine| {
            Box::pin(async move {
                let mut party = Party::query()
                    .where_eq("name_en", line.name_en.as_str())
                    .first(&mut **tx)
                    .await?
                    .ok_or(renox::Error::NotFound)?;
                party.name_ar = line.name_ar;
                tx.update(&mut party).await?;
                search::index_in(tx, &Subject::new(PARTY, party.id)).await?;
                Ok(())
            })
        })
        .await
        .unwrap();
    assert_eq!(report.imported, 1);
    let changed = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(changed.secondary, "سارة الحسن");

    let undone = search::undo_import(app.state(), report.batch_id.unwrap())
        .await
        .unwrap();
    assert_eq!(undone.restored, 1);
    let restored = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.secondary, "سارة حسن");
    // Still found by its ID after the undo.
    assert_eq!(
        search::lookup_identifier(app.db(), "784-1987-7654321-2")
            .await
            .unwrap()
            .len(),
        1
    );
}
