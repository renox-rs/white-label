//! Closing a payer bill from draft and voiding it (wl-invoicing §4 *Payer
//! bill* `transfer_to_customer`, `write_off` and `void`, §5.8 step 5,
//! `FIN.ACL.14`, `PLT.ACL.09`, #81): the acceptance checks of BL-INV-18,
//! driven through the payer bill pages, the approval kernel and the
//! kernel's void route.

mod support;

use std::sync::atomic::{AtomicI64, Ordering};

use renox::chrono::{Days, NaiveDate};
use renox::prelude::*;
use wl_invoicing::{Invoice, PayerAgreement, PayerBill, VatRecipient};
use wl_kernel::Subject;
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::invoicing::{DraftLine, InvoiceDraft, InvoiceStatus, VatCategory};
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::payer_bills::{
    NewPayerBill, NewPayerBillLine, PayerBillId, PayerBillOutcome, PayerBillStatus, payer_bills,
};
use wl_kernel::voids::VoidRecord;

use support::{BLUE_DUNES, Booted, OMAR, boot};

/// The next visit: each customer invoice is for a visit of its own.
fn next_visit() -> i64 {
    static VISIT: AtomicI64 = AtomicI64::new(500);
    VISIT.fetch_add(1, Ordering::SeqCst)
}

/// A day whose first-submission window (30 days) ended 10 days ago.
fn missed() -> NaiveDate {
    today() - Days::new(40)
}

/// A 05 visit: the customer's tax invoice for `payer_share` (the lines the
/// payer is asked to pay) plus `patient_share`, issued at the visit, with
/// the patient's share paid. The payer lines are line 1.
async fn visit_invoice(t: &Booted, payer_share: i64, patient_share: i64) -> Invoice {
    let clerk = t.user_a().await;
    let draft = t
        .draft_of(
            InvoiceDraft::new(OMAR, Subject::new("clinics.visit", next_visit()))
                .line(DraftLine::new(
                    "Consultation and tests",
                    payer_share,
                    VatCategory::ZeroRated,
                ))
                .line(DraftLine::new(
                    "Co-payment",
                    patient_share,
                    VatCategory::ZeroRated,
                )),
        )
        .await;
    let invoice = t.issue(&draft, &clerk).await;
    t.pay(&invoice, 1, patient_share).await;
    t.reload(&invoice).await
}

/// The 05 payer agreement: the customer is the recipient, a write-off above
/// AED 1,000.00 needs approval, late submissions are not accepted.
async fn agreement(t: &Booted) -> PayerAgreement {
    t.agreement(BLUE_DUNES, VatRecipient::Customer).await
}

/// The pending approval requests about `bill`.
async fn pending(t: &Booted, bill: &PayerBill) -> Vec<wl_kernel::approvals::ApprovalRequest> {
    approvals::pending_for(t.db(), &bill.subject()).await.unwrap()
}

/// AC1: given a 05 visit whose customer tax invoice of AED 1,000.00 was
/// issued at the visit (customer share AED 400.00, payer lines AED 600.00),
/// a draft payer bill of AED 600.00 whose window ended and an agreement that
/// does not accept late submissions, `write_off` by user A (limit AED
/// 1,000.00) closes it, issues a credit note of AED 600.00 against the
/// invoice and flags *not submitted in time*.
#[renox::test]
async fn a_late_draft_is_written_off_with_a_credit_note_against_the_customers_invoice() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let agreement = agreement(&t).await;
    let invoice = visit_invoice(&t, 60_000, 40_000).await;
    assert_eq!(invoice.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(invoice.balance, 60_000);
    let bill = t
        .payer_bill_for(
            &agreement,
            OMAR,
            missed(),
            &[("Consultation and tests", 60_000)],
            Some(&invoice),
        )
        .await;
    assert_eq!(bill.status, PayerBillStatus::Draft);

    t.post_payer(&bill, &a, "write-off", &[("reason", "missed the window")])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));

    let closed = t.reload_bill(&bill).await;
    assert_eq!(closed.status, PayerBillStatus::Closed);
    assert!(closed.not_submitted_in_time, "flagged not submitted in time");
    assert_eq!(closed.close_outcome.as_deref(), Some("written_off"));
    assert_eq!(closed.close_reason.as_deref(), Some("missed the window"));
    assert_eq!((closed.written_off_total, closed.balance), (60_000, 0));
    assert!(!closed.tax_document_pending);
    assert!(pending(&t, &bill).await.is_empty(), "within the limit");

    let credits = t.credit_notes_of(&invoice).await;
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].gross_total, 60_000);
    assert_eq!(closed.close_credit_note_id, Some(credits[0].id));
    let invoice = t.reload(&invoice).await;
    assert_eq!(invoice.balance, 0, "the credit settles the payer share");

    let events = t.payer.closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, PayerBillOutcome::WrittenOff);
    assert!(events[0].not_submitted_in_time);
    assert_eq!(events[0].open, 60_000);
}

