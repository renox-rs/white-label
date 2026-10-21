//! A blocked supplier (or party) with the real wl-parties `PartiesBlockGuard`
//! and the real wl-inventory, wl-invoicing and wl-payments in the
//! Construction host (`PTY.ACL.03`, issues #246 and #117): a purchase order
//! to a blocked supplier is refused with the neutral *on hold* text and an
//! offer of the exception, and issues once the finance manager approves it; a
//! revision that moves the delivery date later is refused while one that does
//! not extend the order applies; a set-off to a blocked party waits for the
//! same exception and runs once.

mod support;

use host::roles::{ADMIN, FINANCE_MANAGER};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{block, company, day, entity, noon_clock, person};
use wl_inventory::{Item, Location, PurchaseOrder, PurchaseOrderLine, PurchaseOrderStatus};
use wl_invoicing::bill_machine::approve as approve_bill;
use wl_invoicing::bill_submit::submit;
use wl_invoicing::credit_notes::By;
use wl_invoicing::supplier_bill::{self, BillDraft};
use wl_invoicing::{BillLineDraft, Invoice, SupplierBill, SupplierBillStatus};
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};
use wl_kernel::{BLOCK_EXCEPTION_ACTION, ClientConfig, PartyId};

const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
trn = "100000000000003"
default = true
"#;

async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app
}

/// A draft order of AED 4,000 (10 units at AED 400) to `supplier`.
async fn draft_order(app: &TestApp, supplier: PartyId) -> (PurchaseOrder, PurchaseOrderLine) {
    let mut item = Item::new("PARTS", "Spare parts", "pc");
    item.save(app.db()).await.unwrap();
    let mut store = Location::new("STORE-1", "Site store");
    store.save(app.db()).await.unwrap();
    let mut order = PurchaseOrder::new(supplier, store.id);
    let mut tx = app.db().begin().await.unwrap();
    order
        .insert_in_state(&mut tx, PurchaseOrderStatus::Draft, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut line = PurchaseOrderLine::new(order.id, 1, item.id, 10_000, 40_000);
    line.save(app.db()).await.unwrap();
    (order, line)
}

async fn order_now(app: &TestApp, order: &PurchaseOrder) -> PurchaseOrder {
    PurchaseOrder::find_or_404(app.db(), order.id)
        .await
        .unwrap()
}

async fn pending(app: &TestApp, order: &PurchaseOrder) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(app.db(), &order.subject())
        .await
        .unwrap()
}

