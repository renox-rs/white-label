//! The demo company of the Auto client (`auto:demo`): seeded on a test
//! database it has every state of the machines the spec asks for, seeding
//! again changes nothing, it tells the same story whatever hour it is seeded
//! at (the quiet hours of the notifier included), every login opens the pages
//! of its role, and the bugs the richer data showed stay fixed.

use host::auto_demo::LOGINS;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Auto vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-auto/client.toml"
);

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Auto vertical");
    let app = TestApp::new(host::app_for(config).expect("the app")).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// At least `least` rows of `table` are in each of `states`.
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

/// The states of every machine the demo shows, as `(table, states)`.
async fn assert_stories(app: &TestApp) {
    has_states(
        app,
        "auto_job_cards",
        &[
            "checked_in",
            "awaiting_approval",
            "in_progress",
            "awaiting_parts",
            "quality_check",
            "ready",
            "delivered",
            "cancelled",
            "declined",
            "voided",
        ],
        1,
    )
    .await;
    has_states(
        app,
        "auto_insurance_claims",
        &[
            "registered",
            "awaiting_survey",
            "estimate_submitted",
            "approved",
            "supplementary_pending",
            "rejected",
            "cancelled",
            "total_loss",
            "invoiced",
            "settled",
            "voided",
        ],
        1,
    )
    .await;
    has_states(
        app,
        "auto_fleet_contracts",
        &["active", "suspended", "ended"],
        1,
    )
    .await;
    has_states(app, "auto_supplier_claims", &["raised", "recovered"], 1).await;
    for (what, query) in [
        (
            "a supplier claim the supplier accepted",
            "select count(*) from auto_supplier_claims where supplier_answer = 'accepted' and status = 'raised'",
        ),
        (
            "a supplier claim the supplier refused",
            "select count(*) from auto_supplier_claims where supplier_answer = 'rejected'",
        ),
        ("two insurers", "select count(*) - 1 from auto_insurers"),
        (
            "a customer",
            "select count(*) from wl_parties_parties where legal_name_ar is not null",
        ),
        (
            "a vehicle with a VIN",
            "select count(*) from auto_vehicles where vin is not null",
        ),
        (
            "an estimate waiting on the customer's link",
            "select count(*) from auto_estimate_approvals where status = 'pending'",
        ),
        (
            "a fleet job card with its PO",
            "select count(*) from auto_fleet_jobs where po_reference is not null",
        ),
        (
            "a monthly fleet invoice",
            "select count(*) from auto_fleet_invoices where kind = 'monthly'",
        ),
        (
            "a final fleet invoice",
            "select count(*) from auto_fleet_invoices where kind = 'final'",
        ),
        (
            "a booking made for a fleet",
            "select count(*) from wl_scheduling_bookings where status = 'booked'",
        ),
        (
            "a booking that opened a job card",
            "select count(*) from auto_job_cards where booking_id is not null",
        ),
        (
            "an internal job card from an incident",
            "select count(*) from auto_job_cards where internal = 1",
        ),
        ("an incident", "select count(*) from auto_incidents"),
        (
            "an accident job card",
            "select count(*) from auto_job_cards where accident_damage = 1",
        ),
        (
            "a police report",
            "select count(*) from wl_documents_documents d join wl_documents_kinds k on k.id = d.kind_id where k.key = 'police_report'",
        ),
        (
            "a payer bill",
            "select count(*) from wl_invoicing_payer_bills",
        ),
        ("an export sale", "select count(*) from auto_export_sales"),
        (
            "an export sale flagged for VAT",
            "select count(*) from auto_export_sales where vat_flagged_at is not null",
        ),
        (
            "an export sale with its evidence",
            "select count(*) from auto_export_evidence",
        ),
        (
            "a back-order",
            "select count(*) from wl_sales_sale_lines where backordered_milli > 0",
        ),
        (
            "a reservation",
            "select count(*) from wl_inventory_reservations",
        ),
        (
            "a sales order",
            "select count(*) from wl_sales_sales_orders",
        ),
        (
            "a recall with its traced lines",
            "select count(*) from wl_inventory_recall_lines",
        ),
        (
            "a loyalty member",
            "select count(*) from wl_sales_loyalty_accounts",
        ),
        (
            "a cheque collected in the field",
            "select count(*) from wl_payments_cheques where field_collected = 1",
        ),
        (
            "a cash handover short of a cheque",
            "select count(*) from wl_payments_cash_cheques where state = 'returned_lost'",
        ),
        (
            "a storage reminder to an owner",
            "select count(*) from wl_notify_messages where type_key = 'auto.collect_vehicle'",
        ),
        (
            "a paid invoice",
            "select count(*) from wl_invoicing_invoices where status = 'paid'",
        ),
    ] {
        assert!(count(app, query).await >= 1, "no {what}");
    }
}

