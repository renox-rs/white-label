//! A dispute with the real ticketing module on (#67 part 7, AC1, AC6 and
//! AC13): the `billing_dispute` ticket is opened on the invoice, assigned to
//! its owner, linked to the dispute, and resolved when the dispute is.

use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{Dispute, Invoice};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::tickets::Assignee;
use wl_kernel::{PartyId, Subject};
use wl_parties::Party;
use wl_ticketing::{Category, Ticket, TicketStatus};

use host::roles::{self, RECEIVABLES};

#[renox::test]
async fn the_dispute_opens_a_real_ticket_and_resolving_it_resolves_the_ticket() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    roles::define(app.db()).await.unwrap();
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");

    let mut clerk = User::register(app.db(), "Clara", "clara.ticket@test.example", "secret123")
        .await
        .unwrap();
    clerk.email_verified_at = Some(renox::db::now());
    clerk.save(app.db()).await.unwrap();
    clerk.assign_role(app.db(), RECEIVABLES).await.unwrap();

    let draft = InvoiceDraft::new(PartyId(party.id), Subject::new("property.lease", 31))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard))
        .line(DraftLine::new("Parking", 50_000, VatCategory::Standard));
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(&clerk);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));

    // The category is seeded because invoicing is on.
    let category = Category::by_key(app.db(), "billing_dispute")
        .await
        .unwrap()
        .expect("the billing dispute category");

    app.htmx()
        .post(
            &format!("/invoices/{}/dispute", id.0),
            &[("reason", "Wrong hours"), ("amount", "300.00")],
        )
        .await
        .assert_ok();
    let invoice = Invoice::find_or_404(app.db(), id.0).await.unwrap();
    assert!(invoice.disputed);
    let dispute = Dispute::find(app.db(), invoice.dispute_id.expect("dispute id"))
        .await
        .unwrap()
        .expect("the dispute");
    let ticket = Ticket::find_or_404(app.db(), dispute.ticket_id.expect("a ticket"))
        .await
        .unwrap();
    assert_eq!(ticket.category_id, category.id);
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.assignee(), Some(Assignee::user(clerk.id)));
    assert_eq!(ticket.subject_type, invoice.subject().subject_type);
    assert_eq!(ticket.subject_id, invoice.id);
    assert_eq!(ticket.source_type.as_deref(), Some("invoicing.dispute"));
    assert_eq!(ticket.source_id, Some(dispute.id));
    assert_eq!(ticket.requester_party_id, Some(party.id));
    assert!(ticket.description.contains("Wrong hours"));
    assert!(ticket.description.contains("AED 300.00"));
    app.get(&format!("/invoices/{}", id.0))
        .await
        .assert_ok()
        .assert_see(&ticket.number);

    // Upheld: the flag clears and the ticket is resolved with the note.
    app.htmx()
        .post(
            &format!("/invoices/{}/dispute/resolve", id.0),
            &[("outcome", "upheld"), ("note", "The hours are right")],
        )
        .await
        .assert_ok();
    assert!(!Invoice::find_or_404(app.db(), id.0).await.unwrap().disputed);
    let ticket = Ticket::find_or_404(app.db(), ticket.id).await.unwrap();
    assert_eq!(ticket.status, TicketStatus::Resolved);
    assert!(ticket.resolved_at.is_some());
}
