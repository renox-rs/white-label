//! BL-LOG-09 (#307), BL-LOG-10 (#308), BL-LOG-11 (#309), `FIN.RUL.12`: the
//! invoice of the monthly run is found by the delivery events. The invoice
//! line of a shipment names the shipment as its origin while the delivery
//! carries one line per parcel, so wl-invoicing matches the delivery's
//! source record too. The invoicing module is the real wl-invoicing, the
//! delivery module the real wl-delivery, the shipment the real route.

mod support;

use logistics::{Shipment, billing, rates};
use renox::chrono::NaiveDate;
use renox::db::DateTime;
use renox::prelude::*;
use wl_invoicing::{Invoice, InvoicingTask, TaskKind};
use wl_kernel::PartyId;
use wl_kernel::invoicing::InvoiceStatus;

use support::{World, boot_with_invoicing};

fn at(year: i32, month: u32, day: u32) -> DateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
        .and_utc()
}

/// A shipment of `client` delivered in October and billed on an issued
/// invoice by the November run: the shipment and its invoice.
async fn invoiced(w: &World, client: PartyId, reference: &str) -> (Shipment, Invoice) {
    let shipment = w.delivered(client, reference, "100").await;
    let when = Some(at(2026, 10, 15));
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[("delivered_at", &when)])
        .await
        .unwrap();
    let first = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let drafted = billing::run_monthly(w.state(), first).await.unwrap();
    assert_eq!(drafted.len(), 1);
    let id = drafted[0].invoice.0;
    w.post(&w.docs, &format!("/invoices/{id}/issue"), &[])
        .await
        .assert_status(303);
    let invoice = Invoice::find_or_404(w.db(), id).await.unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Issued);
    (w.reload(&shipment).await, invoice)
}

async fn client_with_price(w: &World) -> PartyId {
    let client = w.client_party("Najm Client LLC").await;
    rates::set_band(w.db(), client, "", 5_000, 2_500)
        .await
        .unwrap();
    client
}

/// AC1 of #307: the issued invoice that bills the disputed delivery is
/// flagged disputed (the half the recording invoicer cannot show), and the
/// upheld review clears it.
#[renox::test]
async fn a_disputed_delivery_flags_the_issued_invoice_of_its_shipment() {
    let w = boot_with_invoicing().await;
    let client = client_with_price(&w).await;
    let (shipment, invoice) = invoiced(&w, client, "ORD-1").await;
    assert!(!invoice.disputed);

    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/dispute", shipment.id),
        &[("reason", "The recipient denies the delivery")],
    )
    .await
    .assert_status(303);
    let flagged = Invoice::find_or_404(w.db(), invoice.id).await.unwrap();
    assert!(flagged.disputed, "the invoice that bills the shipment is disputed");

    let delivery = w.delivery_of(&w.reload(&shipment).await).await;
    w.post(
        &w.dispatcher,
        &format!("/deliveries/{}/close-review", delivery.id),
        &[("outcome", "upheld"), ("note", "The recipient did sign")],
    )
    .await
    .assert_ok();
    let cleared = Invoice::find_or_404(w.db(), invoice.id).await.unwrap();
    assert!(!cleared.disputed, "an upheld review lifts the flag");
}

/// AC2 and AC3 of #308 and #309: a delivery reversed or lost after its
/// dispute gives the invoice owner a task to credit it (`FIN.RUL.12`).
#[renox::test]
async fn a_lost_delivery_gives_the_invoice_owner_a_task_to_credit_it() {
    let w = boot_with_invoicing().await;
    let client = client_with_price(&w).await;
    let (shipment, invoice) = invoiced(&w, client, "ORD-2").await;
    w.post(
        &w.dispatcher,
        &format!("/shipments/{}/dispute", shipment.id),
        &[("reason", "The recipient denies the delivery")],
    )
    .await
    .assert_status(303);

    let delivery = w.delivery_of(&w.reload(&shipment).await).await;
    wl_delivery::lost::mark_lost_as_system(w.state(), &delivery, "No scan for 7 days")
        .await
        .expect("the delivery is lost");
    let tasks = InvoicingTask::query()
        .where_(InvoicingTask::INVOICE_ID.eq(invoice.id))
        .where_(InvoicingTask::KIND.eq(TaskKind::DeliveryLost))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(tasks.len(), 1, "one task to credit the invoice");
}

/// BL-LOG-20 (#318) on the real wl-invoicing: the draft the monthly run makes
/// is a real draft of the client with one line per shipment, the shipment as
/// the line's origin, 5% VAT and the totals the module computes; the shipment
/// is marked billed, and issuing the draft keeps that.
#[renox::test]
async fn the_monthly_draft_is_a_real_invoice_with_the_vat_and_totals() {
    let w = boot_with_invoicing().await;
    let client = client_with_price(&w).await;
    let shipment = w.delivered(client, "ORD-3", "100").await;
    let when = Some(at(2026, 10, 15));
    Shipment::query()
        .where_(Shipment::ID.eq(shipment.id))
        .update(w.db(), &[("delivered_at", &when)])
        .await
        .unwrap();
    let first = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let drafted = billing::run_monthly(w.state(), first).await.unwrap();
    assert_eq!(drafted.len(), 1);

    let invoice = Invoice::find_or_404(w.db(), drafted[0].invoice.0)
        .await
        .unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Draft);
    assert_eq!(invoice.party().0, client.0);
    assert_eq!(invoice.net_total, 2_500);
    assert_eq!(invoice.vat_total, 125);
    assert_eq!(invoice.gross_total, 2_625);
    let lines = wl_invoicing::InvoiceLine::query()
        .where_(wl_invoicing::InvoiceLine::INVOICE_ID.eq(invoice.id))
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].origin(), Some(shipment.subject()));
    assert!(lines[0].description.contains(&shipment.number));
    assert!(w.reload(&shipment).await.invoiced_on.is_some());
    assert_eq!(invoice.source().subject_type, "logistics.billing_run");
}