/// AC2: AED 1,500.00 needs an approval request; nothing changes until a
/// second person approves, then the bill is closed and credited.
#[renox::test]
async fn a_write_off_above_the_limit_waits_for_approval() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let b = t.payer_writer("Bea").await;
    let agreement = agreement(&t).await;
    let invoice = visit_invoice(&t, 150_000, 50_000).await;
    let bill = t
        .payer_bill_for(
            &agreement,
            OMAR,
            missed(),
            &[("Consultation and tests", 150_000)],
            Some(&invoice),
        )
        .await;

    t.post_payer(&bill, &a, "write-off", &[("reason", "missed the window")])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));

    let waiting = t.reload_bill(&bill).await;
    assert_eq!(waiting.status, PayerBillStatus::Draft);
    assert!(t.credit_notes_of(&invoice).await.is_empty());
    let requests = pending(&t, &bill).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "invoicing.payer_write_off");
    assert_eq!(requests[0].payload.0["amount"], 150_000);
    assert!(matches!(
        approvals::approve(t.state(), requests[0].id, &a, "").await,
        Err(Error::Forbidden)
    ));

    approvals::approve(t.state(), requests[0].id, &b, "ok")
        .await
        .unwrap();
    let closed = t.reload_bill(&bill).await;
    assert_eq!(closed.status, PayerBillStatus::Closed);
    assert!(closed.not_submitted_in_time);
    assert_eq!(closed.close_reason.as_deref(), Some("missed the window"));
    let credits = t.credit_notes_of(&invoice).await;
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].gross_total, 150_000);
    assert_eq!(t.payer.closed.all().len(), 1);
}

/// AC3: with the window still open, `write_off` from draft is refused; so is
/// it when the agreement accepts late submissions.
#[renox::test]
async fn a_draft_inside_the_window_is_not_written_off() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let agreement = agreement(&t).await;
    let invoice = visit_invoice(&t, 60_000, 40_000).await;
    let open = t
        .payer_bill_for(
            &agreement,
            OMAR,
            today(),
            &[("Consultation and tests", 60_000)],
            Some(&invoice),
        )
        .await;
    t.post_payer(&open, &a, "write-off", &[("reason", "no")])
        .await
        .assert_status(422);
    t.post_payer(&open, &a, "transfer", &[("reason", "no")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_bill(&open).await.status, PayerBillStatus::Draft);
    assert!(t.credit_notes_of(&invoice).await.is_empty());

    // The window ended, but this payer accepts late submissions.
    let late = t
        .change_agreement(&agreement, |a| a.accepts_late_submissions = true)
        .await;
    let ended = t
        .payer_bill_for(
            &late,
            OMAR,
            missed(),
            &[("Consultation and tests", 60_000)],
            Some(&invoice),
        )
        .await;
    t.post_payer(&ended, &a, "write-off", &[("reason", "late")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_bill(&ended).await.status, PayerBillStatus::Draft);
    assert!(t.payer.closed.all().is_empty());
}

/// A close needs a reason.
#[renox::test]
async fn a_close_needs_a_reason() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let agreement = agreement(&t).await;
    let invoice = visit_invoice(&t, 60_000, 40_000).await;
    let bill = t
        .payer_bill_for(
            &agreement,
            OMAR,
            missed(),
            &[("Consultation and tests", 60_000)],
            Some(&invoice),
        )
        .await;
    t.post_payer(&bill, &a, "write-off", &[("reason", "  ")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Draft);
}

/// AC4: with the same draft bill, window ended, an agreement that does not
/// accept late submissions and allows billing the customer,
/// `transfer_to_customer` closes it, moves AED 600.00 to the customer's
/// share of the invoice (which is not credited) and notifies the customer.
#[renox::test]
async fn a_late_draft_moves_to_the_customer_where_the_agreement_allows() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let agreement = agreement(&t).await;
    let invoice = visit_invoice(&t, 60_000, 40_000).await;
    let bill = t
        .payer_bill_for(
            &agreement,
            OMAR,
            missed(),
            &[("Consultation and tests", 60_000)],
            Some(&invoice),
        )
        .await;

    // Not by default: the agreement must allow billing the customer.
    t.post_payer(&bill, &a, "transfer", &[("reason", "late claim")])
        .await
        .assert_status(422);
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Draft);

    t.change_agreement(&agreement, |a| a.bill_customer_allowed = true)
        .await;
    t.post_payer(&bill, &a, "transfer", &[("reason", "late claim")])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));

    let closed = t.reload_bill(&bill).await;
    assert_eq!(closed.status, PayerBillStatus::Closed);
    assert_eq!(closed.close_outcome.as_deref(), Some("transferred"));
    assert_eq!(closed.transferred_total, 60_000);
    assert_eq!(closed.balance, 0);
    assert!(closed.not_submitted_in_time);
    // The amount stays on the customer's invoice, now their own share: no
    // credit note, no new invoice.
    let same = t.reload(&invoice).await;
    assert_eq!(same.balance, 60_000);
    assert_eq!(same.status, InvoiceStatus::PartiallyPaid);
    assert!(t.credit_notes_of(&invoice).await.is_empty());
    assert!(t.payer_invoices(&closed).await.is_empty());
    // The customer is told.
    let told = t.notices.of_type("invoicing.payer_bill_transferred");
    assert_eq!(told.len(), 1);
    assert_eq!(told[0].subject, closed.subject());
    assert_eq!(told[0].recipients, [NotifyRecipient::Party { id: OMAR.0 }]);
    let events = t.payer.closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, PayerBillOutcome::Transferred);
}

