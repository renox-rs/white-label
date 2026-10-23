//! The pages of F7 as the people of the demo company use them (BL-MFG-14,
//! BL-MFG-07, BL-MFG-03): the work order with the paper job card entered late,
//! the returned goods and the inspection, the conformity flags, the abort that
//! waits for the production manager and the recall.

mod manufacturing_demo_support;
mod support;

use host::manufacturing_demo::{
    MANAGER_EMAIL, OPERATOR_EMAIL, PLANNER_EMAIL, QC_EMAIL, SALES_EMAIL, SALES_MANAGER_EMAIL,
    SUPERVISOR_EMAIL,
};
use manufacturing_demo_support::demo;
use renox::prelude::*;
use renox::testing::{TestApp, TestResponse};

async fn as_user(app: &TestApp, email: &str) -> User {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
    user
}

async fn browse(app: &TestApp, email: &str, path: &str) -> TestResponse {
    as_user(app, email).await;
    app.request()
        .header("accept", "text/html,application/xhtml+xml,*/*;q=0.8")
        .get(path)
        .await
}

async fn post(app: &TestApp, email: &str, path: &str, form: &[(&str, &str)]) -> TestResponse {
    as_user(app, email).await;
    app.htmx().post(path, form).await
}

async fn scalar(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

/// BL-MFG-14 surface "Field PWA": the work order of Al Noor's order shows the
/// reports of the day shift and the paper job card of the night shift with the
/// late-entry flag and its reason, at the business time of the output.
#[renox::test]
async fn the_work_order_shows_the_paper_job_card_with_its_late_entry_flag() {
    let app = demo().await;
    let id = scalar(
        &app,
        "select id from manufacturing_work_orders where sales_order_id is not null and status = 'in_progress' order by id",
    )
    .await;
    let page = browse(
        &app,
        SUPERVISOR_EMAIL,
        &format!("/manufacturing/work-orders/{id}"),
    )
    .await;
    page.assert_ok()
        .assert_see("Entered late: Line tablet offline during the night shift")
        .assert_see("Night shift, paper job card")
        .assert_see("Day shift");
    let at = scalar(
        &app,
        "select count(*) from manufacturing_production_reports r join manufacturing_late_reports l on l.report_id = r.id where r.reported_at < datetime('now', '-8 hours')",
    )
    .await;
    assert_eq!(
        at, 1,
        "the report keeps the business time of the night shift"
    );
}

/// The operator of line 1 cannot enter a late paper job card on line 2's work
/// order; the supervisor of both lines can.
#[renox::test]
async fn only_the_people_of_the_line_enter_its_job_cards() {
    let app = demo().await;
    let id = scalar(
        &app,
        "select id from manufacturing_work_orders where line_id = (select id from manufacturing_lines where code = 'L2') and status = 'in_progress' order by id",
    )
    .await;
    post(
        &app,
        OPERATOR_EMAIL,
        &format!("/manufacturing/work-orders/{id}/report"),
        &[("good", "10")],
    )
    .await
    .assert_forbidden();
    post(
        &app,
        SUPERVISOR_EMAIL,
        &format!("/manufacturing/work-orders/{id}/report"),
        &[("good", "10")],
    )
    .await
    .assert_status(303);
}

/// BL-MFG-07 on the pages: sales sees the returned line of Al Noor's order and
/// makes the new dispatch; QC sees the returned lot and passes it, which
/// releases the lot.
#[renox::test]
async fn sales_and_qc_work_the_returned_goods() {
    let app = demo().await;
    let page = browse(&app, SALES_EMAIL, "/manufacturing/returns").await;
    page.assert_ok()
        .assert_see("damaged in transit")
        .assert_see("New dispatch");
    let decision = scalar(
        &app,
        "select id from manufacturing_return_decisions order by id",
    )
    .await;
    let before = scalar(&app, "select count(*) from wl_inventory_dispatches").await;
    post(
        &app,
        SALES_EMAIL,
        &format!("/manufacturing/returns/{decision}/dispatch"),
        &[("quantity", "20")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        scalar(&app, "select count(*) from wl_inventory_dispatches").await,
        before + 1,
        "a draft dispatch for the open quantity"
    );

    browse(&app, QC_EMAIL, "/manufacturing/inspections")
        .await
        .assert_ok()
        .assert_see("damaged in transit")
        .assert_see("Pass");
    let inspection = scalar(
        &app,
        "select id from manufacturing_return_inspections order by id",
    )
    .await;
    post(
        &app,
        QC_EMAIL,
        &format!("/manufacturing/inspections/{inspection}/pass"),
        &[("note", "Packaging damaged, boxes fine")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        scalar(
            &app,
            "select count(*) from manufacturing_return_inspections where status = 'passed'"
        )
        .await,
        1
    );
    // Sales may not record the result.
    post(
        &app,
        SALES_EMAIL,
        &format!("/manufacturing/inspections/{inspection}/pass"),
        &[],
    )
    .await
    .assert_forbidden();
}

/// The conformity page lists the regulated items with their certificates, the
/// export-only item and the item made for one customer (BL-MFG-14).
#[renox::test]
async fn the_conformity_page_lists_the_flags_and_certificates() {
    let app = demo().await;
    let page = browse(&app, MANAGER_EMAIL, "/manufacturing/conformity").await;
    page.assert_ok()
        .assert_see("BOX-FOOD")
        .assert_see("BOX-RTL")
        .assert_see("SLV-EPH")
        .assert_see("BOX-EXP")
        .assert_see("Made for one customer");
}

/// BL-MFG-01 on the pages: the abort the planner asked for waits; the
/// production manager approves it in the approvals inbox and the work order is
/// aborted. The planner cannot approve their own request.
#[renox::test]
async fn the_production_manager_approves_the_abort_the_planner_asked_for() {
    let app = demo().await;
    let request = scalar(
        &app,
        "select id from wl_approval_requests where action = 'manufacturing.abort' and status = 'pending'",
    )
    .await;
    let planner = as_user(&app, PLANNER_EMAIL).await;
    let own = wl_kernel::approvals::approve(app.state(), request, &planner, "").await;
    assert!(own.is_err(), "the requester cannot approve");
    let manager = as_user(&app, MANAGER_EMAIL).await;
    wl_kernel::approvals::approve(app.state(), request, &manager, "agreed")
        .await
        .unwrap();
    assert_eq!(
        scalar(
            &app,
            "select count(*) from manufacturing_work_orders where status = 'aborted'"
        )
        .await,
        1
    );
}

/// The recall of the liner lot reaches the work order and the page of the
/// recall lists it for the owner; sales sees the credit hold of Sahara.
#[renox::test]
async fn the_recall_and_the_credit_hold_show_on_their_pages() {
    let app = demo().await;
    let recall = scalar(&app, "select id from wl_inventory_recalls order by id").await;
    browse(
        &app,
        "owner@emiratespack.example",
        &format!("/recalls/{recall}"),
    )
    .await
    .assert_ok()
    .assert_see("KL-2291");
    browse(
        &app,
        "owner@emiratespack.example",
        &format!("/manufacturing/recalls/{recall}"),
    )
    .await
    .assert_ok();
    let held = scalar(
        &app,
        "select id from wl_sales_sales_orders where status = 'credit_hold'",
    )
    .await;
    browse(&app, SALES_MANAGER_EMAIL, &format!("/sales-orders/{held}"))
        .await
        .assert_ok()
        .assert_see("Sahara Retail");
}
