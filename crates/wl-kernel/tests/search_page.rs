//! The global search page (BL-AGT-02, wl-kernel §5.8): identifiers first and
//! a direct open on an exact identifier, results grouped by type, filtered by
//! permission and scope before they are counted, *Show all N* with a paged
//! type page, a constant number of queries, masked sensitive identifiers,
//! Arabic and Latin spellings, and the hints of a short query and of a user
//! who may search nothing.
//!
//! The subjects are test-only invoices, parties and leases of two buildings,
//! indexed with `search::index_in` as a module does.

use renox::auth::permissions::{self, Scope};
use renox::auth::Permissions;
use renox::db::{Migration, capture_queries};
use renox::prelude::*;
use renox::testing::TestApp;
use serde::{Deserialize, Serialize};
use wl_kernel::search::{self, Access, SearchDocument, Searchable, kinds};
use wl_kernel::{ClientConfig, Subject};

const INVOICE: &str = "invoicing.invoice";
const PARTY: &str = "parties.party";
const LEASE: &str = "property.lease";
const BUILDINGS: &str = "buildings";
const TOWER_A: i64 = 1;
const TOWER_B: i64 = 2;

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_sp_invoices")]
struct Invoice {
    id: i64,
    number: String,
    customer: String,
    replaces: String,
    building_id: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_sp_parties")]
struct Party {
    id: i64,
    name_en: String,
    name_ar: String,
    phone: String,
    building_id: i64,
}

#[derive(Model, Serialize, Deserialize, Default, Debug, Clone)]
#[model(table = "test_sp_leases")]
struct Lease {
    id: i64,
    number: String,
    tenant: String,
    building_id: i64,
}

const UP: &str = r#"
CREATE TABLE "test_sp_invoices" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    customer TEXT NOT NULL,
    replaces TEXT NOT NULL DEFAULT '',
    building_id INTEGER NOT NULL
);
CREATE TABLE "test_sp_parties" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name_en TEXT NOT NULL,
    name_ar TEXT NOT NULL DEFAULT '',
    phone TEXT NOT NULL DEFAULT '',
    building_id INTEGER NOT NULL
);
CREATE TABLE "test_sp_leases" (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    number TEXT NOT NULL,
    tenant TEXT NOT NULL,
    building_id INTEGER NOT NULL
);
"#;

const DOWN: &str = r#"
DROP TABLE "test_sp_leases";
DROP TABLE "test_sp_parties";
DROP TABLE "test_sp_invoices";
"#;

const MIGRATIONS: &[Migration] = &[Migration::new(
    "20261101000198_create_test_search_page_subjects",
    UP,
    Some(DOWN),
)];

struct Subjects;

impl Module for Subjects {
    fn name(&self) -> &'static str {
        "test-search-page-subjects"
    }

    fn migrations(&self) -> &'static [Migration] {
        MIGRATIONS
    }

    fn register(&self, app: &mut Registry) {
        app.model::<Invoice>().model::<Party>().model::<Lease>();
        Searchable::of::<Invoice>(INVOICE)
            .permission("invoices.view")
            .label("Invoices")
            .link("/invoices/{id}")
            .document(|invoice: &Invoice| {
                SearchDocument::new(&invoice.number)
                    .secondary(&invoice.customer)
                    .keyword(&invoice.replaces)
                    .identifier(kinds::NUMBER, &invoice.number)
                    .status("issued")
                    .scope(Scope::new(BUILDINGS, invoice.building_id))
            })
            .register();
        Searchable::of::<Party>(PARTY)
            .permission("parties.view")
            .label("Parties")
            .link("/parties/{id}")
            .document(|party: &Party| {
                SearchDocument::new(&party.name_en)
                    .secondary(&party.name_ar)
                    .identifier(kinds::PHONE, &party.phone)
                    .scope(Scope::new(BUILDINGS, party.building_id))
            })
            .register();
        Searchable::of::<Lease>(LEASE)
            .permission("leases.view")
            .label("Leases")
            .link("/leases/{id}")
            .document(|lease: &Lease| {
                SearchDocument::new(&lease.number)
                    .secondary(&lease.tenant)
                    .identifier(kinds::NUMBER, &lease.number)
                    .scope(Scope::new(BUILDINGS, lease.building_id))
            })
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

struct World {
    app: TestApp,
    _views: tempfile::TempDir,
    /// `finance@gulftower.example`: invoices, parties and leases, everywhere.
    finance: User,
    /// `viewer@gulftower.example`: parties and leases, no `invoices.view`.
    viewer: User,
    /// A manager of Tower A only.
    manager: User,
    /// No role at all.
    nobody: User,
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
    permissions::define_role(db, "finance", &["invoices.view", "parties.view", "leases.view"])
        .await
        .unwrap();
    permissions::define_role(db, "viewer", &["parties.view", "leases.view"])
        .await
        .unwrap();
    permissions::define_role(db, "manager", &["invoices.view", "parties.view", "leases.view"])
        .await
        .unwrap();
    let user = |name: &'static str| async move {
        User::register(db, name, &format!("{name}@gulftower.example"), "password123")
            .await
            .unwrap()
    };
    let finance = user("finance").await;
    finance.assign_role(db, "finance").await.unwrap();
    let viewer = user("viewer").await;
    viewer.assign_role(db, "viewer").await.unwrap();
    let manager = user("manager").await;
    manager
        .assign_role_in(db, "manager", &Scope::new(BUILDINGS, TOWER_A))
        .await
        .unwrap();
    let nobody = user("nobody").await;
    World {
        app,
        _views: dir,
        finance,
        viewer,
        manager,
        nobody,
    }
}

