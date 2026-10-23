//! The demo centre of the EdTech client (`edtech:demo`): seeded on a test
//! database it has every state of the machines the spec asks for, seeding
//! again changes nothing, every login opens the pages of its role, and the
//! bugs the richer data showed stay fixed.

use host::edtech_demo::LOGINS;
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

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the EdTech vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// At least two rows of `table` are in each of `states`.
async fn has_states(app: &TestApp, table: &str, states: &[&str], least: i64) {
    for state in states {
        let n = count(
            app,
            &format!("select count(*) from {table} where status = '{state}'"),
        )
        .await;
        assert!(
            n >= least,
            "{table} has {n} rows in the state {state}, not {least}"
        );
    }
}

/// The seed runs twice and the second run changes nothing; the stories of
/// the spec are all there.
#[renox::test]
async fn the_edtech_demo_has_every_state_and_is_seeded_once() {
    let app = booted().await;
    let started = std::time::Instant::now();
    assert!(host::edtech_demo::seed(app.kernel()).await.unwrap());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "the seed took {:?}",
        started.elapsed()
    );
    has_states(
        &app,
        "edtech_batches",
        &[
            "planned",
            "open",
            "running",
            "completed",
            "cancelled",
            "terminated",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "edtech_sessions",
        &["scheduled", "held", "cancelled"],
        2,
    )
    .await;
    has_states(
        &app,
        "edtech_enrolments",
        &[
            "pending_payment",
            "active",
            "frozen",
            "completed",
            "withdrawn",
            "transferred",
            "lapsed",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "edtech_certificates",
        &["draft", "issued", "superseded", "revoked"],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_payments_refunds",
        &["requested", "approved", "rejected"],
        1,
    )
    .await;
    has_states(
        &app,
        "wl_payments_payments",
        &["received", "charged_back"],
        1,
    )
    .await;
    has_states(&app, "wl_invoicing_invoices", &["issued", "paid"], 2).await;
    has_states(
        &app,
        "wl_sales_leads",
        &["new", "contacted", "qualified", "won", "lost"],
        1,
    )
    .await;
    has_states(&app, "wl_sales_campaigns", &["draft", "active", "ended"], 1).await;
    for cause in ["student", "block"] {
        assert!(
            count(
                &app,
                &format!("select count(*) from edtech_enrolment_freezes where cause = '{cause}'")
            )
            .await
                >= 1,
            "no freeze for the cause {cause}"
        );
    }
    for mark in ["present", "late", "absent", "excused"] {
        assert!(
            count(
                &app,
                &format!("select count(*) from edtech_attendance where mark = '{mark}'")
            )
            .await
                >= 2,
            "too few marks {mark}"
        );
    }
    for (what, query) in [
        (
            "a make-up right",
            "select count(*) from edtech_make_up_rights",
        ),
        (
            "a waiting-list entry",
            "select count(*) from edtech_batch_waiting",
        ),
        (
            "a package enrolment",
            "select count(*) from edtech_enrolments where kind = 'package'",
        ),
        (
            "an enrolment with instalments",
            "select count(*) from edtech_enrolments where instalments > 1",
        ),
        (
            "a flagged session",
            "select count(*) from edtech_sessions where flag_reason is not null",
        ),
        ("a tutor alert", "select count(*) from edtech_tutor_alerts"),
        ("a chargeback", "select count(*) from edtech_chargebacks"),
        (
            "an expired tutor document",
            "select count(*) from wl_documents_documents where expiry_date < date('now') and number like 'PC-%'",
        ),
        (
            "a tutor document ending within 30 days",
            "select count(*) from wl_documents_documents where expiry_date between date('now') and date('now', '+30 day') and number like 'PC-%'",
        ),
        (
            "the centre permit",
            "select count(*) from wl_documents_documents where number like 'KHDA-CP-%'",
        ),
        (
            "a freelance tutor",
            "select count(*) from wl_hr_employees where employment_type = 'freelance'",
        ),
        (
            "a minor with a guardian",
            "select count(*) from edtech_enrolments where guardian_party_id is not null",
        ),
        ("a ticket", "select count(*) from wl_ticketing_tickets"),
    ] {
        assert!(count(&app, query).await >= 1, "no {what}");
    }
    assert!(count(&app, "select count(*) from edtech_courses").await >= 8);
    assert!(count(&app, "select count(distinct programme) from edtech_courses").await >= 4);
    let tables = [
        "users",
        "wl_parties_parties",
        "wl_hr_employees",
        "wl_hr_qualifications",
        "edtech_courses",
        "edtech_batches",
        "edtech_sessions",
        "edtech_enrolments",
        "edtech_attendance",
        "edtech_certificates",
        "wl_invoicing_invoices",
        "wl_payments_payments",
        "wl_payments_refunds",
        "wl_sales_leads",
        "wl_ticketing_tickets",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(count(&app, &format!("select count(*) from {table}")).await);
    }
    assert!(
        !host::edtech_demo::seed(app.kernel()).await.unwrap(),
        "the second run finds the centre there"
    );
    for (table, was) in tables.iter().zip(before) {
        assert_eq!(
            count(&app, &format!("select count(*) from {table}")).await,
            was,
            "{table} is the same after the second seed"
        );
    }
}

/// Every login opens its dashboard and the pages of its role.
#[renox::test]
async fn every_login_of_the_demo_centre_opens_its_pages() {
    let app = booted().await;
    host::edtech_demo::seed(app.kernel()).await.unwrap();
    let batch = count(&app, "select min(id) from edtech_batches").await;
    let session = count(
        &app,
        "select min(id) from edtech_sessions where flag_reason is not null",
    )
    .await;
    let enrolment = count(&app, "select min(id) from edtech_enrolments").await;
    let certificate = count(&app, "select min(id) from edtech_certificates").await;
    let staff = || -> Vec<String> {
        vec![
            "/dashboard".into(),
            "/edtech/courses".into(),
            "/edtech/batches".into(),
            format!("/edtech/batches/{batch}"),
            "/edtech/sessions".into(),
            format!("/edtech/sessions/{session}"),
            "/edtech/enrolments".into(),
            format!("/edtech/enrolments/{enrolment}"),
            "/edtech/certificates".into(),
            format!("/edtech/certificates/{certificate}"),
        ]
    };
    let pages_of = |email: &str| -> Vec<String> {
        let mut pages = staff();
        match email {
            "admin@gulfskills.example" => pages.extend([
                "/tickets".into(),
                "/campaigns".into(),
                "/invoices".into(),
                "/payments".into(),
                "/refunds".into(),
                "/parties".into(),
                "/documents/flags".into(),
            ]),
            "owner@gulfskills.example" => {}
            "manager@gulfskills.example" => pages.extend(["/tickets".into(), "/approvals".into()]),
            "counsellor@gulfskills.example" => {
                pages.extend(["/tickets".into(), "/payments".into(), "/parties".into()])
            }
            "coordinator@gulfskills.example" => pages.push("/tickets".into()),
            "accountant@gulfskills.example" => {
                // The accountant reads the money, not the certificates.
                pages.retain(|page| !page.contains("certificates"));
                pages.extend(["/invoices".into(), "/payments".into(), "/refunds".into()]);
            }
            // A tutor sees the batches and teaches the sessions, nothing of
            // the money or the certificates.
            _ => {
                pages = vec![
                    "/dashboard".into(),
                    "/edtech/batches".into(),
                    format!("/edtech/batches/{batch}"),
                    "/edtech/sessions".into(),
                    format!("/edtech/sessions/{session}"),
                ];
            }
        }
        pages.push("/hr/leave".into());
        pages
    };
    let mut refused = Vec::new();
    for login in LOGINS {
        let user = User::find_by_email(app.db(), login.email)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the demo login {}", login.email));
        for path in pages_of(login.email) {
            app.acting_as(&user);
            let page = app.get(&path).await;
            if page.status.as_u16() != 200 {
                refused.push(format!("{} {path}: {}", login.email, page.status));
            }
        }
    }
    assert!(
        refused.is_empty(),
        "pages that did not answer 200: {refused:#?}"
    );
}

/// The batch and the session name their tutor and room (they once read
/// `Resource 2`), and a certificate that was issued is verified by its code
/// on the public page.
#[renox::test]
async fn the_pages_name_the_tutor_the_room_and_verify_a_certificate() {
    let app = booted().await;
    host::edtech_demo::seed(app.kernel()).await.unwrap();
    let manager = User::find_by_email(app.db(), "manager@gulfskills.example")
        .await
        .unwrap()
        .expect("the demo centre manager");
    app.acting_as(&manager);
    let batch = count(
        &app,
        "select b.id from edtech_batches b join edtech_courses c on c.id = b.course_id where c.code = 'IELTS-B2' and b.status = 'running'",
    )
    .await;
    let page = app.get(&format!("/edtech/batches/{batch}")).await;
    page.assert_ok();
    let text = page.text();
    assert!(text.contains("Sara Whitfield"), "the tutor is named");
    assert!(text.contains("Room B"), "the room is named");
    assert!(!text.contains("Resource "), "no resource number is shown");
    let sessions = app.get("/edtech/sessions").await;
    sessions.assert_ok();
    assert!(sessions.text().contains("Computer lab"));
    let code = renox::db::sql(
        "select verification_code from edtech_certificates where status = 'issued' order by id limit 1"
            .to_owned(),
    )
    .scalar::<String>(app.db())
    .await
    .unwrap();
    app.logout();
    let verify = app.get(&format!("/verify/certificate/{code}")).await;
    verify.assert_ok();
    assert!(verify.text().contains("Valid"));
}

/// The complaint assigned to the centre manager and the task assigned to the
/// counsellor can be opened by them (the roles of the centre once held no
/// ticketing permission, so the tasks the app gives them led to a 403).
#[renox::test]
async fn the_staff_open_the_tickets_assigned_to_them() {
    let app = booted().await;
    host::edtech_demo::seed(app.kernel()).await.unwrap();
    for (email, role_ticket) in [
        ("manager@gulfskills.example", "student_complaint"),
        ("counsellor@gulfskills.example", "task"),
        ("coordinator@gulfskills.example", "classroom_issue"),
    ] {
        let user = User::find_by_email(app.db(), email)
            .await
            .unwrap()
            .expect("the demo login");
        let ticket = count(
            &app,
            &format!(
                "select t.id from wl_ticketing_tickets t join wl_ticketing_categories c on c.id = t.category_id where c.key = '{role_ticket}' order by t.id limit 1"
            ),
        )
        .await;
        app.acting_as(&user);
        app.get(&format!("/tickets/{ticket}")).await.assert_ok();
    }
}

/// The login page of a local server lists the people of the centre, one tap
/// each, once the demo is made.
#[renox::test]
async fn the_login_page_lists_the_people_of_the_demo_centre() {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the EdTech vertical");
    let app = TestApp::with_config(host::app_for(config).expect("the app"), |config| {
        config.env = renox::Environment::Local;
    })
    .await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app.get("/login")
        .await
        .assert_ok()
        .assert_dont_see("owner@gulfskills.example");
    host::edtech_demo::seed(app.kernel()).await.unwrap();
    let login = app.get("/login").await;
    for person in LOGINS {
        login.assert_see(&format!("data-demo-email=\"{}\"", person.email));
    }
}
