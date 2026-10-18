//! Writing off an invoice balance as a bad debt (wl-invoicing §4
//! `write_off`, §5.9, `FIN.ACL.15`): the acceptance checks AC6–AC14 of #64.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::{AdjustmentKind, AdjustmentStatus, Invoice, InvoiceLine, VatAdjustment};
use wl_invoicing::{WlInvoicing, write_off};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::invoicing::{InvoiceStatus, VatCategory};

use support::{BLUE_DUNES_EMAIL, Booted, boot, boot_with};

/// 1,000.00 plus 5% VAT.
const LINE: (i64, VatCategory) = (100_000, VatCategory::Standard);
const GROSS: i64 = 105_000;

/// `date` as `18 Oct 2026`, as the notice writes it.
fn date_text(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

/// The adjustments of `invoice`.
async fn adjustments(t: &Booted, invoice: &Invoice) -> Vec<VatAdjustment> {
    VatAdjustment::query()
        .where_(VatAdjustment::SOURCE_TYPE.eq(invoice.subject().subject_type.as_str()))
        .where_(VatAdjustment::SOURCE_ID.eq(invoice.id))
        .get(t.db())
        .await
        .unwrap()
}

/// The request, as stored now.
async fn request_now(t: &Booted, request: &ApprovalRequest) -> ApprovalRequest {
    ApprovalRequest::find(t.db(), request.id)
        .await
        .unwrap()
        .unwrap()
}

/// User A asks to write `invoice` off as `insolvent`; the one pending
/// request.
async fn ask(t: &Booted, invoice: &Invoice, a: &User) -> ApprovalRequest {
    t.post_write_off(invoice, a, "insolvent", "")
        .await
        .assert_ok();
    let pending = t.pending(invoice).await;
    assert_eq!(pending.len(), 1);
    pending.into_iter().next().unwrap()
}

/// Marks `invoice` as disputed (the `dispute` action comes with #67).
async fn dispute(t: &Booted, invoice: &Invoice) {
    Invoice::query()
        .where_(Invoice::ID.eq(invoice.id))
        .update(t.db(), &[("disputed", &true)])
        .await
        .unwrap();
}

/// Runs the queue and returns the mails sent to the billing e-mail.
async fn notices(t: &Booted) -> Vec<renox::mail::Mail> {
    t.app.run_all_jobs().await;
    t.app
        .sent_mail()
        .into_iter()
        .filter(|mail| mail.to == [BLUE_DUNES_EMAIL])
        .collect()
}

/// AC6: user A runs `write_off` on an overdue invoice of 1,050.00: a pending
/// request for 1,050.00, the invoice stays `overdue` with the pending badge,
/// and no adjustment exists.
#[renox::test]
async fn a_write_off_waits_for_approval() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.overdue_invoice(&a).await;
    assert_eq!(invoice.status, InvoiceStatus::Overdue);
    assert_eq!((invoice.gross_total, invoice.balance), (GROSS, GROSS));

    let request = ask(&t, &invoice, &a).await;
    assert_eq!(request.status, ApprovalStatus::Pending);
    assert_eq!(request.action, "invoicing.write_off");
    assert_eq!(request.requester_id, a.id);
    assert_eq!(request.payload.0["amount"], GROSS);
    assert_eq!(request.payload.0["reason_code"], "insolvent");

    let same = t.reload(&invoice).await;
    assert_eq!(same.status, InvoiceStatus::Overdue);
    assert_eq!(same.write_off_amount, None);
    assert_eq!(same.write_off_approval_id, None);
    assert!(adjustments(&t, &invoice).await.is_empty());
    assert!(t.written_off.all().is_empty());

    t.app.acting_as(&a);
    t.app
        .get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see("Pending approval");
    // While a request waits, a second one is refused.
    t.post_write_off(&invoice, &a, "insolvent", "")
        .await
        .assert_status(422);
    assert_eq!(t.pending(&invoice).await.len(), 1);
}

/// AC7: A's own approval is refused (SoD), also when A is an admin.
#[renox::test]
async fn the_requester_cannot_approve_their_own_write_off() {
    for admin in [false, true] {
        let t = boot().await;
        let a = if admin {
            t.admin_a().await
        } else {
            t.user_a().await
        };
        let b = t.user_b().await;
        let invoice = t.overdue_invoice(&a).await;
        let request = ask(&t, &invoice, &a).await;

        let refused = approvals::approve(t.state(), request.id, &a, "").await;
        assert!(matches!(refused, Err(Error::Forbidden)), "admin: {admin}");
        assert_eq!(
            request_now(&t, &request).await.status,
            ApprovalStatus::Pending
        );
        assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Overdue);
        // B still can.
        approvals::approve(t.state(), request.id, &b, "")
            .await
            .unwrap();
        assert_eq!(
            t.reload(&invoice).await.status,
            InvoiceStatus::WrittenOff,
            "admin: {admin}"
        );
    }
}