impl World {
    /// Writes `record` and its search document in one transaction.
    async fn indexed<M>(&self, subject_type: &str, mut record: M) -> M
    where
        M: Model<Key = i64> + Send + Sync,
    {
        let mut tx = self.app.db().begin().await.unwrap();
        record.insert(&mut tx).await.unwrap();
        assert!(
            search::index_in(&mut tx, &Subject::new(subject_type, record.id()))
                .await
                .unwrap()
        );
        tx.commit().await.unwrap();
        record
    }

    async fn invoice(&self, number: &str, customer: &str, building: i64) -> Invoice {
        self.indexed(
            INVOICE,
            Invoice {
                number: number.into(),
                customer: customer.into(),
                building_id: building,
                ..Default::default()
            },
        )
        .await
    }

    async fn party(&self, name_en: &str, name_ar: &str, phone: &str, building: i64) -> Party {
        self.indexed(
            PARTY,
            Party {
                name_en: name_en.into(),
                name_ar: name_ar.into(),
                phone: phone.into(),
                building_id: building,
                ..Default::default()
            },
        )
        .await
    }

    async fn lease(&self, number: &str, tenant: &str, building: i64) -> Lease {
        self.indexed(
            LEASE,
            Lease {
                number: number.into(),
                tenant: tenant.into(),
                building_id: building,
                ..Default::default()
            },
        )
        .await
    }

    async fn get(&self, user: &User, uri: &str) -> renox::testing::TestResponse {
        self.app.acting_as(user);
        self.app.get(uri).await
    }
}

/// How many results of `subject_type` the page lists.
fn rows_of(page: &str, subject_type: &str) -> usize {
    let Some(start) = page.find(&format!("data-type=\"{subject_type}\"")) else {
        return 0;
    };
    let rest = &page[start..];
    let end = rest[1..].find("data-type=\"").map_or(rest.len(), |i| i + 1);
    rest[..end].matches("data-result=").count()
}

#[renox::test]
async fn an_invoice_number_opens_the_invoice_however_it_is_typed() {
    let w = world().await;
    let invoice = w
        .invoice("INV-PGP-2026-0004", "Mohammed Al Ali", TOWER_A)
        .await;
    w.invoice("INV-PGP-2026-0005", "Sara Hassan", TOWER_A).await;
    let to = format!("/invoices/{}", invoice.id);
    for typed in ["INV-PGP-2026-0004", "inv+pgp+2026+0004", "invpgp20260004"] {
        w.get(&w.finance, &format!("/search?q={typed}"))
            .await
            .assert_redirect(&to);
    }
}

#[renox::test]
async fn without_invoices_view_there_is_no_invoice_group_and_no_count() {
    let w = world().await;
    w.invoice("INV-PGP-2026-0004", "Mohammed Al Ali", TOWER_A)
        .await;
    let page = w.get(&w.viewer, "/search?q=INV-PGP-2026-0004").await;
    page.assert_ok();
    page.assert_dont_see("INV-PGP-2026-0004</strong>");
    page.assert_dont_see("data-type=\"invoicing.invoice\"");
    page.assert_dont_see("1 found");
    page.assert_see("Nothing found for");
    // Nor through the type page, nor the functions behind it.
    let only = w
        .get(&w.viewer, "/search?q=INV-PGP-2026-0004&type=invoicing.invoice")
        .await;
    only.assert_ok();
    only.assert_dont_see("data-type=\"invoicing.invoice\"");
    let access = Access::of_user(w.app.db(), &w.viewer).await.unwrap();
    let results = search::search(w.app.db(), &access, "INV-PGP-2026-0004", 5)
        .await
        .unwrap();
    assert!(results.groups.is_empty());
    assert!(results.open.is_none());
}

