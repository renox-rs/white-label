//! The demo firm of the Accounting client (`accounting:demo`, F4): seeded on
//! a test database it has the states the screens show (engagements active,
//! onboarding, on hold, ended and declined; CDD files approved, pending and
//! due for review; returns preparing, in review, with the client, filed,
//! superseded and cancelled; AML cases under review and closed; KPI packs
//! published and in draft; retainer invoices issued and overdue), seeding
//! again changes nothing, and every person of the demo has a login.

mod accounting_demo_support;
mod support;

use accounting_demo_support::demo;
use host::accounting_demo::{LOGINS, is_seeded, seed};
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
async fn the_demo_firm_is_seeded_once() {
    let app = demo().await;
    assert!(is_seeded(app.db()).await.unwrap());
    let engagements = count(&app, "select count(*) from accounting_engagements").await;
    assert!(
        !seed(app.kernel()).await.unwrap(),
        "a second run is a no-op"
    );
    assert_eq!(
        count(&app, "select count(*) from accounting_engagements").await,
        engagements
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
async fn the_demo_firm_has_every_state_the_screens_show() {
    let app = demo().await;
    has_states(
        &app,
        "accounting_engagements",
        &["active", "onboarding", "suspended", "ended", "declined"],
    )
    .await;
    has_states(
        &app,
        "accounting_cdd_files",
        &["approved", "pending_approval", "review_due"],
    )
    .await;
    has_states(
        &app,
        "accounting_returns",
        &[
            "preparing",
            "in_review",
            "client_approval",
            "filed",
            "superseded",
            "cancelled",
        ],
    )
    .await;
    has_states(
        &app,
        "accounting_aml_cases",
        &["under_review", "closed_no_action"],
    )
    .await;
    has_states(&app, "accounting_kpi_packs", &["published", "draft"]).await;
    has_states(&app, "wl_invoicing_invoices", &["issued", "overdue"]).await;
    // The activations laid out the compliance calendar.
    assert!(count(&app, "select count(*) from wl_ticketing_deadline_tasks").await >= 10);
    // A high-risk client with enhanced due diligence, and a possible match
    // the MLRO has not decided.
    assert_eq!(
        count(
            &app,
            "select count(*) from accounting_cdd_files where risk_rating = 'high'"
        )
        .await,
        1
    );
    assert!(
        count(
            &app,
            "select count(*) from wl_parties_screenings where status = 'possible_match'"
        )
        .await
            >= 1
    );
    // The partner decides a tax profile change.
    assert_eq!(
        count(
            &app,
            "select count(*) from wl_approval_requests where status = 'pending'"
        )
        .await,
        1
    );
}
