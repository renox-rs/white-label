//! BL-PRP-26 (#276) items 14 and 15 with the real modules of the host: the
//! reversal of a payment (the *void* of a payment entered in error, and the
//! reversal of a confirmed bank match) is a request that waits for a property
//! manager (or the company admin) who did not ask; the requester cannot
//! approve it, an accountant cannot approve it; at the approval the
//! allocations are removed and the invoices reopen.

mod support;

use host::roles::{ACCOUNTANT, ADMIN, PROPERTY_MANAGER, RECEIVABLES};
use renox::prelude::*;
use renox::testing::TestApp;
use support::*;
use wl_invoicing::Invoice;
use wl_kernel::approvals::{self, ApprovalRequest};
use wl_kernel::invoicing::{InvoiceId, InvoiceStatus, VatCategory};
use wl_kernel::payments::{AllocationTarget, PaymentMethod, PaymentStatus, RecordPayment};
use wl_kernel::{PartyId, payments::payment_recorder};
use wl_payments::{Allocation, Payment};

/// An issued invoice of AED 1,000 and a transfer of the same amount allocated
/// to it: the invoice is paid.
async fn paid(app: &TestApp, clerk: &User, tenant: PartyId) -> (Invoice, Payment) {
    let invoice = issued_invoice(app, clerk, tenant, 100_000, VatCategory::Exempt, day(10)).await;
    let entity = entity(app, "PGP").await;
    let mut request = RecordPayment::new(tenant, entity.id, PaymentMethod::Transfer, 100_000);
    request.reference = Some("TRF-1".to_owned());
    let payment = payment_recorder(app.state())
        .record(app.state(), request, None)
        .await
        .expect("the payment is recorded");
    wl_payments::allocate::allocate(
        app.state(),
        payment.id,
        AllocationTarget::Invoice(InvoiceId(invoice.id)),
        100_000,
        None,
    )
    .await
    .expect("the invoice is paid");
    assert_eq!(reload(app, &invoice).await.status, InvoiceStatus::Paid);
    let payment = Payment::find_or_404(app.db(), payment.id.0).await.unwrap();
    (invoice, payment)
}

async fn request_of(app: &TestApp, payment: &Payment) -> ApprovalRequest {
    let mut pending = approvals::pending_for(app.db(), &payment.subject())
        .await
        .unwrap();
    assert_eq!(pending.len(), 1, "one pending reversal");
    pending.remove(0)
}

/// Item 14: accountant A asks to void a payment entered twice (*entered in
/// error*); the request waits; A cannot approve it, accountant B cannot
/// either; property manager P approves it and the allocations are reversed
/// and the invoice reopens.
#[renox::test]
async fn a_payment_void_is_approved_by_another_user_and_reverses_its_allocations() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let a = person(&app, "Amal", ACCOUNTANT).await;
    let b = person(&app, "Bilal", ACCOUNTANT).await;
    let manager = person(&app, "Pam", PROPERTY_MANAGER).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let (invoice, payment) = paid(&app, &clerk, tenant).await;

    app.acting_as(&a);
    app.htmx()
        .post(
            &format!("/payments/{}/reverse", payment.id),
            &[("reason", "entered_in_error"), ("note", "Entered twice")],
        )
        .await
        .assert_ok();
    // Nothing changed yet.
    let waiting = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(waiting.status, PaymentStatus::Received);
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Paid);
    let request = request_of(&app, &payment).await;
    assert_eq!(request.requester_id, a.id);
    assert_eq!(request.action, "payments.reverse");

    // A may not approve it, and an accountant is no approver at all.
    let own = approvals::approve(app.state(), request.id, &a, "").await;
    assert!(matches!(own, Err(Error::Forbidden)), "{own:?}");
    let other = approvals::approve(app.state(), request.id, &b, "").await;
    assert!(matches!(other, Err(Error::Forbidden)), "{other:?}");
    assert_eq!(
        Payment::find_or_404(app.db(), payment.id)
            .await
            .unwrap()
            .status,
        PaymentStatus::Received
    );

    // The property manager approves: the payment is reversed, its allocation
    // is removed (kept, marked) and the invoice is open again.
    approvals::approve(app.state(), request.id, &manager, "agreed")
        .await
        .unwrap()
        .expect("the approval runs");
    let reversed = Payment::find_or_404(app.db(), payment.id).await.unwrap();
    assert_eq!(reversed.status, PaymentStatus::Reversed);
    assert_eq!(reversed.reversed_by, Some(a.id));
    let rows = Allocation::all_of(app.db(), payment.id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].removed_at.is_some(), "the allocation is removed");
    let reopened = reload(&app, &invoice).await;
    assert_eq!(reopened.status, InvoiceStatus::Issued);
    assert_eq!((reopened.balance, reopened.allocated_total), (100_000, 0));
}

/// Item 15: the reversal of a confirmed match needs a property manager: the
/// accountant who confirmed it cannot complete the reversal; the company admin
/// or a property manager can, and the invoice reopens.
#[renox::test]
async fn only_a_property_manager_or_the_admin_completes_the_reversal_of_a_match() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let accountant = person(&app, "Amal", ACCOUNTANT).await;
    let admin = person(&app, "Alia", ADMIN).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let (invoice, payment) = paid(&app, &clerk, tenant).await;

    app.acting_as(&accountant);
    app.htmx()
        .post(
            &format!("/payments/{}/reverse", payment.id),
            &[("reason", "entered_in_error"), ("note", "Wrong match")],
        )
        .await
        .assert_ok();
    let request = request_of(&app, &payment).await;
    let refused = approvals::approve(app.state(), request.id, &accountant, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Paid);

    approvals::approve(app.state(), request.id, &admin, "")
        .await
        .unwrap()
        .expect("the approval runs");
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);
    assert_eq!(
        Payment::find_or_404(app.db(), payment.id)
            .await
            .unwrap()
            .status,
        PaymentStatus::Reversed
    );
}

/// A reversal by the system (a bank return, a failed refund) is not a user's
/// request and runs at once.
#[renox::test]
async fn a_reversal_by_the_system_is_not_held_for_approval() {
    let app = booted().await;
    let clerk = person(&app, "Clara", RECEIVABLES).await;
    let tenant = tenant(&app, "Tara Nasser").await;
    let (invoice, payment) = paid(&app, &clerk, tenant).await;
    payment_recorder(app.state())
        .reverse(
            app.state(),
            payment.payment_id(),
            wl_kernel::payments::ReversalReason::BankReturn,
            None,
            Some(clerk.id),
        )
        .await
        .expect("the payment is reversed");
    assert_eq!(reload(&app, &invoice).await.status, InvoiceStatus::Issued);
}
