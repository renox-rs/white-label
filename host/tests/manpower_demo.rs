//! The demo agency of the Manpower client (`manpower:demo`): seeded on a test
//! database it has every state of the machines the spec asks for, seeding
//! again changes nothing, and the main pages answer for the role that works
//! with them.

use host::manpower_demo::LOGINS;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Manpower vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-manpower/client.toml"
);

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Manpower vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Every one of `states` has a row in `table`.
async fn has_states(app: &TestApp, table: &str, states: &[&str]) {
    for state in states {
        let n = count(
            app,
            &format!("select count(*) from {table} where status = '{state}'"),
        )
        .await;
        assert!(n >= 1, "{table} has no row in the state {state}");
    }
}

/// The seed runs twice without an error and the second run changes nothing;
/// the states of the spec are all there: the requests, the deployments, the
/// timesheets, the payroll run, the settlement, the pay accounts.
#[renox::test]
async fn the_manpower_demo_has_every_state_and_is_seeded_once() {
    let app = booted().await;
    assert!(host::manpower_demo::seed(app.kernel()).await.unwrap());
    has_states(
        &app,
        "manpower_requests",
        &["draft", "open", "partially_filled", "filled"],
    )
    .await;
    has_states(
        &app,
        "manpower_deployments",
        &["planned", "active", "suspended", "ended"],
    )
    .await;
    has_states(
        &app,
        "wl_attendance_timesheets",
        &["open", "submitted", "approved"],
    )
    .await;
    has_states(&app, "wl_hr_payroll_runs", &["draft"]).await;
    has_states(&app, "wl_hr_final_settlements", &["draft"]).await;
    has_states(
        &app,
        "wl_hr_pay_accounts",
        &["active", "cooling", "pending_approval"],
    )
    .await;
    has_states(
        &app,
        "wl_invoicing_rate_card_versions",
        &["accepted", "pending"],
    )
    .await;
    has_states(&app, "wl_invoicing_invoices", &["overdue"]).await;
    assert_eq!(
        count(&app, "select count(*) from wl_payments_cheques").await,
        1
    );
    assert_eq!(
        count(&app, "select count(*) from wl_hr_visa_cases").await,
        1
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_hr_payroll_lines where status = 'held'"
        )
        .await,
        1,
        "one held payroll line"
    );
    // A contract ends in 45 days (the renewal warning) and the dates are
    // relative to today.
    assert_eq!(
        count(
            &app,
            "select count(*) from manpower_client_contracts where status = 'active'"
        )
        .await,
        3
    );

    let tables = [
        "manpower_client_contracts",
        "manpower_requests",
        "manpower_deployments",
        "wl_hr_employees",
        "wl_attendance_timesheets",
        "wl_documents_documents",
        "wl_parties_parties",
        "users",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(count(&app, &format!("select count(*) from {table}")).await);
    }
    assert!(
        !host::manpower_demo::seed(app.kernel()).await.unwrap(),
        "the second run finds the agency there"
    );
    for (table, was) in tables.iter().zip(before) {
        assert_eq!(
            count(&app, &format!("select count(*) from {table}")).await,
            was,
            "{table} is the same after the second seed"
        );
    }
}

/// Every role of the agency opens the pages it works with.
#[renox::test]
async fn the_main_pages_answer_for_the_roles_of_the_demo_agency() {
    let app = booted().await;
    host::manpower_demo::seed(app.kernel()).await.unwrap();
    let employee = count(&app, "select min(id) from wl_hr_employees").await;
    let run = count(&app, "select min(id) from wl_hr_payroll_runs").await;
    let settlement = count(&app, "select min(id) from wl_hr_final_settlements").await;
    let account = count(&app, "select min(id) from wl_hr_pay_accounts").await;
    let cheque = count(&app, "select min(id) from wl_payments_cheques").await;
    let invoice = count(&app, "select min(id) from wl_invoicing_invoices").await;

    let pages: [(&str, Vec<String>); 8] = [
        (
            "admin@desertstar.example",
            vec![
                "/dashboard".into(),
                "/manpower/contracts".into(),
                "/manpower/requests".into(),
                "/manpower/deployments".into(),
                "/hr/employees".into(),
                format!("/hr/employees/{employee}"),
                format!("/hr/payroll/{run}"),
                format!("/hr/settlements/{settlement}"),
                format!("/hr/pay-accounts/{account}"),
                "/cheques".into(),
                format!("/cheques/{cheque}"),
                "/invoices".into(),
                format!("/invoices/{invoice}"),
                "/documents/review".into(),
                "/parties".into(),
            ],
        ),
        (
            "md@desertstar.example",
            vec![
                "/dashboard".into(),
                "/manpower/contracts".into(),
                format!("/hr/payroll/{run}"),
                format!("/hr/settlements/{settlement}"),
            ],
        ),
        (
            "hr@desertstar.example",
            vec![
                "/dashboard".into(),
                "/hr/employees".into(),
                format!("/hr/employees/{employee}"),
                format!("/hr/payroll/{run}"),
                format!("/hr/pay-accounts/{account}"),
                "/manpower/absence".into(),
            ],
        ),
        (
            "accountmanager@desertstar.example",
            vec![
                "/dashboard".into(),
                "/manpower/contracts".into(),
                "/manpower/requests".into(),
                "/manpower/deployments".into(),
            ],
        ),
        (
            "accountant@desertstar.example",
            vec![
                "/dashboard".into(),
                "/invoices".into(),
                format!("/invoices/{invoice}"),
                "/cheques".into(),
                format!("/cheques/{cheque}"),
            ],
        ),
        (
            "supervisor@desertstar.example",
            vec![
                "/dashboard".into(),
                "/manpower/deployments".into(),
                "/attendance/crew".into(),
            ],
        ),
        (
            "pro@desertstar.example",
            vec![
                "/dashboard".into(),
                "/hr/employees".into(),
                format!("/hr/employees/{employee}"),
            ],
        ),
        (
            "approver@gulftower.example",
            vec![
                "/dashboard".into(),
                "/manpower/requests".into(),
                "/manpower/deployments".into(),
            ],
        ),
    ];
    assert_eq!(pages.len(), LOGINS.len(), "every login has its pages");
    for (email, paths) in pages {
        let user = User::find_by_email(app.db(), email)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the demo login {email}"));
        for path in paths {
            app.acting_as(&user);
            let page = app.get(&path).await;
            assert_eq!(
                page.status.as_u16(),
                200,
                "{path} answers 200 for {email}"
            );
        }
    }
}
