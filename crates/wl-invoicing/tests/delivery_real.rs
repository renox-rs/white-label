//! A disputed delivery with the real wl-delivery and wl-ticketing modules
//! (BL-INV-06 #69, BL-DLV-04 #160): no scripted events. A dispatcher puts a
//! delivered delivery under review on the delivery page; wl-delivery opens
//! its ticket and emits `DeliveryDisputed`; wl-invoicing flags the issued
//! invoice that bills its lines with that very ticket (no second ticket);
//! closing the review as upheld resolves the dispute and the ticket.

mod support;

use renox::auth::{Permissions, permissions};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_delivery::{Deliveries, Delivery, NewRun, VehicleType, WlDelivery};
use wl_delivery::permissions as dlv;
use wl_invoicing::dispute::Dispute;
use wl_invoicing::{Invoice, WlInvoicing, issue};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DisputeOutcome, DraftLine, InvoiceDraft, VatCategory};
use wl_kernel::{ClientConfig, DeliveryItem, DeliveryRequest, Subject};
use wl_ticketing::{Ticket, TicketStatus, WlTicketing};

use support::{BLUE_DUNES, Recording, TestDirectory};

const LAYOUT: &str = r#"<!doctype html>
<html><body><rx-outlet name="content"/></body></html>
"#;

const CLIENT: &str = r#"
[client]
name = "Gulf Express Logistics LLC"
app = "logistics"

[modules]
enabled = ["parties", "invoicing", "delivery", "ticketing"]

[[legal_entities]]
code = "GEL"
name_en = "Gulf Express Logistics LLC"
trn = "100000000000003"
default = true
"#;

struct Real {
    app: TestApp,
    _views: tempfile::TempDir,
}

async fn boot() -> Real {
    let views = tempfile::tempdir().expect("a temporary views directory");
    let path = views.path().join("layouts/app.html");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, LAYOUT).unwrap();
    let directory: std::sync::Arc<dyn wl_kernel::parties::PartyDirectory> =
        std::sync::Arc::new(TestDirectory::default());
    let notifier: std::sync::Arc<dyn wl_kernel::notify::Notifier> =
        std::sync::Arc::new(Recording::default());
    let app = App::new()
        .module(Auth::new())
        .module(renox::audit::Audit)
        .module(Permissions)
        .module(WlInvoicing::new().auto_issue_source("sales.order"))
        .module(WlDelivery::new())
        .module(WlTicketing::new())
        .provide(directory)
        .provide(notifier);
    let config = ClientConfig::parse(CLIENT).expect("a valid client.toml");
    let app = wl_kernel::apply(app, config).expect("the kernel applies");
    let dir = views.path().to_path_buf();
    let app = TestApp::with_config(app, |c| c.views_path = dir).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.kernel()
        .call("ticketing:install", Vec::<String>::new())
        .await
        .expect("ticketing:install runs");
    wl_delivery::seed::install(app.db())
        .await
        .expect("the delivery defaults seed");
    Real { app, _views: views }
}

async fn user(t: &Real, name: &str, grants: &[&str]) -> User {
    let email = format!("{}@example.com", name.to_lowercase());
    let user = User::register(t.app.db(), name, &email, "password123")
        .await
        .unwrap();
    let role = format!("role_{}", name.to_lowercase());
    permissions::define_role(t.app.db(), &role, grants)
        .await
        .unwrap();
    user.assign_role(t.app.db(), &role).await.unwrap();
    user
}

async fn post(t: &Real, by: &User, path: &str, fields: &[(&str, &str)]) -> renox::testing::TestResponse {
    t.app.acting_as(by);
    t.app.htmx().post(path, fields).await
}

