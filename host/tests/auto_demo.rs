//! The demo company of the Auto client (`auto:demo`, F8): seeded on a test
//! database it has the states the screens show (job cards from checked in to
//! delivered, declined and waiting for parts; insurance claims approved and
//! with the insurer; fleet contracts active and in draft; counter sales; an
//! overdue B2B invoice; an export waiting for its evidence; a recall traced
//! to a customer), seeding again changes nothing, and every person of the
//! demo has a login.

mod auto_demo_support;
mod support;

use auto_demo_support::demo;
use host::auto_demo::{LOGINS, is_seeded, seed};
use renox::prelude::*;
use renox::testing::TestApp;

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
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

/// The seed runs and marks itself done; a second run changes nothing.
#[renox::test]
async fn the_demo_company_is_seeded_once() {
    let app = demo().await;
    assert!(is_seeded(app.db()).await.unwrap());
    let jobs = count(&app, "select count(*) from auto_job_cards").await;
    assert!(
        !seed(app.kernel()).await.unwrap(),
        "a second run is a no-op"
    );
    assert_eq!(
        count(&app, "select count(*) from auto_job_cards").await,
        jobs
    );
    for login in LOGINS {
        assert!(
            User::find_by_email(app.db(), login.email)
                .await
                .unwrap()
                .is_some(),
            "{}",
            login.email
        );
    }
}

/// The states of the screens are all there.
#[renox::test]
async fn the_demo_company_has_every_state_the_screens_show() {
    let app = demo().await;
    has_states(
        &app,
        "auto_job_cards",
        &[
            "checked_in",
            "inspecting",
            "awaiting_approval",
            "in_progress",
            "awaiting_parts",
            "quality_check",
            "ready",
            "delivered",
            "declined",
        ],
    )
    .await;
    has_states(
        &app,
        "auto_insurance_claims",
        &["approved", "estimate_submitted"],
    )
    .await;
    has_states(&app, "auto_fleet_contracts", &["active", "draft"]).await;
    has_states(&app, "wl_invoicing_invoices", &["issued", "overdue"]).await;
    // Every job card came from a booking of the workshop's bay.
    assert_eq!(
        count(
            &app,
            "select count(*) from auto_job_cards where booking_id is null or workshop_id is null"
        )
        .await,
        0
    );
    // The delivered job card raised its invoice; parts left the store.
    assert_eq!(
        count(
            &app,
            "select count(*) from auto_job_cards where status = 'delivered' and invoice_id is not null"
        )
        .await,
        1
    );
    assert!(count(&app, "select count(*) from wl_sales_counter_sales").await >= 3);
    assert_eq!(
        count(&app, "select count(*) from auto_export_sales").await,
        1
    );
    assert_eq!(
        count(&app, "select count(*) from wl_inventory_recalls").await,
        1
    );
}
