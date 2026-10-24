//! The clinic's records in the global search and the Related panel
//! (BL-AGT-12): on the demo clinic, a receptionist finds a patient by the
//! file number and the Emirates ID (stored only as a hash), a visit by its
//! number and the patient's file number; a clinic manager finds a claim and
//! a prior authorisation by their numbers and the payer's reference, each
//! with its related records. A patient's result shows the name, the file
//! number and the next appointment only, and a diagnosis written in a visit
//! note finds nothing.

use host::roles::{self, CLINIC_MANAGER, RECEPTIONIST};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-clinics/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the clinics client");
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
async fn the_desk_finds_patients_and_visits_and_billing_finds_claims_and_authorisations() {
    let app = booted().await;
    search::rebuild(app.db()).await.unwrap();
    let receptionist = person(&app, "Rita", RECEPTIONIST).await;
    let desk = Access::of_user(app.db(), &receptionist).await.unwrap();

    // A patient by the file number and by the Emirates ID, typed any way.
    let (patient, eid) = (
        scalar_i64(
            &app,
            "select d.subject_id from wl_documents_documents d join wl_documents_kinds k on k.id = d.kind_id \
             where k.key = 'patient_emirates_id' and d.number is not null \
             and d.subject_id in (select patient_party_id from clinics_visits) order by d.id limit 1",
        )
        .await,
        scalar_text(
            &app,
            "select d.number from wl_documents_documents d join wl_documents_kinds k on k.id = d.kind_id \
             where k.key = 'patient_emirates_id' and d.number is not null \
             and d.subject_id in (select patient_party_id from clinics_visits) order by d.id limit 1",
        )
        .await,
    );
    let patient_subject = Subject::new("clinics.patient", patient);
    let file = scalar_text(
        &app,
        &format!("select code from wl_parties_parties where id = {patient}"),
    )
    .await;
    assert!(finds(&app, &desk, &file.to_lowercase(), &patient_subject).await);
    assert!(finds(&app, &desk, &eid, &patient_subject).await);
    assert!(finds(&app, &desk, &eid.replace('-', ""), &patient_subject).await);
    // The Emirates ID is a hash; the result shows the name and file only.
    let identifiers = search::identifiers_of(app.db(), &patient_subject)
        .await
        .unwrap();
    let eid_row = identifiers
        .iter()
        .find(|row| row.kind == "emirates_id")
        .expect("the Emirates ID is an identifier");
    assert!(eid_row.hashed);
    assert!(!eid_row.value.contains("784"));
    let document = search::document_of(app.db(), &patient_subject)
        .await
        .unwrap()
        .unwrap();
    assert!(document.secondary.starts_with(&format!("File {file}")));
    assert!(!document.keywords.contains("784"));
    assert_eq!(document.sensitivity, "personal");

    // A supplier or a payer is never a patient result.
    let payer = scalar_i64(
        &app,
        "select party_id from wl_parties_roles where role_key = 'payer' order by id limit 1",
    )
    .await;
    let payer_code = scalar_text(
        &app,
        &format!("select code from wl_parties_parties where id = {payer}"),
    )
    .await;
    assert!(
        !finds(
            &app,
            &desk,
            &payer_code,
            &Subject::new("clinics.patient", payer)
        )
        .await
    );

    // A visit by its number and by the patient's file number.
    let visit = scalar_i64(
        &app,
        &format!(
            "select id from clinics_visits where patient_party_id = {patient} order by id limit 1"
        ),
    )
    .await;
    let visit_subject = Subject::new("clinics.visit", visit);
    assert!(finds(&app, &desk, &format!("vis {visit:04}"), &visit_subject).await);
    assert!(finds(&app, &desk, &file, &visit_subject).await);
    assert_eq!(
        related_count(&app, &desk, &visit_subject, "patient").await,
        1
    );
    assert!(related_count(&app, &desk, &patient_subject, "visits").await >= 1);

    // A claim by its number and the payer's reference, with its visit.
    let manager = person(&app, "Mona", CLINIC_MANAGER).await;
    let billing = Access::of_user(app.db(), &manager).await.unwrap();
    let claim = scalar_i64(
        &app,
        "select id from clinics_claims where submission_reference is not null order by id limit 1",
    )
    .await;
    let claim_subject = Subject::new("clinics.claim", claim);
    assert!(finds(&app, &billing, &format!("CLM-{claim:04}"), &claim_subject).await);
    let reference = scalar_text(
        &app,
        &format!("select submission_reference from clinics_claims where id = {claim}"),
    )
    .await;
    assert!(finds(&app, &billing, &reference, &claim_subject).await);
    assert_eq!(
        related_count(&app, &billing, &claim_subject, "visit").await,
        1
    );
    // The desk does not hold `claims.manage`: no claim for the receptionist.
    assert!(!finds(&app, &desk, &format!("CLM-{claim:04}"), &claim_subject).await);

    // An authorisation by its number and the payer's authorisation number.
    let authorisation = scalar_i64(
        &app,
        "select id from clinics_prior_authorisations where reference = 'GS-PA-90417'",
    )
    .await;
    let authorisation_subject = Subject::new("clinics.prior_authorisation", authorisation);
    assert!(
        finds(
            &app,
            &billing,
            &format!("PA-{authorisation:04}"),
            &authorisation_subject
        )
        .await
    );
    assert!(finds(&app, &billing, "gs pa 90417", &authorisation_subject).await);
    assert_eq!(
        related_count(&app, &billing, &authorisation_subject, "patient").await,
        1
    );

    // The visit page loads its Related panel.
    app.acting_as(&receptionist);
    app.get(&format!("/clinics/visits/{visit}"))
        .await
        .assert_ok()
        .assert_see(&format!("hx-get=\"/related/clinics.visit:{visit}/panel\""));
    app.get(&format!("/related/clinics.visit:{visit}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"patient\" data-count=\"1\"");
}

#[renox::test]
async fn a_diagnosis_written_in_a_visit_note_finds_nothing() {
    let app = booted().await;
    // The booker's note on the booking of a visit: health data.
    let booking = scalar_i64(
        &app,
        "select booking_id from clinics_visits order by id limit 1",
    )
    .await;
    renox::db::sql(format!(
        "update wl_scheduling_bookings set notes = 'Suspected psoriasis flare, plaque on both elbows' where id = {booking}"
    ))
    .execute(app.db())
    .await
    .unwrap();
    search::rebuild(app.db()).await.unwrap();

    let manager = person(&app, "Mona", CLINIC_MANAGER).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();
    for word in ["psoriasis", "plaque", "elbows"] {
        let results = search::search(app.db(), &access, word, 50).await.unwrap();
        assert!(
            results.groups.is_empty(),
            "`{word}` found {:?}",
            results.groups
        );
    }
    let leaked = renox::db::sql(
        "select count(*) from wl_search_documents where title like '%psoriasis%' \
         or secondary like '%psoriasis%' or keywords like '%psoriasis%'"
            .to_owned(),
    )
    .scalar::<i64>(app.db())
    .await
    .unwrap();
    assert_eq!(leaked, 0);
}
