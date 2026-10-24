//! The construction records in the global search and the Related panel
//! (BL-AGT-12): on the demo company, a project is found by its code, an IPA
//! and a variation by their numbers, a guarantee by the bank's number,
//! each typed in any case and with spaces, and each shows its related
//! records; the IPA page loads its Related panel; a QS of one project finds
//! nothing of another.

use host::roles::{self, QUANTITY_SURVEYOR};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-construction/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Construction vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    // The demo is seeded row by row; the index is built as after an import.
    search::rebuild(app.db()).await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str, scope: Option<&Scope>) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@search.example", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    match scope {
        Some(scope) => user.assign_role_in(app.db(), role, scope).await.unwrap(),
        None => user.assign_role(app.db(), role).await.unwrap(),
    }
    user
}

async fn id(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn text(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

/// Whether a search for `typed` lists `subject`.
async fn finds(app: &TestApp, access: &Access, typed: &str, subject: &Subject) -> bool {
    search::search(app.db(), access, typed, 50)
        .await
        .unwrap()
        .groups
        .iter()
        .flat_map(|group| group.hits.iter())
        .any(|hit| &hit.subject == subject)
}

/// The count of the group `name` of the related records of `subject`.
async fn related_count(app: &TestApp, access: &Access, subject: &Subject, name: &str) -> u64 {
    related::related(app.db(), access, subject, PANEL_DEPTH)
        .await
        .unwrap()
        .iter()
        .find(|group| group.name == name)
        .map_or(0, |group| group.count)
}

#[renox::test]
async fn a_qs_finds_each_construction_record_by_its_identifiers_with_its_related_records() {
    let app = booted().await;
    let qs = person(&app, "Quentin", QUANTITY_SURVEYOR, None).await;
    let access = Access::of_user(app.db(), &qs).await.unwrap();

    // A project by its code, typed in lower case.
    let project = id(
        &app,
        "select id from construction_projects where code = 'TWR-01'",
    )
    .await;
    let project_subject = Subject::new("construction.project", project);
    assert!(finds(&app, &access, "twr 01", &project_subject).await);
    assert!(related_count(&app, &access, &project_subject, "ipas").await >= 2);
    assert!(related_count(&app, &access, &project_subject, "variations").await >= 1);
    assert!(related_count(&app, &access, &project_subject, "guarantees").await >= 1);

    // An IPA by its number: the project's code and its sequence.
    let ipa = id(
        &app,
        "select i.id from construction_ipas i join construction_projects p on p.id = i.project_id \
         where p.code = 'TWR-01' and i.number = 2 order by i.id desc limit 1",
    )
    .await;
    let ipa_subject = Subject::new("construction.ipa", ipa);
    assert!(finds(&app, &access, "TWR-01-IPA-2", &ipa_subject).await);
    assert!(finds(&app, &access, "twr 01 ipa 2", &ipa_subject).await);
    assert_eq!(
        related_count(&app, &access, &ipa_subject, "project").await,
        1
    );

    // A variation by its number.
    let (variation, code) = (
        id(&app, "select v.id from construction_variations v join construction_projects p on p.id = v.project_id where p.code = 'TWR-01' order by v.id limit 1").await,
        text(&app, "select v.code from construction_variations v join construction_projects p on p.id = v.project_id where p.code = 'TWR-01' order by v.id limit 1").await,
    );
    let variation_subject = Subject::new("construction.variation", variation);
    assert!(finds(&app, &access, &format!("TWR-01-{code}"), &variation_subject).await);
    assert!(finds(&app, &access, &code.to_lowercase(), &variation_subject).await);
    assert_eq!(
        related_count(&app, &access, &variation_subject, "project").await,
        1
    );

    // A guarantee by the bank's guarantee number (its documents' reference).
    let guarantee = id(
        &app,
        "select guarantee_id from construction_guarantee_documents order by id limit 1",
    )
    .await;
    let reference = text(
        &app,
        &format!(
            "select reference from construction_guarantee_documents where guarantee_id = {guarantee} order by id limit 1"
        ),
    )
    .await;
    let guarantee_subject = Subject::new("construction.guarantee", guarantee);
    assert!(finds(&app, &access, &reference, &guarantee_subject).await);
    assert!(finds(&app, &access, &reference.to_uppercase(), &guarantee_subject).await);
    assert_eq!(
        related_count(&app, &access, &guarantee_subject, "project").await,
        1
    );

    // A retention part shows with its project.
    let retention = id(
        &app,
        "select id from construction_retentions order by id limit 1",
    )
    .await;
    let retention_subject = Subject::new("construction.retention", retention);
    assert!(
        search::document_of(app.db(), &retention_subject)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        related_count(&app, &access, &retention_subject, "project").await,
        1
    );

    // The IPA page loads its Related panel, which lists the project.
    app.acting_as(&qs);
    app.get(&format!("/construction/ipas/{ipa}"))
        .await
        .assert_ok()
        .assert_see(&format!("hx-get=\"/related/construction.ipa:{ipa}/panel\""));
    app.get(&format!("/related/construction.ipa:{ipa}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"project\" data-count=\"1\"");
}

#[renox::test]
async fn a_qs_of_one_project_finds_nothing_of_another() {
    let app = booted().await;
    let other = id(
        &app,
        "select id from construction_projects where code <> 'TWR-01' order by id limit 1",
    )
    .await;
    let scope = Scope::new("construction.project", other);
    let qs = person(&app, "Quinn", QUANTITY_SURVEYOR, Some(&scope)).await;
    let access = Access::of_user(app.db(), &qs).await.unwrap();
    let project = id(
        &app,
        "select id from construction_projects where code = 'TWR-01'",
    )
    .await;
    assert!(
        !finds(
            &app,
            &access,
            "TWR-01",
            &Subject::new("construction.project", project)
        )
        .await
    );
    let code = text(
        &app,
        &format!("select code from construction_projects where id = {other}"),
    )
    .await;
    assert!(
        finds(
            &app,
            &access,
            &code,
            &Subject::new("construction.project", other)
        )
        .await
    );
}
