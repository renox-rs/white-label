//! BL-SAL-18 (#137): with *Unique customer PO number per customer* on, a
//! second order with the same customer PO number (and call-off number) for
//! the customer is refused at `submit` and points to the existing order.

mod support;

use wl_kernel::PartyId;
use wl_sales::SalesOrderStatus;
use wl_sales::WlSales;
use wl_sales::po_number::{PoKey, buyer_refusal};
use wl_sales::sale_line::NewLine;
use wl_sales::sales_orders::{self, NewSalesOrder};
use wl_sales::{SalesOrder, permissions as perm};

use renox::prelude::*;
use support::Booted;
use support::credit::Credit;
use support::sell::{CUSTOMER, boot_with, reload};
use wl_kernel::invoicing::VatCategory;

/// The other customer.
const OTHER: i64 = 9;

async fn boot_on() -> (Booted, Credit, User) {
    let (t, credit) = boot_with(WlSales::new().unique_customer_po(true)).await;
    let by = t
        .user(
            "Sam",
            &[
                perm::SALES_ORDERS_VIEW,
                perm::SALES_ORDERS_CREATE,
                perm::SALES_ORDERS_CANCEL,
            ],
        )
        .await;
    t.app.acting_as(&by);
    (t, credit, by)
}

/// A draft order of `party` with one line and the PO reference, entered
/// through the services.
async fn draft(t: &Booted, by: &User, party: i64, po: &str, call_off: &str) -> SalesOrder {
    let order = sales_orders::create(
        t.state(),
        Some(by.id),
        NewSalesOrder::new(PartyId(party))
            .customer_po(po)
            .call_off(call_off),
    )
    .await
    .unwrap();
    sales_orders::add_line(
        t.state(),
        order.id,
        NewLine::new("pump", 800_000).vat_category(VatCategory::ZeroRated),
    )
    .await
    .unwrap();
    reload(t, order.id).await
}

/// Submits `order` through the page and answers the response.
async fn submit(t: &Booted, order: &SalesOrder) -> renox::testing::TestResponse {
    t.app
        .post(&format!("/sales-orders/{}/submit", order.id), &[("x", "")])
        .await
}

/// A submitted (confirmed) order with the reference.
async fn placed(t: &Booted, by: &User, party: i64, po: &str, call_off: &str) -> SalesOrder {
    let order = draft(t, by, party, po, call_off).await;
    submit(t, &order).await.assert_status(303);
    let order = reload(t, order.id).await;
    assert_eq!(order.status, SalesOrderStatus::Confirmed);
    order
}

/// AC1: SO-70 of customer C carries "PO-8812"; a new order for C with
/// "po-8812 " is refused and names SO-70; nothing is confirmed for it.
#[renox::test]
async fn a_second_order_with_the_same_po_number_is_refused_naming_the_first() {
    let (t, credit, by) = boot_on().await;
    let first = placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    assert_eq!(credit.confirmed.lock().unwrap().len(), 1);

    let second = draft(&t, &by, CUSTOMER, "po-8812 ", "").await;
    let refused = submit(&t, &second).await;
    refused.assert_status(422);
    let text = refused.text();
    assert!(text.contains(&first.number), "{text}");
    assert!(text.contains("Confirmed"), "{text}");
    assert_eq!(reload(&t, second.id).await.status, SalesOrderStatus::Draft);
    assert_eq!(credit.confirmed.lock().unwrap().len(), 1);
    assert_eq!(PoKey::query().get(t.db()).await.unwrap().len(), 1);
}

/// AC2: a buyer's PO upload refused for a duplicate shows only the existing
/// order's number and status, and only among the buyer's own customer's
/// orders.
#[renox::test]
async fn a_buyer_is_told_only_the_number_and_status_of_the_existing_order() {
    let (t, _credit, by) = boot_on().await;
    let first = placed(&t, &by, CUSTOMER, "PO-8812", "").await;

    let refusal = buyer_refusal(t.state(), CUSTOMER, "PO-8812", "")
        .await
        .unwrap()
        .expect("the number is used");
    assert_eq!(refusal.number, first.number);
    assert_eq!(refusal.status, "Confirmed");
    let sentence = refusal.text();
    assert!(sentence.contains(&first.number) && sentence.contains("Confirmed"));
    // No prices, no notes: nothing of the order but its number and status.
    assert!(!sentence.contains("8,000") && !sentence.contains("pump"));
    // Another customer's buyer is told nothing.
    assert!(
        buyer_refusal(t.state(), OTHER, "PO-8812", "")
            .await
            .unwrap()
            .is_none()
    );
    // A number nobody uses, and an empty one, pass.
    assert!(
        buyer_refusal(t.state(), CUSTOMER, "PO-1", "")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        buyer_refusal(t.state(), CUSTOMER, " ", "")
            .await
            .unwrap()
            .is_none()
    );
}

/// AC3: a call-off number tells the calls against a blanket PO apart.
#[renox::test]
async fn a_call_off_number_tells_deliveries_against_a_blanket_po_apart() {
    let (t, _credit, by) = boot_on().await;
    placed(&t, &by, CUSTOMER, "BL-1", "CO-1").await;

    let other_call_off = draft(&t, &by, CUSTOMER, "BL-1", "CO-2").await;
    submit(&t, &other_call_off).await.assert_status(303);

    let same = draft(&t, &by, CUSTOMER, "bl-1", " co-1").await;
    let refused = submit(&t, &same).await;
    refused.assert_status(422);
    assert!(refused.text().contains("call-off"), "{}", refused.text());
    // The blanket number alone is another key.
    let bare = draft(&t, &by, CUSTOMER, "BL-1", "").await;
    submit(&t, &bare).await.assert_status(303);
}

