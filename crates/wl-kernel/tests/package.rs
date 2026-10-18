use std::path::Path;

use renox::auth::{Permissions, permissions};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, LegalEntity, Reset, Series};

/// The pages extend the app's layout; a package test has no app, so it
/// brings the smallest one.
const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

/// A group of two companies: a mainland LLC (the default) and a free-zone one.
const GROUP: &str = r#"
[client]
name = "Acme Properties LLC"
app = "property"

[modules]
enabled = ["parties", "invoicing"]

[[legal_entities]]
code = "ACME"
name_en = "Acme Properties LLC"
name_ar = "أكمي للعقارات ش.ذ.م.م"
trn = "100123456700003"
default = true

[[legal_entities]]
code = "ACFZ"
name_en = "Acme FZ-LLC"
"#;

const INVOICE: Series = Series {
    key: "invoice",
    pattern: "{prefix}-{entity}-{yyyy}-{0000}",
    prefix: "INV",
    reset: Reset::Yearly,
    gapless: true,
    per_entity: true,
};

/// A views directory holding the layout and `files` (the app's own views).
fn views(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a temporary views directory");
    for (name, text) in [("layouts/app.html", LAYOUT)].iter().chain(files) {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().expect("a file path")).expect("a directory");
        std::fs::write(path, text).expect("a view file");
    }
    dir
}

/// The kernel for `GROUP` in an app with auth, in `views`, installed.
async fn boot(views: &Path) -> TestApp {
    let config = ClientConfig::parse(GROUP).expect("a valid client.toml");
    let app = wl_kernel::apply(App::new().module(Auth::new()).module(Permissions), config)
        .expect("the kernel applies");
    let app = TestApp::with_config(app, |c| c.views_path = views.to_path_buf()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 18).unwrap()
}

#[renox::test]
async fn install_creates_the_entities_once_and_keeps_admin_changes() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    assert_eq!(LegalEntity::query().count(db).await.unwrap(), 2);
    let mut acme = LegalEntity::default_entity(db).await.unwrap();
    assert_eq!(acme.code, "ACME");
    assert!(acme.vat_registered);

    // A change made in the admin survives the next install.
    acme.name_en = "Acme Properties L.L.C.".into();
    acme.save(db).await.unwrap();
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    assert_eq!(LegalEntity::query().count(db).await.unwrap(), 2);
    let again = LegalEntity::default_entity(db).await.unwrap();
    assert_eq!(again.name_en, "Acme Properties L.L.C.");
}

#[renox::test]
async fn a_scope_issues_from_its_entity_else_the_default() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("ACFZ"))
        .first(db)
        .await
        .unwrap()
        .unwrap();
    free_zone
        .link_scope(db, "property.building", 7)
        .await
        .unwrap();

    let linked = LegalEntity::for_scope(db, Some(("property.building", 7)))
        .await
        .unwrap();
    assert_eq!(linked.code, "ACFZ");
    let unlinked = LegalEntity::for_scope(db, Some(("property.building", 8)))
        .await
        .unwrap();
    assert_eq!(unlinked.code, "ACME");
    assert_eq!(LegalEntity::for_scope(db, None).await.unwrap().code, "ACME");
}

#[renox::test]
async fn the_snapshot_keeps_the_trn_only_when_vat_registered() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let acme = LegalEntity::default_entity(db).await.unwrap();
    let snapshot = acme.snapshot();
    assert_eq!(snapshot.trn.as_deref(), Some("100123456700003"));
    assert_eq!(snapshot.name_ar.as_deref(), Some("أكمي للعقارات ش.ذ.م.م"));

    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("ACFZ"))
        .first(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(free_zone.snapshot().trn, None);
}

#[renox::test]
async fn gapless_numbers_count_per_entity_and_a_rollback_gives_one_back() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let acme = LegalEntity::default_entity(db).await.unwrap();
    let free_zone = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("ACFZ"))
        .first(db)
        .await
        .unwrap()
        .unwrap();

    let mut tx = db.begin().await.unwrap();
    let first = INVOICE
        .next(&mut tx, None, Some(&acme), day())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(first, "INV-ACME-2026-0001");

    // Taken, then rolled back: the next document gets the same number.
    let mut tx = db.begin().await.unwrap();
    let lost = INVOICE
        .next(&mut tx, None, Some(&acme), day())
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let mut tx = db.begin().await.unwrap();
    let second = INVOICE
        .next(&mut tx, Some("ACM-INV"), Some(&acme), day())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(lost, "INV-ACME-2026-0002");
    assert_eq!(second, "ACM-INV-ACME-2026-0002");

    let mut tx = db.begin().await.unwrap();
    let other = INVOICE
        .next(&mut tx, None, Some(&free_zone), day())
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(other, "INV-ACFZ-2026-0001");

    // A new year starts again at 1.
    let next_year = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
    let mut tx = db.begin().await.unwrap();
    let january = INVOICE
        .next(&mut tx, None, Some(&acme), next_year)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(january, "INV-ACME-2027-0001");
}

#[renox::test]
async fn an_entity_series_needs_an_active_entity() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let mut acme = LegalEntity::default_entity(db).await.unwrap();
    acme.status = "inactive".into();
    let mut tx = db.begin().await.unwrap();
    assert!(INVOICE.next(&mut tx, None, None, day()).await.is_err());
    assert!(
        INVOICE
            .next(&mut tx, None, Some(&acme), day())
            .await
            .is_err()
    );
}

#[renox::test]
async fn the_legal_entities_page_needs_the_permission() {
    let dir = views(&[]);
    let app = boot(dir.path()).await;
    let db = app.db();
    let user = User::register(db, "Mona", "mona@example.com", "password123")
        .await
        .unwrap();
    app.acting_as(&user);
    app.get("/legal-entities").await.assert_forbidden();

    permissions::define_role(
        db,
        "accountant",
        &[wl_kernel::permissions::LEGAL_ENTITIES_VIEW],
    )
    .await
    .unwrap();
    user.assign_role(db, "accountant").await.unwrap();
    app.get("/legal-entities")
        .await
        .assert_ok()
        .assert_view("wl-kernel/legal_entities.html")
        .assert_see("Acme FZ-LLC")
        .assert_see("100123456700003");
}

#[renox::test]
async fn the_models_match_the_schema() {
    let dir = views(&[]);
    boot(dir.path()).await.assert_models_match_schema().await;
}

#[renox::test]
async fn every_view_compiles() {
    let dir = views(&[]);
    boot(dir.path()).await.assert_views_compile();
}

#[test]
fn a_missing_module_dependency_stops_the_kernel() {
    let text = GROUP.replace(
        r#"enabled = ["parties", "invoicing"]"#,
        r#"enabled = ["invoicing"]"#,
    );
    let err = ClientConfig::parse(&text).unwrap_err();
    assert_eq!(
        err.to_string(),
        "client.toml: [modules] `invoicing` needs `parties`; enable it too"
    );
}
