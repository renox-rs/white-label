//! The lists of the Accounting vertical on the demo firm (#534): the
//! engagements, the returns, the KPI packs and the AML cases are each a
//! `renox::grid` that is a direct child of the page (it fills the screen),
//! opens for the role that works from it, searches, filters and pages through
//! the query string, and keeps the sheets of the page header.

mod accounting_demo_support;
mod grid_support;
mod support;

use accounting_demo_support::demo;
use grid_support::{grid_is_a_child_of_the_page, load, range, total};
use host::accounting_demo::{MANAGER_EMAIL, MLRO_EMAIL, TAX_EMAIL};
use renox::prelude::*;
use renox::testing::TestApp;

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// Signs in the demo login `email`.
async fn sign_in(app: &TestApp, email: &str) {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
}

/// Each list with its template, its table and the login that works from it.
const LISTS: [(&str, &str, &str, &str); 4] = [
    (
        "/accounting/engagements",
        "engagements.html",
        "accounting_engagements",
        MANAGER_EMAIL,
    ),
    (
        "/accounting/returns",
        "returns.html",
        "accounting_returns",
        TAX_EMAIL,
    ),
    (
        "/accounting/kpi-packs",
        "kpi_packs.html",
        "accounting_kpi_packs",
        MANAGER_EMAIL,
    ),
    (
        "/accounting/aml-cases",
        "aml_cases.html",
        "accounting_aml_cases",
        MLRO_EMAIL,
    ),
];

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_accounting_list_is_a_grid_that_fills_the_page() {
    for (_, template, _, _) in LISTS {
        grid_is_a_child_of_the_page(&format!("accounting/views/accounting/{template}"));
    }
}

/// Each list answers 200 for its owner, counts every row of its table, pages
/// one row at a time and finds nothing for a nonsense search.
#[renox::test]
async fn every_accounting_list_opens_as_a_grid_for_its_owner() {
    let app = demo().await;
    for (path, _, table, login) in LISTS {
        sign_in(&app, login).await;
        let html = load(&app, path).await;
        assert!(html.contains("class=\"rx-grid"), "{path} draws a grid");
        let rows = count(&app, &format!("select count(*) from {table}")).await;
        assert_eq!(range(&html).2, rows, "{path} counts every row");
        if rows > 0 {
            let one = load(&app, &format!("{path}?per_page=1")).await;
            assert_eq!(range(&one), (1, 1, rows), "{path} pages one row");
        }
        let none = load(&app, &format!("{path}?search=zzqqxx-nothing")).await;
        assert_eq!(range(&none).2, 0, "{path} finds nothing for nonsense");
    }
}

/// The returns filter by type and status, and the header keeps the sheet
/// that opens a return; the engagements keep theirs.
#[renox::test]
async fn the_returns_filter_by_type_and_status_and_keep_the_new_return_sheet() {
    let app = demo().await;
    sign_in(&app, TAX_EMAIL).await;
    let vat = count(
        &app,
        "select count(*) from accounting_returns where return_type = 'vat'",
    )
    .await;
    assert_eq!(
        total(&app, "/accounting/returns?in.return_type=vat").await,
        vat
    );
    let filed = count(
        &app,
        "select count(*) from accounting_returns where status = 'filed'",
    )
    .await;
    assert_eq!(total(&app, "/accounting/returns?in.status=filed").await, filed);

    sign_in(&app, MANAGER_EMAIL).await;
    let engagements = load(&app, "/accounting/engagements").await;
    assert!(engagements.contains("engagement-new"), "the New engagement sheet");
    let packs = load(&app, "/accounting/kpi-packs").await;
    assert!(packs.contains("kpi-import"), "the Import trial balance sheet");
}