/// AC4: the same number for another customer is accepted; orders without a
/// reference are never checked.
#[renox::test]
async fn the_same_number_for_another_customer_and_orders_without_one_are_accepted() {
    let (t, _credit, by) = boot_on().await;
    placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    placed(&t, &by, OTHER, "PO-8812", "").await;
    placed(&t, &by, CUSTOMER, "", "").await;
    placed(&t, &by, CUSTOMER, "  ", "").await;
}

/// A cancelled order still counts: the customer sends a new number.
#[renox::test]
async fn a_cancelled_order_keeps_its_number_used() {
    let (t, _credit, by) = boot_on().await;
    let first = placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    t.app
        .post(
            &format!("/sales-orders/{}/transitions/cancel", first.id),
            &[("reason", "Customer withdrew")],
        )
        .await
        .assert_status(303);
    assert_eq!(
        reload(&t, first.id).await.status,
        SalesOrderStatus::Cancelled
    );

    let again = draft(&t, &by, CUSTOMER, "PO-8812", "").await;
    let refused = submit(&t, &again).await;
    refused.assert_status(422);
    assert!(refused.text().contains("Cancelled"), "{}", refused.text());
    assert!(refused.text().contains(&first.number));
}

/// AC5: changing the reference of an order to a number the customer's other
/// order holds is refused (the same check `amend` of BL-SAL-05 calls).
#[renox::test]
async fn changing_the_reference_to_a_used_number_is_refused() {
    let (t, _credit, by) = boot_on().await;
    placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    let other = draft(&t, &by, CUSTOMER, "PO-9000", "").await;

    t.app
        .post(
            &format!("/sales-orders/{}/reference", other.id),
            &[("customer_po", "PO-8812")],
        )
        .await
        .assert_status(422);
    assert_eq!(
        reload(&t, other.id).await.customer_po.as_deref(),
        Some("PO-9000")
    );
    t.app
        .post(
            &format!("/sales-orders/{}/reference", other.id),
            &[("customer_po", "PO-9001"), ("call_off", "CO-1")],
        )
        .await
        .assert_status(303);
    let changed = reload(&t, other.id).await;
    assert_eq!(changed.customer_po.as_deref(), Some("PO-9001"));
    assert_eq!(changed.call_off.as_deref(), Some("CO-1"));
    // A submitted order's reference is changed by `amend`, not here.
    let placed = reload(
        &t,
        PoKey::query()
            .first(t.db())
            .await
            .unwrap()
            .unwrap()
            .order_id,
    )
    .await;
    t.app
        .post(
            &format!("/sales-orders/{}/reference", placed.id),
            &[("customer_po", "PO-1")],
        )
        .await
        .assert_status(422);
}

/// AC6: with the setting off the second "PO-8812" order is accepted, and
/// claims no key.
#[renox::test]
async fn with_the_setting_off_the_same_number_is_accepted() {
    let (t, _credit) = boot_with(WlSales::new()).await;
    let by = t
        .user("Sam", &[perm::SALES_ORDERS_VIEW, perm::SALES_ORDERS_CREATE])
        .await;
    t.app.acting_as(&by);
    placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    assert!(PoKey::query().get(t.db()).await.unwrap().is_empty());
}

/// Surface (system): the unique key lets only one of two concurrent submits
/// claim a number: a second row for the same customer, reference and
/// call-off number cannot be written, whatever the pre-check saw.
#[renox::test]
async fn the_unique_key_lets_only_one_order_claim_a_number() {
    let (t, _credit, by) = boot_on().await;
    let first = placed(&t, &by, CUSTOMER, "PO-8812", "").await;
    let racer = draft(&t, &by, CUSTOMER, "PO-8812", "").await;
    let duplicate = PoKey {
        order_id: racer.id,
        party_id: CUSTOMER,
        po_key: "po-8812".to_owned(),
        call_off_key: String::new(),
        ..PoKey::default()
    }
    .insert(t.db())
    .await;
    assert!(duplicate.is_err());
    assert_eq!(PoKey::query().get(t.db()).await.unwrap().len(), 1);
    assert_eq!(
        PoKey::query()
            .first(t.db())
            .await
            .unwrap()
            .unwrap()
            .order_id,
        first.id
    );
}

/// Surface (back office): the order page shows the PO number, warns with a
/// link to the order that holds it and disables *Submit* with the reason;
/// the order sheet takes the number.
#[renox::test]
async fn the_order_page_shows_the_reference_and_the_clash() {
    let (t, _credit, by) = boot_on().await;
    let first = placed(&t, &by, CUSTOMER, "PO-8812", "CO-7").await;
    let second = draft(&t, &by, CUSTOMER, "po-8812", "co-7").await;

    t.app
        .get(&format!("/sales-orders/{}", first.id))
        .await
        .assert_ok()
        .assert_see("PO-8812, call-off CO-7");
    let page = t.app.get(&format!("/sales-orders/{}", second.id)).await;
    page.assert_ok()
        .assert_see("is already used by order")
        .assert_see(&format!("/sales-orders/{}", first.id))
        .assert_see("Change the PO number");
    assert!(!page.text().contains("po.submit_blocked"));

    // The create sheet takes the number from the form.
    t.app
        .post(
            "/sales-orders",
            &[
                ("party_id", &CUSTOMER.to_string()),
                ("customer_po", "PO-77"),
                ("call_off", "CO-1"),
            ],
        )
        .await
        .assert_status(303);
    let created = SalesOrder::query()
        .order_by_desc(SalesOrder::ID)
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(created.customer_po.as_deref(), Some("PO-77"));
    assert_eq!(created.call_off.as_deref(), Some("CO-1"));
    t.app.assert_views_compile();
}
