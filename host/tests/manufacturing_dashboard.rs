//! The dashboard of the Manufacturing demo company (BL-MFG-14, SCREENS-BRIEF):
//! the people of the plant see the figures they can act on, each linking to
//! its page, and a person with fewer permissions sees fewer.

mod manufacturing_demo_support;
mod support;

use host::manufacturing_demo::{
    ACCOUNTANT_EMAIL, BUYER_EMAIL, MANAGER_EMAIL, OPERATOR_EMAIL, OWNER_EMAIL, PLANNER_EMAIL,
    QC_EMAIL, SALES_EMAIL, STORE_EMAIL,
};
use manufacturing_demo_support::demo;
use renox::prelude::*;
use renox::testing::TestApp;

/// The text of the figure under `label` on the page, and its link.
fn tile(html: &str, label: &str) -> Option<(String, String)> {
    let at = html.find(&format!("<p class=\"rx-stat__label\">{label}</p>"))?;
    let before = &html[..at];
    let link = before.rfind("href=\"")?;
    let href = before[link + 6..].split('"').next()?.to_owned();
    let after = &html[at..];
    let value = after.split("rx-stat__value\">").nth(1)?;
    let value = value.split("</p>").next()?.trim().to_owned();
    Some((value, href))
}

async fn dashboard(app: &TestApp, email: &str) -> String {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
    app.request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get("/dashboard")
        .await
        .assert_ok()
        .text()
}

fn assert_tile(html: &str, label: &str, link: &str) {
    let (value, href) = tile(html, label).unwrap_or_else(|| panic!("no tile `{label}`"));
    assert!(
        value.parse::<u32>().is_ok_and(|n| n >= 1),
        "{label}: {value}"
    );
    assert_eq!(href, link, "{label}");
}

/// The production manager sees the work orders by state, the open NCRs, the
/// returned goods and the certificate that ends within 30 days, each linking
/// to its list.
#[renox::test]
async fn the_production_manager_sees_the_plant_tiles_each_linking_to_its_page() {
    let app = demo().await;
    let html = dashboard(&app, MANAGER_EMAIL).await;
    assert_tile(
        &html,
        "Work orders in progress",
        "/manufacturing/work-orders?status=in_progress",
    );
    assert_tile(
        &html,
        "Work order on hold",
        "/manufacturing/work-orders?status=on_hold",
    );
    assert_tile(
        &html,
        "Work order to release",
        "/manufacturing/work-orders?status=planned",
    );
    assert_tile(&html, "Open non-conformance", "/manufacturing/ncrs");
    assert_tile(&html, "Returned line to decide", "/manufacturing/returns");
    assert_tile(
        &html,
        "Certificate ending within 30 days",
        "/manufacturing/conformity",
    );
    assert!(html.contains("What needs your action in the plant today."));
}

/// QC sees the NCRs and the lots that came back; not the receivables.
#[renox::test]
async fn qc_sees_the_ncrs_and_the_returned_lots() {
    let app = demo().await;
    let html = dashboard(&app, QC_EMAIL).await;
    assert_tile(&html, "Open non-conformance", "/manufacturing/ncrs");
    assert_tile(
        &html,
        "Returned lot to inspect",
        "/manufacturing/inspections",
    );
    assert!(tile(&html, "Orders on credit hold").is_none());
    assert!(tile(&html, "Overdue invoices").is_none());
}

/// Sales sees what to decide for the customers: the returned line and the
/// order on credit hold.
#[renox::test]
async fn sales_sees_the_returned_line_and_the_order_on_credit_hold() {
    let app = demo().await;
    let html = dashboard(&app, SALES_EMAIL).await;
    assert_tile(&html, "Returned line to decide", "/manufacturing/returns");
    assert_tile(
        &html,
        "Order on credit hold",
        "/sales-orders?in.status=credit_hold",
    );
    assert!(tile(&html, "Open non-conformance").is_none());
}

/// The accountant sees the overdue invoices and the export without evidence.
#[renox::test]
async fn the_accountant_sees_the_overdue_invoices_and_the_export_without_evidence() {
    let app = demo().await;
    let html = dashboard(&app, ACCOUNTANT_EMAIL).await;
    let (balance, link) = tile(&html, "Overdue invoices").expect("the overdue invoices");
    assert_eq!(
        (balance.as_str(), link.as_str()),
        ("AED 44,100.00", "/invoices?in.status=overdue")
    );
    assert_tile(
        &html,
        "Export without exit evidence",
        "/manufacturing/exports",
    );
    assert!(tile(&html, "Work orders in progress").is_none());
}

/// The planner sees the work orders to release; the owner the recall.
#[renox::test]
async fn the_planner_and_the_owner_see_their_figures() {
    let app = demo().await;
    let html = dashboard(&app, PLANNER_EMAIL).await;
    assert_tile(
        &html,
        "Work order to release",
        "/manufacturing/work-orders?status=planned",
    );
    let html = dashboard(&app, OWNER_EMAIL).await;
    assert_tile(&html, "Open recall", "/recalls");
}

/// The storekeeper, the operator and the customer's buyer open the page
/// without a tile that is not theirs.
#[renox::test]
async fn others_see_no_tile_that_is_not_theirs() {
    let app = demo().await;
    for email in [STORE_EMAIL, OPERATOR_EMAIL, BUYER_EMAIL] {
        let html = dashboard(&app, email).await;
        assert!(tile(&html, "Open non-conformance").is_none(), "{email}");
        assert!(tile(&html, "Order on credit hold").is_none(), "{email}");
        assert!(tile(&html, "Overdue invoices").is_none(), "{email}");
    }
}