#[renox::test]
async fn a_manager_of_tower_a_finds_nothing_of_tower_b() {
    let w = world().await;
    w.party("Sara Hassan", "سارة حسن", "", TOWER_B).await;
    w.lease("LSE-PGP-2026-0009", "Sara Hassan", TOWER_B).await;
    w.invoice("INV-PGP-2026-0031", "Sara Hassan", TOWER_B).await;
    w.party("Omar Khalid", "", "", TOWER_A).await;

    let page = w.get(&w.manager, "/search?q=Sara+Hassan").await;
    page.assert_ok();
    page.assert_see("Nothing found for");
    page.assert_dont_see("data-type=");
    // Not even by the exact number: no redirect, no count.
    let page = w.get(&w.manager, "/search?q=INV-PGP-2026-0031").await;
    page.assert_ok();
    page.assert_dont_see("data-type=");
    // Tower A is in reach.
    let page = w.get(&w.manager, "/search?q=Omar").await;
    page.assert_ok();
    page.assert_see("data-type=\"parties.party\"");
    page.assert_see("Omar Khalid");
    // The finance user, global, sees Tower B.
    let page = w.get(&w.finance, "/search?q=Sara+Hassan").await;
    page.assert_see("data-type=\"parties.party\"");
    page.assert_see("data-type=\"property.lease\"");
    page.assert_see("data-type=\"invoicing.invoice\"");
}

#[renox::test]
async fn al_ali_shows_parties_leases_and_invoices_in_their_groups_identifiers_first() {
    let w = world().await;
    w.party("Mohammed Al Ali", "محمد العلي", "", TOWER_A).await;
    w.lease("LSE-PGP-2026-0001", "Mohammed Al Ali", TOWER_A).await;
    w.invoice("INV-PGP-2026-0004", "Mohammed Al Ali", TOWER_A)
        .await;
    w.invoice("INV-PGP-2026-0006", "Gulf Tower Owners", TOWER_A)
        .await;

    let page = w.get(&w.finance, "/search?q=Al+Ali").await;
    page.assert_ok();
    let text = page.text();
    assert_eq!(rows_of(&text, PARTY), 1, "{text}");
    assert_eq!(rows_of(&text, LEASE), 1);
    assert_eq!(rows_of(&text, INVOICE), 1);
    page.assert_see("aria-label=\"Parties\"");
    page.assert_see("aria-label=\"Leases\"");
    page.assert_see("aria-label=\"Invoices\"");

    // An invoice that names another one's number is found too, after the
    // record whose identifier it is.
    let credit = w
        .indexed(
            INVOICE,
            Invoice {
                number: "INV-PGP-2026-0007".into(),
                customer: "Mohammed Al Ali".into(),
                replaces: "INV-PGP-2026-0004".into(),
                building_id: TOWER_A,
                ..Default::default()
            },
        )
        .await;
    let access = Access::of_user(w.app.db(), &w.finance).await.unwrap();
    let results = search::search(w.app.db(), &access, "INV-PGP-2026-0004", 5)
        .await
        .unwrap();
    let invoices = &results.groups[0];
    assert_eq!(invoices.subject_type, INVOICE);
    assert_eq!(invoices.total, 2);
    assert!(invoices.hits[0].identifier_match);
    assert_eq!(invoices.hits[0].title, "INV-PGP-2026-0004");
    assert_eq!(invoices.hits[1].subject, Subject::new(INVOICE, credit.id));
    assert!(!invoices.hits[1].identifier_match);
    // One record matches the identifier: it opens directly.
    assert!(results.open.is_some());
}

#[renox::test]
async fn forty_invoices_show_five_and_show_all_forty_and_the_type_page_pages_them() {
    let w = world().await;
    for n in 1..=40 {
        w.invoice(
            &format!("INV-PGP-2026-{n:04}"),
            "Gulf Tower Owners",
            TOWER_A,
        )
        .await;
    }
    let page = w.get(&w.finance, "/search?q=Gulf+Tower+Owners").await;
    page.assert_ok();
    let text = page.text();
    assert_eq!(rows_of(&text, INVOICE), 5);
    page.assert_see("data-total=\"40\"");
    page.assert_see("Show all 40");
    page.assert_see("/search?q=Gulf+Tower+Owners&amp;type=invoicing.invoice");

    let first = w
        .get(&w.finance, "/search?q=Gulf+Tower+Owners&type=invoicing.invoice")
        .await;
    first.assert_ok();
    assert_eq!(rows_of(&first.text(), INVOICE), 25);
    first.assert_see("Page 1 of 2");
    first.assert_dont_see("Show all");
    let second = w
        .get(
            &w.finance,
            "/search?q=Gulf+Tower+Owners&type=invoicing.invoice&page=2",
        )
        .await;
    second.assert_ok();
    assert_eq!(rows_of(&second.text(), INVOICE), 15);
    second.assert_see("Page 2 of 2");
}

