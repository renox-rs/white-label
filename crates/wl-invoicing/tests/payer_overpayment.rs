//! A payer's overpayment and recoupment (wl-invoicing §3, §4 *Payer bill*
//! `record_remittance` and `record_recoupment`, §5.8 step 7, `FIN.ACL.16`,
//! #80): the acceptance checks of BL-INV-17, driven through the payer bill
//! pages, the approval kernel and the event of a reversed payment.

mod support;

use renox::prelude::*;
use wl_invoicing::payer_credit;
use wl_invoicing::payer_recoupment::PayerRecoupment;
use wl_invoicing::payer_remittance::{PayerAllocation, PayerRemittance};
use wl_invoicing::{PayerBill, VatRecipient};
use wl_kernel::approvals::{self, ApprovalRequest, ApprovalStatus};
use wl_kernel::documents::today;
use wl_kernel::payer_bills::PayerBillStatus;
use wl_kernel::payments::{PaymentId, PaymentReversed, PaymentSource, ReversalReason};

use support::{BLUE_DUNES, Booted, boot};

/// A payer bill with one line of AED 400.00, issued.
async fn issued_bill(t: &Booted) -> PayerBill {
    let cora = t.coordinator_cora().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let bill = t
        .payer_bill(&agreement, today(), &[("Consultation", 40_000)])
        .await;
    t.issue_payer_bill(&bill, &cora).await
}

/// The field of the paid amount of `line` in the remittance sheet.
fn paid_field(line: i64) -> String {
    format!("line_{line}_paid")
}

/// User A records a remittance paying `amount` on the only line.
async fn remit(
    t: &Booted,
    bill: &PayerBill,
    by: &User,
    reference: &str,
    amount: &str,
    extra: &[(&str, &str)],
) -> renox::testing::TestResponse {
    let line = t.bill_lines(bill).await.remove(0);
    let field = paid_field(line.id);
    let mut fields = vec![
        ("reference", reference),
        ("date", "2026-12-10"),
        (field.as_str(), amount),
    ];
    fields.extend_from_slice(extra);
    t.post_payer(bill, by, "remittance", &fields).await
}

/// The pending approval requests about `bill`.
async fn pending(t: &Booted, bill: &PayerBill) -> Vec<ApprovalRequest> {
    approvals::pending_for(t.db(), &bill.subject())
        .await
        .unwrap()
}

/// The credit balance of the payer of `bill`.
async fn credit(t: &Booted, bill: &PayerBill) -> i64 {
    payer_credit::balance(t.db(), BLUE_DUNES, bill.legal_entity_id, "AED")
        .await
        .unwrap()
}

/// AC1: with a payer bill line open AED 400.00 and a remittance paying AED
/// 450.00, the remittance is held; user B approves; AED 400.00 is allocated
/// and AED 50.00 goes to the payer credit balance.
#[renox::test]
async fn an_overpayment_is_held_until_a_second_person_approves() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    let b = t.reconciler("Bea").await;

    remit(&t, &bill, &a, "ADV-1", "450.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));

    // Held: nothing is recorded, the bill is as it was.
    let held = t.reload_bill(&bill).await;
    assert_eq!(held.status, PayerBillStatus::Issued);
    assert!(!held.has_remittance);
    assert_eq!(held.paid_total, 0);
    assert!(
        PayerRemittance::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(credit(&t, &bill).await, 0);
    assert!(t.payer.remitted.all().is_empty());
    let requests = pending(&t, &bill).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "invoicing.payer_money");
    assert_eq!(requests[0].requester_id, a.id);
    assert_eq!(requests[0].payload.0["amount"], 5_000);

    approvals::approve(t.state(), requests[0].id, &b, "ok")
        .await
        .unwrap();

    let paid = t.reload_bill(&bill).await;
    assert_eq!(paid.status, PayerBillStatus::Paid);
    assert_eq!((paid.paid_total, paid.balance), (40_000, 0));
    assert!(paid.has_remittance);
    assert!(!paid.in_ageing, "nothing is open any more");
    assert_eq!(
        paid.last_remittance_on,
        Some(support::payer::d("2026-12-10"))
    );
    let line = t.bill_lines(&paid).await.remove(0);
    assert_eq!(line.paid, 40_000);
    let allocations = PayerAllocation::query().get(t.db()).await.unwrap();
    assert_eq!(allocations.len(), 1);
    assert_eq!(allocations[0].amount, 40_000, "AED 400.00 is allocated");
    assert_eq!(credit(&t, &bill).await, 5_000, "AED 50.00 is credit");
    let rows = PayerRemittance::query().get(t.db()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].reference, "ADV-1");
    assert_eq!(rows[0].overpayment, 5_000);
    assert_eq!(rows[0].approval_request_id, Some(requests[0].id));
    let events = t.payer.remitted.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].overpayment, 5_000);
    assert_eq!(events[0].paid, 40_000);
}