/// AC5: given a draft payer bill of AED 600.00 where the payer is the
/// recipient and no payer tax invoice was issued, `write_off` after the
/// window closes it with no credit note.
#[renox::test]
async fn a_never_issued_payer_invoice_needs_no_credit_note() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Payer).await;
    let bill = t
        .payer_bill(&agreement, missed(), &[("Consultation and tests", 60_000)])
        .await;
    assert_eq!(bill.payer_invoice_id, None);

    t.post_payer(&bill, &a, "write-off", &[("reason", "missed the window")])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));

    let closed = t.reload_bill(&bill).await;
    assert_eq!(closed.status, PayerBillStatus::Closed);
    assert_eq!(closed.close_credit_note_id, None);
    assert!(!closed.tax_document_pending);
    assert!(t.payer_invoices(&closed).await.is_empty());
    assert_eq!(t.payer.closed.all().len(), 1);
}

/// With the payer as the recipient and its tax invoice issued, moving the
/// amount to the customer credits the payer's invoice and invoices the
/// customer.
#[renox::test]
async fn a_transfer_credits_the_payers_invoice_and_invoices_the_customer() {
    let t = boot().await;
    let a = t.payer_writer("Ana").await;
    let cora = t.coordinator_cora().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Payer).await;
    let agreement = t
        .change_agreement(&agreement, |a| a.bill_customer_allowed = true)
        .await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 90_000)])
        .await;
    let issued = t.issue_payer_bill(&bill, &cora).await;
    let payer_invoice = t.payer_invoices(&issued).await.remove(0);
    // Sent back, and the first-submission window ended since.
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(t.db(), &[("service_date", &missed())])
        .await
        .unwrap();

    t.post_payer(&bill, &a, "transfer", &[("reason", "payer will not pay")])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));

    let closed = t.reload_bill(&bill).await;
    assert_eq!(closed.status, PayerBillStatus::Closed);
    let credits = t.credit_notes_of(&payer_invoice).await;
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].gross_total, 90_000);
    assert_eq!(closed.close_credit_note_id, Some(credits[0].id));
    let invoices = t.payer_invoices(&closed).await;
    assert_eq!(invoices.len(), 2, "the payer's and the customer's");
    let customers = &invoices[1];
    assert_eq!(customers.party_id, OMAR.0);
    assert_eq!(customers.gross_total, 90_000);
    assert_eq!(customers.status, InvoiceStatus::Issued);
    assert_eq!(closed.close_invoice_id, Some(customers.id));
    assert!(!closed.tax_document_pending);
}

/// Submits `reason` to the kernel's void route as `by`.
async fn post_void(t: &Booted, bill: &PayerBill, by: &User, reason: &str) {
    t.app.acting_as(by);
    t.app
        .htmx()
        .post(
            &format!("/payer-bills/{}/transitions/void", bill.id),
            &[("reason", reason)],
        )
        .await;
}

