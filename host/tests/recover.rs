//! A payment allocated to a written-off invoice, end to end (#66 AC4): the
//! demo client has invoicing and payments on, wl-payments allocates through
//! the kernel's `Invoicer`, and wl-invoicing recovers.

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_invoicing::{Allocation as InvoiceAllocation, Invoice, Recovery};
use wl_kernel::approvals::{self};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{
    DraftLine, InvoiceDraft, InvoiceId, InvoiceStatus, VatCategory, allocation_proposal, invoicer,
};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, RecordPayment, payment_recorder};
use wl_kernel::{LegalEntity, PartyId, Subject};
use wl_parties::Party;

use host::roles::{self, PROPERTY_MANAGER, RECEIVABLES};

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

/// An invoice of AED 1,050.00 to `party`, overdue and written off through
/// the approval of a property manager (FIN.ACL.15 in the Property client).
async fn written_off(app: &TestApp, party: PartyId) -> Invoice {
    let clerk = person(app, "Wendy", RECEIVABLES).await;
    let approver = person(app, "Wade", PROPERTY_MANAGER).await;
    let due: NaiveDate = today().checked_sub_days(Days::new(5)).unwrap();
    let draft = InvoiceDraft::new(party, Subject::new("property.lease", 9))
        .line(DraftLine::new("Rent", 100_000, VatCategory::Standard))
        .due_date(due);
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
    wl_invoicing::overdue::run(app.state()).await.unwrap();
    app.htmx()
        .post(
            &format!("/invoices/{}/write-off", id.0),
            &[("reason_code", "insolvent"), ("reason_text", "")],
        )
        .await
        .assert_ok();
    let invoice = Invoice::find_or_404(app.db(), id.0).await.unwrap();
    let pending = approvals::pending_for(app.db(), &invoice.subject())
        .await
        .unwrap();
    approvals::approve(app.state(), pending[0].id, &approver, "")
        .await
        .unwrap()
        .expect("the approval runs");
    let invoice = Invoice::find_or_404(app.db(), id.0).await.unwrap();
    assert_eq!(invoice.status, InvoiceStatus::WrittenOff);
    assert_eq!(invoice.balance, 105_000);
    invoice
}

/// AC4: AED 2,000.00 is paid for a written-off invoice of AED 1,050.00. The
/// proposal is 1,050.00, 1,050.01 is refused and writes nothing, and
/// allocating the proposal leaves the invoice `paid` and AED 950.00 of
/// credit.
#[renox::test]
async fn a_payment_recovers_a_written_off_invoice() {
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
    let invoice = written_off(&app, party_id).await;
    let id = InvoiceId(invoice.id);

    // The demo data may leave credit of its own to the party.
    let credit_before = payment_recorder(app.state())
        .credit_balance(app.db(), party_id, "AED")
        .await
        .unwrap();
    let mut request = RecordPayment::new(party_id, entity.id, PaymentMethod::Transfer, 200_000);
    request.reference = Some("TRF-66".to_owned());
    let payment = payment_recorder(app.state())
        .record(app.state(), request, None)
        .await
        .expect("the payment is recorded");
    assert_eq!(payment.unallocated, 200_000);
    assert_eq!(
        payment_recorder(app.state())
            .credit_balance(app.db(), party_id, "AED")
            .await
            .unwrap(),
        credit_before + 200_000
    );

    // The proposal for the written-off invoice.
    let summary = invoicer(app.state())
        .find(app.db(), id)
        .await
        .unwrap()
        .expect("the invoice");
    assert_eq!(summary.status, InvoiceStatus::WrittenOff);
    assert_eq!(allocation_proposal(&summary, payment.unallocated), 105_000);

    // More than is left: refused, nothing is written.
    let refused = wl_payments::allocate::allocate(
        app.state(),
        payment.id,
        AllocationTarget::Invoice(id),
        105_001,
        None,
    )
    .await;
    assert!(
        matches!(refused, Err(Error::Status(code, _)) if code == StatusCode::UNPROCESSABLE_ENTITY),
        "{refused:?}"
    );
    let same = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(same.status, InvoiceStatus::WrittenOff);
    assert_eq!(
        (same.balance, same.write_off_recovered_amount),
        (105_000, 0)
    );
    assert!(
        Recovery::of_invoice(app.db(), invoice.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        InvoiceAllocation::query()
            .where_(InvoiceAllocation::INVOICE_ID.eq(invoice.id))
            .count(app.db())
            .await
            .unwrap(),
        0
    );
    let unchanged = payment_recorder(app.state())
        .find(app.db(), payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.unallocated, 200_000);

    // The proposal is allocated.
    wl_payments::allocate::allocate(
        app.state(),
        payment.id,
        AllocationTarget::Invoice(id),
        105_000,
        None,
    )
    .await
    .expect("the written-off invoice is recovered");
    let paid = Invoice::find_or_404(app.db(), invoice.id).await.unwrap();
    assert_eq!(paid.status, InvoiceStatus::Paid);
    assert_eq!(
        (paid.balance, paid.write_off_recovered_amount),
        (0, 105_000)
    );
    let rows = Recovery::of_invoice(app.db(), invoice.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].source(), payment.id.subject());
    let after = payment_recorder(app.state())
        .find(app.db(), payment.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.unallocated, 95_000);
    let credit = payment_recorder(app.state())
        .credit_balance(app.db(), party_id, "AED")
        .await
        .unwrap();
    assert_eq!(credit, credit_before + 95_000);
}
