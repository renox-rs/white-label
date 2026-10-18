//! The credit note pages and the credit actions on the invoice page render
//! and post for a user with `invoices.credit`, show to a user with only
//! `invoices.view` and are closed to everyone else (#71 part 6).

use renox::auth::permissions;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::permissions as perm;
use wl_invoicing::{CreditNote, Invoice};
use wl_kernel::invoicing::{
    CreditNoteStatus, DraftLine, InvoiceDraft, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::{PartyId, Subject};
use wl_parties::Party;

use host::roles::{self, MEMBER, RECEIVABLES};

/// A verified user with `role`.
async fn person(app: &TestApp, name: &str, role: &str) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@test.example", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

/// An issued invoice of AED 1,050.00 (one line of AED 1,000.00 and VAT).
async fn issued(app: &TestApp, party: PartyId, origin: i64, clerk: &User) -> Invoice {
    let draft = InvoiceDraft::new(party, Subject::new("property.lease", origin))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard));
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(clerk);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

#[renox::test]
async fn the_credit_note_pages_and_actions_follow_the_credit_permission() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let party = PartyId(party.id);

    // Closed to a member, shown to a user who may only view invoices.
    let member = person(&app, "Maya", MEMBER).await;
    app.acting_as(&member);
    app.get("/credit-notes").await.assert_forbidden();
    let viewer = User::register(app.db(), "Vera", "vera@test.example", "secret123")
        .await
        .unwrap();
    permissions::define_role(app.db(), "credit_viewer", &[perm::VIEW])
        .await
        .unwrap();
    viewer.assign_role(app.db(), "credit_viewer").await.unwrap();
    app.acting_as(&viewer);
    app.get("/credit-notes")
        .await
        .assert_ok()
        .assert_see("Credit notes");
    app.htmx()
        .post("/invoices/1/credit", &[("reason", "No")])
        .await
        .assert_forbidden();
    app.htmx()
        .post("/invoices/1/credit-notes", &[("reason", "No")])
        .await
        .assert_forbidden();
    app.htmx()
        .post("/credit-notes/1/refund", &[("amount", "1")])
        .await
        .assert_forbidden();

    // A new credit note of AED 200.00 on the line: gross AED 210.00, used
    // on the invoice, so the credit note is applied.
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let invoice = issued(&app, party, 11, &clerk).await;
    app.get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see("Credit in full")
        .assert_see("New credit note");
    app.htmx()
        .post(
            &format!("/invoices/{}/credit-notes", invoice.id),
            &[("net_1", "200.00"), ("reason", "Damaged goods")],
        )
        .await
        .assert_ok();
    let note = CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(invoice.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the credit note");
    assert_eq!(note.gross_total, 21_000);
    assert_eq!(note.status, CreditNoteStatus::Applied);
    let number = note.number.clone().expect("a number");

    app.get("/credit-notes")
        .await
        .assert_ok()
        .assert_see(&number);
    app.get(&format!("/credit-notes/{}", note.id))
        .await
        .assert_ok()
        .assert_see(&number)
        .assert_see("Applied")
        .assert_see("AED 210.00")
        .assert_see("Used on an invoice")
        .assert_see("VAT by category");
    app.get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see(&number)
        .assert_dont_see("Credit in full");

    // Refusals answer 422: above the line's remaining credit, a blank
    // reason, a text that is not an amount, a full credit with an
    // allocation, a refund or a use of an applied credit note.
    let id = invoice.id.to_string();
    let credit_notes = format!("/invoices/{}/credit-notes", invoice.id);
    app.htmx()
        .post(
            &credit_notes,
            &[("net_1", "900.00"), ("reason", "Too much")],
        )
        .await
        .assert_status(422);
    app.htmx()
        .post(&credit_notes, &[("net_1", "10"), ("reason", " ")])
        .await
        .assert_status(422);
    app.htmx()
        .post(&credit_notes, &[("net_1", "ten"), ("reason", "Typo")])
        .await
        .assert_status(422);
    app.htmx()
        .post(
            &format!("/invoices/{}/credit", invoice.id),
            &[("reason", "Cancelled")],
        )
        .await
        .assert_status(422);
    app.htmx()
        .post(
            &format!("/credit-notes/{}/refund", note.id),
            &[("amount", "1.00"), ("reason", "Back")],
        )
        .await
        .assert_status(422);
    app.htmx()
        .post(
            &format!("/credit-notes/{}/use", note.id),
            &[("invoice_id", id.as_str()), ("amount", "1.00")],
        )
        .await
        .assert_status(422);

    // Another invoice credited in full: it is `credited` and its credit
    // note is applied, full and for the whole gross.
    let second = issued(&app, party, 12, &clerk).await;
    app.htmx()
        .post(
            &format!("/invoices/{}/credit", second.id),
            &[("reason", "Lease cancelled"), ("release_origins", "on")],
        )
        .await
        .assert_ok();
    let second = Invoice::find_or_404(app.db(), second.id).await.unwrap();
    assert_eq!(second.status, InvoiceStatus::Credited);
    let full = CreditNote::query()
        .where_(CreditNote::INVOICE_ID.eq(second.id))
        .first(app.db())
        .await
        .unwrap()
        .expect("the full credit note");
    assert_eq!(full.gross_total, second.gross_total);
    assert!(full.release_origins);
    app.get(&format!("/credit-notes/{}", full.id))
        .await
        .assert_ok()
        .assert_see("Applied")
        .assert_see("Invoice cancelled");
}

