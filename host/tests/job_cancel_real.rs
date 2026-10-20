//! BL-PRP-24 (#274) point 2 with the real modules of the Property client:
//! a job cancelled with "charge tenant" is invoiced to the tenant and the
//! invoice is issued at once (wl-invoicing is set to issue the ticket source
//! automatically), AED 200 plus VAT; a cancel without the tick invoices
//! nothing.

use host::roles::{COORDINATOR, MEMBER, TECHNICIAN};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::Invoice;
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, invoicer};
use wl_kernel::tickets::{Assignee, OpenTicket};
use wl_kernel::{PartyId, PartyKind, Subject};
use wl_parties::{NewParty, PersonDetails, create};
use wl_ticketing::{Ticket, TicketStatus, Tickets};

async fn booted() -> TestApp {
    let app = TestApp::new(host::app()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@example.com", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    host::roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

async fn tenant_party(app: &TestApp) -> PartyId {
    create(
        app.state(),
        NewParty {
            kind: PartyKind::Person,
            legal_name_en: "Tara Nasser".into(),
            person: Some(PersonDetails::default()),
            source: "back_office".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap()
    .party_id()
}

/// A job of the tenant, assigned to the technician, started and put on hold.
async fn held_job(app: &TestApp, technician: &User, tenant: &User, party: PartyId) -> Ticket {
    let opened = Tickets::open_at(
        app.state(),
        OpenTicket::new(Subject::new("property.unit", 12), "general", "AC repair")
            .requester_user(tenant.id)
            .bill_to(party)
            .assign_to(Assignee::User { id: technician.id }),
        renox::db::now(),
    )
    .await
    .unwrap();
    let id = opened.id.0;
    app.acting_as(technician);
    app.htmx()
        .post(&format!("/tickets/{id}/actions/start"), &[])
        .await
        .assert_status(303);
    app.htmx()
        .post(&format!("/tickets/{id}/hold"), &[("hold_reason", "parts")])
        .await
        .assert_ok();
    let held = Ticket::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(held.status, TicketStatus::OnHold);
    held
}

/// AC2: "charge tenant" ticked, reason "tenant damage", one line of AED 200:
/// the invoice is issued to the tenant (number, AED 200 plus VAT), at once.
#[renox::test]
async fn bl_prp_24_the_cancel_with_a_charge_invoice_is_issued_at_once() {
    let app = booted().await;
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let technician = person(&app, "Tim", TECHNICIAN).await;
    let tenant = person(&app, "Tara", MEMBER).await;
    let party = tenant_party(&app).await;
    let held = held_job(&app, &technician, &tenant, party).await;

    app.acting_as(&coordinator);
    app.htmx()
        .post(
            &format!("/tickets/{}/cancel", held.id),
            &[
                ("reason", "tenant damage"),
                ("charge", "1"),
                ("charge_reason", "tenant damage"),
                ("description", "Call-out"),
                ("unit_price", "200"),
            ],
        )
        .await
        .assert_ok();
    let cancelled = Ticket::find_or_404(app.db(), held.id).await.unwrap();
    assert_eq!(cancelled.status, TicketStatus::Cancelled);
    let id = cancelled.invoice_id.expect("the invoice");
    let invoice = Invoice::find_or_404(app.db(), id).await.unwrap();
    assert_eq!(invoice.status, InvoiceStatus::Issued);
    assert!(invoice.number.is_some());
    assert_eq!(invoice.party_id, party.0);
    assert_eq!(invoice.net_total, 20_000);
    assert_eq!(invoice.gross_total, 21_000);
    let summary = invoicer(app.state())
        .find(app.db(), InvoiceId(id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.status, InvoiceStatus::Issued);
}

/// A cancel without the tick invoices nothing.
#[renox::test]
async fn bl_prp_24_a_cancel_without_a_charge_invoices_nothing() {
    let app = booted().await;
    let coordinator = person(&app, "Carla", COORDINATOR).await;
    let technician = person(&app, "Tim", TECHNICIAN).await;
    let tenant = person(&app, "Tara", MEMBER).await;
    let party = tenant_party(&app).await;
    let held = held_job(&app, &technician, &tenant, party).await;
    let before = Invoice::query().count(app.db()).await.unwrap();
    app.acting_as(&coordinator);
    app.htmx()
        .post(
            &format!("/tickets/{}/cancel", held.id),
            &[("reason", "tenant fixed it")],
        )
        .await
        .assert_ok();
    let cancelled = Ticket::find_or_404(app.db(), held.id).await.unwrap();
    assert_eq!(cancelled.status, TicketStatus::Cancelled);
    assert_eq!(cancelled.invoice_id, None);
    assert_eq!(Invoice::query().count(app.db()).await.unwrap(), before);
}
