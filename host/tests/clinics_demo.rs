//! The demo clinic of the Clinics client (`clinics:demo`): seeded on a test
//! database it has every state of the machines the spec asks for, seeding
//! again changes nothing, and each login opens its dashboard and the pages of
//! its role.

use host::clinics_demo::LOGINS;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the Clinics vertical.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-clinics/client.toml"
);

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client of the Clinics vertical");
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

/// The seed runs twice and the second run changes nothing; the stories of the
/// spec are all there: the bookings in every state, the visits and claims,
/// the insurance, the stock with its recall, the sales and the money.
#[renox::test]
async fn the_clinics_demo_has_every_state_and_is_seeded_once() {
    let app = booted().await;
    assert!(host::clinics_demo::seed(app.kernel()).await.unwrap());
    has_states(
        &app,
        "wl_scheduling_bookings",
        &[
            "booked",
            "confirmed",
            "checked_in",
            "in_progress",
            "completed",
            "cancelled",
            "no_show",
        ],
    )
    .await;
    has_states(
        &app,
        "clinics_visits",
        &["checked_in", "in_progress", "completed"],
    )
    .await;
    has_states(
        &app,
        "clinics_claims",
        &[
            "draft",
            "ready",
            "submitted",
            "paid",
            "denied",
            "written_off",
        ],
    )
    .await;
    has_states(
        &app,
        "clinics_prior_authorisations",
        &["requested", "approved"],
    )
    .await;
    has_states(&app, "wl_inventory_lots", &["released", "on_hold"]).await;
    has_states(&app, "wl_inventory_transfers", &["in_transit", "received"]).await;
    has_states(&app, "wl_hr_leave_requests", &["requested", "approved"]).await;
    has_states(&app, "wl_sales_packages", &["active"]).await;
    has_states(&app, "wl_parties_parties", &["blocked"]).await;
    assert!(count(&app, "select count(*) from wl_scheduling_bookings").await >= 25);
    assert!(count(&app, "select count(*) from wl_scheduling_conflicts").await >= 1);
    assert!(count(&app, "select count(*) from wl_scheduling_waiting_entries").await >= 1);
    assert_eq!(
        count(
            &app,
            "select count(*) from clinics_insurance_policies where valid_to < date('now')"
        )
        .await,
        1,
        "one policy has expired"
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_scheduling_bookings where deposit_outcome = 'forfeited'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_scheduling_bookings where deposit_outcome = 'refunded'"
        )
        .await,
        1
    );
    assert_eq!(
        count(&app, "select count(*) from wl_inventory_recalls").await,
        1
    );
    assert_eq!(
        count(&app, "select count(*) from clinics_recall_notices").await,
        1
    );
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_payments_payments where dispute_reference is not null"
        )
        .await,
        1
    );
    assert_eq!(
        count(&app, "select count(*) from wl_sales_loyalty_accounts").await,
        5
    );

    let tables = [
        "users",
        "wl_parties_parties",
        "wl_hr_employees",
        "wl_hr_qualifications",
        "wl_scheduling_bookings",
        "wl_scheduling_resources",
        "clinics_visits",
        "clinics_charge_lines",
        "clinics_claims",
        "clinics_insurance_policies",
        "clinics_payer_contracts",
        "wl_inventory_lots",
        "wl_inventory_stock_movements",
        "wl_sales_counter_sales",
        "wl_sales_loyalty_accounts",
        "wl_invoicing_invoices",
        "wl_payments_payments",
        "wl_ticketing_tickets",
    ];
    let mut before = Vec::new();
    for table in tables {
        before.push(count(&app, &format!("select count(*) from {table}")).await);
    }
    assert!(
        !host::clinics_demo::seed(app.kernel()).await.unwrap(),
        "the second run finds the clinic there"
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
async fn every_login_of_the_demo_clinic_opens_its_pages() {
    let app = booted().await;
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let booking = count(&app, "select min(id) from wl_scheduling_bookings").await;
    let visit = count(&app, "select min(id) from clinics_visits").await;
    let claim = count(&app, "select min(id) from clinics_claims").await;
    let authorisation = count(&app, "select min(id) from clinics_prior_authorisations").await;
    let recall = count(&app, "select min(id) from wl_inventory_recalls").await;
    let employee = count(&app, "select min(id) from wl_hr_employees").await;
    let invoice = count(&app, "select min(id) from wl_invoicing_invoices").await;

    let desk = || -> Vec<String> {
        vec![
            "/dashboard".into(),
            "/scheduling/bookings".into(),
            format!("/scheduling/bookings/{booking}"),
            "/clinics/appointments".into(),
            format!("/clinics/appointments/{booking}"),
            format!("/clinics/visits/{visit}"),
            "/clinics/policies".into(),
            "/clinics/call-list".into(),
            "/loyalty".into(),
        ]
    };
    let billing = || -> Vec<String> {
        vec![
            "/dashboard".into(),
            "/clinics/claims".into(),
            format!("/clinics/claims/{claim}"),
            "/clinics/authorisations".into(),
            format!("/clinics/authorisations/{authorisation}"),
            "/clinics/contracts".into(),
            "/payer-bills".into(),
        ]
    };
    let manager = || -> Vec<String> {
        let mut pages = desk();
        pages.extend([
            "/clinics/claims".into(),
            "/scheduling/conflicts".into(),
            format!("/clinics/recalls/{recall}"),
            "/clinics/recall-calls".into(),
        ]);
        pages
    };
    let pages_of = |email: &str| -> Vec<String> {
        match email {
            "admin@palmgrove.example" => {
                let mut pages = manager();
                pages.extend([
                    format!("/hr/employees/{employee}"),
                    "/pending-issues".into(),
                    "/counter-sales".into(),
                    "/parties".into(),
                    "/invoices".into(),
                    format!("/invoices/{invoice}"),
                    "/payments".into(),
                    "/campaigns".into(),
                ]);
                pages
            }
            "owner@palmgrove.example" => vec![
                "/dashboard".into(),
                "/clinics/campaign-permits".into(),
                format!("/clinics/recalls/{recall}"),
            ],
            "manager.jumeirah@palmgrove.example" | "manager.bay@palmgrove.example" => manager(),
            "reception.jumeirah@palmgrove.example" | "reception.bay@palmgrove.example" => {
                let mut pages = desk();
                pages.push("/counter-sales".into());
                pages
            }
            "nadia@palmgrove.example" | "samir@palmgrove.example" | "yara@palmgrove.example" => {
                vec![
                    "/dashboard".into(),
                    "/clinics/appointments".into(),
                    format!("/clinics/visits/{visit}"),
                ]
            }
            "nurse@palmgrove.example" => vec![
                "/dashboard".into(),
                "/clinics/appointments".into(),
                format!("/clinics/visits/{visit}"),
            ],
            "store@palmgrove.example" => vec!["/dashboard".into(), "/pending-issues".into()],
            "billing@palmgrove.example" => billing(),
            "callcentre@palmgrove.example" => vec![
                "/dashboard".into(),
                "/scheduling/bookings".into(),
                "/clinics/appointments".into(),
                "/clinics/call-list".into(),
            ],
            "accountant@palmgrove.example" => vec![
                "/dashboard".into(),
                "/invoices".into(),
                format!("/invoices/{invoice}"),
                "/payments".into(),
                "/refunds".into(),
            ],
            "marketing@palmgrove.example" => {
                vec!["/dashboard".into(), "/campaigns".into(), "/segments".into()]
            }
            other => panic!("the login {other} has no pages"),
        }
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

/// A service shows by its name, not by its code, on the appointments, the
/// visit, the claim and the payer's contract (`dental_checkup` reads *Dental
/// check-up*), and the history of a visit starts with its check-in.
#[renox::test]
async fn the_pages_name_a_service_in_words() {
    let app = booted().await;
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@palmgrove.example")
        .await
        .unwrap()
        .expect("the demo administrator");
    app.acting_as(&admin);
    let visit = count(&app, "select min(id) from clinics_visits").await;
    let claim = count(&app, "select min(id) from clinics_claims").await;
    let codes = [
        "dental_checkup",
        "dental_xray",
        "follow_up",
        "physio_session",
    ];
    for path in [
        "/clinics/appointments".to_owned(),
        "/clinics/contracts".to_owned(),
        format!("/clinics/visits/{visit}"),
        format!("/clinics/claims/{claim}"),
    ] {
        let page = app.get(&path).await;
        page.assert_ok();
        for code in codes {
            page.assert_dont_see(code);
        }
    }
    app.get("/clinics/appointments")
        .await
        .assert_see("Dental check-up")
        .assert_see("Follow-up visit");
    // The prior authorisations list a service by name with the code its forms
    // take, and its page says Services, not "Service codes, comma separated".
    let authorisation = count(&app, "select min(id) from clinics_prior_authorisations").await;
    app.get("/clinics/authorisations")
        .await
        .assert_ok()
        .assert_see("Procedure (procedure)");
    app.get(&format!("/clinics/authorisations/{authorisation}"))
        .await
        .assert_ok()
        .assert_see("Procedure (procedure)")
        .assert_dont_see("comma separated");
    app.get(&format!("/clinics/visits/{visit}"))
        .await
        .assert_see("Checked in")
        .assert_dont_see("clinics.visits.transition");
    // A bill names the visit it comes from (`VIS-0009`), not `clinics.visit:9`.
    let bill = count(&app, "select min(id) from wl_invoicing_payer_bills").await;
    app.get("/payer-bills")
        .await
        .assert_ok()
        .assert_see("VIS-")
        .assert_dont_see("clinics.visit:");
    app.get(&format!("/payer-bills/{bill}"))
        .await
        .assert_ok()
        .assert_dont_see("clinics.visit:");
}

/// A branch is named (*Business Bay*), never shown as its scope key
/// (`clinics.branch 2`), on the recall's trace and the recall calls.
#[renox::test]
async fn the_pages_name_a_branch_in_words() {
    let app = booted().await;
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@palmgrove.example")
        .await
        .unwrap()
        .expect("the demo administrator");
    app.acting_as(&admin);
    let recall = count(&app, "select min(id) from wl_inventory_recalls").await;
    app.get(&format!("/clinics/recalls/{recall}"))
        .await
        .assert_ok()
        .assert_dont_see("clinics.branch")
        .assert_see("Business Bay");
    app.get("/clinics/recall-calls")
        .await
        .assert_ok()
        .assert_dont_see("clinics.branch");
}

/// The advertising permits name their campaign (code and name), not its
/// number.
#[renox::test]
async fn the_permits_name_their_campaign() {
    let app = booted().await;
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let owner = User::find_by_email(app.db(), "owner@palmgrove.example")
        .await
        .unwrap()
        .expect("the demo owner");
    app.acting_as(&owner);
    let campaign = renox::db::sql(
        "select code || ' · ' || name from wl_sales_campaigns where id = (select min(campaign_id) from clinics_advertising_permits)"
            .to_owned(),
    )
    .scalar::<String>(app.db())
    .await
    .unwrap();
    app.get("/clinics/campaign-permits")
        .await
        .assert_ok()
        .assert_see(&campaign);
}

/// A ticket names the record it is about: the claim by its number
/// (`CLM-0004`), never `clinics.claim #4`, on its page and in the list.
#[renox::test]
async fn the_tickets_name_the_record_they_are_about() {
    let app = booted().await;
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@palmgrove.example")
        .await
        .unwrap()
        .expect("the demo administrator");
    app.acting_as(&admin);
    let ticket = count(
        &app,
        "select min(id) from wl_ticketing_tickets where subject_type = 'clinics.claim'",
    )
    .await;
    assert!(ticket > 0, "the demo has a ticket about a claim");
    let claim = count(
        &app,
        &format!("select subject_id from wl_ticketing_tickets where id = {ticket}"),
    )
    .await;
    app.get(&format!("/tickets/{ticket}"))
        .await
        .assert_ok()
        .assert_see(&format!("CLM-{claim:04}"))
        .assert_dont_see("clinics.claim");
    app.get("/tickets")
        .await
        .assert_ok()
        .assert_dont_see("clinics.claim");
    // The stock shortage notice names the visit by its number.
    app.get("/notifications")
        .await
        .assert_ok()
        .assert_dont_see("clinics.visit:");
}

/// The money pages name the documents they link: an invoice shows the receipt
/// that paid it (`RCT-2026-0011`), a payment the invoice it paid, never
/// `payments.payment:11` or `invoicing.invoice:6`.
#[renox::test]
async fn the_money_pages_name_the_documents_they_link() {
    let app = booted().await;
    host::clinics_demo::seed(app.kernel()).await.unwrap();
    let admin = User::find_by_email(app.db(), "admin@palmgrove.example")
        .await
        .unwrap()
        .expect("the demo administrator");
    app.acting_as(&admin);
    let invoice = count(
        &app,
        "select min(invoice_id) from wl_invoicing_allocations where removed_at is null",
    )
    .await;
    assert!(invoice > 0, "the demo has an allocated invoice");
    app.get(&format!("/invoices/{invoice}"))
        .await
        .assert_ok()
        .assert_see("RCT-")
        .assert_dont_see("payments.payment:");
    let payment = count(
        &app,
        "select min(payment_id) from wl_payments_allocations where removed_at is null",
    )
    .await;
    app.get(&format!("/payments/{payment}"))
        .await
        .assert_ok()
        .assert_dont_see("invoicing.invoice:");
}
