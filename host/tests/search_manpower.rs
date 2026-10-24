//! The agency's records in the global search and the Related panel
//! (BL-AGT-13): on the demo agency, an account manager finds a client
//! request and a deployment by their numbers, a client contract by its
//! number and a site by its name, each with its related records; an account
//! manager of one client finds nothing of another; no worker's identity
//! number is in the index in clear text.

use host::roles::{self, ACCOUNT_MANAGER};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manpower/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the manpower client");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
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

async fn scalar_i64(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn scalar_text(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn finds(app: &TestApp, access: &Access, typed: &str, subject: &Subject) -> bool {
    search::search(app.db(), access, typed, 50)
        .await
        .unwrap()
        .groups
        .iter()
        .flat_map(|group| group.hits.iter())
        .any(|hit| &hit.subject == subject)
}

async fn related_count(app: &TestApp, access: &Access, subject: &Subject, name: &str) -> u64 {
    related::related(app.db(), access, subject, PANEL_DEPTH)
        .await
        .unwrap()
        .iter()
        .find(|group| group.name == name)
        .map_or(0, |group| group.count)
}

#[renox::test]
async fn an_account_manager_finds_requests_deployments_contracts_and_sites() {
    let app = booted().await;
    let manager = person(&app, "Aisha", ACCOUNT_MANAGER, None).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();

    // A deployment by its number, with its contract and site.
    let deployment = scalar_i64(
        &app,
        "select id from manpower_deployments order by id limit 1",
    )
    .await;
    let deployment_subject = Subject::new("manpower.deployment", deployment);
    assert!(
        finds(
            &app,
            &access,
            &format!("dep {deployment:04}"),
            &deployment_subject
        )
        .await
    );
    assert_eq!(
        related_count(&app, &access, &deployment_subject, "contract").await,
        1
    );
    assert_eq!(
        related_count(&app, &access, &deployment_subject, "site").await,
        1
    );

    // A request by its number, with the deployments that fill it.
    let request = scalar_i64(
        &app,
        "select request_id from manpower_deployments where request_id is not null order by id limit 1",
    )
    .await;
    let request_subject = Subject::new("manpower.request", request);
    assert!(
        finds(
            &app,
            &access,
            &format!("REQ-{request:04}"),
            &request_subject
        )
        .await
    );
    assert!(related_count(&app, &access, &request_subject, "deployments").await >= 1);

    // A contract by its number.
    let contract = scalar_i64(
        &app,
        "select id from manpower_client_contracts order by id limit 1",
    )
    .await;
    let contract_subject = Subject::new("manpower.client_contract", contract);
    let reference = scalar_text(
        &app,
        &format!("select reference from manpower_client_contracts where id = {contract}"),
    )
    .await;
    assert!(finds(&app, &access, &reference.to_lowercase(), &contract_subject).await);
    assert!(related_count(&app, &access, &contract_subject, "sites").await >= 1);

    // A site by its name.
    let site = scalar_i64(&app, "select id from manpower_sites order by id limit 1").await;
    let site_subject = Subject::new("manpower.site", site);
    let name = scalar_text(
        &app,
        &format!("select name from manpower_sites where id = {site}"),
    )
    .await;
    assert!(finds(&app, &access, &name, &site_subject).await, "{name}");

    // The deployment page loads its Related panel.
    app.acting_as(&manager);
    app.get(&format!("/manpower/deployments/{deployment}"))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/manpower.deployment:{deployment}/panel\""
        ));
    app.get(&format!("/related/manpower.deployment:{deployment}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"site\" data-count=\"1\"");
}

#[renox::test]
async fn an_account_manager_of_one_client_finds_nothing_of_another() {
    let app = booted().await;
    let (mine, theirs) = (
        scalar_i64(&app, "select id from manpower_client_contracts order by id limit 1").await,
        renox::db::sql(
            "select c.id from manpower_client_contracts c where c.client_party_id <> \
             (select client_party_id from manpower_client_contracts order by id limit 1) order by c.id limit 1"
                .to_owned(),
        )
        .scalar_optional::<i64>(app.db())
        .await
        .unwrap(),
    );
    let client = scalar_i64(
        &app,
        &format!("select client_party_id from manpower_client_contracts where id = {mine}"),
    )
    .await;
    let scope = Scope::new("manpower.client", client);
    let manager = person(&app, "Omar", ACCOUNT_MANAGER, Some(&scope)).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();
    let reference = scalar_text(
        &app,
        &format!("select reference from manpower_client_contracts where id = {mine}"),
    )
    .await;
    assert!(
        finds(
            &app,
            &access,
            &reference,
            &Subject::new("manpower.client_contract", mine)
        )
        .await
    );
    if let Some(theirs) = theirs {
        let other = scalar_text(
            &app,
            &format!("select reference from manpower_client_contracts where id = {theirs}"),
        )
        .await;
        assert!(
            !finds(
                &app,
                &access,
                &other,
                &Subject::new("manpower.client_contract", theirs)
            )
            .await
        );
    }
}

#[renox::test]
async fn no_worker_identity_number_is_indexed_in_clear_text() {
    let app = booted().await;
    // The workers' documents (passport, Emirates ID, visa, labour card).
    let numbers: Vec<String> = renox::db::sql(
        "select number from wl_documents_documents where number is not null and subject_type = 'hr.employee'"
            .to_owned(),
    )
    .fetch_all(app.db())
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get::<String>("number").unwrap())
    .collect();
    let documents: Vec<String> = renox::db::sql(
        "select title || ' ' || secondary || ' ' || keywords as text from wl_search_documents"
            .to_owned(),
    )
    .fetch_all(app.db())
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get::<String>("text").unwrap())
    .collect();
    let plain: Vec<String> =
        renox::db::sql("select value from wl_search_identifiers where hashed = false".to_owned())
            .fetch_all(app.db())
            .await
            .unwrap()
            .iter()
            .map(|row| row.try_get::<String>("value").unwrap())
            .collect();
    for number in &numbers {
        let normalised = search::normalize_identifier(search::kinds::PASSPORT, number);
        assert!(
            documents.iter().all(|text| !text.contains(number.as_str())),
            "{number}"
        );
        assert!(!plain.contains(&normalised), "{number}");
    }
    // A deployment is found by its number only.
    let kinds: Vec<String> = renox::db::sql(
        "select distinct kind from wl_search_identifiers where subject_type = 'manpower.deployment'"
            .to_owned(),
    )
    .fetch_all(app.db())
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get::<String>("kind").unwrap())
    .collect();
    assert_eq!(kinds, vec!["number".to_owned()]);
}
