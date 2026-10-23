//! BL-MFG-06 (#387) and BL-SAL-18 (#137) in the manufacturing deployment:
//! with the 07 values (`Unique customer PO number per customer` on) the real
//! wl-sales routes refuse a second order of a customer with a PO number the
//! customer's other order holds, at submit and when an order is amended,
//! naming the first order; another customer may use the number.

mod support;

use host::roles::SALES;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::PartyId;
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{company, manufacturing_booted, person};

/// A draft order of `customer` with a line and the PO reference, entered
/// through the pages of wl-sales.
async fn draft(app: &TestApp, by: &User, customer: PartyId, po: &str) -> SalesOrder {
    app.acting_as(by);
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
                ("item_code", "PANEL"),
                ("quantity", "10"),
                ("unit_price", "10.00"),
                ("vat", "standard"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/reference", order.id),
            &[("customer_po", po)],
        )
        .await
        .assert_status(303);
    SalesOrder::find_or_404(app.db(), order.id).await.unwrap()
}

async fn submit(app: &TestApp, order: &SalesOrder) -> renox::testing::TestResponse {
    app.htmx()
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
}

/// AC1: in a new 07 deployment the setting is on without anybody turning it
/// on: SO-A of customer C carries "PO-8812"; a second order of C with
/// "po-8812 " is refused at submit naming SO-A and stays a draft; the same
/// number for customer D is accepted; amending SO-C of C to "PO-8812" is
/// refused naming SO-A as well.
#[renox::test]
async fn the_manufacturing_deployment_refuses_a_second_order_with_the_same_po_number() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let c = company(&app, "Al Noor Trading LLC").await;
    let d = company(&app, "Bin Rashid Contracting LLC").await;

    // Both drafts carry the number: only the key claimed at submit counts.
    let first = draft(&app, &sales, c, "PO-8812").await;
    let second = draft(&app, &sales, c, "po-8812 ").await;
    submit(&app, &first).await.assert_status(303);
    let first = SalesOrder::find_or_404(app.db(), first.id).await.unwrap();
    assert_ne!(first.status, SalesOrderStatus::Draft, "the first is placed");

    let refused = submit(&app, &second).await;
    refused.assert_status(422);
    assert!(refused.text().contains(&first.number), "{}", refused.text());
    let second = SalesOrder::find_or_404(app.db(), second.id).await.unwrap();
    assert_eq!(second.status, SalesOrderStatus::Draft);

    let other = draft(&app, &sales, d, "PO-8812").await;
    submit(&app, &other).await.assert_status(303);
    assert_ne!(
        SalesOrder::find_or_404(app.db(), other.id)
            .await
            .unwrap()
            .status,
        SalesOrderStatus::Draft,
        "another customer may use the number"
    );

    // SO-C of C has its own number and is placed; the amendment to the number
    // SO-A holds is refused, a free one is accepted.
    let third = draft(&app, &sales, c, "PO-9000").await;
    submit(&app, &third).await.assert_status(303);
    let amend = format!("/sales-orders/{}/amend", third.id);
    let refused = app
        .htmx()
        .post(
            &amend,
            &[
                ("reason", "The buyer corrected it"),
                ("customer_po", "PO-8812"),
            ],
        )
        .await;
    refused.assert_status(422);
    assert!(refused.text().contains(&first.number), "{}", refused.text());
    app.htmx()
        .post(
            &amend,
            &[
                ("reason", "The buyer corrected it"),
                ("customer_po", "PO-9001"),
            ],
        )
        .await
        .assert_status(303);
    let amended = SalesOrder::find_or_404(app.db(), third.id).await.unwrap();
    assert_eq!(amended.customer_po.as_deref(), Some("PO-9001"));
}