/// The seed runs twice and the second run changes nothing; the stories of the
/// spec are all there.
#[renox::test]
async fn the_auto_demo_has_every_state_and_is_seeded_once() {
    let app = booted().await;
    let started = std::time::Instant::now();
    assert!(host::auto_demo::seed(app.kernel()).await.unwrap());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(120),
        "the seed took {:?}",
        started.elapsed()
    );
    assert_stories(&app).await;
    let tables = [
        "users",
        "wl_parties_parties",
        "auto_vehicles",
        "auto_job_cards",
        "auto_insurance_claims",
        "auto_fleet_contracts",
        "auto_supplier_claims",
        "wl_invoicing_invoices",
        "wl_payments_payments",
        "wl_inventory_stock_movements",
        "wl_sales_sales_orders",
        "wl_notify_messages",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(count(&app, &format!("select count(*) from {table}")).await);
    }
    assert!(
        !host::auto_demo::seed(app.kernel()).await.unwrap(),
        "the second run finds the company there"
    );
    for (table, was) in tables.iter().zip(before) {
        assert_eq!(
            count(&app, &format!("select count(*) from {table}")).await,
            was,
            "{table} is the same after the second seed"
        );
    }
}

/// No step of the seed runs today: every step is set to 10:00 of a day gone
/// by, so the story does not depend on the hour it is seeded at (a step set
/// to a moment in the future would run at the real time, which can fall in the
/// quiet hours of the notifier, 21:00-08:00), nor on the weekday.
#[renox::test]
async fn no_step_of_the_auto_demo_runs_today() {
    let app = booted().await;
    host::auto_demo::seed(app.kernel()).await.unwrap();
    let midnight = wl_kernel::BusinessCalendar::default().from_local(
        wl_kernel::documents::today()
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default(),
    );
    let cutoff = midnight.format("%Y-%m-%d %H:%M:%S").to_string();
    for table in [
        "auto_job_cards",
        "auto_insurance_claims",
        "auto_fleet_contracts",
        "auto_supplier_claims",
        "auto_export_sales",
        "wl_notify_messages",
        "wl_ticketing_tickets",
        "wl_inventory_stock_movements",
    ] {
        let late = count(
            &app,
            &format!("select count(*) from {table} where created_at >= '{cutoff}'"),
        )
        .await;
        assert_eq!(late, 0, "{table} has rows made today");
    }
    // The vehicles are ready for collection and the owners were reminded.
    assert!(
        count(
            &app,
            "select count(*) from wl_notify_messages where type_key = 'auto.collect_vehicle'"
        )
        .await
            >= 1
    );
}

