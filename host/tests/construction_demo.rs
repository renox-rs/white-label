//! The demo company of the Construction client (`construction:demo`): seeded
//! on a test database it has every state of the state machines the spec asks
//! for, every screen of the map opens as the MD, and seeding again changes
//! nothing.

use std::collections::BTreeMap;

use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Construction vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-construction/client.toml"
);

/// The number of rows of `table` in each status.
async fn by_status(app: &TestApp, table: &str) -> BTreeMap<String, i64> {
    let rows = renox::db::sql(format!(
        "select status, count(*) as n from {table} group by status"
    ))
    .fetch_all(app.db())
    .await
    .unwrap();
    rows.iter()
        .map(|r| {
            (
                r.try_get::<String>("status").unwrap(),
                r.try_get::<i64>("n").unwrap(),
            )
        })
        .collect()
}

/// Every one of `states` has at least `least` rows in `table`.
async fn has_states(app: &TestApp, table: &str, states: &[&str], least: i64) {
    let found = by_status(app, table).await;
    for state in states {
        let n = found.get(*state).copied().unwrap_or(0);
        assert!(
            n >= least,
            "{table}: {n} in {state}, wanted at least {least}; found {found:?}"
        );
    }
}

async fn total(app: &TestApp, table: &str) -> i64 {
    renox::db::sql(format!("select count(*) from {table}"))
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// The count of the rows `query` finds.
async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Construction vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

#[renox::test]
async fn the_construction_demo_has_every_state_and_is_seeded_once() {
    let app = booted().await;
    // The seeder of `db:seed` fills nothing in a test: only the command does.
    assert_eq!(total(&app, "construction_projects").await, 0);

    let started = std::time::Instant::now();
    let report = host::construction_demo::seed(app.kernel()).await.unwrap();
    eprintln!("construction demo seeded in {:?}", started.elapsed());
    assert!(!report.already);

    // The people: a login per role, the clients and the suppliers.
    assert!(host::construction_demo::LOGINS.len() >= 10);
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_parties_parties where legal_name_en in \
             ('Gulf Horizon Developments LLC', 'Festival Plaza Retail Holdings LLC', \
              'Al Noor Healthcare Group LLC')"
        )
        .await,
        3
    );
    assert_eq!(
        count(
            &app,
            "select count(distinct party_id) from wl_parties_roles where role_key in ('supplier', 'subcontractor')"
        )
        .await,
        5
    );
    has_states(
        &app,
        "wl_documents_documents",
        &["accepted", "pending_review"],
        1,
    )
    .await;
    // One subcontractor's Dubai Municipality registration is valid, one expired.
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_documents_documents d join wl_documents_kinds k on k.id = d.kind_id \
             where k.key = 'dubai_municipality_contractor_registration' and d.status = 'accepted' \
             and d.expiry_date < date('now')"
        )
        .await,
        1
    );

    // The projects: four in progress, one in setup, one handed over, one terminated.
    let projects = by_status(&app, "construction_projects").await;
    assert_eq!(projects.get("in_progress"), Some(&4), "{projects:?}");
    assert_eq!(projects.get("setup"), Some(&1), "{projects:?}");
    assert_eq!(
        projects.get("practical_completion"),
        Some(&1),
        "{projects:?}"
    );
    assert_eq!(projects.get("terminated"), Some(&1), "{projects:?}");
    assert!(total(&app, "construction_ipas").await >= 20);
    has_states(
        &app,
        "construction_ipas",
        &["draft", "submitted", "certified", "invoiced", "superseded"],
        1,
    )
    .await;
    // Two IPAs submitted more than 14 days ago and not certified.
    assert!(
        count(
            &app,
            "select count(*) from construction_ipas where status = 'submitted' \
             and julianday('now') - julianday(submitted_on) > 14"
        )
        .await
            >= 2
    );
    has_states(
        &app,
        "construction_ipa_deductions",
        &["accepted", "contested", "undecided"],
        1,
    )
    .await;
    has_states(
        &app,
        "construction_retentions",
        &["held", "due", "invoiced", "settled"],
        1,
    )
    .await;
    has_states(
        &app,
        "construction_variations",
        &[
            "draft",
            "submitted",
            "approved",
            "rejected",
            "claimed",
            "abandoned",
            "withdrawn",
        ],
        1,
    )
    .await;
    for kind in ["addition", "omission", "time"] {
        assert!(
            count(
                &app,
                &format!("select count(*) from construction_variations where kind = '{kind}'")
            )
            .await
                >= 1,
            "{kind}"
        );
    }
    has_states(&app, "construction_guarantees", &["active", "released"], 1).await;
    assert_eq!(total(&app, "construction_guarantee_calls").await, 1);
    // The performance bond that expires in 25 days.
    assert_eq!(
        count(
            &app,
            "select count(*) from construction_guarantees where kind = 'performance' \
             and status = 'active' and julianday(expires_on) - julianday('now') between 24 and 26"
        )
        .await,
        1
    );

    // The people of the sites: onboarding, flags, an absence case.
    has_states(&app, "wl_hr_employees", &["onboarding", "active"], 2).await;
    assert!(total(&app, "wl_attendance_records").await >= 200);
    for flag in ["overtime_cap", "midday_break", "permit_invalid"] {
        assert!(
            count(
                &app,
                &format!("select count(*) from wl_attendance_flags where flag_type = '{flag}'")
            )
            .await
                >= 1,
            "{flag}"
        );
    }
    assert_eq!(total(&app, "construction_absence_cases").await, 1);

    // Stock and procurement.
    has_states(
        &app,
        "wl_inventory_purchase_orders",
        &["draft", "issued", "partially_received", "received"],
        1,
    )
    .await;
    assert_eq!(total(&app, "wl_inventory_locations").await, 7);
    assert!(total(&app, "wl_inventory_items").await >= 10);
    has_states(&app, "wl_inventory_goods_receipts", &["posted"], 8).await;
    assert!(total(&app, "wl_inventory_supplier_returns").await >= 2);
    assert!(total(&app, "wl_invoicing_supplier_debit_notes").await >= 1);
    has_states(&app, "wl_inventory_reorder_suggestions", &["open"], 1).await;
    has_states(
        &app,
        "wl_inventory_preferred_supplier_requests",
        &["pending", "approved"],
        1,
    )
    .await;
    assert!(total(&app, "wl_inventory_supplier_scores").await >= 1);

    // Money.
    has_states(
        &app,
        "wl_invoicing_invoices",
        &["issued", "overdue", "paid"],
        1,
    )
    .await;
    assert!(
        count(
            &app,
            "select count(*) from wl_invoicing_invoices where disputed = 1"
        )
        .await
            >= 1
    );
    has_states(
        &app,
        "wl_invoicing_supplier_bills",
        &["draft", "approved", "paid", "partially_paid"],
        1,
    )
    .await;
    has_states(&app, "wl_payments_cheques", &["bounced", "received"], 1).await;
    has_states(
        &app,
        "wl_payments_refunds",
        &["requested", "approved", "rejected"],
        1,
    )
    .await;
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_payments_payments where method = 'set_off'"
        )
        .await,
        1
    );
    has_states(&app, "wl_invoicing_company_budgets", &["approved"], 1).await;

    // Every screen of the map opens for the MD, and has something on it.
    let admin = User::find_by_email(app.db(), "admin@alwaha.example")
        .await
        .unwrap()
        .expect("the MD's login");
    app.acting_as(&admin);
    let screens = host::construction_demo::screen_map(app.state())
        .await
        .unwrap();
    assert!(screens.len() >= 60, "{} screens", screens.len());
    for screen in &screens {
        let response = app.get(&screen.url).await;
        assert!(
            response.status.is_success(),
            "{} ({}) answered {}",
            screen.url,
            screen.what,
            response.status
        );
    }

    // Seeding again changes nothing.
    let projects = total(&app, "construction_projects").await;
    let ipas = total(&app, "construction_ipas").await;
    let invoices = total(&app, "wl_invoicing_invoices").await;
    let parties = total(&app, "wl_parties_parties").await;
    let again = host::construction_demo::seed(app.kernel()).await.unwrap();
    assert!(again.already);
    assert_eq!(total(&app, "construction_projects").await, projects);
    assert_eq!(total(&app, "construction_ipas").await, ipas);
    assert_eq!(total(&app, "wl_invoicing_invoices").await, invoices);
    assert_eq!(total(&app, "wl_parties_parties").await, parties);
}

#[renox::test]
async fn the_property_demo_does_not_take_the_construction_marker() {
    // The marker is its own audit subject: the Property demo's check of its
    // own marker finds nothing on a database the Construction demo filled.
    let app = booted().await;
    host::construction_demo::seed(app.kernel()).await.unwrap();
    assert!(
        host::construction_demo::is_seeded(app.db()).await.unwrap(),
        "the Construction demo is done"
    );
    assert!(
        !host::demo_data::is_seeded(app.db()).await.unwrap(),
        "the Property marker is not the Construction one"
    );
}