/// AC2: user A (the reconciler) cannot approve it, also when A is an admin.
#[renox::test]
async fn the_reconciler_cannot_approve_their_own_overpayment() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    remit(&t, &bill, &a, "ADV-1", "450.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));
    let request = pending(&t, &bill).await.remove(0);

    let refused = approvals::approve(t.state(), request.id, &a, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)));
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Issued);
    assert_eq!(credit(&t, &bill).await, 0);
}

/// A rejected overpayment records nothing: the bill stays issued and the
/// reconciler corrects the advice.
#[renox::test]
async fn a_rejected_overpayment_records_nothing() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    let b = t.reconciler("Bea").await;
    remit(&t, &bill, &a, "ADV-1", "450.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));
    let request = pending(&t, &bill).await.remove(0);

    approvals::reject(t.state(), request.id, &b, "The advice is wrong")
        .await
        .unwrap();
    let stored = ApprovalRequest::find(t.db(), request.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.status, ApprovalStatus::Rejected);
    let same = t.reload_bill(&bill).await;
    assert_eq!(same.status, PayerBillStatus::Issued);
    assert!(!same.has_remittance);
    assert!(
        PayerRemittance::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(credit(&t, &bill).await, 0);

    // The corrected advice, paying what is open, needs no approval and uses
    // the same reference.
    remit(&t, &bill, &a, "ADV-1", "400.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Paid);
    assert!(pending(&t, &bill).await.is_empty());
}

/// The same advice is not recorded twice: its reference is unique per payer.
#[renox::test]
async fn an_advice_reference_is_recorded_once_per_payer() {
    let t = boot().await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let cora = t.coordinator_cora().await;
    let a = t.reconciler("Ana").await;
    let first = t
        .payer_bill(&agreement, today(), &[("Consultation", 40_000)])
        .await;
    let first = t.issue_payer_bill(&first, &cora).await;
    let second = t.payer_bill(&agreement, today(), &[("Lab", 20_000)]).await;
    let second = t.issue_payer_bill(&second, &cora).await;

    remit(&t, &first, &a, "ADV-7", "200.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", first.id));
    assert_eq!(
        t.reload_bill(&first).await.status,
        PayerBillStatus::PartiallyPaid
    );
    remit(&t, &second, &a, "ADV-7", "200.00", &[])
        .await
        .assert_status(422);
    assert_eq!(t.reload_bill(&second).await.status, PayerBillStatus::Issued);
}

/// AC3: given a `paid` bill (one line AED 400.00) and a later remittance
/// recouping AED 400.00 with code "R07", after user B approves the bill is
/// `denied`, the line shows code R07, the payer credit balance of AED 50.00
/// is used first and AED 350.00 is deducted from the remittance,
/// `PayerRecoupmentRecorded` is emitted.
///
/// Seam: the spec's entry is a negative line on an imported remittance
/// advice. The remittance import is "Later" (`payer_remittance.rs`), so the
/// recoupment form of the bill page (`recoupment` with its carrier
/// reference) is the only entry until the import exists; this test drives
/// that form and the import must call the same service.
#[renox::test]
async fn a_recoupment_reopens_the_line_as_denied_and_uses_the_credit_first() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    let b = t.reconciler("Bea").await;
    // AED 400.00 paid, AED 50.00 over (approved) as the credit balance.
    remit(&t, &bill, &a, "ADV-1", "450.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));
    let request = pending(&t, &bill).await.remove(0);
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();
    assert_eq!(credit(&t, &bill).await, 5_000);
    let line = t.bill_lines(&bill).await.remove(0);

    let field = format!("line_{}_recouped", line.id);
    t.post_payer(
        &bill,
        &a,
        "recoupment",
        &[
            ("code", "R07"),
            ("reason", "Not covered after audit"),
            ("carrier_reference", "ADV-2"),
            ("carrier_date", "2026-12-20"),
            (field.as_str(), "400.00"),
        ],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));

    // Waits for B: the earlier bill is unchanged.
    let waiting = t.reload_bill(&bill).await;
    assert_eq!(waiting.status, PayerBillStatus::Paid);
    assert_eq!(waiting.paid_total, 40_000);
    assert!(
        PayerRecoupment::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    let request = pending(&t, &bill).await.remove(0);
    assert_eq!(request.payload.0["amount"], 40_000);
    // A cannot approve it.
    assert!(matches!(
        approvals::approve(t.state(), request.id, &a, "").await,
        Err(Error::Forbidden)
    ));

    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();

    let denied = t.reload_bill(&bill).await;
    assert_eq!(denied.status, PayerBillStatus::Denied);
    assert_eq!((denied.paid_total, denied.denied_total), (0, 40_000));
    assert_eq!(denied.balance, 40_000, "the receivable is open again");
    assert!(denied.in_ageing);
    let line = t.bill_lines(&denied).await.remove(0);
    assert_eq!((line.paid, line.denied), (0, 40_000));
    assert_eq!(line.recoupment_code.as_deref(), Some("R07"));
    let rows = PayerRecoupment::query().get(t.db()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount, 40_000);
    assert_eq!(rows[0].from_credit, 5_000, "the credit is used first");
    assert_eq!(rows[0].deducted, 35_000, "the rest is deducted");
    assert_eq!(rows[0].carrier_reference.as_deref(), Some("ADV-2"));
    assert!(!rows[0].reversal);
    assert_eq!(credit(&t, &bill).await, 0);
    let events = t.payer.recouped.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].code, "R07");
    assert_eq!(
        (events[0].amount, events[0].from_credit, events[0].deducted),
        (40_000, 5_000, 35_000)
    );
    assert!(!events[0].reversal);
}

