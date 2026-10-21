//! The COD remittance lifecycle (base for BL-LOG-14 #312 and BL-LOG-15 #313),
//! 03 §7.1 *COD remittance*, `LOG.ACL.07`, `LOG.COD.01`, `PTY.ACL.03`: a
//! draft is approved by a user other than its preparer, an approved one is
//! reopened to a draft, and `pay` records the payout in the real wl-payments
//! (an outgoing transfer to the client's bank account) only when its guards
//! hold.

mod support;

use logistics::RemittanceStatus;
use renox::prelude::*;
use wl_kernel::payments::{self, Direction, PaymentId, PaymentStatus};
use wl_payments::payment::Payment;

use support::{SOUQ, World, boot, boot_with_payments};

const ACCOUNT: &str = "AE070331234567890123456";

/// A draft of AED 300 for Souq Express that `preparer` built by hand.
async fn draft_by(w: &World, preparer: &User) -> logistics::Remittance {
    w.delivered(SOUQ, "ORD-1", "300").await;
    w.post(preparer, "/remittances/build", &[("client_party_id", "501")])
        .await
        .assert_status(303);
    w.the_remittance_of(SOUQ).await
}

/// The draft the preparer built is approved by another user: the preparer is
/// refused, the second accountant approves, and who approved is kept.
#[renox::test]
async fn a_draft_is_approved_by_another_user_than_its_preparer() {
    let w = boot().await;
    let draft = draft_by(&w, &w.amal).await;
    assert_eq!(draft.status, RemittanceStatus::Draft);
    assert_eq!(draft.prepared_by, Some(w.amal.id));

    let path = format!("/remittances/{}/approve", draft.id);
    w.post(&w.amal, &path, &[])
        .await
        .assert_status(422)
        .assert_see("You prepared this remittance");
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Draft);

    w.post(&w.bashar, &path, &[]).await.assert_status(303);
    let approved = w.remittance(&draft).await;
    assert_eq!(approved.status, RemittanceStatus::Approved);
    assert_eq!(approved.approved_by, Some(w.bashar.id));
    assert!(approved.approved_at.is_some());
}

/// Only a holder of `remittances.approve` approves or reopens, and building a
/// draft needs `remittances.prepare`.
#[renox::test]
async fn the_permissions_decide_who_builds_approves_reopens_and_pays() {
    let w = boot().await;
    w.delivered(SOUQ, "ORD-1", "300").await;
    w.post(&w.dispatcher, "/remittances/build", &[("client_party_id", "501")])
        .await
        .assert_forbidden();
    w.post(&w.amal, "/remittances/build", &[("client_party_id", "501")])
        .await
        .assert_status(303);
    let draft = w.the_remittance_of(SOUQ).await;
    let approve = format!("/remittances/{}/approve", draft.id);
    w.post(&w.dispatcher, &approve, &[]).await.assert_forbidden();
    w.post(&w.bashar, &approve, &[]).await.assert_status(303);
    w.post(
        &w.dispatcher,
        &format!("/remittances/{}/reopen", draft.id),
        &[],
    )
    .await
    .assert_forbidden();
    w.post(
        &w.dispatcher,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_forbidden();
    // Reading needs `remittances.view`.
    w.get(&w.hub, "/remittances").await.assert_forbidden();
    w.get(&w.amal, "/remittances").await.assert_ok();
    w.get(&w.amal, &format!("/remittances/{}", draft.id))
        .await
        .assert_ok()
        .assert_see(&draft.number);
}

/// An approved remittance is taken back to a draft, and only then can it be
/// approved again; a draft cannot be reopened.
#[renox::test]
async fn an_approved_remittance_is_reopened_to_a_draft() {
    let w = boot().await;
    let draft = draft_by(&w, &w.amal).await;
    let reopen = format!("/remittances/{}/reopen", draft.id);
    w.post(&w.bashar, &reopen, &[]).await.assert_status(422);
    w.post(&w.bashar, &format!("/remittances/{}/approve", draft.id), &[])
        .await
        .assert_status(303);
    w.post(&w.bashar, &reopen, &[]).await.assert_status(303);
    let reopened = w.remittance(&draft).await;
    assert_eq!(reopened.status, RemittanceStatus::Draft);
    assert_eq!(reopened.approved_by, None);
}

/// Paying records one outgoing transfer of the net to the client in the real
/// wl-payments, with the bank reference and account, and the voucher is on
/// the remittance.
#[renox::test]
async fn paying_records_the_payout_in_the_real_payments_module() {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    let draft = draft_by(&w, &w.amal).await;
    w.post(&w.bashar, &format!("/remittances/{}/approve", draft.id), &[])
        .await
        .assert_status(303);

    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-0001")],
    )
    .await
    .assert_status(303);

    let paid = w.remittance(&draft).await;
    assert_eq!(paid.status, RemittanceStatus::Paid);
    assert_eq!(paid.paid_by, Some(w.amal.id));
    assert_eq!(paid.payment_reference.as_deref(), Some("TRF-0001"));
    assert_eq!(paid.bank_account.as_deref(), Some(ACCOUNT));
    let payment = payments::payment_recorder(w.state())
        .find(w.db(), PaymentId(paid.payment_id.expect("a voucher")))
        .await
        .unwrap()
        .expect("the payment exists in wl-payments");
    assert_eq!(payment.direction, Direction::Out);
    assert_eq!(payment.amount, 30_000);
    assert_eq!(payment.party, SOUQ);
    assert_eq!(payment.status, PaymentStatus::Received);
    assert_eq!(payment.number, paid.payment_number);
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 1);
    // A paid remittance cannot be paid, approved or reopened again.
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-0002")],
    )
    .await
    .assert_status(422);
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 1);
}

