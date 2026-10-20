//! The demo company (`property:demo`): seeded on a test database it has every
//! state of the state machines the spec asks for, at least twice, and seeding
//! again changes nothing.

use renox::testing::TestApp;
use std::collections::BTreeMap;

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

#[renox::test]
async fn the_demo_company_has_every_state_and_is_seeded_once() {
    let app = TestApp::new(host::app()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();

    let started = std::time::Instant::now();
    let report = host::demo_data::seed(app.kernel()).await.unwrap();
    eprintln!("demo seeded in {:?}", started.elapsed());
    assert!(!report.already);

    has_states(
        &app,
        "property_leases",
        &[
            "draft",
            "pending_signature",
            "active",
            "renewal_due",
            "notice_given",
            "terminated",
            "cancelled",
            "expired",
            "renewed",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "property_units",
        &[
            "available",
            "occupied",
            "notice_given",
            "under_maintenance",
            "off_market",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "property_management_agreements",
        &["draft", "active", "ending", "ended"],
        2,
    )
    .await;
    has_states(
        &app,
        "property_owner_statements",
        &["draft", "approved", "published", "settled", "superseded"],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_payments_cheques",
        &["received", "cleared", "returned", "legal_case", "replaced"],
        2,
    )
    .await;
    has_states(&app, "wl_payments_cheques", &["bounced"], 1).await;
    has_states(
        &app,
        "wl_payments_deposits",
        &[
            "held",
            "refunded",
            "forfeited",
            "transferred",
            "settlement_pending",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_payments_refunds",
        &["requested", "approved", "rejected"],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_invoicing_invoices",
        &[
            "draft",
            "issued",
            "paid",
            "overdue",
            "written_off",
            "credited",
        ],
        2,
    )
    .await;
    has_states(&app, "wl_invoicing_invoices", &["partially_paid"], 1).await;
    has_states(&app, "wl_invoicing_credit_notes", &["applied"], 2).await;
    has_states(
        &app,
        "wl_invoicing_supplier_bills",
        &[
            "draft",
            "matched",
            "approved",
            "paid",
            "partially_paid",
            "exception",
            "void",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_ticketing_tickets",
        &[
            "open",
            "assigned",
            "in_progress",
            "on_hold",
            "resolved",
            "closed",
            "cancelled",
        ],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_documents_documents",
        &["accepted", "pending_review", "rejected", "superseded"],
        2,
    )
    .await;
    has_states(
        &app,
        "wl_sales_leads",
        &["new", "contacted", "qualified", "won", "lost"],
        2,
    )
    .await;
    has_states(&app, "wl_sales_campaigns", &["draft", "active", "ended"], 2).await;
    has_states(&app, "wl_hr_employees", &["onboarding", "active"], 2).await;
    assert!(total(&app, "wl_payments_payments").await >= 100);
    assert!(total(&app, "property_buildings").await >= 5);

    // Seeding again changes nothing.
    let leases = total(&app, "property_leases").await;
    let invoices = total(&app, "wl_invoicing_invoices").await;
    let parties = total(&app, "wl_parties_parties").await;
    let again = host::demo_data::seed(app.kernel()).await.unwrap();
    assert!(again.already);
    assert_eq!(total(&app, "property_leases").await, leases);
    assert_eq!(total(&app, "wl_invoicing_invoices").await, invoices);
    assert_eq!(total(&app, "wl_parties_parties").await, parties);
}