/// AC5: a recoupment of AED 500.00 on lines paid AED 400.00 is refused.
#[renox::test]
async fn a_recoupment_above_what_was_paid_is_refused() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    remit(&t, &bill, &a, "ADV-1", "400.00", &[])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Paid);
    let line = t.bill_lines(&bill).await.remove(0);
    let field = format!("line_{}_recouped", line.id);

    t.post_payer(
        &bill,
        &a,
        "recoupment",
        &[("code", "R07"), (field.as_str(), "500.00")],
    )
    .await
    .assert_status(422);
    assert!(pending(&t, &bill).await.is_empty(), "no request is made");
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Paid);
    assert!(t.payer.recouped.all().is_empty());
}

/// The partial recoupment of one line of two leaves the bill partly paid
/// with the recouped amount denied.
#[renox::test]
async fn a_partial_recoupment_leaves_the_bill_partly_paid() {
    let t = boot().await;
    let cora = t.coordinator_cora().await;
    let a = t.reconciler("Ana").await;
    let b = t.reconciler("Bea").await;
    let agreement = t.agreement(BLUE_DUNES, VatRecipient::Customer).await;
    let bill = t
        .payer_bill(
            &agreement,
            today(),
            &[("Consultation", 40_000), ("Lab", 10_000)],
        )
        .await;
    let bill = t.issue_payer_bill(&bill, &cora).await;
    let lines = t.bill_lines(&bill).await;
    let (first, second) = (paid_field(lines[0].id), paid_field(lines[1].id));
    t.post_payer(
        &bill,
        &a,
        "remittance",
        &[
            ("reference", "ADV-1"),
            (first.as_str(), "400.00"),
            (second.as_str(), "100.00"),
        ],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Paid);

    let field = format!("line_{}_recouped", lines[1].id);
    t.post_payer(
        &bill,
        &a,
        "recoupment",
        &[("code", "R09"), (field.as_str(), "100.00")],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));
    let request = pending(&t, &bill).await.remove(0);
    approvals::approve(t.state(), request.id, &b, "")
        .await
        .unwrap();

    let after = t.reload_bill(&bill).await;
    assert_eq!(after.status, PayerBillStatus::PartiallyPaid);
    assert_eq!((after.paid_total, after.denied_total), (40_000, 10_000));
    let lines = t.bill_lines(&after).await;
    assert_eq!((lines[0].paid, lines[0].denied), (40_000, 0));
    assert_eq!((lines[1].paid, lines[1].denied), (0, 10_000));
    assert_eq!(lines[1].recoupment_code.as_deref(), Some("R09"));
}

