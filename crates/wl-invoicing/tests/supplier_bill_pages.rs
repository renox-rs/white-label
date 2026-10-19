//! The supplier bill back office pages (wl-invoicing §6, A9, #74 part 4): the
//! list and its permission, the draft form with lines, the bill page with its
//! exception reasons and the link to the earlier bill, the submit route and
//! the approve and void sheets.

mod support;

use renox::auth::User;
use renox::prelude::*;
use wl_invoicing::duplicates::exceptions_of;
use wl_invoicing::permissions as perm;
use wl_invoicing::supplier_bill;
use wl_invoicing::{SupplierBill, SupplierBillStatus};

use support::{Booted, boot};

/// A header and one line, as the form sends them.
fn form<'a>(number: &'a str, price: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("supplier", "P-00001"),
        ("supplier_invoice_number", number),
        ("bill_date", "2026-10-05"),
        ("currency", "AED"),
        ("line_1_description", "Pump repair"),
        ("line_1_quantity", "2"),
        ("line_1_unit_price", price),
        ("line_1_vat", "standard"),
    ]
}

async fn post(
    t: &Booted,
    by: &User,
    path: &str,
    fields: &[(&str, &str)],
) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app.htmx().post(path, fields).await
}

/// The newest bill with this number (as stored: trimmed).
async fn find(t: &Booted, number: &str) -> SupplierBill {
    SupplierBill::query()
        .where_(SupplierBill::SUPPLIER_INVOICE_NUMBER.eq(number.trim()))
        .order_by(SupplierBill::ID)
        .get(t.db())
        .await
        .unwrap()
        .pop()
        .unwrap_or_else(|| panic!("no bill {number}"))
}

/// Enters and submits a bill as `by`; returns it as stored.
async fn submitted(t: &Booted, by: &User, number: &str) -> SupplierBill {
    post(t, by, "/supplier-bills", &form(number, "500.00"))
        .await
        .assert_status(303);
    let bill = find(t, number).await;
    post(t, by, &format!("/supplier-bills/{}/submit", bill.id), &[]).await;
    SupplierBill::find_or_404(t.db(), bill.id).await.unwrap()
}

#[renox::test]
async fn the_list_needs_the_view_permission_and_the_form_the_create_permission() {
    let t = boot().await;
    let anna = t.accountant("Anna").await;
    let viewer = t.user("Vera", &[perm::SUPPLIER_BILLS_VIEW]).await;
    let nobody = t.user("Nora", &[]).await;
    let bill = submitted(&t, &anna, "INV-1").await;
    assert_eq!(bill.status, SupplierBillStatus::Matched);

    for user in [&anna, &viewer] {
        t.app.acting_as(user);
        t.app
            .get("/supplier-bills")
            .await
            .assert_ok()
            .assert_see("INV-1");
        t.app
            .get(&format!("/supplier-bills/{}", bill.id))
            .await
            .assert_ok()
            .assert_see("Pump repair");
    }
    t.app.acting_as(&nobody);
    t.app.get("/supplier-bills").await.assert_forbidden();
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_forbidden();

    // Viewing is not enough to enter, submit or approve.
    t.app.acting_as(&viewer);
    t.app.get("/supplier-bills/new").await.assert_forbidden();
    post(&t, &viewer, "/supplier-bills", &form("INV-2", "1"))
        .await
        .assert_forbidden();
    post(
        &t,
        &viewer,
        &format!("/supplier-bills/{}/submit", bill.id),
        &[],
    )
    .await
    .assert_forbidden();
    post(
        &t,
        &viewer,
        &format!("/supplier-bills/{}/transitions/approve", bill.id),
        &[("reason", "ok")],
    )
    .await
    .assert_forbidden();
    assert_eq!(
        find(&t, "INV-1").await.status,
        SupplierBillStatus::Matched,
        "nothing changed"
    );
    // The viewer is offered no action on the page.
    t.app.acting_as(&viewer);
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_ok()
        .assert_dont_see("approve-supplier-bill");
}

