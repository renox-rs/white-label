//! BL-MFG-03 (#384) and BL-MFG-14 (#395) with the real wl-sales: the work
//! orders follow an amended order (`MFG.RUL.09`), the cancel sheets propose
//! the outcome the item's customer-specific flag asks for, and blocking a
//! customer holds its orders that are not dispatched while its work orders
//! continue (07 §7.1 *Party blocked*). The orders are entered and amended
//! through the pages of wl-sales, the work orders are fixtures linked to the
//! lines, the events, the listeners and the pages are the real ones.

mod support;

use host::roles::{PRODUCTION_MANAGER, SALES, SALES_MANAGER};
use manufacturing::{WorkOrder, WorkOrderSource, WorkOrderStatus};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_sales::{SaleLine, SalesOrder, SalesOrderStatus};

use support::{block, company, manufacturing_booted, person};

/// A confirmed order of `customer` for `quantity` of item `code` entered by
/// the sales person, and its line.
async fn place(
    app: &TestApp,
    sales: &User,
    customer: wl_kernel::PartyId,
    code: &str,
    quantity: &str,
) -> (SalesOrder, SaleLine) {
    app.acting_as(sales);
    app.htmx()
        .post(
            "/sales-orders",
            &[
                ("party_id", &customer.0.to_string()),
                ("channel", "phone"),
                ("payment_mode", "on_account"),
            ],
        )
        .await
        .assert_status(303);
    let order = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(app.db())
        .await
        .unwrap()
        .expect("the order");
    app.htmx()
        .post(
            &format!("/sales-orders/{}/lines", order.id),
            &[
                ("item_code", code),
                ("quantity", quantity),
                ("unit_price", "10.00"),
                ("vat", "standard"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
        .assert_status(303);
    let mut order = SalesOrder::find_or_404(app.db(), order.id).await.unwrap();
    if order.status == SalesOrderStatus::PendingAcceptance {
        app.htmx()
            .post(
                &format!("/sales-orders/{}/transitions/accept", order.id),
                &[("x", "")],
            )
            .await
            .assert_status(303);
        order = SalesOrder::find_or_404(app.db(), order.id).await.unwrap();
    }
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    let line = SaleLine::of(app.db(), &order.subject())
        .await
        .unwrap()
        .remove(0);
    (order, line)
}

/// A work order of `quantity` units of the item for the order line, in
/// `status`.
async fn work_order_for(
    app: &TestApp,
    order: &SalesOrder,
    line: &SaleLine,
    item: &wl_inventory::Item,
    quantity_milli: i64,
    status: WorkOrderStatus,
) -> WorkOrder {
    let plant = match manufacturing::Plant::query().first(app.db()).await.unwrap() {
        Some(plant) => plant,
        None => manufacturing::Plant::create(
            app.db(),
            manufacturing::Plant {
                name: "Plant 1".into(),
                active: true,
                ..manufacturing::Plant::default()
            },
        )
        .await
        .unwrap(),
    };
    let mut made = WorkOrder {
        number: format!("WO-{}-{}", order.id, line.id),
        plant_id: plant.id,
        item_id: item.id,
        quantity_milli,
        source: WorkOrderSource::SalesOrder,
        sales_order_id: Some(order.id),
        sales_order_line_id: Some(line.id),
        status,
        ..WorkOrder::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    made.insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    made
}

async fn item(app: &TestApp, code: &str) -> wl_inventory::Item {
    let mut item = wl_inventory::Item::new(code, code, "pcs");
    item.save(app.db()).await.unwrap();
    item
}

/// BL-MFG-03 Done-when "not started": the sales person amends line 1 of the
/// order from 10 to 6 on the sales order page; the planned work order for the
/// line takes the new open quantity.
#[renox::test]
async fn amending_an_order_changes_its_planned_work_order() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let panel = item(&app, "PANEL").await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let (order, line) = place(&app, &sales, customer, "PANEL", "10").await;
    let made = work_order_for(
        &app,
        &order,
        &line,
        &panel,
        10_000,
        WorkOrderStatus::Planned,
    )
    .await;

    app.acting_as(&sales);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/amend", order.id),
            &[
                ("revision", &order.revision.to_string()),
                ("reason", "The customer needs fewer"),
                (&format!("line_{}_quantity", line.id), "6"),
            ],
        )
        .await
        .assert_status(303);

    let after = WorkOrder::find_or_404(app.db(), made.id).await.unwrap();
    assert_eq!(after.quantity_milli, 6_000);
    assert_eq!(after.status, WorkOrderStatus::Planned);
}

/// BL-MFG-14 Done-when "Cancellation" through the sheet: the cancel-order
/// sheet of an order whose standard item started proposes *finish to stock*,
/// and *scrap* for the item flagged made for one customer; the person may
/// change it.
#[renox::test]
async fn the_cancel_sheet_preselects_the_outcome_of_the_items_flag() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER).await;
    let standard = item(&app, "LID").await;
    let label = item(&app, "LABEL").await;
    app.acting_as(&manager);
    app.htmx()
        .post(
            "/manufacturing/conformity",
            &[("code", "LABEL"), ("customer_specific", "1")],
        )
        .await
        .assert_status(303);
    let customer = company(&app, "Al Noor Trading LLC").await;
    let (lid_order, lid_line) = place(&app, &sales, customer, "LID", "10").await;
    let (label_order, label_line) = place(&app, &sales, customer, "LABEL", "10").await;
    work_order_for(
        &app,
        &lid_order,
        &lid_line,
        &standard,
        10_000,
        WorkOrderStatus::InProgress,
    )
    .await;
    work_order_for(
        &app,
        &label_order,
        &label_line,
        &label,
        10_000,
        WorkOrderStatus::InProgress,
    )
    .await;

    let page = |order: &SalesOrder| {
        let (app, sales, id) = (&app, &sales, order.id);
        async move {
            app.acting_as(sales);
            app.request()
                .header("accept", "text/html")
                .get(&format!("/sales-orders/{id}"))
                .await
                .text()
        }
    };
    let selected = |html: &str, line: i64| -> Option<String> {
        let name = format!("name=\"line_{line}_outcome\"");
        let start = html.find(&name)?;
        let select = &html[start..html[start..].find("</select>").map(|e| start + e)?];
        let chosen = select.find("selected")?;
        let from = select[..chosen].rfind("value=\"")? + 7;
        Some(select[from..from + select[from..].find('"')?].to_owned())
    };
    let html = page(&lid_order).await;
    assert_eq!(
        selected(&html, lid_line.id).as_deref(),
        Some("finish_to_stock")
    );
    let html = page(&label_order).await;
    assert_eq!(selected(&html, label_line.id).as_deref(), Some("scrap"));
}

