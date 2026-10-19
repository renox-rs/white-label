//! A reversed payment reopens its invoice, end to end (#70 AC1, AC7 and
//! AC9): the demo client has invoicing and payments on, wl-payments reverses
//! through the kernel's `PaymentRecorder` and emits the real
//! `PaymentReversed`, and wl-invoicing reopens the invoice.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{
    Allocation as InvoiceAllocation, Invoice, InvoicingTask, Unallocation, UnallocationOutcome,
};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, invoicer,
};
use wl_kernel::payments::{
    AllocationTarget, PaymentMethod, RecordPayment, ReversalReason, payment_recorder,
};
use wl_kernel::{LegalEntity, PartyId, Subject};
use wl_notify::Message;
use wl_parties::Party;

use host::roles::{self, RECEIVABLES};

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

/// An issued invoice of AED 1,050.00 to `party`, due in 20 days.
async fn issued(app: &TestApp, party: PartyId, by: &User) -> Invoice {
    let due: NaiveDate = today().checked_add_days(Days::new(20)).unwrap();
    let draft = InvoiceDraft::new(party, Subject::new("property.lease", 9))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard))
        .due_date(due);
    let id = invoicer(app.state())
        .draft(app.db(), draft)
        .await
        .unwrap()
        .expect("invoicing is on");
    app.acting_as(by);
    app.htmx()
        .post(&format!("/invoices/{}/issue", id.0), &[])
        .await
        .assert_redirect(&format!("/invoices/{}", id.0));
    Invoice::find_or_404(app.db(), id.0).await.unwrap()
}

async fn reload(app: &TestApp, invoice: &Invoice) -> Invoice {
    Invoice::find_or_404(app.db(), invoice.id).await.unwrap()
}

#[renox::test]
async fn reversing_a_payment_reopens_the_invoice_it_paid() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();
    let party = Party::query()
        .first(app.db())
        .await
        .unwrap()
        .expect("a seeded party");
    let party_id = PartyId(party.id);
    let entity = LegalEntity::query()
        .where_(LegalEntity::CODE.eq("PGP"))
        .first(app.db())
        .await
        .unwrap()
        .expect("the PGP entity");
    let clerk = person(&app, "Wendy", RECEIVABLES).await;
    let invoice = issued(&app, party_id, &clerk).await;
    let id = InvoiceId(invoice.id);
    assert_eq!(invoice.status, InvoiceStatus::Issued);

    let mut request = RecordPayment::new(party_id, entity.id, PaymentMethod::Transfer, 105_000);
    request.reference = Some("TRF-70".to_owned());
    let payment = payment_recorder(app.state())
        .record(app.state(), request, None)
        .await
        .expect("the payment is recorded");
    wl_payments::allocate::allocate(
        app.state(),
        payment.id,
        AllocationTarget::Invoice(id),
        105_000,
        None,
    )
    .await
    .expect("the invoice is paid");
    let paid = reload(&app, &invoice).await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    let number = paid.number.clone();
    let gross = paid.gross_total;

    payment_recorder(app.state())
        .reverse(
            app.state(),
            payment.id,
            ReversalReason::BankReturn,
            None,
            Some(clerk.id),
        )
        .await
        .expect("the payment is reversed");

    // The invoice is open again; its allocation is marked removed, kept.
    let reopened = reload(&app, &invoice).await;
    assert_eq!(reopened.status, InvoiceStatus::Issued);
    assert_eq!((reopened.balance, reopened.allocated_total), (105_000, 0));
    assert_eq!(
        (reopened.number.clone(), reopened.gross_total),
        (number, gross)
    );
    let rows = InvoiceAllocation::query()
        .where_(InvoiceAllocation::INVOICE_ID.eq(invoice.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].is_active());
    assert_eq!(rows[0].removed_reason.as_deref(), Some("payment_reversed"));

    let log = Unallocation::query()
        .where_(Unallocation::INVOICE_ID.eq(invoice.id))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, UnallocationOutcome::Done);
    assert!(
        log[0].key.starts_with("payments.allocation:"),
        "{}",
        log[0].key
    );
    assert_eq!(log[0].reversal_reason.as_deref(), Some("bank_return"));
    assert!(
        InvoicingTask::query()
            .where_(InvoicingTask::INVOICE_ID.eq(invoice.id))
            .get(app.db())
            .await
            .unwrap()
            .is_empty()
    );

    // The owner is told, with the reason, once per channel.
    let messages = Message::query()
        .where_(Message::TYPE_KEY.eq("invoicing.invoice_reopened"))
        .where_(Message::SUBJECT_ID.eq(invoice.id))
        .get(app.db())
        .await
        .unwrap();
    assert!(!messages.is_empty());
    assert!(messages.iter().any(|m| m.user_id == Some(clerk.id)));
    assert!(messages.iter().all(|m| {
        m.data.0["reason"]
            .as_str()
            .is_some_and(|r| r.contains("bank return"))
    }));

    // A second reversal is refused by wl-payments and changes nothing.
    let again = payment_recorder(app.state())
        .reverse(
            app.state(),
            payment.id,
            ReversalReason::BankReturn,
            None,
            Some(clerk.id),
        )
        .await;
    assert!(again.is_err());
    assert_eq!(
        Unallocation::query()
            .where_(Unallocation::INVOICE_ID.eq(invoice.id))
            .count(app.db())
            .await
            .unwrap(),
        1
    );
    let after = reload(&app, &invoice).await;
    assert_eq!(
        (after.status, after.balance),
        (InvoiceStatus::Issued, 105_000)
    );
    let sent = Message::query()
        .where_(Message::TYPE_KEY.eq("invoicing.invoice_reopened"))
        .where_(Message::SUBJECT_ID.eq(invoice.id))
        .count(app.db())
        .await
        .unwrap();
    assert_eq!(sent, messages.len() as u64);
}
