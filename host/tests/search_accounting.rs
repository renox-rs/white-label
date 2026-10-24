//! The accounting records in the global search and the Related panel
//! (BL-AGT-12): on the demo firm, a partner finds an engagement by its
//! number, the client's TRN and the client's name, a return by its number,
//! its period and the authority's acknowledgement, a CDD file by its
//! number, each with its related records; an AML case is found by the MLRO
//! alone, never by a partner or the company admin, and no panel of theirs
//! lists it.

use accounting::aml::{AmlCase, AmlStatus};
use host::roles::{self, ADMIN, COMPLIANCE, PARTNER};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-accounting/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the accounting client");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str) -> User {
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
    user.assign_role(app.db(), role).await.unwrap();
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
async fn a_partner_finds_engagements_returns_and_cdd_files_by_their_identifiers() {
    let app = booted().await;
    search::rebuild(app.db()).await.unwrap();
    let partner = person(&app, "Pat", PARTNER).await;
    let access = Access::of_user(app.db(), &partner).await.unwrap();

    // An engagement by its number, the client's TRN and the client's name.
    let trn = scalar_text(
        &app,
        "select trn from accounting_tax_profiles where trn is not null and trn <> '' order by id limit 1",
    )
    .await;
    let client = scalar_i64(
        &app,
        &format!("select client_party_id from accounting_tax_profiles where trn = '{trn}' order by id limit 1"),
    )
    .await;
    let engagement = scalar_i64(
        &app,
        &format!("select id from accounting_engagements where client_party_id = {client} order by id limit 1"),
    )
    .await;
    let engagement_subject = Subject::new("accounting.engagement", engagement);
    assert!(
        finds(
            &app,
            &access,
            &format!("eng {engagement:04}"),
            &engagement_subject
        )
        .await
    );
    assert!(finds(&app, &access, &trn, &engagement_subject).await);
    let name = scalar_text(
        &app,
        &format!("select display_name from wl_parties_parties where id = {client}"),
    )
    .await;
    assert!(
        finds(&app, &access, &name, &engagement_subject).await,
        "{name}"
    );

    // A return by its number, its period and the acknowledgement.
    let ret = scalar_i64(
        &app,
        "select id from accounting_returns where acknowledgement = 'FTA-ACK-DEMO-204518'",
    )
    .await;
    let return_subject = Subject::new("accounting.return", ret);
    assert!(finds(&app, &access, &format!("RET-{ret:04}"), &return_subject).await);
    assert!(finds(&app, &access, "fta ack demo 204518", &return_subject).await);
    let kind = scalar_text(
        &app,
        &format!("select return_type from accounting_returns where id = {ret}"),
    )
    .await;
    let period_end = scalar_text(
        &app,
        &format!("select cast(period_end as text) from accounting_returns where id = {ret}"),
    )
    .await;
    let month = &period_end[..7];
    assert!(
        finds(
            &app,
            &access,
            &format!("{} {month}", kind.to_uppercase()),
            &return_subject
        )
        .await
    );
    assert!(finds(&app, &access, &period_end, &return_subject).await);
    assert_eq!(
        related_count(&app, &access, &return_subject, "engagement").await,
        1
    );
    let owner = scalar_i64(
        &app,
        &format!("select engagement_id from accounting_returns where id = {ret}"),
    )
    .await;
    assert!(
        related_count(
            &app,
            &access,
            &Subject::new("accounting.engagement", owner),
            "returns"
        )
        .await
            >= 1
    );

    // A CDD file by its number, with the client's engagements.
    let cdd = scalar_i64(
        &app,
        "select id from accounting_cdd_files order by id limit 1",
    )
    .await;
    let cdd_subject = Subject::new("accounting.cdd_file", cdd);
    assert!(finds(&app, &access, &format!("CDD-{cdd:04}"), &cdd_subject).await);
    assert!(related_count(&app, &access, &cdd_subject, "engagements").await >= 1);

    // The engagement page loads its Related panel.
    app.acting_as(&partner);
    app.get(&format!("/accounting/engagements/{engagement}"))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/accounting.engagement:{engagement}/panel\""
        ));
    app.get(&format!(
        "/related/accounting.engagement:{engagement}/panel"
    ))
    .await
    .assert_ok()
    .assert_see("data-group=\"returns\"");
}

#[renox::test]
async fn an_aml_case_is_found_by_the_mlro_alone() {
    let app = booted().await;
    let client = scalar_i64(
        &app,
        "select client_party_id from accounting_engagements order by id limit 1",
    )
    .await;
    let mut case = AmlCase {
        client_party_id: client,
        facts: "Cash deposits structured below the reporting threshold".to_owned(),
        goaml_reference: Some("GOAML-2026-88123".to_owned()),
        status: AmlStatus::Open,
        ..AmlCase::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    case.insert_in_state(&mut tx, AmlStatus::Open, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    search::rebuild(app.db()).await.unwrap();
    let subject = Subject::new("accounting.aml_case", case.id);
    let number = case.number();

    let mlro = person(&app, "Maryam", COMPLIANCE).await;
    let access = Access::of_user(app.db(), &mlro).await.unwrap();
    assert!(finds(&app, &access, &number, &subject).await);
    assert!(finds(&app, &access, "goaml 2026 88123", &subject).await);
    // The facts are never indexed, even for the MLRO.
    let document = search::document_of(app.db(), &subject)
        .await
        .unwrap()
        .unwrap();
    assert!(!document.keywords.contains("structured"));
    assert!(!document.title.contains("Cash"));

    for (name, role) in [("Pat", PARTNER), ("Ada", ADMIN)] {
        let user = person(&app, name, role).await;
        let access = Access::of_user(app.db(), &user).await.unwrap();
        let results = search::search(app.db(), &access, &number, 50)
            .await
            .unwrap();
        assert!(
            results
                .groups
                .iter()
                .all(|group| group.subject_type != "accounting.aml_case"),
            "{role} must not find an AML case"
        );
        let party = Subject::new("parties.party", client);
        let groups = related::related(app.db(), &access, &party, PANEL_DEPTH)
            .await
            .unwrap();
        assert!(
            groups
                .iter()
                .all(|group| group.to_type != "accounting.aml_case")
        );
        app.acting_as(&user);
        app.get(&format!("/related/accounting.aml_case:{}/panel", case.id))
            .await
            .assert_not_found();
    }
}