/// A delivery of lines 11 (AED 800) and 12 (AED 2,200) of order 9, delivered
/// by the driver, and the draft invoice that bills them, issued.
async fn delivered_and_invoiced(t: &Real) -> (Delivery, Invoice, User, User) {
    let dispatcher = user(
        t,
        "Dina",
        &[dlv::VIEW, dlv::DISPATCH, dlv::MANAGE, dlv::RUNS_VIEW, dlv::RUNS_DISPATCH],
    )
    .await;
    let driver = user(t, "Dev", &[dlv::RUNS_DRIVE, dlv::COMPLETE]).await;
    let request = DeliveryRequest::new(Subject::new("sales.order", 9), "Mariam Al Hosani")
        .address("Villa 12, Al Barsha 2", "Al Barsha")
        .item(
            DeliveryItem::new("Tiles", 10_000)
                .source_line(Subject::new("sales.order_line", 11))
                .amount(80_000),
        )
        .item(
            DeliveryItem::new("Grout", 5_000)
                .source_line(Subject::new("sales.order_line", 12))
                .amount(220_000),
        );
    let delivery = Deliveries::request(t.app.state(), request).await.unwrap();
    let run = wl_delivery::plan::create_run(
        t.app.state(),
        NewRun::new(today(), driver.id).vehicle(VehicleType::OnFoot, None),
    )
    .await
    .unwrap();
    let run_id = run.id.to_string();
    post(t, &dispatcher, &format!("/deliveries/{}/assign", delivery.id), &[("run_id", &run_id)])
        .await
        .assert_ok();
    post(t, &dispatcher, &format!("/runs/{}/dispatch", run.id), &[])
        .await
        .assert_ok();
    let lines = delivery.lines(t.app.db()).await.unwrap();
    let a = format!("delivered_{}", lines[0].id);
    let b = format!("delivered_{}", lines[1].id);
    let ra = format!("refused_{}", lines[0].id);
    let rb = format!("refused_{}", lines[1].id);
    post(
        t,
        &driver,
        &format!("/driver/stops/{}/deliver", delivery.id),
        &[
            ("recipient_name", "Mariam Al Hosani"),
            ("photo", "photo-1"),
            ("signature", "signature-1"),
            ("latitude", "25.204849"),
            ("longitude", "55.270783"),
            ("cash_collected", "3000.00"),
            (&a, "10"),
            (&ra, "0"),
            (&b, "5"),
            (&rb, "0"),
        ],
    )
    .await
    .assert_status(303);
    let delivered = Delivery::find_or_404(t.app.db(), delivery.id).await.unwrap();
    assert_eq!(delivered.status.as_str(), "delivered");

    let mut draft = InvoiceDraft::new(BLUE_DUNES, Subject::new("sales.order", 9));
    for (n, price) in [(11, 80_000), (12, 220_000)] {
        draft = draft.line(
            DraftLine::new(&format!("Order line {n}"), price, VatCategory::ZeroRated)
                .origin(Subject::new("sales.order_line", n)),
        );
    }
    let id = wl_kernel::invoicing::invoicer(t.app.state())
        .draft(t.app.db(), draft)
        .await
        .unwrap()
        .unwrap();
    let draft = Invoice::find_or_404(t.app.db(), id.0).await.unwrap();
    issue::issue_by_system(t.app.state(), &draft).await.unwrap();
    let invoice = Invoice::find_or_404(t.app.db(), id.0).await.unwrap();
    (delivered, invoice, dispatcher, driver)
}


/// The dispatcher puts the delivery under review on its page: the ticket
/// wl-delivery opened is the dispute's ticket, the invoice is flagged for
/// the delivery's lines (AED 3,000.00 here) and no second ticket exists;
/// closing the review as upheld resolves the dispute and the ticket.
#[renox::test]
async fn a_review_on_the_delivery_page_flags_the_invoice_and_closing_it_resolves_both() {
    let t = boot().await;
    let (delivery, invoice, dispatcher, _driver) = delivered_and_invoiced(&t).await;
    assert_eq!(invoice.gross_total, 300_000);

    post(
        &t,
        &dispatcher,
        &format!("/deliveries/{}/open-review", delivery.id),
        &[("reason", "recipient denies")],
    )
    .await
    .assert_ok();

    let under_review = Delivery::find_or_404(t.app.db(), delivery.id).await.unwrap();
    assert_eq!(under_review.status.as_str(), "under_review");
    let ticket_id = under_review.review_ticket_id.expect("a real ticket was opened");
    let flagged = Invoice::find_or_404(t.app.db(), invoice.id).await.unwrap();
    assert!(flagged.disputed);
    let dispute = Dispute::find_or_404(t.app.db(), flagged.dispute_id.unwrap())
        .await
        .unwrap();
    assert_eq!(dispute.reason, "delivery disputed");
    assert_eq!(dispute.amount, 300_000);
    assert_eq!(dispute.ticket_id, Some(ticket_id));
    let tickets = Ticket::query().get(t.app.db()).await.unwrap();
    assert_eq!(tickets.len(), 1, "the delivery's ticket is the only one");
    assert_eq!(tickets[0].id, ticket_id);

    post(
        &t,
        &dispatcher,
        &format!("/deliveries/{}/close-review", delivery.id),
        &[("outcome", "upheld"), ("note", "Photo and signature match")],
    )
    .await
    .assert_ok();

    let closed = Delivery::find_or_404(t.app.db(), delivery.id).await.unwrap();
    assert_eq!(closed.status.as_str(), "delivered");
    assert!(!closed.frozen);
    let released = Invoice::find_or_404(t.app.db(), invoice.id).await.unwrap();
    assert!(!released.disputed);
    let settled = Dispute::find_or_404(t.app.db(), dispute.id).await.unwrap();
    assert_eq!(settled.outcome, Some(DisputeOutcome::Upheld));
    let ticket = Ticket::find_or_404(t.app.db(), ticket_id).await.unwrap();
    assert_eq!(ticket.status, TicketStatus::Resolved);
}
