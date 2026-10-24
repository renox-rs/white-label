//! The lists of the EdTech vertical on the demo centre (#538): the
//! enrolments, sessions, batches, certificates and courses are
//! `renox::grid`s that are direct children of their pages, search (the
//! enrolments also by the student's name, kept by the parties module),
//! filter by their columns, sort and page through the query string.

mod grid_support;

use grid_support::{encode, grid_is_a_child_of_the_page, load, range, total};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the EdTech vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-edtech/client.toml"
);

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn text_of(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap()
}

/// The demo centre, seeded, with the administrator signed in.
async fn demo() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the EdTech vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    host::edtech_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@gulfskills.example")
        .await
        .unwrap()
        .expect("the demo administrator");
    app.acting_as(&admin);
    app
}

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_edtech_list_is_a_grid_that_fills_the_page() {
    for template in [
        "edtech/views/edtech/enrolments.html",
        "edtech/views/edtech/sessions.html",
        "edtech/views/edtech/batches.html",
        "edtech/views/edtech/certificates.html",
        "edtech/views/edtech/courses.html",
    ] {
        grid_is_a_child_of_the_page(template);
    }
}

/// The enrolments: pages of ten, newest first, a filter by batch and the
/// student found by name.
#[renox::test]
async fn the_enrolments_page_filter_and_find_the_student() {
    let app = demo().await;
    let enrolments = count(&app, "select count(*) from edtech_enrolments").await;
    assert!(enrolments > 10, "the demo has {enrolments} enrolments");
    let first = load(&app, "/edtech/enrolments?per_page=10&sort=id").await;
    assert_eq!(range(&first), (1, 10, enrolments));
    let second = load(&app, "/edtech/enrolments?per_page=10&sort=id&page=2").await;
    assert_eq!(range(&second), (11, enrolments.min(20), enrolments));
    let lowest = count(&app, "select min(id) from edtech_enrolments").await;
    let number = format!("ENR-{lowest:04}");
    assert!(first.contains(&number) && !second.contains(&format!(">{number}<")));

    let batch = count(
        &app,
        "select min(batch_id) from edtech_enrolments where batch_id is not null",
    )
    .await;
    let of_batch = count(
        &app,
        &format!("select count(*) from edtech_enrolments where batch_id = {batch}"),
    )
    .await;
    assert_eq!(
        total(&app, &format!("/edtech/enrolments?in.batch_id={batch}")).await,
        of_batch
    );

    let student = text_of(
        &app,
        &format!(
            "select display_name from wl_parties_parties where id = \
             (select student_party_id from edtech_enrolments where id = {lowest})"
        ),
    )
    .await;
    let found = load(
        &app,
        &format!("/edtech/enrolments?search={}&per_page=100", encode(&student)),
    )
    .await;
    assert!(found.contains(&number), "{student}'s enrolment is found");
    assert_eq!(
        total(&app, "/edtech/enrolments?search=zzqqxx-nobody").await,
        0
    );
}

/// The sessions, batches, certificates and courses count their rows, page
/// and filter by status.
#[renox::test]
async fn the_sessions_batches_certificates_and_courses_page_and_filter() {
    let app = demo().await;
    let sessions = count(&app, "select count(*) from edtech_sessions").await;
    let shown = total(&app, "/edtech/sessions?per_page=10").await;
    assert!(shown >= 1 && shown <= sessions);
    if shown > 10 {
        let second = load(&app, "/edtech/sessions?per_page=10&page=2").await;
        assert_eq!(range(&second), (11, shown.min(20), shown));
    }
    let batches = count(&app, "select count(*) from edtech_batches").await;
    assert_eq!(total(&app, "/edtech/batches?per_page=10").await, batches);
    let status = text_of(&app, "select status from edtech_batches order by id limit 1").await;
    let of_status = count(
        &app,
        &format!("select count(*) from edtech_batches where status = '{status}'"),
    )
    .await;
    assert_eq!(
        total(&app, &format!("/edtech/batches?in.status={status}")).await,
        of_status
    );
    let certificates = count(&app, "select count(*) from edtech_certificates").await;
    assert_eq!(
        total(&app, "/edtech/certificates?per_page=10").await,
        certificates
    );
    let courses = count(&app, "select count(*) from edtech_courses").await;
    let listed = total(&app, "/edtech/courses?per_page=10").await;
    assert!(listed >= 1 && listed <= courses);
    let code = text_of(&app, "select code from edtech_courses order by id limit 1").await;
    let found = load(&app, &format!("/edtech/courses?search={}", encode(&code))).await;
    assert!(range(&found).2 >= 1 && found.contains(&code));
}