/// AC8 and AC9: B approves; the invoice is written off with the approval,
/// one pending adjustment of 50.00 exists, the notice is sent to the billing
/// e-mail and its date is stored, `InvoiceWrittenOff` is emitted once, and
/// lines, totals and VAT are unchanged.
#[renox::test]
async fn an_approved_write_off_writes_off_and_notifies() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let lines_before: Vec<_> = wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap();
    let request = ask(&t, &invoice, &a).await;

    let decided = approvals::approve(t.state(), request.id, &b, "bad debt")
        .await
        .unwrap();
    assert!(decided.is_some());
    let done = request_now(&t, &request).await;
    assert_eq!(done.status, ApprovalStatus::Approved);

    let written_off = t.reload(&invoice).await;
    assert_eq!(written_off.status, InvoiceStatus::WrittenOff);
    assert_eq!(written_off.write_off_amount, Some(GROSS));
    assert_eq!(
        written_off.write_off_reason_code.as_deref(),
        Some("insolvent")
    );
    assert_eq!(written_off.write_off_approval_id, Some(request.id));
    assert_eq!(written_off.write_off_date, Some(today()));
    assert_eq!(written_off.write_off_recovered_amount, 0);
    assert!(written_off.write_off_export_pending);

    // Lines, totals and VAT never change.
    assert_eq!(written_off.net_total, invoice.net_total);
    assert_eq!(written_off.vat_total, invoice.vat_total);
    assert_eq!(written_off.gross_total, invoice.gross_total);
    assert_eq!(written_off.vat_total_aed, invoice.vat_total_aed);
    assert_eq!(written_off.balance, GROSS);
    let lines_after = wl_invoicing::invoicer::lines_of(t.db(), invoice.id)
        .await
        .unwrap();
    assert_eq!(lines_after.len(), lines_before.len());
    for (before, after) in lines_before.iter().zip(&lines_after) {
        assert_eq!(
            (before.net_amount, before.vat_amount, before.gross_amount),
            (after.net_amount, after.vat_amount, after.gross_amount)
        );
    }

    let rows = adjustments(&t, &invoice).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, AdjustmentKind::BadDebtRelief);
    assert_eq!(rows[0].status, AdjustmentStatus::Pending);
    assert_eq!(rows[0].amount, 5_000);
    assert_eq!(rows[0].legal_entity_id, invoice.legal_entity_id);

    // The notice job sends one mail; its date is the date it was sent.
    let mails = notices(&t).await;
    assert_eq!(mails.len(), 1);
    let sent = t.reload(&invoice).await;
    assert_eq!(sent.write_off_notice_on, Some(today()));
    // Run again: still one mail, one date.
    assert_eq!(notices(&t).await.len(), 1);

    let events = t.written_off.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].invoice.0, invoice.id);
    assert_eq!(events[0].amount, GROSS);
    assert_eq!(events[0].reason_code, "insolvent");
    assert_eq!(events[0].approver, Some(b.id));

    let history = t.history(&invoice).await;
    assert_eq!(
        history.last().map(String::as_str),
        Some("write_off_approved")
    );

    // The approved request cannot run twice.
    assert!(
        approvals::approve(t.state(), request.id, &b, "")
            .await
            .is_err()
    );
    assert_eq!(adjustments(&t, &invoice).await.len(), 1);
    assert_eq!(t.written_off.all().len(), 1);
}

/// AC9: the notice carries the invoice number, issue date, date of supply,
/// gross total, the amount written off, its VAT share and the write-off
/// date.
#[renox::test]
async fn the_notice_carries_the_invoice_figures() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let request = ask(&t, &invoice, &a).await;
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    let mails = notices(&t).await;
    assert_eq!(mails.len(), 1);
    let number = invoice.number.clone().unwrap();
    assert_eq!(
        mails[0].subject,
        format!("Invoice {number} written off as a bad debt")
    );
    let text = &mails[0].text;
    let day = date_text(today());
    for expected in [
        number.as_str(),
        // No date of supply on the invoice: the issue date.
        &format!("Date of supply {day}"),
        &format!("Issue date {day}"),
        "Total including VAT AED 1,050.00",
        "Amount written off AED 1,050.00",
        "VAT in the amount written off AED 50.00",
        &format!("Write-off date {day}"),
    ] {
        assert!(text.contains(expected), "`{expected}` is not in:\n{text}");
    }
}