/// Issuing to a blocked supplier is refused (*on hold*, the reason not told,
/// the exception offered); asking for the exception makes a request only the
/// finance manager decides; the order is issued once, and a revision that
/// moves the delivery date later is refused for the blocked supplier while a
/// lower total applies.
#[renox::test]
async fn a_blocked_supplier_cannot_be_issued_to_or_extended_without_an_exception() {
    let app = boot().await;
    let buyer = person(&app, "Amal", ADMIN).await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let supplier = company(&app, "Gulf Fasteners LLC").await;
    let (order, line) = draft_order(&app, supplier).await;
    block(&app, supplier).await;

    app.acting_as(&buyer);
    let refused = app
        .htmx()
        .post(&format!("/purchase-orders/{}/issue", order.id), &[])
        .await;
    refused.assert_status(422);
    let body = refused.text();
    assert!(body.contains("This party is on hold"), "{body}");
    assert!(!body.contains("3 bounced cheques"), "{body}");
    assert!(body.contains(r#"name="block_exception""#), "{body}");
    assert_eq!(
        order_now(&app, &order).await.status,
        PurchaseOrderStatus::Draft
    );
    assert!(pending(&app, &order).await.is_empty());

    app.acting_as(&buyer);
    app.htmx()
        .post(
            &format!("/purchase-orders/{}/issue", order.id),
            &[("block_exception", "1")],
        )
        .await
        .assert_ok();
    let requests = pending(&app, &order).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, BLOCK_EXCEPTION_ACTION);
    let own = approvals::approve(app.state(), requests[0].id, &buyer, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");

    approvals::approve(app.state(), requests[0].id, &finance, "approved")
        .await
        .unwrap();
    let issued = order_now(&app, &order).await;
    assert_eq!(issued.status, PurchaseOrderStatus::Issued);
    assert!(issued.number.is_some());

    // A revision that moves the delivery date later is refused.
    let line_qty = format!("qty_{}", line.id);
    let line_price = format!("price_{}", line.id);
    let later = day(30).to_string();
    app.acting_as(&buyer);
    let extended = app
        .htmx()
        .post(
            &format!("/purchase-orders/{}/revise", order.id),
            &[
                ("reason", "supplier is late"),
                ("expected_date", later.as_str()),
                ("requested_by", "supplier"),
            ],
        )
        .await;
    extended.assert_status(422);
    let body = extended.text();
    assert!(body.contains("on hold"), "{body}");
    assert_eq!(order_now(&app, &order).await.revision, 1);

    // A lower total does not extend the order: it applies.
    app.acting_as(&buyer);
    app.htmx()
        .post(
            &format!("/purchase-orders/{}/revise", order.id),
            &[
                ("reason", "discount"),
                (line_qty.as_str(), "10"),
                (line_price.as_str(), "350"),
            ],
        )
        .await
        .assert_ok();
    assert_eq!(order_now(&app, &order).await.revision, 2);
}

/// SB-77 of AED 5,250.00 from `party`, approved.
async fn approved_bill(app: &TestApp, party: PartyId, entity_id: i64) -> SupplierBill {
    let mut draft = BillDraft::new(
        party,
        "SB-77",
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
    )
    .line(BillLineDraft::new(
        "Back-charge",
        500_000,
        VatCategory::Standard,
    ));
    draft.legal_entity_id = Some(entity_id);
    let bill = supplier_bill::create(app.db(), &draft, Some(7))
        .await
        .unwrap();
    submit(app.state(), By::System(Some(7)), bill.id)
        .await
        .unwrap();
    approve_bill(app.state(), By::System(Some(8)), bill.id, "")
        .await
        .unwrap()
}

/// A set-off of the blocked party's bill against its invoice waits for the
/// exception like any other money out, and settles both once it is given.
#[renox::test]
async fn a_set_off_to_a_blocked_party_waits_for_the_exception() {
    let app = boot().await;
    let clerk = person(&app, "Amal", ADMIN).await;
    let finance = person(&app, "Fiona", FINANCE_MANAGER).await;
    let party = company(&app, "Gulf Fasteners LLC").await;
    let gbc = entity(&app, "GBC").await;
    let draft = support::draft_invoice(
        &app,
        party,
        wl_kernel::Subject::new("construction.project", 1),
        10_000_000,
        VatCategory::Standard,
        day(10),
    )
    .await;
    app.acting_as(&clerk);
    let issuing = app
        .htmx()
        .post(&format!("/invoices/{}/issue", draft.id), &[])
        .await;
    let text = issuing.text();
    let shown = text.split("<main").nth(1).unwrap_or(&text);
    assert_eq!(issuing.status.as_u16(), 303, "{shown}");
    let invoice = Invoice::find_or_404(app.db(), draft.id).await.unwrap();
    let bill = approved_bill(&app, party, gbc.id).await;
    block(&app, party).await;

    let code = wl_parties::Party::find_or_404(app.db(), party.0)
        .await
        .unwrap()
        .code;
    let entity_id = gbc.id.to_string();
    let invoice_id = invoice.id.to_string();
    let bill_id = bill.id.to_string();
    let form = [
        ("legal_entity_id", entity_id.as_str()),
        ("party_code", code.as_str()),
        ("method", "set_off"),
        ("amount", "5250.00"),
        ("set_off_invoice_id", invoice_id.as_str()),
        ("set_off_bill_id", bill_id.as_str()),
    ];
    app.acting_as(&clerk);
    app.htmx().post("/payments", &form).await.assert_status(303);

    // Nothing is settled yet.
    let waiting = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(waiting.status, InvoiceStatus::Issued);
    let still = SupplierBill::find_or_404(app.db(), bill.id).await.unwrap();
    assert_eq!(still.status, SupplierBillStatus::Approved);
    let requests = ApprovalRequest::query()
        .where_(ApprovalRequest::ACTION.eq(BLOCK_EXCEPTION_ACTION))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1, "one request for the exception");

    approvals::approve(app.state(), requests[0].id, &finance, "approved")
        .await
        .unwrap();
    let settled = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(settled.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(settled.balance, 10_500_000 - 525_000);
    let paid = SupplierBill::find_or_404(app.db(), bill.id).await.unwrap();
    assert_eq!(paid.status, SupplierBillStatus::Paid);
}
