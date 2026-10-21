//! A technical rejection of a payer bill's submission (wl-invoicing §4
//! *Payer bill* `submission_rejected`, §5.8 step 6, #79): the acceptance
//! checks of BL-INV-16, driven through the payer bill pages, the kernel
//! contract and the real transitions.

mod support;

use renox::prelude::*;
use wl_invoicing::payer_rejection::PayerSubmissionError;
use wl_invoicing::{PayerBill, VatRecipient};
use wl_kernel::calendar::BusinessCalendar;
use wl_kernel::documents::today;
use wl_kernel::invoicing::InvoiceStatus;
use wl_kernel::payer_bills::{PayerBillId, PayerBillStatus, payer_bills};

use support::payer::d;
use support::{BLUE_DUNES, Booted, boot};

/// An issued payer bill for the customer-recipient case (05): one line of
/// AED 900.00 for a service today, issued by the coordinator.
async fn issued_bill(t: &Booted, by: &User) -> PayerBill {
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 90_000)])
        .await;
    t.issue_payer_bill(&bill, by).await
}

/// AC1: when the channel rejects an issued payer bill with "invalid member
/// ID", it is `draft`, the error is stored, the resubmission count stays 0,
/// the receivable leaves the ageing, the coordinator gets a task with the
/// deadline and `PayerBillSubmissionRejected` is emitted.
#[renox::test]
async fn a_technical_rejection_returns_the_bill_to_draft_with_its_errors() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let bill = issued_bill(&t, &cora).await;
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert!(bill.in_ageing);
    let number = bill.number.clone().expect("the first issue numbers it");

    t.post_payer(
        &bill,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));

    let rejected = t.reload_bill(&bill).await;
    assert_eq!(rejected.status, PayerBillStatus::Draft);
    assert_eq!(rejected.resubmission_count, 0);
    assert!(!rejected.in_ageing, "the receivable leaves the ageing");
    assert_eq!(rejected.number.as_deref(), Some(number.as_str()));
    let errors = PayerSubmissionError::of_bill(t.db(), bill.id).await.unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(
        (errors[0].attempt, errors[0].code.as_str()),
        (1, "E104")
    );
    assert_eq!(errors[0].message, "invalid member ID");

    // The coordinator's task names the errors and the last day to send it
    // again: 30 days after the service.
    let deadline = today() + renox::chrono::Days::new(30);
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert!(task.description.contains("E104 invalid member ID"));
    assert!(
        task.description
            .contains(&deadline.format("%-d %b %Y").to_string())
    );
    assert_eq!(
        task.due_at,
        Some(BusinessCalendar::default().from_local(deadline.and_hms_opt(23, 59, 0).unwrap()))
    );
    assert_eq!(
        task.source_key.as_deref(),
        Some(format!("invoicing.payer_bill_rejected:{}:1", bill.id).as_str())
    );

    let events = t.payer.rejected.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].payer_bill, PayerBillId(bill.id));
    assert_eq!(events[0].error_codes, ["E104"]);
}

/// A rejection needs an error with its code and message, and only an
/// `issued` bill is rejected.
#[renox::test]
async fn a_rejection_without_an_error_changes_nothing() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let bill = issued_bill(&t, &cora).await;

    t.post_payer(&bill, &cora, "submission-rejected", &[("errors", " ")])
        .await
        .assert_status(422);
    assert_eq!(
        t.reload_bill(&bill).await.status,
        PayerBillStatus::Issued
    );
    assert!(t.tickets.opened().is_empty());
    assert!(t.payer.rejected.all().is_empty());

    // A draft is not rejected.
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let draft = t.payer_bill(&agreement, today(), &[("Lab", 10_000)]).await;
    t.post_payer(
        &draft,
        &cora,
        "submission-rejected",
        &[("errors", "E1: bad")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload_bill(&draft).await.status, PayerBillStatus::Draft);
}