#[renox::test]
async fn the_page_runs_the_same_number_of_queries_whatever_it_finds() {
    let w = world().await;
    w.party("Gulf Tower Owners", "", "", TOWER_A).await;
    for n in 1..=3 {
        w.invoice(&format!("INV-PGP-2026-{n:04}"), "Gulf Tower Owners", TOWER_A)
            .await;
    }
    w.app.acting_as(&w.finance);
    // Warm the session and the roles.
    w.app.get("/search?q=Gulf").await.assert_ok();
    let (page, few) = capture_queries(w.app.get("/search?q=Gulf+Tower+Owners")).await;
    page.assert_ok();
    for n in 4..=40 {
        w.invoice(&format!("INV-PGP-2026-{n:04}"), "Gulf Tower Owners", TOWER_A)
            .await;
    }
    for n in 1..=12 {
        w.lease(&format!("LSE-PGP-2026-{n:04}"), "Gulf Tower Owners", TOWER_A)
            .await;
    }
    w.app.acting_as(&w.finance);
    let (page, many) = capture_queries(w.app.get("/search?q=Gulf+Tower+Owners")).await;
    page.assert_ok();
    page.assert_see("Show all 40");
    assert_eq!(few.len(), many.len(), "{few:#?}\n{many:#?}");
    // The type page too.
    let uri = "/search?q=Gulf+Tower+Owners&type=property.lease";
    let (page, paged) = capture_queries(w.app.get(uri)).await;
    page.assert_ok();
    assert!(paged.len() <= many.len() + 1, "{paged:#?}");
}

#[renox::test]
async fn a_shared_phone_lists_both_parties_with_the_phone_masked() {
    let w = world().await;
    w.party("Ali Hassan", "", "050 123 4567", TOWER_A).await;
    w.party("Sara Hassan", "", "050 123 4567", TOWER_A).await;
    let page = w.get(&w.finance, "/search?q=0501234567").await;
    page.assert_ok();
    assert_eq!(rows_of(&page.text(), PARTY), 2);
    page.assert_see("Matched •••• 4567");
    page.assert_dont_see("501234567<");
    page.assert_dont_see("+971501234567");
}

#[renox::test]
async fn arabic_and_latin_spellings_find_each_other() {
    let w = world().await;
    w.party("Mohammed Al Ali", "", "", TOWER_A).await;
    w.party("F. Noor", "فاطمة نور", "", TOWER_A).await;
    // Arabic typed (محمد), the record is in Latin letters only.
    let page = w.get(&w.finance, "/search?q=%D9%85%D8%AD%D9%85%D8%AF").await;
    page.assert_ok();
    page.assert_see("Mohammed Al Ali");
    // Latin typed, the first name is in Arabic letters only.
    let page = w.get(&w.finance, "/search?q=Fatima").await;
    page.assert_see("F. Noor");
}

#[renox::test]
async fn a_short_query_gets_a_hint_and_a_user_who_may_search_nothing_a_reason() {
    let w = world().await;
    w.party("Ali Hassan", "", "", TOWER_A).await;
    let page = w.get(&w.finance, "/search?q=A").await;
    page.assert_ok();
    page.assert_see("Type at least 2 characters to search.");
    page.assert_dont_see("data-type=");

    let page = w.get(&w.nobody, "/search?q=Ali").await;
    page.assert_ok();
    page.assert_see("Your role gives access to no record that can be searched.");
    page.assert_dont_see("data-type=");
    page.assert_see("disabled");

    // Results are a list in cards (no table to scroll sideways on a phone).
    let page = w.get(&w.finance, "/search?q=Ali").await;
    page.assert_see("data-type=\"parties.party\"");
    page.assert_dont_see("<table");
    // The page's own box is the one mod+k focuses there.
    page.assert_see("id=\"wl-search-page-q\"");

    // Guests log in first.
    w.app.logout();
    let guest = w.app.get("/search?q=Ali").await;
    assert!(guest.status.is_redirection(), "{}", guest.status);
}
