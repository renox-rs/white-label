//! The centre's records in the global search and the Related panel
//! (BL-AGT-13): on the demo centre, a centre manager finds an enrolment by
//! its number and the student's name, a batch by its number, a course by
//! its code and a certificate by its number, each with its related records;
//! the certificate's public verification code is not in the index, and no
//! student identity number is in it in clear text.

use host::roles::{self, CENTRE_MANAGER};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-edtech/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the edtech client");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    search::rebuild(app.db()).await.unwrap();
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
async fn a_centre_manager_finds_enrolments_batches_courses_and_certificates() {
    let app = booted().await;
    let manager = person(&app, "Huda", CENTRE_MANAGER).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();

    // A certificate with a number, of an enrolment in a batch.
    let certificate = scalar_i64(
        &app,
        "select id from edtech_certificates where number is not null order by id limit 1",
    )
    .await;
    let certificate_subject = Subject::new("edtech.certificate", certificate);
    let number = scalar_text(
        &app,
        &format!("select number from edtech_certificates where id = {certificate}"),
    )
    .await;
    assert!(finds(&app, &access, &number.to_lowercase(), &certificate_subject).await);
    assert_eq!(
        related_count(&app, &access, &certificate_subject, "enrolment").await,
        1
    );
    // The public verification code is not a way into the index.
    let code = scalar_text(
        &app,
        &format!("select verification_code from edtech_certificates where id = {certificate}"),
    )
    .await;
    assert!(!finds(&app, &access, &code, &certificate_subject).await);

    // The enrolment by its number and the student's name.
    let enrolment = scalar_i64(
        &app,
        &format!("select enrolment_id from edtech_certificates where id = {certificate}"),
    )
    .await;
    let enrolment_subject = Subject::new("edtech.enrolment", enrolment);
    assert!(
        finds(
            &app,
            &access,
            &format!("enr {enrolment:04}"),
            &enrolment_subject
        )
        .await
    );
    let student = scalar_text(
        &app,
        &format!(
            "select p.display_name from wl_parties_parties p join edtech_enrolments e on e.student_party_id = p.id where e.id = {enrolment}"
        ),
    )
    .await;
    assert!(
        finds(&app, &access, &student, &enrolment_subject).await,
        "{student}"
    );
    assert!(related_count(&app, &access, &enrolment_subject, "certificates").await >= 1);

    // A batch by its number, with its course and enrolments.
    let batch = scalar_i64(
        &app,
        "select batch_id from edtech_enrolments where batch_id is not null order by id limit 1",
    )
    .await;
    let batch_subject = Subject::new("edtech.batch", batch);
    assert!(finds(&app, &access, &format!("BAT-{batch:04}"), &batch_subject).await);
    assert_eq!(
        related_count(&app, &access, &batch_subject, "course").await,
        1
    );
    assert!(related_count(&app, &access, &batch_subject, "enrolments").await >= 1);

    // A course by its code.
    let course = scalar_i64(
        &app,
        &format!("select course_id from edtech_batches where id = {batch}"),
    )
    .await;
    let course_code = scalar_text(
        &app,
        &format!("select code from edtech_courses where id = {course}"),
    )
    .await;
    assert!(
        finds(
            &app,
            &access,
            &course_code,
            &Subject::new("edtech.course", course)
        )
        .await
    );

    // The enrolment page loads its Related panel.
    app.acting_as(&manager);
    app.get(&format!("/edtech/enrolments/{enrolment}"))
        .await
        .assert_ok()
        .assert_see(&format!(
            "hx-get=\"/related/edtech.enrolment:{enrolment}/panel\""
        ));
    app.get(&format!("/related/edtech.enrolment:{enrolment}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"certificates\"");
}

#[renox::test]
async fn no_student_identity_number_is_indexed_in_clear_text() {
    let app = booted().await;
    let numbers: Vec<String> = renox::db::sql(
        "select d.number as number from wl_documents_documents d \
         where d.number is not null and d.subject_type = 'parties.party' \
         and d.subject_id in (select student_party_id from edtech_enrolments)"
            .to_owned(),
    )
    .fetch_all(app.db())
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get::<String>("number").unwrap())
    .collect();
    let texts: Vec<String> = renox::db::sql(
        "select title || ' ' || secondary || ' ' || keywords as text from wl_search_documents \
         where subject_type like 'edtech.%'"
            .to_owned(),
    )
    .fetch_all(app.db())
    .await
    .unwrap()
    .iter()
    .map(|row| row.try_get::<String>("text").unwrap())
    .collect();
    assert!(!texts.is_empty());
    for number in &numbers {
        assert!(
            texts.iter().all(|text| !text.contains(number.as_str())),
            "{number}"
        );
    }
    let kinds: Vec<String> = renox::db::sql(
        "select distinct kind from wl_search_identifiers where subject_type like 'edtech.%'"
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