/// AC2: issued again with an unchanged amount, the bill keeps its number and
/// no new tax document is created: the payer's tax invoice of the first
/// issue stays the only one, and the resubmission count stays 0.
#[renox::test]
async fn issued_again_it_keeps_its_number_and_makes_no_new_tax_document() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Payer).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 90_000)])
        .await;
    let issued = t.issue_payer_bill(&bill, &cora).await;
    let number = issued.number.clone().unwrap();
    let invoices = t.payer_invoices(&issued).await;
    assert_eq!(invoices.len(), 1, "the payer's tax invoice is issued once");
    assert_eq!(invoices[0].status, InvoiceStatus::Issued);
    assert_eq!(invoices[0].gross_total, 90_000);
    assert_eq!(invoices[0].party_id, BLUE_DUNES.0);
    assert_eq!(issued.payer_invoice_id, Some(invoices[0].id));

    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));
    let again = t.issue_payer_bill(&bill, &cora).await;
    assert_eq!(again.status, PayerBillStatus::Issued);
    assert_eq!(again.number.as_deref(), Some(number.as_str()));
    assert_eq!(again.resubmission_count, 0);
    assert!(again.in_ageing, "the receivable is back in the ageing");
    assert_eq!(again.issue_count, 2);
    assert!(!again.tax_document_pending);

    let invoices = t.payer_invoices(&again).await;
    assert_eq!(invoices.len(), 1, "no new tax document");
    assert!(t.credit_notes_of(&invoices[0]).await.is_empty());
    assert_eq!(t.payer.issued.all().len(), 2);
}

/// AC3: with an existing payer tax invoice of AED 900.00 and a corrected
/// amount of AED 850.00, a credit note of AED 50.00 is issued; the invoice
/// itself is not edited and not issued again.
#[renox::test]
async fn a_lower_corrected_amount_is_credited_by_the_difference() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Payer).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 90_000)])
        .await;
    let issued = t.issue_payer_bill(&bill, &cora).await;
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_ok();
    let line = t.bill_lines(&bill).await.remove(0);
    payer_bills(t.state())
        .correct_line(t.state(), PayerBillId(bill.id), line.id, 85_000)
        .await
        .expect("the draft is corrected");
    assert_eq!(t.reload_bill(&bill).await.claimed_total, 85_000);

    let again = t.issue_payer_bill(&bill, &cora).await;
    assert_eq!(again.status, PayerBillStatus::Issued);
    let invoices = t.payer_invoices(&again).await;
    assert_eq!(invoices.len(), 1, "no second invoice for a lower amount");
    assert_eq!(invoices[0].gross_total, 90_000, "the invoice is not edited");
    let credits = t.credit_notes_of(&invoices[0]).await;
    assert_eq!(credits.len(), 1);
    assert_eq!(credits[0].gross_total, 5_000);
    assert!(credits[0].number.is_some(), "the credit note is issued");
    assert!(!again.tax_document_pending);
    let lines = t.bill_lines(&again).await;
    assert_eq!((lines[0].claimed, lines[0].invoiced), (85_000, 85_000));
}

/// AC4: with an existing payer tax invoice of AED 900.00 and a corrected
/// amount of AED 950.00, an extra tax invoice of AED 50.00 is issued to the
/// payer; the AED 900.00 invoice is unchanged.
#[renox::test]
async fn a_higher_corrected_amount_gets_an_extra_invoice_for_the_difference() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Payer).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation and tests", 90_000)])
        .await;
    let issued = t.issue_payer_bill(&bill, &cora).await;
    let original = t.payer_invoices(&issued).await.remove(0);
    t.post_payer(
        &issued,
        &cora,
        "submission-rejected",
        &[("errors", "E7: missing field")],
    )
    .await
    .assert_ok();
    let line = t.bill_lines(&bill).await.remove(0);
    payer_bills(t.state())
        .correct_line(t.state(), PayerBillId(bill.id), line.id, 95_000)
        .await
        .expect("the draft is corrected");

    let again = t.issue_payer_bill(&bill, &cora).await;
    let invoices = t.payer_invoices(&again).await;
    assert_eq!(invoices.len(), 2);
    let unchanged = &invoices[0];
    assert_eq!(unchanged.id, original.id);
    assert_eq!(unchanged.gross_total, 90_000);
    assert_eq!(unchanged.status, InvoiceStatus::Issued);
    let extra = &invoices[1];
    assert_eq!(extra.gross_total, 5_000, "the difference only");
    assert_eq!(extra.status, InvoiceStatus::Issued);
    assert_eq!(extra.party_id, BLUE_DUNES.0);
    assert!(t.credit_notes_of(unchanged).await.is_empty());
    assert!(!again.tax_document_pending);
}

