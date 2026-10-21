//! BL-LOG-16 (#314): net open invoices against the COD in the remittance,
//! 03 §4.1 row *Client owes invoices*, §7.1 `pay`, `LOG.RUL.06`, `LOG.COD.01`,
//! `FIN.PAY.01`. The offsets are payments in the real wl-payments allocated to
//! the invoices of the test invoicer.

mod support;

use logistics::netting::Offset;
use logistics::remit;
use logistics::{Remittance, RemittanceStatus, Statement};
use renox::prelude::*;
use wl_kernel::invoicing::InvoiceId;
use wl_kernel::payments::{Direction, PaymentMethod};
use wl_payments::payment::Payment;

use support::{SOUQ, World, boot_with_payments};

const ACCOUNT: &str = "AE070331234567890123456";

/// Netting allowed for Souq Express, a delivered shipment of AED 2,000 COD,
/// and the open invoices INV-1 (AED 500, due 2026-10-01) and INV-2
/// (AED 1,800, due 2026-11-01).
async fn scene() -> (World, InvoiceId, InvoiceId) {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    remit::set_netting_allowed(w.db(), SOUQ, true)
        .await
        .unwrap();
    let first = w.delivery_invoice(SOUQ, "INV-1", 50_000, (2026, 10, 1)).await;
    let second = w
        .delivery_invoice(SOUQ, "INV-2", 180_000, (2026, 11, 1))
        .await;
    w.delivered(SOUQ, "ORD-1", "2000").await;
    (w, first, second)
}

async fn offsets(w: &World, remittance: &Remittance) -> Vec<(String, i64)> {
    remittance
        .offsets(w.db())
        .await
        .unwrap()
        .into_iter()
        .map(|offset: Offset| (offset.invoice_number, offset.amount))
        .collect()
}

/// The draft is approved by bashar and paid by amal.
async fn approve_and_pay(
    w: &World,
    draft: &Remittance,
    reference: &str,
) -> renox::testing::TestResponse {
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", reference)],
    )
    .await
}

/// AC1: given netting allowed, COD net AED 2,000 and open invoices INV-1
/// (AED 500, due 2026-10-01) and INV-2 (AED 1,800, due 2026-11-01), the draft
/// offsets AED 500 and AED 1,500 (part of INV-2) and the net to pay is AED 0.
#[renox::test]
async fn the_draft_offsets_the_oldest_invoice_first_and_the_last_in_part() {
    let (w, _first, _second) = scene().await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(draft.status, RemittanceStatus::Draft);
    assert_eq!(draft.cod_total, 200_000);
    assert_eq!(
        offsets(&w, &draft).await,
        vec![("INV-1".to_owned(), 50_000), ("INV-2".to_owned(), 150_000)]
    );
    assert_eq!(draft.offset_total, 200_000);
    assert_eq!(draft.net, 0);
    // Nothing is recorded before `pay`.
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 0);
    assert!(w.invoices.allocations().is_empty());
}

/// AC2: given INV-1 `disputed`, it is not offset.
#[renox::test]
async fn a_disputed_invoice_is_not_offset() {
    let (w, first, _second) = scene().await;
    w.invoices.dispute(first);
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    assert_eq!(
        offsets(&w, &draft).await,
        vec![("INV-2".to_owned(), 180_000)]
    );
    assert_eq!(draft.offset_total, 180_000);
    assert_eq!(draft.net, 20_000);
}

/// AC3: on `pay`, two payments (AED 500 and AED 1,500, method *COD netting*)
/// are allocated to INV-1 and INV-2, the statement lists them, and nothing
/// goes out to the bank for a net of AED 0.
#[renox::test]
async fn paying_records_a_payment_per_offset_allocated_to_its_invoice() {
    let (w, first, second) = scene().await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    approve_and_pay(&w, &draft, "TRF-1")
        .await
        .assert_status(303);

    let paid = w.remittance(&draft).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    assert_eq!(
        paid.payment_id, None,
        "a net of 0 sends nothing to the bank"
    );
    let payments = Payment::query()
        .order_by(Payment::ID)
        .get(w.db())
        .await
        .unwrap();
    assert_eq!(payments.len(), 2);
    for (payment, amount) in payments.iter().zip([50_000, 150_000]) {
        assert_eq!(payment.direction, Direction::In);
        assert_eq!(payment.method, PaymentMethod::Transfer);
        assert_eq!(payment.detail("kind").as_deref(), Some(remit::COD_NETTING));
        assert_eq!(payment.reference.as_deref(), Some(draft.number.as_str()));
        assert_eq!(payment.amount, amount);
    }
    let allocations = w.invoices.allocations();
    assert_eq!(allocations.len(), 2);
    assert_eq!((allocations[0].0, allocations[0].2), (first, 50_000));
    assert_eq!((allocations[1].0, allocations[1].2), (second, 150_000));
    assert_eq!(allocations[0].1.subject_id, payments[0].id);
    assert_eq!(w.invoices.invoice(first).balance, 0);
    assert_eq!(w.invoices.invoice(second).balance, 30_000);
    // The offsets keep the payment they became.
    let stored = paid.offsets(w.db()).await.unwrap();
    assert!(stored.iter().all(|offset| offset.payment_id.is_some()));

    // The published statement lists each offset invoice with its amount.
    w.get(&w.amal, &format!("/remittances/{}/statement", draft.id))
        .await
        .assert_ok()
        .assert_see("INV-1")
        .assert_see("50000")
        .assert_see("INV-2")
        .assert_see("150000");
    // The client user of the company reads it, another client's user does not.
    w.get(&w.noor, &format!("/remittances/{}/statement", draft.id))
        .await
        .assert_ok();
    w.get(&w.farah, &format!("/remittances/{}/statement", draft.id))
        .await
        .assert_forbidden();
}