/// BL-MFG-14 Done-when "Party block": blocking customer C holds its undelivered
/// sales orders (the sales manager releases them); its work orders stay
/// `in_progress`.
#[renox::test]
async fn blocking_a_customer_holds_its_orders_and_leaves_its_work_orders_running() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let _manager = person(&app, "Sam", SALES_MANAGER).await;
    let panel = item(&app, "PANEL").await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let other = company(&app, "Other Trading LLC").await;
    let (order, line) = place(&app, &sales, customer, "PANEL", "10").await;
    let (spared, _) = place(&app, &sales, other, "PANEL", "5").await;
    let running = work_order_for(
        &app,
        &order,
        &line,
        &panel,
        10_000,
        WorkOrderStatus::InProgress,
    )
    .await;

    block(&app, customer).await;

    let held = SalesOrder::find_or_404(app.db(), order.id).await.unwrap();
    assert_eq!(held.status, SalesOrderStatus::CreditHold);
    assert_eq!(held.held_from.as_deref(), Some("confirmed"));
    let free = SalesOrder::find_or_404(app.db(), spared.id).await.unwrap();
    assert_eq!(free.status, SalesOrderStatus::Confirmed, "another customer");
    let work = WorkOrder::find_or_404(app.db(), running.id).await.unwrap();
    assert_eq!(work.status, WorkOrderStatus::InProgress);
}

/// BL-MFG-03 Done-when "cancelled": the sales manager cancels the order from
/// the page; the planned work order for its line is cancelled by the rule and
/// the planner is told; a work order that started keeps running (for stock)
/// when the outcome is *finish to stock*.
#[renox::test]
async fn cancelling_an_order_cancels_the_planned_work_order_and_unlinks_the_started_one() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let manager = person(&app, "Sam", SALES_MANAGER).await;
    let _planner = person(&app, "Amal", host::roles::PRODUCTION_PLANNER).await;
    let panel = item(&app, "PANEL").await;
    let customer = company(&app, "Al Noor Trading LLC").await;
    let (planned_order, planned_line) = place(&app, &sales, customer, "PANEL", "10").await;
    let (started_order, started_line) = place(&app, &sales, customer, "PANEL", "8").await;
    let planned = work_order_for(
        &app,
        &planned_order,
        &planned_line,
        &panel,
        10_000,
        WorkOrderStatus::Planned,
    )
    .await;
    let started = work_order_for(
        &app,
        &started_order,
        &started_line,
        &panel,
        8_000,
        WorkOrderStatus::InProgress,
    )
    .await;

    app.acting_as(&sales);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/cancel", planned_order.id),
            &[("reason", "The customer withdrew the order")],
        )
        .await
        .assert_status(303);
    // The started line waits for the sales manager's approval (`SAL.ACL.05`).
    app.htmx()
        .post(
            &format!("/sales-orders/{}/cancel", started_order.id),
            &[
                ("reason", "The customer withdrew the order"),
                (
                    &format!("line_{}_outcome", started_line.id),
                    "finish_to_stock",
                ),
            ],
        )
        .await
        .assert_status(303);
    let waiting =
        wl_kernel::approvals::ApprovalRequest::pending_about(app.db(), &started_order.subject())
            .await
            .unwrap();
    assert_eq!(waiting.len(), 1, "the cancellation of a started line waits");
    assert_eq!(
        WorkOrder::find_or_404(app.db(), started.id)
            .await
            .unwrap()
            .source,
        WorkOrderSource::SalesOrder,
        "nothing changes before the approval"
    );
    wl_kernel::approvals::approve(app.state(), waiting[0].id, &manager, "agreed")
        .await
        .unwrap();

    let after = WorkOrder::find_or_404(app.db(), planned.id).await.unwrap();
    assert_eq!(after.status, WorkOrderStatus::Cancelled);
    let kept = WorkOrder::find_or_404(app.db(), started.id).await.unwrap();
    assert_eq!(kept.status, WorkOrderStatus::InProgress);
    assert_eq!(kept.source, WorkOrderSource::Stock, "now for stock");
}