/// AC10: the VAT share of the written-off amount, per category.
#[renox::test]
async fn the_relief_is_the_vat_share_of_the_written_off_amount() {
    // (lines, paid before, written off, relief), in fils.
    type Case<'a> = (&'a [(i64, VatCategory)], i64, i64, i64);
    let cases: [Case; 3] = [
        // 420.00 paid of 1,050.00: 630.00 written off, relief 30.00.
        (&[LINE], 42_000, 63_000, 3_000),
        // 1,000 standard (VAT 50) and 500 zero-rated: 775.00 of 1,550.00.
        (
            &[LINE, (50_000, VatCategory::ZeroRated)],
            77_500,
            77_500,
            2_500,
        ),
        // 333.33 of 1,050.00.
        (&[LINE], 71_667, 33_333, 1_587),
    ];
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    for (lines, paid, written_off, relief) in cases {
        let invoice = t.issued(lines, None, &a).await;
        t.pay(&invoice, invoice.id, paid).await;
        let invoice = t.reload(&invoice).await;
        assert_eq!(invoice.status, InvoiceStatus::PartiallyPaid);
        assert_eq!(invoice.balance, written_off);

        let request = ask(&t, &invoice, &a).await;
        assert_eq!(request.payload.0["amount"], written_off);
        approvals::approve(t.state(), request.id, &b, "")
            .await
            .unwrap();
        let done = t.reload(&invoice).await;
        assert_eq!(done.status, InvoiceStatus::WrittenOff);
        assert_eq!(done.write_off_amount, Some(written_off));
        let rows = adjustments(&t, &invoice).await;
        assert_eq!(rows.len(), 1, "{written_off}");
        assert_eq!(rows[0].amount, relief, "{written_off}");
        assert_eq!(rows[0].kind, AdjustmentKind::BadDebtRelief);
        assert_eq!(rows[0].status, AdjustmentStatus::Pending);
        // What was already paid stays allocated.
        assert_eq!(done.allocated_total, paid);
    }
    assert_eq!(VatAdjustment::query().count(t.db()).await.unwrap(), 3);
}

/// AC11: a payment of 100.00 comes in while the request waits; B's approval
/// ends in `failed`: the invoice is `partially_paid` with balance 950.00,
/// and no adjustment and no notice exist.
#[renox::test]
async fn a_payment_meanwhile_fails_the_request() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let request = ask(&t, &invoice, &a).await;

    t.pay(&invoice, 1, 10_000).await;
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();

    assert_eq!(
        request_now(&t, &request).await.status,
        ApprovalStatus::Failed
    );
    let same = t.reload(&invoice).await;
    assert_eq!(same.status, InvoiceStatus::PartiallyPaid);
    assert_eq!(same.balance, 95_000);
    assert_eq!(same.write_off_amount, None);
    assert!(adjustments(&t, &invoice).await.is_empty());
    assert!(notices(&t).await.is_empty());
    assert!(t.written_off.all().is_empty());
    // A failed request is over; a new one is for the new balance.
    assert!(
        approvals::approve(t.state(), request.id, &b, "")
            .await
            .is_err()
    );
    let again = ask(&t, &same, &a).await;
    assert_eq!(again.payload.0["amount"], 95_000);
}

/// AC12: B rejects: the invoice keeps its state, and no adjustment and no
/// notice exist.
#[renox::test]
async fn a_rejected_write_off_changes_nothing() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let request = ask(&t, &invoice, &a).await;

    approvals::reject(t.state(), request.id, &b, "still collectable")
        .await
        .unwrap();
    assert_eq!(
        request_now(&t, &request).await.status,
        ApprovalStatus::Rejected
    );
    let same = t.reload(&invoice).await;
    assert_eq!(same.status, InvoiceStatus::Overdue);
    assert_eq!(same.write_off_amount, None);
    assert!(adjustments(&t, &invoice).await.is_empty());
    assert!(notices(&t).await.is_empty());
    assert!(t.written_off.all().is_empty());
    assert!(t.pending(&invoice).await.is_empty());
}

/// AC13: a disputed invoice is not offered the write-off and a POST is
/// refused (422).
#[renox::test]
async fn a_disputed_invoice_cannot_be_written_off() {
    let t = boot().await;
    let a = t.user_a().await;
    let invoice = t.overdue_invoice(&a).await;
    assert!(t.write_off_offered(&invoice, &a).await);

    dispute(&t, &invoice).await;
    let disputed = t.reload(&invoice).await;
    assert!(disputed.disputed);
    assert!(!t.write_off_offered(&disputed, &a).await);
    assert!(
        !t.available(&disputed, &a)
            .await
            .split(',')
            .any(|name| name == write_off::WRITE_OFF)
    );
    t.post_write_off(&disputed, &a, "insolvent", "")
        .await
        .assert_status(422);
    assert!(t.pending(&disputed).await.is_empty());
    assert_eq!(t.reload(&invoice).await.status, InvoiceStatus::Overdue);
}