#[renox::test]
async fn a_draft_is_entered_changed_and_deleted_from_its_form() {
    let t = boot().await;
    let anna = t.accountant("Anna").await;
    t.app.acting_as(&anna);
    t.app
        .get("/supplier-bills/new")
        .await
        .assert_ok()
        .assert_see("New supplier bill");

    // An unknown supplier code and a bad amount are 422 with the input kept.
    let mut bad = form("INV-1", "10");
    bad[0] = ("supplier", "P-99999");
    post(&t, &anna, "/supplier-bills", &bad)
        .await
        .assert_status(422);
    post(&t, &anna, "/supplier-bills", &form("INV-1", "ten"))
        .await
        .assert_status(422);
    post(&t, &anna, "/supplier-bills", &form("  ", "10"))
        .await
        .assert_status(422);
    assert!(SupplierBill::query().get(t.db()).await.unwrap().is_empty());

    post(&t, &anna, "/supplier-bills", &form("INV-1", "500.00"))
        .await
        .assert_status(303);
    let bill = find(&t, "INV-1").await;
    assert_eq!(bill.status, SupplierBillStatus::Draft);
    assert_eq!(bill.party_id, 1);
    assert_eq!(bill.net_total, 100_000, "2 x AED 500.00");
    assert_eq!(bill.vat_total, 5_000);
    assert_eq!(bill.created_by, Some(anna.id));

    t.app.acting_as(&anna);
    t.app
        .get(&format!("/supplier-bills/{}/edit", bill.id))
        .await
        .assert_ok()
        .assert_see("INV-1")
        .assert_see("P-00001")
        .assert_see("500.00");
    t.app
        .get(&format!("/supplier-bills/{}", bill.id))
        .await
        .assert_ok()
        .assert_see("Blue Dunes LLC")
        .assert_see("AED 1,050.00")
        .assert_see("submit-supplier-bill");

    let mut changed = form("INV-1B", "600.00");
    changed.push(("line_2_description", "Call-out"));
    changed.push(("line_2_unit_price", "100"));
    changed.push(("line_2_vat", "exempt"));
    post(&t, &anna, &format!("/supplier-bills/{}", bill.id), &changed)
        .await
        .assert_status(303);
    let bill = SupplierBill::find_or_404(t.db(), bill.id).await.unwrap();
    assert_eq!(bill.supplier_invoice_number, "INV-1B");
    assert_eq!(bill.net_total, 130_000);
    assert_eq!(
        supplier_bill::lines_of(t.db(), bill.id)
            .await
            .unwrap()
            .len(),
        2
    );

    // Submitted, it can no longer be changed or deleted.
    post(
        &t,
        &anna,
        &format!("/supplier-bills/{}/submit", bill.id),
        &[],
    )
    .await;
    let done = SupplierBill::find_or_404(t.db(), bill.id).await.unwrap();
    assert_eq!(done.status, SupplierBillStatus::Matched);
    t.app.acting_as(&anna);
    t.app
        .get(&format!("/supplier-bills/{}/edit", bill.id))
        .await
        .assert_status(422);
    post(&t, &anna, &format!("/supplier-bills/{}", bill.id), &changed)
        .await
        .assert_status(422);
    post(
        &t,
        &anna,
        &format!("/supplier-bills/{}/delete", bill.id),
        &[],
    )
    .await
    .assert_status(422);

    // A second draft is deleted.
    post(&t, &anna, "/supplier-bills", &form("INV-9", "1"))
        .await
        .assert_status(303);
    let draft = find(&t, "INV-9").await;
    post(
        &t,
        &anna,
        &format!("/supplier-bills/{}/delete", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    assert!(
        SupplierBill::find(t.db(), draft.id)
            .await
            .unwrap()
            .is_none()
    );
    // Submit without lines is refused (422).
    let mut empty = form("INV-E", "1");
    empty.truncate(4);
    post(&t, &anna, "/supplier-bills", &empty)
        .await
        .assert_status(303);
    let empty = find(&t, "INV-E").await;
    post(
        &t,
        &anna,
        &format!("/supplier-bills/{}/submit", empty.id),
        &[],
    )
    .await
    .assert_status(422);
}

#[renox::test]
async fn a_held_duplicate_shows_its_reason_and_the_creator_cannot_approve_it() {
    let t = boot().await;
    let anna = t.accountant("Anna").await;
    let ben = t.accountant("Ben").await;
    let first = submitted(&t, &anna, "INV-77").await;
    post(
        &t,
        &ben,
        &format!("/supplier-bills/{}/transitions/approve", first.id),
        &[],
    )
    .await
    .assert_status(303);
    let first = SupplierBill::find_or_404(t.db(), first.id).await.unwrap();
    assert_eq!(first.status, SupplierBillStatus::Approved);
    assert_eq!(first.approved_by, Some(ben.id));

    let second = submitted(&t, &anna, "inv-77 ").await;
    assert_eq!(second.status, SupplierBillStatus::Exception);
    assert_eq!(second.duplicate_of, Some(first.id));
    assert_eq!(exceptions_of(t.db(), second.id).await.unwrap().len(), 1);

    // The page names the reason and links to the earlier bill.
    t.app.acting_as(&anna);
    let page = t.app.get(&format!("/supplier-bills/{}", second.id)).await;
    page.assert_ok()
        .assert_see("Possible duplicate")
        .assert_see(&format!("href=\"/supplier-bills/{}\"", first.id))
        .assert_see("Earlier bill: INV-77")
        // The creator gets the void sheet but not the approve sheet.
        .assert_see("void-supplier-bill")
        .assert_dont_see("approve-supplier-bill");
    t.app.acting_as(&ben);
    t.app
        .get(&format!("/supplier-bills/{}", second.id))
        .await
        .assert_ok()
        .assert_see("approve-supplier-bill")
        .assert_see("void-supplier-bill");

    let approve = format!("/supplier-bills/{}/transitions/approve", second.id);
    post(&t, &anna, &approve, &[("reason", "same delivery")])
        .await
        .assert_status(422);
    post(&t, &ben, &approve, &[("reason", "  ")])
        .await
        .assert_status(422);
    post(&t, &ben, &approve, &[]).await.assert_status(422);
    let held = SupplierBill::find_or_404(t.db(), second.id).await.unwrap();
    assert_eq!(held.status, SupplierBillStatus::Exception);

    post(
        &t,
        &ben,
        &approve,
        &[("reason", "two deliveries, same number")],
    )
    .await
    .assert_status(303);
    let done = SupplierBill::find_or_404(t.db(), second.id).await.unwrap();
    assert_eq!(done.status, SupplierBillStatus::Approved);
    assert_eq!(done.approved_by, Some(ben.id));
    t.app.acting_as(&anna);
    t.app
        .get(&format!("/supplier-bills/{}", second.id))
        .await
        .assert_ok()
        .assert_see("two deliveries, same number")
        .assert_see("Ben")
        .assert_dont_see("approve-supplier-bill");
}

#[renox::test]
async fn a_held_bill_is_voided_with_a_reason_and_kept() {
    let t = boot().await;
    let anna = t.accountant("Anna").await;
    submitted(&t, &anna, "INV-80").await;
    let held = submitted(&t, &anna, "INV-80").await;
    assert_eq!(held.status, SupplierBillStatus::Exception);

    let void = format!("/supplier-bills/{}/transitions/void", held.id);
    post(&t, &anna, &void, &[("reason", " ")])
        .await
        .assert_status(422);
    post(&t, &anna, &void, &[("reason", "duplicate")])
        .await
        .assert_status(303);
    let done = SupplierBill::find_or_404(t.db(), held.id).await.unwrap();
    assert_eq!(done.status, SupplierBillStatus::Void);
    assert_eq!(done.void_reason.as_deref(), Some("duplicate"));
    t.app.acting_as(&anna);
    t.app
        .get(&format!("/supplier-bills/{}", held.id))
        .await
        .assert_ok()
        .assert_see("duplicate")
        .assert_see("Void reason")
        .assert_dont_see("void-supplier-bill");
    // The row and its lines are still there.
    assert_eq!(
        supplier_bill::lines_of(t.db(), held.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[renox::test]
async fn the_demo_bills_need_the_demo_supplier() {
    // The test directory has no `P-00003`; the host seeding covers the bills.
    let t = boot().await;
    assert_eq!(wl_invoicing::seed::bills(t.state()).await.unwrap(), 0);
    assert!(SupplierBill::query().get(t.db()).await.unwrap().is_empty());
}