/// AC6: `void` with reason "wrong payer" sets `voided`, keeps the number,
/// removes the split; the visit can be split to another payer.
#[renox::test]
async fn a_bill_made_in_error_is_voided_and_the_source_can_be_split_again() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = agreement(&t).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 60_000)])
        .await;
    // Numbered by its first issue, then sent back by the channel.
    let issued = t.issue_payer_bill(&bill, &cora).await;
    let number = issued.number.clone().expect("numbered at the first issue");
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));

    post_void(&t, &bill, &cora, "wrong payer").await;

    let voided = t.reload_bill(&bill).await;
    assert_eq!(voided.status, PayerBillStatus::Voided);
    assert_eq!(voided.number.as_deref(), Some(number.as_str()), "kept");
    assert_eq!(voided.close_outcome.as_deref(), Some("voided"));
    assert_eq!(voided.close_reason.as_deref(), Some("wrong payer"));
    assert!(!voided.in_ageing);
    let record = VoidRecord::of_subject(t.db(), &voided.subject())
        .await
        .unwrap()
        .expect("the kernel keeps a Void record");
    assert_eq!(record.number.as_deref(), Some(number.as_str()));
    assert_eq!(record.reason, "wrong payer");

    // The split is released: the source is billed to no one and can be split
    // again, to the right payer, as a new payer bill.
    let listed = payer_bills(t.state())
        .of_source(t.db(), &bill.source())
        .await
        .unwrap();
    assert!(listed.is_empty());
    let other = t.agreement(OMAR, VatRecipient::Customer).await;
    let again = payer_bills(t.state())
        .create(
            t.state(),
            NewPayerBill::new(other.id, OMAR, bill.source(), today())
                .line(NewPayerBillLine::new("Consultation and tests", 60_000)),
            None,
        )
        .await
        .expect("the source is split again");
    assert_ne!(again, PayerBillId(bill.id));

    // The coordinators are told, and the event says it was voided.
    let told = t.notices.of_type("invoicing.payer_bill_voided");
    assert_eq!(told.len(), 1);
    let events = t.payer.closed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].outcome, PayerBillOutcome::Voided);
    // A voided bill never moves again.
    post_void(&t, &bill, &cora, "again").await;
    assert_eq!(t.payer.closed.all().len(), 1);
}

/// A payer tax invoice already issued is cancelled by a credit note when the
/// bill is voided.
#[renox::test]
async fn voiding_cancels_an_issued_payer_invoice_with_a_credit_note() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Payer).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 90_000)])
        .await;
    let issued = t.issue_payer_bill(&bill, &cora).await;
    let invoice = t.payer_invoices(&issued).await.remove(0);
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));

    post_void(&t, &bill, &cora, "duplicate").await;

    let voided = t.reload_bill(&bill).await;
    assert_eq!(voided.status, PayerBillStatus::Voided);
    assert!(!voided.tax_document_pending);
    let credits = t.credit_notes_of(&invoice).await;
    assert_eq!(credits.len(), 1, "cancelled by a credit note");
    assert_eq!(credits[0].gross_total, 90_000);
    assert_eq!(voided.close_credit_note_id, Some(credits[0].id));
    assert_eq!(
        t.invoice(invoice.id).await.status,
        InvoiceStatus::Credited
    );
}

/// AC7: `void` of a 05 bill that was submitted once is refused; a bill that
/// was never submitted, made the same way, is voided.
#[renox::test]
async fn a_clinic_claim_submitted_once_cannot_be_voided() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = agreement(&t).await;
    let make = |n: i64| {
        NewPayerBill::new(agreement.id, OMAR, Subject::new("clinics.claim", n), today())
            .void_before_first_submission()
            .line(NewPayerBillLine::new("Consultation and tests", 60_000))
    };
    let submitted_id = payer_bills(t.state())
        .create(t.state(), make(9_001), None)
        .await
        .unwrap();
    let submitted = t.payer_bill_now(submitted_id).await;
    let issued = t.issue_payer_bill(&submitted, &cora).await;
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", submitted.id));

    post_void(&t, &submitted, &cora, "created in error").await;
    let same = t.reload_bill(&submitted).await;
    assert_eq!(same.status, PayerBillStatus::Draft, "refused: submitted once");
    assert!(t.payer.closed.all().is_empty());

    let fresh_id = payer_bills(t.state())
        .create(t.state(), make(9_002), None)
        .await
        .unwrap();
    let fresh = t.payer_bill_now(fresh_id).await;
    post_void(&t, &fresh, &cora, "created in error").await;
    assert_eq!(
        t.reload_bill(&fresh).await.status,
        PayerBillStatus::Voided,
        "before the first submission it can be"
    );
    assert_eq!(fresh.number, None, "never issued: no number to keep");
}

/// Never once a remittance was recorded (`PLT.ACL.09`).
#[renox::test]
async fn a_bill_with_a_remittance_is_not_voided() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = agreement(&t).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 60_000)])
        .await;
    let issued = t.issue_payer_bill(&bill, &cora).await;
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(t.db(), &[("has_remittance", &true)])
        .await
        .unwrap();

    post_void(&t, &bill, &cora, "wrong payer").await;
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Draft);
    assert!(t.payer.closed.all().is_empty());
}

/// Voiding needs `payer_bills.issue`: a user who may only write off cannot.
#[renox::test]
async fn voiding_needs_the_issue_permission() {
    let t = boot().await;
    let reconciler = t.reconciler("Rita").await;
    let agreement = agreement(&t).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 60_000)])
        .await;
    post_void(&t, &bill, &reconciler, "wrong payer").await;
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Draft);
}