/// Rejected several times: each attempt keeps its own errors and the count
/// stays; the task of each attempt has its own key.
#[renox::test]
async fn every_attempt_keeps_its_own_errors() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let bill = issued_bill(&t, &cora).await;
    for errors in ["E104: invalid member ID", "E7: missing field\nE9: bad date"] {
        t.post_payer(&bill, &cora, "submission-rejected", &[("errors", errors)])
            .await
            .assert_ok();
        t.issue_payer_bill(&bill, &cora).await;
    }
    let all = PayerSubmissionError::of_bill(t.db(), bill.id).await.unwrap();
    let by_attempt: Vec<(i64, &str)> = all.iter().map(|e| (e.attempt, e.code.as_str())).collect();
    assert_eq!(by_attempt, [(1, "E104"), (2, "E7"), (2, "E9")]);
    let now = t.reload_bill(&bill).await;
    assert_eq!(now.resubmission_count, 0);
    assert_eq!(now.status, PayerBillStatus::Issued);
    let keys: Vec<_> = t
        .tickets
        .opened()
        .into_iter()
        .filter_map(|ticket| ticket.source_key)
        .collect();
    assert_eq!(
        keys,
        [
            format!("invoicing.payer_bill_rejected:{}:1", bill.id),
            format!("invoicing.payer_bill_rejected:{}:2", bill.id),
        ]
    );
}

/// A rejection that arrives after a remittance was recorded is refused by
/// the guard, and the coordinator gets a task.
#[renox::test]
async fn a_rejection_after_a_remittance_is_refused_with_a_task() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let bill = issued_bill(&t, &cora).await;
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(t.db(), &[("has_remittance", &true)])
        .await
        .unwrap();

    t.post_payer(
        &bill,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_status(422);
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Issued);
    assert!(t.payer.rejected.all().is_empty());
    let tasks = t.tickets.opened();
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains("refused"));
    assert!(tasks[0].description.contains("E104 invalid member ID"));
}

/// The deadline of a rejected resubmission counts from the remittance, not
/// from the service.
#[renox::test]
async fn the_deadline_of_a_rejected_resubmission_counts_from_the_remittance() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let bill = issued_bill(&t, &cora).await;
    // A resubmitted bill: one remittance on 2026-12-10, count 1.
    PayerBill::query()
        .where_(PayerBill::ID.eq(bill.id))
        .update(
            t.db(),
            &[
                ("resubmission_count", &1_i64),
                ("last_remittance_on", &Some(d("2026-12-10"))),
            ],
        )
        .await
        .unwrap();

    t.post_payer(
        &bill,
        &cora,
        "submission-rejected",
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_ok();
    let rejected = t.reload_bill(&bill).await;
    assert_eq!(rejected.status, PayerBillStatus::Draft);
    assert_eq!(rejected.resubmission_count, 1, "the count stays");
    let tasks = t.tickets.opened();
    assert!(tasks[0].description.contains("25 Dec 2026"));
}

/// The vertical's channel rejects through the kernel contract as the system
/// (05 claim `reject_submission` in the integration, Later): the same
/// transition runs, and the answer carries the deadline the task names.
#[renox::test]
async fn the_vertical_rejects_through_the_kernel_contract() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let bill = issued_bill(&t, &cora).await;

    let answer = payer_bills(t.state())
        .submission_rejected(
            t.state(),
            PayerBillId(bill.id),
            vec![wl_kernel::payer_bills::SubmissionError::new(
                "E104",
                "invalid member ID",
            )],
            None,
        )
        .await
        .expect("the system records the rejection");
    assert_eq!(
        answer.deadline,
        Some(today() + renox::chrono::Days::new(30)),
        "the end of the first-submission window"
    );
    let rejected = t.reload_bill(&bill).await;
    assert_eq!(rejected.status, PayerBillStatus::Draft);
    assert_eq!(rejected.number, bill.number);
    assert_eq!(t.payer.rejected.all().len(), 1);
    assert_eq!(t.tickets.opened().len(), 1);

    // It is found by its source and can be issued again by the system.
    let found = payer_bills(t.state())
        .of_source(t.db(), &bill.source())
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].status, PayerBillStatus::Draft);
    payer_bills(t.state())
        .issue(t.state(), PayerBillId(bill.id), None)
        .await
        .expect("the system issues it again");
    let again = t.reload_bill(&bill).await;
    assert_eq!(again.status, PayerBillStatus::Issued);
    assert_eq!(again.number, bill.number);
    assert_eq!(again.resubmission_count, 0);
}
