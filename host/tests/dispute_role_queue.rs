//! BL-TKT-12 (#472) with the real invoicing and ticketing modules: a dispute
//! on an invoice the system issued opens the billing dispute ticket assigned
//! to the receivables role in the invoice's scope, both accountants see it
//! in their queue and are told, one takes it; an invoice an accountant
//! issued still goes to that accountant.

use renox::auth::permissions;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{Dispute, Invoice};
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, VatCategory, invoicer};
use wl_kernel::tickets::Assignee;
use wl_kernel::{PartyId, Subject};
use wl_notify::Message;
use wl_parties::Party;
use wl_ticketing::role_queue::{self, TAKEN_FROM_QUEUE};
use wl_ticketing::{Assignment, Ticket, TicketStatus};

use host::roles::{self, RECEIVABLES};

/// A role that only lets its holders open the ticket pages.
const TICKET_READER: &str = "ticket_reader";

async fn boot() -> TestApp {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    roles::define(app.db()).await.unwrap();
    permissions::define_role(app.db(), TICKET_READER, &[wl_ticketing::permissions::VIEW])
        .await
        .unwrap();
    app
}

/// A verified accountant holding the receivables role and the ticket pages.
async fn accountant(app: &TestApp, name: &str) -> User {
    let email = format!("{}.queue@test.example", name.to_lowercase());
    let mut user = User::register(app.db(), name, &email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    user.assign_role(app.db(), RECEIVABLES).await.unwrap();
    user.assign_role(app.db(), TICKET_READER).await.unwrap();
    user
}

/// An invoice issued by `issuer` through the real page.
async fn issued(app: &TestApp, issuer: &User) -> Invoice {
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let draft = InvoiceDraft::new(PartyId(party.id), Subject::new("property.lease", 31))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard));
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(issuer);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

/// `by` disputes `invoice`; the ticket it opened.
async fn dispute(app: &TestApp, by: &User, invoice: &Invoice) -> Ticket {
    app.acting_as(by);
    app.htmx()
        .post(
            &format!("/invoices/{}/dispute", invoice.id),
            &[("reason", "Wrong rent"), ("amount", "100.00")],
        )
        .await
        .assert_ok();
    let invoice = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    let dispute = Dispute::find(app.db(), invoice.dispute_id.expect("dispute id"))
        .await
        .unwrap()
        .expect("the dispute");
    Ticket::find_or_404(app.db(), dispute.ticket_id.expect("a ticket"))
        .await
        .unwrap()
}

/// Done when 1 and 2.
#[renox::test]
async fn a_system_issued_invoice_disputes_into_the_receivables_queue_and_omar_takes_it() {
    let app = boot().await;
    let nadia = accountant(&app, "Nadia").await;
    let omar = accountant(&app, "Omar").await;
    let invoice = issued(&app, &nadia).await;
    // Issued by the daily rent run: the system owns it.
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(app.db(), &[("owner_id", &None::<i64>)])
        .await
        .unwrap();

    let ticket = dispute(&app, &nadia, &invoice).await;
    assert_eq!(ticket.status, TicketStatus::Assigned);
    assert_eq!(ticket.assignee(), None, "never the lowest user id");
    assert_eq!(ticket.assignee_role.as_deref(), Some("receivables"));
    let display = ticket.assignee_role_display.clone().unwrap_or_default();
    assert!(display.starts_with("Receivables"), "{display}");
    let response_due = ticket.response_due.expect("the SLA started");

    for user in [&nadia, &omar] {
        let queue = role_queue::queue_of(app.db(), user.id).await.unwrap();
        assert!(
            queue.iter().any(|q| q.id == ticket.id),
            "in {}'s queue",
            user.name
        );
        app.acting_as(user);
        app.get("/tickets?queue=mine")
            .await
            .assert_ok()
            .assert_see(&ticket.number);
    }
    let told: Vec<Option<i64>> = Message::query()
        .where_(Message::TYPE_KEY.eq("ticketing.assigned"))
        .where_(Message::SUBJECT_ID.eq(ticket.id))
        .get(app.db())
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.user_id)
        .collect();
    assert!(
        told.contains(&Some(nadia.id)) && told.contains(&Some(omar.id)),
        "both are told: {told:?}"
    );

    app.acting_as(&omar);
    app.post(&format!("/tickets/{}/take", ticket.id), &[])
        .await
        .assert_redirect(&format!("/tickets/{}", ticket.id));
    let taken = Ticket::find_or_404(app.db(), ticket.id).await.unwrap();
    assert_eq!(taken.assignee(), Some(Assignee::user(omar.id)));
    assert_eq!(
        taken.response_due,
        Some(response_due),
        "the SLA is unchanged"
    );
    let history = Assignment::history(app.db(), ticket.id).await.unwrap();
    assert_eq!(
        history.last().and_then(|a| a.reason_code.as_deref()),
        Some(TAKEN_FROM_QUEUE)
    );
    assert!(
        !role_queue::queue_of(app.db(), nadia.id)
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == ticket.id),
        "it left Nadia's queue"
    );
}

/// Done when 3: an invoice Nadia issued still opens the ticket assigned to
/// Nadia.
#[renox::test]
async fn an_invoice_nadia_issued_goes_to_nadia() {
    let app = boot().await;
    let nadia = accountant(&app, "Nadia").await;
    let _omar = accountant(&app, "Omar").await;
    let invoice = issued(&app, &nadia).await;
    let ticket = dispute(&app, &nadia, &invoice).await;
    assert_eq!(ticket.assignee(), Some(Assignee::user(nadia.id)));
    assert_eq!(ticket.assignee_role, None);
}

/// Done when 6: staff without the receivables role do not see the role
/// ticket in the list.
#[renox::test]
async fn staff_without_the_role_do_not_see_the_role_ticket() {
    let app = boot().await;
    let nadia = accountant(&app, "Nadia").await;
    let invoice = issued(&app, &nadia).await;
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(app.db(), &[("owner_id", &None::<i64>)])
        .await
        .unwrap();
    let ticket = dispute(&app, &nadia, &invoice).await;

    let mut sam = User::register(app.db(), "Sam", "sam.queue@test.example", "secret123")
        .await
        .unwrap();
    sam.email_verified_at = Some(renox::db::now());
    sam.save(app.db()).await.unwrap();
    sam.assign_role(app.db(), TICKET_READER).await.unwrap();
    app.acting_as(&sam);
    app.get("/tickets")
        .await
        .assert_ok()
        .assert_dont_see(&ticket.number);
    app.get(&format!("/tickets/{}", ticket.id))
        .await
        .assert_not_found();
}