/// A net above 0 is paid to the bank and the offsets are paid beside it.
#[renox::test]
async fn the_cash_part_goes_to_the_bank_and_the_offsets_are_settled_beside_it() {
    let (w, first, _second) = scene().await;
    w.invoices.dispute(first);
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    approve_and_pay(&w, &draft, "TRF-1")
        .await
        .assert_status(303);
    let paid = w.remittance(&draft).await;
    assert!(paid.payment_id.is_some());
    let payments = Payment::query().get(w.db()).await.unwrap();
    assert_eq!(payments.len(), 2);
    let out: Vec<&Payment> = payments
        .iter()
        .filter(|payment| payment.direction == Direction::Out)
        .collect();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].amount, 20_000);
}

/// AC4: given INV-2 disputed after approval, `pay` does not pay: the
/// remittance goes back to `draft` without INV-2's offset, the accountant sees
/// the message "offset INV-2 removed: invoice disputed", no payment and no
/// statement are recorded, and it must be approved again before `pay`.
#[renox::test]
async fn an_invoice_disputed_after_approval_sends_the_remittance_back_to_a_draft() {
    let (w, _first, second) = scene().await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    w.invoices.dispute(second);

    let pay = format!("/remittances/{}/pay", draft.id);
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(422)
        .assert_see("offset INV-2 removed: invoice disputed");

    let back = w.remittance(&draft).await;
    assert_eq!(back.status, RemittanceStatus::Draft);
    assert_eq!(
        back.message.as_deref(),
        Some("offset INV-2 removed: invoice disputed")
    );
    assert_eq!(offsets(&w, &back).await, vec![("INV-1".to_owned(), 50_000)]);
    assert_eq!(back.net, 150_000);
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 0);
    assert_eq!(Statement::query().count(w.db()).await.unwrap(), 0);
    assert!(w.invoices.allocations().is_empty());
    // The accountant reads the message on the page.
    w.get(&w.amal, &format!("/remittances/{}", draft.id))
        .await
        .assert_ok()
        .assert_see("offset INV-2 removed: invoice disputed");

    // Paying a draft is refused; it is approved again first, and then pays.
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(422);
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(w.remittance(&draft).await.message, None);
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(303);
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Paid);
    assert_eq!(w.invoices.allocations().len(), 1);
}

/// An invoice that was paid meanwhile also sends the remittance back, with
/// its own reason.
#[renox::test]
async fn an_invoice_written_off_meanwhile_is_removed_with_its_reason() {
    let (w, first, _second) = scene().await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    w.post(
        &w.bashar,
        &format!("/remittances/{}/approve", draft.id),
        &[],
    )
    .await
    .assert_status(303);
    w.invoices.write_off(first);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_status(422)
    .assert_see("offset INV-1 removed: invoice written off");
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Draft);
}

/// AC5: given netting not allowed, no offsets appear.
#[renox::test]
async fn without_netting_in_the_contract_no_offset_appears() {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    w.delivery_invoice(SOUQ, "INV-1", 50_000, (2026, 10, 1)).await;
    w.delivered(SOUQ, "ORD-1", "2000").await;
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    assert!(offsets(&w, &draft).await.is_empty());
    assert_eq!(draft.offset_total, 0);
    assert_eq!(draft.net, 200_000);
    approve_and_pay(&w, &draft, "TRF-1")
        .await
        .assert_status(303);
    assert!(w.invoices.allocations().is_empty());
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 1);
}

/// AC1 (delivery invoices only): an open invoice of the client that is not
/// an invoice of the monthly delivery run (a freight job's, say) is never
/// set off against the COD.
#[renox::test]
async fn an_invoice_for_other_services_is_not_offset() {
    let (w, _first, _second) = scene().await;
    w.invoices.open(SOUQ, "FRT-1", 90_000, (2026, 9, 1));
    w.rule().await;
    let draft = w.the_remittance_of(SOUQ).await;
    let taken = offsets(&w, &draft).await;
    assert_eq!(
        taken,
        vec![("INV-1".to_owned(), 50_000), ("INV-2".to_owned(), 150_000)],
        "FRT-1 is older but not a delivery invoice"
    );
    assert_eq!(draft.offset_total, 200_000);
}
