//! The demo company of the Manufacturing client (`manufacturing:demo`, F7):
//! seeded on a test database it has the states the spec asks for (work orders
//! planned, running, held, closed and waiting to be aborted; NCRs; a recall;
//! dispatches that are drafts, picked, out and exports; customers with credit
//! accounts, one on hold; conformity documents; export evidence; returned
//! goods), seeding again changes nothing, and every person of the demo logs
//! in.

mod manufacturing_demo_support;
mod support;

use host::manufacturing_demo::{LOGINS, is_seeded, seed};
use renox::prelude::*;
use renox::testing::TestApp;

use manufacturing_demo_support::demo;

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// The seed runs and marks itself done; a second run changes nothing.
#[renox::test]
async fn the_demo_company_is_seeded_once() {
    let app = demo().await;
    assert!(is_seeded(app.db()).await.unwrap());
    assert!(
        !seed(app.kernel()).await.unwrap(),
        "a second run is a no-op"
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

/// The states of the spec are all there.
#[renox::test]
async fn the_demo_company_has_every_state_the_screens_show() {
    let app = demo().await;
    has_states(
        &app,
        "manufacturing_work_orders",
        &["planned", "in_progress", "on_hold", "closed"],
    )
    .await;
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_approval_requests where action = 'manufacturing.abort' and status = 'pending'"
        )
        .await,
        1,
        "an abort waits for the production manager"
    );
    assert!(
        count(
            &app,
            "select count(*) from manufacturing_late_reports where late = 1"
        )
        .await
            >= 1,
        "a paper job card was entered late"
    );
    has_states(&app, "manufacturing_ncrs", &["open"]).await;
    assert!(count(&app, "select count(*) from manufacturing_ncrs").await >= 2);
    has_states(
        &app,
        "wl_inventory_dispatches",
        &["draft", "picked", "dispatched"],
    )
    .await;
    has_states(&app, "wl_sales_sales_orders", &["credit_hold", "confirmed"]).await;
    assert!(count(&app, "select count(*) from wl_inventory_recalls").await >= 1);
    assert!(
        count(
            &app,
            "select count(*) from manufacturing_return_inspections"
        )
        .await
            >= 1,
        "refused goods wait for QC"
    );
    assert!(
        count(&app, "select count(*) from manufacturing_return_decisions").await >= 1,
        "and sales decides what happens to the line"
    );
    assert!(
        count(
            &app,
            "select count(*) from manufacturing_item_flags where customer_specific = 1"
        )
        .await
            >= 1
    );
    assert!(
        count(
            &app,
            "select count(*) from manufacturing_item_flags where regulated = 1"
        )
        .await
            >= 2
    );
    assert!(
        count(
            &app,
            "select count(*) from manufacturing_item_flags where export_only = 1"
        )
        .await
            >= 1
    );
    assert!(count(&app, "select count(*) from manufacturing_export_watches").await >= 1);
    assert!(
        count(
            &app,
            "select count(*) from wl_invoicing_invoices where status = 'overdue'"
        )
        .await
            >= 1
    );
    assert!(count(&app, "select count(*) from wl_sales_customer_accounts").await >= 5);
    assert!(count(&app, "select count(*) from wl_portal_access_grants").await >= 1);
}