/// Every login opens its dashboard and the pages of its role: the lists and
/// the records (the lists of the claims, the contracts and the supplier claims
/// once failed in the host's layout, which has a `tabs` of its own).
#[renox::test]
async fn every_login_of_the_demo_company_opens_its_pages() {
    let app = booted().await;
    host::auto_demo::seed(app.kernel()).await.unwrap();
    let first = |table: &str| {
        let query = format!("select min(id) from {table}");
        let app = &app;
        async move { count(app, &query).await }
    };
    let card = first("auto_job_cards").await;
    let vehicle = first("auto_vehicles").await;
    let claim = first("auto_insurance_claims").await;
    let insurer = first("auto_insurers").await;
    let contract = first("auto_fleet_contracts").await;
    let supplier = first("auto_supplier_claims").await;
    let recall = first("wl_inventory_recalls").await;
    let waiting = count(
        &app,
        "select min(id) from auto_job_cards where status = 'awaiting_approval'",
    )
    .await;
    let pages_of = |email: &str| -> Vec<String> {
        let mut pages: Vec<String> = vec!["/dashboard".into()];
        let jobs = || {
            vec![
                "/auto/job-cards".to_owned(),
                format!("/auto/job-cards/{card}"),
                format!("/auto/job-cards/{waiting}"),
                format!("/auto/job-cards/{waiting}/estimate"),
                "/auto/vehicles".to_owned(),
                format!("/auto/vehicles/{vehicle}"),
            ]
        };
        match email {
            "admin@alnoorauto.example" => pages.extend([
                "/tickets".into(),
                "/invoices".into(),
                "/payments".into(),
                "/parties".into(),
            ]),
            "owner@alnoorauto.example" | "basma@alnoorauto.example" => {
                pages.extend(jobs());
                pages.push(format!("/auto/recalls/{recall}"));
                pages.push(format!("/recalls/{recall}"));
            }
            "sami@alnoorauto.example" => {
                pages.extend(jobs());
                pages.push("/auto/labour-menu".into());
                pages.push("/hr/leave".into());
            }
            "waleed@alnoorauto.example" => pages.extend(jobs()),
            "tareq@alnoorauto.example" | "nabil@alnoorauto.example" => {
                pages.push("/auto/technician".into());
            }
            "hussein@alnoorauto.example" => pages.push("/auto/vehicles".into()),
            "karim@alnoorauto.example" => {}
            "pia@alnoorauto.example" => pages.extend([
                "/auto/supplier-claims".into(),
                format!("/auto/supplier-claims/{supplier}"),
            ]),
            "carla@alnoorauto.example" => pages.extend([
                "/auto/claims".into(),
                format!("/auto/claims/{claim}"),
                "/auto/insurers".into(),
                format!("/auto/insurers/{insurer}"),
            ]),
            "fatima@alnoorauto.example" => {
                pages.extend(["/auto/fleet".into(), format!("/auto/fleet/{contract}")])
            }
            "aisha@alnoorauto.example" => pages.extend([
                "/auto/claims".into(),
                format!("/auto/claims/{claim}"),
                "/auto/export-sales".into(),
                "/invoices".into(),
                "/payments".into(),
                "/cash".into(),
            ]),
            _ => {}
        }
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

/// The estimate the customer has not answered is on a link that opens without
/// signing in; a job card delivered shows its invoice.
#[renox::test]
async fn the_customers_link_opens_without_signing_in() {
    let app = booted().await;
    host::auto_demo::seed(app.kernel()).await.unwrap();
    let token = renox::db::sql(
        "select token from auto_estimate_approvals where status = 'pending' order by id limit 1"
            .to_owned(),
    )
    .scalar::<String>(app.db())
    .await
    .unwrap();
    app.logout();
    let page = app.get(&format!("/auto/approve/{token}")).await;
    page.assert_ok();
    assert!(page.text().contains("Brake pads"), "the estimate is shown");
}

/// The login page of a local server lists the people of the company, one tap
/// each, once the demo is made.
#[renox::test]
async fn the_login_page_lists_the_people_of_the_demo_company() {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Auto vertical");
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
        .assert_dont_see("owner@alnoorauto.example");
    host::auto_demo::seed(app.kernel()).await.unwrap();
    let login = app.get("/login").await;
    for person in LOGINS {
        login.assert_see(&format!("data-demo-email=\"{}\"", person.email));
    }
}

/// #558: the insurance claims, fleet contracts and supplier claims lists
/// answered 500 in the demo (their link tabs were named like the kit's own
/// `tabs` tag). As the demo admin, each answers 200 and shows its status
/// filter and its rows.
#[renox::test]
async fn the_admin_opens_the_claims_fleet_and_supplier_claims_lists() {
    let app = booted().await;
    host::auto_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@alnoorauto.example")
        .await
        .unwrap()
        .expect("the demo admin");
    for (path, table) in [
        ("/auto/claims", "auto_insurance_claims"),
        ("/auto/fleet", "auto_fleet_contracts"),
        ("/auto/supplier-claims", "auto_supplier_claims"),
    ] {
        app.acting_as(&admin);
        let page = app.get(path).await;
        assert_eq!(page.status.as_u16(), 200, "{path} as the demo admin");
        let html = page.text();
        assert!(html.contains("rx-grid"), "{path} draws its grid");
        let first = count(&app, &format!("select min(id) from {table}")).await;
        assert!(
            html.contains(&format!("{path}/{first}")),
            "{path} lists its first row"
        );
    }
}