/// AC4: a `PaymentReversed` for a payment allocated to the bill runs
/// `record_recoupment` without approval.
#[renox::test]
async fn a_reversed_payment_recoups_without_approval() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    remit(&t, &bill, &a, "ADV-1", "400.00", &[("payment", "77")])
        .await
        .assert_redirect(&format!("/payer-bills/{}", bill.id));
    assert_eq!(t.reload_bill(&bill).await.status, PayerBillStatus::Paid);
    let allocations = PayerAllocation::query().get(t.db()).await.unwrap();
    assert_eq!(allocations[0].payment_id, Some(77));

    t.state()
        .emit(PaymentReversed {
            payment: PaymentId(77),
            party: BLUE_DUNES,
            legal_entity_id: bill.legal_entity_id,
            reason: ReversalReason::BankReturn,
            source: PaymentSource::Manual,
            statement_line: None,
            removed: vec![],
            prepaid: None,
            reversed_on: today(),
        })
        .await
        .expect("the event is handled");

    let denied = t.reload_bill(&bill).await;
    assert_eq!(denied.status, PayerBillStatus::Denied);
    assert_eq!((denied.paid_total, denied.denied_total), (0, 40_000));
    assert!(pending(&t, &bill).await.is_empty(), "no approval is needed");
    let rows = PayerRecoupment::query().get(t.db()).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].reversal);
    assert_eq!(rows[0].code, "payment_reversed");
    assert_eq!(rows[0].payment_id, Some(77));
    assert_eq!((rows[0].from_credit, rows[0].deducted), (0, 0));
    let allocations = PayerAllocation::query().get(t.db()).await.unwrap();
    assert!(allocations[0].removed_at.is_some());
    let events = t.payer.recouped.all();
    assert_eq!(events.len(), 1);
    assert!(events[0].reversal);
    assert_eq!(events[0].amount, 40_000);

    // The same event again changes nothing.
    t.state()
        .emit(PaymentReversed {
            payment: PaymentId(77),
            party: BLUE_DUNES,
            legal_entity_id: bill.legal_entity_id,
            reason: ReversalReason::BankReturn,
            source: PaymentSource::Manual,
            statement_line: None,
            removed: vec![],
            prepaid: None,
            reversed_on: today(),
        })
        .await
        .unwrap();
    assert_eq!(t.payer.recouped.all().len(), 1);
}

/// A remittance that denies every line leaves the bill `denied`, with the
/// payer's code and reason on the line; the denied amount is still
/// receivable.
#[renox::test]
async fn a_remittance_denying_everything_denies_the_bill() {
    let t = boot().await;
    let bill = issued_bill(&t).await;
    let a = t.reconciler("Ana").await;
    let line = t.bill_lines(&bill).await.remove(0);
    let denied = format!("line_{}_denied", line.id);
    let code = format!("line_{}_code", line.id);
    let reason = format!("line_{}_reason", line.id);
    t.post_payer(
        &bill,
        &a,
        "remittance",
        &[
            ("reference", "ADV-3"),
            (denied.as_str(), "400.00"),
            (code.as_str(), "R12"),
            (reason.as_str(), "Not covered"),
        ],
    )
    .await
    .assert_redirect(&format!("/payer-bills/{}", bill.id));
    let after = t.reload_bill(&bill).await;
    assert_eq!(after.status, PayerBillStatus::Denied);
    assert_eq!((after.paid_total, after.denied_total), (0, 40_000));
    let line = t.bill_lines(&after).await.remove(0);
    assert_eq!(line.denial_code.as_deref(), Some("R12"));
    assert_eq!(line.denial_reason.as_deref(), Some("Not covered"));
    assert!(after.in_ageing, "the denied amount is still receivable");
}