/// AC13: `disputed` set while a request waits makes B's approval end in
/// `failed`.
#[renox::test]
async fn a_dispute_meanwhile_fails_the_request() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let request = ask(&t, &invoice, &a).await;

    dispute(&t, &invoice).await;
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    assert_eq!(
        request_now(&t, &request).await.status,
        ApprovalStatus::Failed
    );
    let same = t.reload(&invoice).await;
    assert_eq!(same.status, InvoiceStatus::Overdue);
    assert!(adjustments(&t, &invoice).await.is_empty());
    assert!(t.written_off.all().is_empty());
}

/// AC13: a draft or a paid invoice is refused (`NotFrom`), `credited` is not
/// a from-state, and the permission is needed.
#[renox::test]
async fn only_open_invoices_can_be_written_off() {
    let t = boot().await;
    let a = t.user_a().await;
    let viewer = t.viewer().await;

    let draft = t.draft(&[LINE], None).await;
    t.post_write_off(&draft, &a, "insolvent", "")
        .await
        .assert_status(422);
    assert_eq!(t.reload(&draft).await.status, InvoiceStatus::Draft);

    let paid = t.issued(&[LINE], None, &a).await;
    t.pay(&paid, 1, GROSS).await;
    let paid = t.reload(&paid).await;
    assert_eq!(paid.status, InvoiceStatus::Paid);
    t.post_write_off(&paid, &a, "insolvent", "")
        .await
        .assert_status(422);
    assert_eq!(t.reload(&paid).await.status, InvoiceStatus::Paid);
    assert!(t.requests(&paid).await.is_empty());

    for name in [write_off::WRITE_OFF, write_off::WRITE_OFF_APPROVED] {
        let machine = <Invoice as renox::state_machine::StateMachine>::machine();
        let transition = machine.find(name).unwrap();
        assert!(!transition.from_states().contains(&"credited".to_owned()));
        assert!(!transition.from_states().contains(&"draft".to_owned()));
    }

    let open = t.overdue_invoice(&a).await;
    t.post_write_off(&open, &viewer, "insolvent", "")
        .await
        .assert_forbidden();
    assert!(t.requests(&open).await.is_empty());
}

/// AC13: `other` without a text, or an unknown code, is refused; `other`
/// with a text is kept as the reason.
#[renox::test]
async fn the_reason_must_be_a_known_code_and_other_needs_a_text() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;

    t.post_write_off(&invoice, &a, "other", "  ")
        .await
        .assert_status(422);
    t.post_write_off(&invoice, &a, "bored", "tired of it")
        .await
        .assert_status(422);
    assert!(t.requests(&invoice).await.is_empty());

    t.post_write_off(&invoice, &a, "other", "Moved abroad")
        .await
        .assert_ok();
    let request = t.pending(&invoice).await.remove(0);
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    let done = t.reload(&invoice).await;
    assert_eq!(done.write_off_reason_code.as_deref(), Some("other"));
    assert_eq!(done.write_off_reason_text.as_deref(), Some("Moved abroad"));
}

/// AC14: with the notice setting off, approval writes the invoice off and
/// creates the pending adjustment, but sends no mail and leaves the notice
/// date empty.
#[renox::test]
async fn with_the_notice_off_nothing_is_sent() {
    let t = boot_with(WlInvoicing::new().write_off_notice(false)).await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let request = ask(&t, &invoice, &a).await;
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();

    let done = t.reload(&invoice).await;
    assert_eq!(done.status, InvoiceStatus::WrittenOff);
    assert_eq!(done.write_off_amount, Some(GROSS));
    let rows = adjustments(&t, &invoice).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, AdjustmentStatus::Pending);
    assert_eq!(rows[0].amount, 5_000);

    assert!(notices(&t).await.is_empty());
    assert_eq!(t.reload(&invoice).await.write_off_notice_on, None);
    assert_eq!(t.written_off.all().len(), 1);

    // The invoice page says why there is no notice.
    t.app.acting_as(&a);
    t.app
        .get(&format!("/invoices/{}", invoice.id))
        .await
        .assert_ok()
        .assert_see("The customer write-off notice is turned off, so none was sent.");
}

/// The write-off section of the invoice page after approval.
#[renox::test]
async fn the_invoice_page_shows_the_write_off() {
    let t = boot().await;
    let a = t.user_a().await;
    let b = t.user_b().await;
    let invoice = t.overdue_invoice(&a).await;
    let request = ask(&t, &invoice, &a).await;
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    notices(&t).await;
    t.app.acting_as(&a);
    let page = t.app.get(&format!("/invoices/{}", invoice.id)).await;
    page.assert_ok()
        .assert_see(&invoice.number.clone().unwrap())
        .assert_see("AED 1,050.00")
        .assert_see("AED 50.00")
        .assert_dont_see("Pending approval");
    // The lines are as issued.
    let lines = InvoiceLine::query()
        .where_(InvoiceLine::INVOICE_ID.eq(invoice.id))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(lines, 1);
}