/// #71 AC6 end to end, with wl-invoicing and wl-payments both on: a refund
/// requested from a credit note is approved and then fails (`RefundFailed`,
/// as a gateway decline or a bank return sends it); the credit note is
/// `issued` again with its whole remaining balance, once. The refund is
/// real; the failure is the event, because a test cannot pay a refund as a
/// user, and a gateway refund needs a transaction a credit note has not.
#[renox::test]
async fn a_failed_refund_gives_the_credit_note_back_through_payments() {
    use wl_invoicing::credit_note_movement::{CreditNoteMovement, MovementKind};
    use wl_invoicing::credit_notes::By;
    use wl_invoicing::refund::request_refund;
    use wl_kernel::invoicing::{CreditLine, InvoiceId};
    use wl_kernel::payments::{
        PaymentMethod, RefundAttemptRef, RefundFailed, RefundFailedFrom, RefundFailureKind,
        RefundSource,
    };
    use wl_payments::Refund;
    use wl_payments::refund::RefundStatus;

    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a party");
    let party = PartyId(party.id);
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let requester = person(&app, "Cara", roles::ACCOUNTANT).await;
    let approver = person(&app, "Cleo", roles::ACCOUNTANT).await;

    // A paid invoice, credited for AED 500.00 net: all of it stays open.
    let invoice = issued(&app, party, 21, &clerk).await;
    invoicer(app.state())
        .allocate(
            app.state(),
            InvoiceId(invoice.id),
            Subject::new("payments.payment", 1),
            invoice.gross_total,
        )
        .await
        .unwrap();
    let id = invoicer(app.state())
        .credit_partly(
            app.state(),
            InvoiceId(invoice.id),
            vec![CreditLine::new(1, 50_000)],
            "Service not delivered".into(),
            Some(clerk.id),
        )
        .await
        .unwrap();
    let note = CreditNote::find_or_404(app.db(), id.0).await.unwrap();
    assert_eq!(note.status, CreditNoteStatus::Issued);
    let gross = note.gross_total;
    assert_eq!(note.remaining, gross);

    // The refund takes it all; payments holds the request.
    let refund = request_refund(
        app.state(),
        By::System(Some(requester.id)),
        &note,
        gross,
        Some(PaymentMethod::Transfer),
        "Customer left",
    )
    .await
    .unwrap();
    let applied = CreditNote::find_or_404(app.db(), note.id).await.unwrap();
    assert_eq!(applied.status, CreditNoteStatus::Applied);
    assert_eq!(applied.remaining, 0);

    app.acting_as(&approver);
    app.htmx()
        .post(&format!("/refunds/{}/transitions/approve", refund.0), &[])
        .await
        .assert_status(303);
    assert_eq!(
        Refund::find_or_404(app.db(), refund.0)
            .await
            .unwrap()
            .status,
        RefundStatus::Approved
    );

    // The refund fails (the event a gateway decline or a bank return
    // sends): the credit note is open again, with its number, lines and
    // totals as they were.
    let event = RefundFailed {
        refund,
        party,
        amount: gross,
        currency: "AED".into(),
        source: RefundSource::CreditNote(note.subject()),
        method: PaymentMethod::Transfer,
        kind: RefundFailureKind::CardUnavailable,
        reason: "card expired".into(),
        attempt: RefundAttemptRef { id: 1, number: 1 },
        from: RefundFailedFrom::Approved,
        outgoing_payment: None,
    };
    app.state().emit(event.clone()).await.unwrap();
    let reopened = CreditNote::find_or_404(app.db(), note.id).await.unwrap();
    assert_eq!(reopened.status, CreditNoteStatus::Issued);
    assert_eq!(reopened.remaining, gross);
    assert_eq!(reopened.number, note.number);
    assert_eq!(
        (reopened.net_total, reopened.vat_total, reopened.gross_total),
        (note.net_total, note.vat_total, note.gross_total)
    );
    let restored = CreditNoteMovement::query()
        .where_(CreditNoteMovement::CREDIT_NOTE_ID.eq(note.id))
        .where_(CreditNoteMovement::KIND.eq(MovementKind::RefundRestored))
        .count(app.db())
        .await
        .unwrap();
    assert_eq!(restored, 1);

    // Delivered again: nothing changes.
    app.state().emit(event).await.unwrap();
    let again = CreditNote::find_or_404(app.db(), note.id).await.unwrap();
    assert_eq!(again.remaining, gross);
}