/// Each guard of `pay` refuses with its reason and records no payout: no bank
/// reference, no bank account on file, a blocked client.
#[renox::test]
async fn a_payment_that_a_guard_refuses_records_no_payout() {
    let w = boot_with_payments().await;
    let draft = draft_by(&w, &w.amal).await;
    w.post(&w.bashar, &format!("/remittances/{}/approve", draft.id), &[])
        .await
        .assert_status(303);
    let pay = format!("/remittances/{}/pay", draft.id);

    // No account on file.
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(422)
        .assert_see("no bank account on file");
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    // No reference.
    w.post(&w.amal, &pay, &[("payment_reference", " ")])
        .await
        .assert_status(422)
        .assert_see("bank reference");
    // A blocked client waits.
    w.parties.block(SOUQ);
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(422)
        .assert_see("on hold");

    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 0);
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Approved);

    w.parties.unblock(SOUQ);
    w.post(&w.amal, &pay, &[("payment_reference", "TRF-1")])
        .await
        .assert_status(303);
    assert_eq!(w.remittance_status(&draft).await, RemittanceStatus::Paid);
}

/// A draft with nothing to pay (net AED 0) cannot be paid and records no
/// payout.
#[renox::test]
async fn a_remittance_of_zero_has_nothing_to_pay() {
    let w = boot_with_payments().await;
    w.cod_terms(SOUQ, 0, Some(ACCOUNT)).await;
    let draft = draft_by(&w, &w.amal).await;
    // The shipment line is removed from the draft by a rebuild once its COD
    // is marked already remitted; the draft stays with a net of 0.
    logistics::Shipment::query()
        .where_(logistics::Shipment::CLIENT_PARTY_ID.eq(SOUQ.0))
        .update(w.db(), &[("cod_already_remitted", &true)])
        .await
        .unwrap();
    w.post(&w.amal, "/remittances/build", &[("client_party_id", "501")])
        .await
        .assert_status(303);
    let empty = w.remittance(&draft).await;
    assert_eq!(empty.net, 0);
    w.post(&w.bashar, &format!("/remittances/{}/approve", draft.id), &[])
        .await
        .assert_status(303);
    w.post(
        &w.amal,
        &format!("/remittances/{}/pay", draft.id),
        &[("payment_reference", "TRF-1")],
    )
    .await
    .assert_status(422)
    .assert_see("nothing to pay");
    assert_eq!(Payment::query().count(w.db()).await.unwrap(), 0);
}
