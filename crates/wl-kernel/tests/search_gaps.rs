//! The search gaps found when the first modules registered their types
//! (#544), and the portal search (BL-AGT-05):
//!
//! - a document reads the scope of another record **in the indexing
//!   transaction** (`search::scope_in`): from a resolver registered with
//!   `register_scope_in`, or from that record's own document;
//! - `search:rebuild` builds a type after the types it declared with
//!   `.after(..)`, so an invoice rebuilt from nothing still reads its
//!   customer's name;
//! - a record that needs a permission of its own (`SearchDocument::requires`)
//!   is found, related and opened only by the holders of that permission;
//! - the portal search finds the portal-visible records of the portal
//!   user's parties only, never another party's, never a type that is not
//!   portal-visible.

use renox::auth::Permissions;
use renox::auth::permissions::{self, Scope};
use renox::db::Migration;
use renox::prelude::*;
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::search::{self, Access, PortalReach, SearchDocument, Searchable, kinds};
use wl_kernel::{ClientConfig, Subject};

/// Alphabetically before the party type: only `.after` puts it later.
const INVOICE: &str = "gaps.invoice";
const PARTY: &str = "gaps.party";
const LEASE: &str = "gaps.lease";
const UNIT: &str = "gaps.unit";
const NOTE: &str = "gaps.note";
const BUILDINGS: &str = "gap_buildings";

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_gap_parties")]
struct Party {
    id: i64,
    name: String,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_gap_leases")]
struct Lease {
    id: i64,
    number: String,
    building_id: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_gap_units")]
struct Unit {
    id: i64,
    number: String,
    building_id: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_gap_invoices")]
struct Invoice {
    id: i64,
    number: String,
    party_id: i64,
    source_type: String,
    source_id: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_gap_notes")]
struct Note {
    id: i64,
    number: String,
    text: String,
    secret: bool,
    party_id: i64,
    building_id: i64,
}

const UP: &str = r#"
CREATE TABLE "test_gap_parties" (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL);
CREATE TABLE "test_gap_leases" (id INTEGER PRIMARY KEY AUTOINCREMENT, number TEXT NOT NULL, building_id INTEGER NOT NULL);
CREATE TABLE "test_gap_units" (id INTEGER PRIMARY KEY AUTOINCREMENT, number TEXT NOT NULL, building_id INTEGER NOT NULL);
CREATE TABLE "test_gap_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    party_id INTEGER NOT NULL,
    source_type TEXT NOT NULL,
    source_id INTEGER NOT NULL
);
CREATE TABLE "test_gap_notes" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    text TEXT NOT NULL,
    secret INTEGER NOT NULL DEFAULT 0,
    party_id INTEGER NOT NULL,
    building_id INTEGER NOT NULL
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_gap_notes";
DROP TABLE "test_gap_invoices";
DROP TABLE "test_gap_units";
DROP TABLE "test_gap_leases";
DROP TABLE "test_gap_parties";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20270702300199_create_test_search_gap_subjects",
    UP,
    Some(DOWN),
)];

/// The name of party `id`, read from its search document in `tx`.
async fn party_name_in(tx: &mut renox::db::Transaction, id: i64) -> Result<String> {
    Ok(search::SearchDocumentRow::query()
        .where_eq("subject_type", PARTY)
        .where_eq("subject_id", id)
        .first(&mut *tx)
        .await?
        .map(|d| d.title)
        .unwrap_or_default())
}

async fn invoice_document(
    tx: &mut renox::db::Transaction,
    invoice: &Invoice,
) -> Result<SearchDocument> {
    let customer = party_name_in(tx, invoice.party_id).await?;
    let scope =
        search::scope_in(tx, &[Subject::new(&invoice.source_type, invoice.source_id)]).await?;
    Ok(SearchDocument::new(&invoice.number)
        .secondary(customer)
        .identifier(kinds::NUMBER, &invoice.number)
        .scope(scope)
        .party(invoice.party_id))
}

fn note_document(note: &Note) -> SearchDocument {
    let document = SearchDocument::new(&note.number)
        .secondary(&note.text)
        .identifier(kinds::NUMBER, &note.number)
        .scope(Scope::new(BUILDINGS, note.building_id))
        // An internal note names the party, yet its type is not portal-visible.
        .party(note.party_id);
    if note.secret {
        document.requires("notes.secret.view")
    } else {
        document
    }
}

struct Subjects;

impl Module for Subjects {
    fn name(&self) -> &'static str {
        "test-search-gap-subjects"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Party>()
            .model::<Lease>()
            .model::<Unit>()
            .model::<Invoice>()
            .model::<Note>();
        Searchable::of::<Party>(PARTY)
            .permission("parties.view")
            .document(|party: &Party| SearchDocument::new(&party.name))
            .register();
        Searchable::of::<Unit>(UNIT)
            .permission("units.view")
            .document(|unit: &Unit| {
                SearchDocument::new(&unit.number).scope(Scope::new(BUILDINGS, unit.building_id))
            })
            .register();
        Searchable::of::<Invoice>(INVOICE)
            .after(PARTY)
            .permission("invoices.view")
            .portal_visible()
            .portal_link("/portal/{party}/invoices")
            .portal_permission("portal.invoices.view")
            .document_in(|tx, invoice| Box::pin(invoice_document(tx, invoice)))
            .register();
        Searchable::of::<Note>(NOTE)
            .permission("notes.view")
            .record_permission("notes.secret.view")
            .document(note_document)
            .register();
        // A lease has no machine and no document: its scope is read in the
        // transaction by a resolver.
        search::register_scope_in(LEASE, |tx, id| {
            Box::pin(async move {
                let lease = Lease::query().where_eq("id", id).first(&mut *tx).await?;
                Ok(lease.map_or_else(Scope::global, |l| Scope::new(BUILDINGS, l.building_id)))
            })
        });
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

/// Writes `record` and its search document in one transaction, as a module
/// does.
async fn indexed<M>(app: &TestApp, subject_type: &str, mut record: M) -> M
where
    M: Model<Key = i64> + Send + Sync,
{
    let mut tx = app.db().begin().await.unwrap();
    record.insert(&mut tx).await.unwrap();
    assert!(
        search::index_in(&mut tx, &Subject::new(subject_type, record.id()))
            .await
            .unwrap()
    );
    tx.commit().await.unwrap();
    record
}

async fn user_with(app: &TestApp, name: &str, role: &str, grants: &[&str]) -> User {
    let db = app.db();
    permissions::define_role(db, role, grants).await.unwrap();
    let user = User::register(
        db,
        name,
        &format!("{name}@gulftower.example"),
        "password123",
    )
    .await
    .unwrap();
    user.assign_role(db, role).await.unwrap();
    user
}

#[renox::test]
async fn a_document_reads_another_records_scope_inside_the_indexing_transaction() {
    let app = app().await;
    let party = indexed(
        &app,
        PARTY,
        Party {
            id: 0,
            name: "Al Noor Trading".into(),
        },
    )
    .await;
    let lease = Lease::create(
        app.db(),
        Lease {
            id: 0,
            number: "L-1".into(),
            building_id: 7,
        },
    )
    .await
    .unwrap();
    let unit = indexed(
        &app,
        UNIT,
        Unit {
            id: 0,
            number: "1204".into(),
            building_id: 9,
        },
    )
    .await;

    // The lease's scope comes from the resolver, read in the transaction
    // that writes the invoice (a second connection would never come on
    // SQLite).
    let by_lease = indexed(
        &app,
        INVOICE,
        Invoice {
            id: 0,
            number: "INV-1".into(),
            party_id: party.id,
            source_type: LEASE.into(),
            source_id: lease.id,
        },
    )
    .await;
    let row = search::document_of(app.db(), &Subject::new(INVOICE, by_lease.id))
        .await
        .unwrap()
        .expect("a document");
    assert_eq!(
        (row.scope_type.as_str(), row.scope_id.as_str()),
        (BUILDINGS, "7")
    );
    assert_eq!(row.secondary, "Al Noor Trading");

    // The unit has no resolver: its own document gives the scope.
    let by_unit = indexed(
        &app,
        INVOICE,
        Invoice {
            id: 0,
            number: "INV-2".into(),
            party_id: party.id,
            source_type: UNIT.into(),
            source_id: unit.id,
        },
    )
    .await;
    let row = search::document_of(app.db(), &Subject::new(INVOICE, by_unit.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (row.scope_type.as_str(), row.scope_id.as_str()),
        (BUILDINGS, "9")
    );

    // A record that is gone is global, and so is an unknown type.
    let mut tx = app.db().begin().await.unwrap();
    let gone = search::scope_in(
        &mut tx,
        &[Subject::new(LEASE, 999), Subject::new("gaps.unknown", 1)],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(gone.is_global());
}

#[renox::test]
async fn rebuild_builds_the_parties_before_the_invoices_that_read_their_names() {
    let app = app().await;
    let order = search::rebuild_sequence();
    let at = |name: &str| order.iter().position(|t| t == name).unwrap();
    assert!(at(PARTY) < at(INVOICE), "{order:?}");

    let party = indexed(
        &app,
        PARTY,
        Party {
            id: 0,
            name: "Fatima Al Zaabi".into(),
        },
    )
    .await;
    let invoice = indexed(
        &app,
        INVOICE,
        Invoice {
            id: 0,
            number: "INV-9".into(),
            party_id: party.id,
            source_type: UNIT.into(),
            source_id: 0,
        },
    )
    .await;
    // The index is lost; a rebuild from nothing still names the customer.
    renox::db::sql("DELETE FROM wl_search_documents")
        .execute(app.db())
        .await
        .unwrap();
    let counts = search::rebuild(app.db()).await.unwrap();
    let built: Vec<&str> = counts.iter().map(|(t, _)| t.as_str()).collect();
    let pos = |name: &str| built.iter().position(|t| *t == name).unwrap();
    assert!(pos(PARTY) < pos(INVOICE), "{built:?}");
    let row = search::document_of(app.db(), &Subject::new(INVOICE, invoice.id))
        .await
        .unwrap()
        .expect("rebuilt");
    assert_eq!(row.secondary, "Fatima Al Zaabi");
    assert_eq!(
        search::parties_of(app.db(), &Subject::new(INVOICE, invoice.id))
            .await
            .unwrap(),
        [party.id]
    );
}

#[renox::test]
async fn a_secret_record_is_found_only_by_the_holders_of_its_own_permission() {
    let app = app().await;
    let plain = indexed(
        &app,
        NOTE,
        Note {
            id: 0,
            number: "NOTE-1".into(),
            text: "boiler service".into(),
            secret: false,
            party_id: 1,
            building_id: 3,
        },
    )
    .await;
    let secret = indexed(
        &app,
        NOTE,
        Note {
            id: 0,
            number: "NOTE-2".into(),
            text: "boiler complaint about the manager".into(),
            secret: true,
            party_id: 1,
            building_id: 3,
        },
    )
    .await;
    let reader = user_with(&app, "reader", "note_reader", &["notes.view"]).await;
    let trusted = user_with(
        &app,
        "trusted",
        "note_trusted",
        &["notes.view", "notes.secret.view"],
    )
    .await;
    let db = app.db();
    let reader = Access::of_user(db, &reader).await.unwrap();
    let trusted = Access::of_user(db, &trusted).await.unwrap();

    let ids = |access: &Access, query: &'static str| {
        let access = access.clone();
        async move {
            search::search(db, &access, query, 20)
                .await
                .unwrap()
                .groups
                .iter()
                .flat_map(|g| g.hits.iter().map(|h| h.subject.subject_id))
                .collect::<Vec<i64>>()
        }
    };
    // By words and by its exact number, and in the count.
    assert_eq!(ids(&reader, "boiler").await, [plain.id]);
    assert!(ids(&reader, "NOTE-2").await.is_empty());
    let mut both = ids(&trusted, "boiler").await;
    both.sort_unstable();
    assert_eq!(both, [plain.id, secret.id]);
    assert_eq!(ids(&trusted, "NOTE-2").await, [secret.id]);
    let group = search::search_type(db, &reader, "boiler", NOTE, 1, 25)
        .await
        .unwrap()
        .expect("the reader may search notes");
    assert_eq!(group.total, 1, "the count does not reveal the secret note");

    // The related records and the agents read the same check.
    let searchable = search::searchable_type(NOTE).unwrap();
    let row = search::document_of(db, &Subject::new(NOTE, secret.id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.permission, "notes.secret.view");
    assert!(!reader.sees_document(&searchable, &row));
    assert!(trusted.sees_document(&searchable, &row));
    assert!(
        wl_kernel::related::can_open(db, &reader, &Subject::new(NOTE, secret.id))
            .await
            .unwrap()
            .is_none()
    );
}

#[renox::test]
async fn the_portal_finds_the_records_of_the_users_own_parties_only() {
    let app = app().await;
    let tenant_a = indexed(
        &app,
        PARTY,
        Party {
            id: 0,
            name: "Tenant A".into(),
        },
    )
    .await;
    let tenant_b = indexed(
        &app,
        PARTY,
        Party {
            id: 0,
            name: "Tenant B".into(),
        },
    )
    .await;
    let invoice = |number: &str, party_id: i64| Invoice {
        id: 0,
        number: number.into(),
        party_id,
        source_type: UNIT.into(),
        source_id: 0,
    };
    let own = indexed(&app, INVOICE, invoice("INV-PGP-2026-0101", tenant_a.id)).await;
    let other = indexed(&app, INVOICE, invoice("INV-PGP-2026-0202", tenant_b.id)).await;
    // An internal note about tenant A's invoice: not a portal-visible type.
    indexed(
        &app,
        NOTE,
        Note {
            id: 0,
            number: "NOTE-7".into(),
            text: "INV-PGP-2026-0101 chased by phone".into(),
            secret: false,
            party_id: tenant_a.id,
            building_id: 1,
        },
    )
    .await;
    let reach = [PortalReach {
        party_id: tenant_a.id,
        types: vec![INVOICE.to_owned(), NOTE.to_owned()],
    }];
    let db = app.db();

    // Tenant B's number finds nothing for tenant A.
    let found = search::portal_search(db, &reach, "INV-PGP-2026-0202", 50)
        .await
        .unwrap();
    assert!(found.is_empty(), "{found:?}");
    let _ = other;

    // Their own number finds the invoice, which opens in the portal.
    let found = search::portal_search(db, &reach, "inv pgp 2026 0101", 50)
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].subject, Subject::new(INVOICE, own.id));
    assert!(found[0].identifier_match);
    assert_eq!(found[0].url, format!("/portal/{}/invoices", tenant_a.id));

    // The note mentioning the invoice is never found in the portal, even
    // when the caller passes its type.
    let found = search::portal_search(db, &reach, "chased", 50)
        .await
        .unwrap();
    assert!(found.is_empty(), "{found:?}");

    // A user acting for both parties finds each party's invoice, with the
    // party it was found for.
    let both = [
        PortalReach {
            party_id: tenant_a.id,
            types: vec![INVOICE.to_owned()],
        },
        PortalReach {
            party_id: tenant_b.id,
            types: vec![INVOICE.to_owned()],
        },
    ];
    let found = search::portal_search(db, &both, "INV-PGP-2026", 50)
        .await
        .unwrap();
    let mut parties: Vec<i64> = found.iter().map(|h| h.party_id).collect();
    parties.sort_unstable();
    assert_eq!(parties, [tenant_a.id, tenant_b.id], "{found:?}");
    assert!(
        search::portal_types()
            .iter()
            .all(|t| t.subject_type() != NOTE)
    );
}
