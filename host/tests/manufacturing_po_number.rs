//! BL-MFG-06 (#387) and BL-SAL-18 (#137) in the manufacturing deployment:
//! with the 07 values (`Unique customer PO number per customer` on) the real
//! wl-sales routes refuse a second order of a customer with a PO number the
//! customer's other order holds, at submit and when an order is amended,
//! naming the first order; another customer may use the number. The buyer's
//! portal page `/portal/manufacturing/orders` is tested against the same real
//! orders: a refused upload shows the number and the status of the order
//! that holds the PO and nothing else of any order.

mod support;

use host::roles::SALES;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use wl_kernel::PartyId;
use wl_sales::{SalesOrder, SalesOrderStatus};

use support::{company, day, manufacturing_booted, person};

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

/// `user` acts for `client` in the portal (a buyer of the company).
async fn buyer(app: &TestApp, name: &str, client: PartyId) -> User {
    let user = person(app, name, SALES).await;
    let contact = company(app, &format!("{name} (portal person)")).await;
    let mut grant = wl_portal::AccessGrant {
        user_id: Some(user.id),
        person_party_id: contact.0,
        party_id: client.0,
        audience: "tenant".into(),
        roles: "buyer".into(),
        source: "invitation".into(),
        valid_from: day(-30),
        status: wl_portal::GrantStatus::Active,
        ..wl_portal::AccessGrant::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    grant
        .insert_in_state(&mut tx, wl_portal::GrantStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    user
}

/// BL-SAL-18 (#137) point 2: the buyer sees only the number and the status of
/// the order that holds a PO. Order SO-A of customer C holds "PO-8812" (a line
/// of AED 123.45, a delivery address, the customer's reference and the sales
/// rep's name are on it); customer D has an order with "PO-D-1". C's buyer
/// uploads "po-8812 " through the portal page and is refused with the number
/// and the status of SO-A and nothing else: not its price, line, address, rep,
/// nor anything of D's order; C's buyer may use D's number; D's buyer may use
/// C's; the page lists only the buyer's own uploads, as a page and as JSON.
#[renox::test]
async fn the_buyer_is_told_only_the_number_and_status_of_the_order_holding_the_po() {
    let app = manufacturing_booted().await;
    let sales = person(&app, "Sara", SALES).await;
    let c = company(&app, "Al Noor Trading LLC").await;
    let d = company(&app, "Bin Rashid Contracting LLC").await;
    let carol = buyer(&app, "Carol", c).await;
    let dave = buyer(&app, "Dave", d).await;

    let held = draft(&app, &sales, c, "PO-8812").await;
    app.acting_as(&sales);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/lines", held.id),
            &[
                ("item_code", "SECRET-PART"),
                ("quantity", "3"),
                ("unit_price", "123.45"),
            ],
        )
        .await
        .assert_status(303);
    app.htmx()
        .post(
            &format!("/sales-orders/{}/delivery", held.id),
            &[
                ("delivery_address", "Warehouse 7, Al Quoz"),
                ("delivery_country", "AE"),
            ],
        )
        .await
        .assert_status(303);
    submit(&app, &held).await.assert_status(303);
    let held = SalesOrder::find_or_404(app.db(), held.id).await.unwrap();
    let theirs = draft(&app, &sales, d, "PO-D-1").await;
    submit(&app, &theirs).await.assert_status(303);
    let theirs = SalesOrder::find_or_404(app.db(), theirs.id).await.unwrap();

    app.acting_as(&carol);
    let refused = app
        .htmx()
        .post(
            "/portal/manufacturing/orders",
            &[("po", "po-8812 "), ("note", "Please confirm by Friday")],
        )
        .await;
    refused.assert_status(422);
    let text = refused.text();
    assert!(text.contains(&held.number), "the number is told: {text}");
    assert!(text.contains("Confirmed"), "and the status: {text}");
    for secret in [
        "123.45",
        "370.35",
        "SECRET-PART",
        "Warehouse 7",
        "Sara",
        "Al Quoz",
        &theirs.number,
        "PO-D-1",
        "Bin Rashid",
    ] {
        assert!(!text.contains(secret), "{secret} leaked: {text}");
    }
    // The same through a plain request (the JSON answer).
    let plain = app
        .post("/portal/manufacturing/orders", &[("po", "PO-8812")])
        .await;
    plain.assert_status(422);
    assert!(plain.text().contains(&held.number), "{}", plain.text());
    assert!(!plain.text().contains("123.45"), "{}", plain.text());

    // Another customer's number is free to use and says nothing about it.
    let free = app
        .htmx()
        .post("/portal/manufacturing/orders", &[("po", "PO-D-1")])
        .await;
    free.assert_status(303);
    app.acting_as(&dave);
    app.htmx()
        .post("/portal/manufacturing/orders", &[("po", "PO-8812")])
        .await
        .assert_status(303);

    // Each buyer's page lists their own uploads only.
    for (who, own, other) in [(&carol, "PO-D-1", "PO-8812"), (&dave, "PO-8812", "PO-D-1")] {
        app.acting_as(who);
        let json = app.get("/portal/manufacturing/orders").await;
        json.assert_ok();
        let body = json.text();
        assert!(body.contains(own), "{} sees their upload: {body}", who.name);
        assert!(
            !body.contains(other),
            "{} sees only their own: {body}",
            who.name
        );
        assert!(!body.contains("123.45"), "{body}");
    }
}
