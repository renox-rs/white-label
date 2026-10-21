//! Recording the payer's remittance on a claim (`CLN.CLM.01`, 05 §7.1
//! *Claim* `record_remittance`; base for #356): the advice is matched to the
//! lines of the payer bill, the claim moves to the state the payer bill
//! reaches, and an advice that pays more than is open on a line is held for a
//! second person and records nothing. With the stand-in payer bill; the
//! approval itself, by a second person, is the real wl-invoicing's and is in
//! `claim_money_real.rs`.

mod support;

use renox::testing::TestResponse;
use wl_kernel::approvals::{ApprovalDecided, ApprovalEnd};
use wl_kernel::payer_bills::{PayerBillId, PayerBillStatus};

use clinics::{AnswerKind, Claim, ClaimRemittance, ClaimStatus};
use support::claims::{Billed, billed};
use support::day;

/// The coordinator records an advice with `reference` paying `paid`, denying
/// `denied` (with code R12) on the only line of the claim.
async fn advise(b: &Billed, reference: &str, paid: &str, denied: &str) -> TestResponse {
    let line = b.lines().await.remove(0);
    let date = day(0).to_string();
    let paid_field = format!("line_{}_paid", line.id);
    let denied_field = format!("line_{}_denied", line.id);
    let code_field = format!("line_{}_code", line.id);
    let reason_field = format!("line_{}_reason", line.id);
    b.act(
        "remittance",
        &[
            ("reference", reference),
            ("date", date.as_str()),
            (paid_field.as_str(), paid),
            (denied_field.as_str(), denied),
            (code_field.as_str(), "R12"),
            (reason_field.as_str(), "not covered"),
        ],
    )
    .await
}

/// A remittance that pays the line as claimed makes the claim `paid`; the
/// payer bill, the line, the totals and the record of the advice agree.
#[renox::test]
async fn a_remittance_paying_every_line_makes_the_claim_paid() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "360.00", "").await.assert_ok();

    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Paid);
    assert_eq!((claim.paid, claim.denied), (36_000, 0));
    assert!(claim.has_remittance);
    assert_eq!(claim.last_remittance_on, Some(day(0)));
    assert_eq!(claim.last_remittance_reference.as_deref(), Some("ADV-1"));
    assert!(!claim.is_held());
    let line = b.lines().await.remove(0);
    assert_eq!((line.paid, line.denied), (36_000, 0));
    assert_eq!(b.bill().status, PayerBillStatus::Paid);
    let answers = ClaimRemittance::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].kind, AnswerKind::Remittance);
    assert_eq!(answers[0].reference.as_deref(), Some("ADV-1"));
    assert_eq!(answers[0].recorded_by, Some(b.bea.id));
    assert_eq!(answers[0].advice_date, day(0));
    assert_eq!(answers[0].overpayment, 0);
}

/// Paid and denied lines: `partially_paid` with the denial code and reason
/// kept on the line; nothing paid on any line: `denied`.
#[renox::test]
async fn a_remittance_that_denies_leaves_the_claim_partly_paid_or_denied() {
    let partly = billed().await;
    partly.send().await;
    advise(&partly, "ADV-1", "200.00", "160.00")
        .await
        .assert_ok();
    let claim = partly.claim().await;
    assert_eq!(claim.status, ClaimStatus::PartiallyPaid);
    assert_eq!((claim.paid, claim.denied), (20_000, 16_000));
    let line = partly.lines().await.remove(0);
    assert_eq!(line.denial_code.as_deref(), Some("R12"));
    assert_eq!(line.denial_reason.as_deref(), Some("not covered"));

    let denied = billed().await;
    denied.send().await;
    advise(&denied, "ADV-2", "0", "360.00").await.assert_ok();
    let claim = denied.claim().await;
    assert_eq!(claim.status, ClaimStatus::Denied);
    assert_eq!((claim.paid, claim.denied), (0, 36_000));
    assert_eq!(denied.bill().status, PayerBillStatus::Denied);
}

/// An advice that pays more than the line has open waits for a second
/// person: the claim stays `submitted`, nothing is recorded, and another
/// advice is refused while it waits.
#[renox::test]
async fn an_overpayment_is_held_and_records_nothing() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "450.00", "").await.assert_ok();

    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Submitted);
    assert_eq!(claim.held_kind.as_deref(), Some("remittance"));
    assert_eq!(claim.held_request_id, Some(77));
    assert_eq!(claim.held_reference.as_deref(), Some("ADV-1"));
    assert_eq!(claim.held_date, Some(day(0)));
    assert_eq!(claim.held_by, Some(b.bea.id));
    assert!(!claim.has_remittance);
    assert_eq!(claim.paid, 0);
    assert!(
        ClaimRemittance::of_claim(b.db(), claim.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(b.bill().status, PayerBillStatus::Issued);
    assert_eq!(b.lines().await[0].paid, 0);

    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Nothing is recorded until it is given");
    advise(&b, "ADV-2", "360.00", "").await.assert_status(422);
    assert_eq!(b.claim().await.status, ClaimStatus::Submitted);
}

/// When the approval ends without a yes the hold is cleared, nothing was
/// recorded, and the way it ended is shown.
#[renox::test]
async fn a_hold_that_ends_without_a_yes_records_nothing() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "450.00", "").await.assert_ok();

    b.d.t
        .state()
        .emit(ApprovalDecided {
            request: 77,
            decision: ApprovalEnd::Rejected,
            approvers: vec![],
            subject: PayerBillId(b.claim.payer_bill_id).subject(),
            state_returned_to: None,
        })
        .await
        .expect("the event is handled");

    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Submitted);
    assert!(!claim.is_held());
    assert_eq!(claim.held_result.as_deref(), Some("rejected"));
    assert!(!claim.has_remittance);
    assert!(
        ClaimRemittance::of_claim(b.db(), claim.id)
            .await
            .unwrap()
            .is_empty()
    );
    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("ended without a yes");
    // The advice can be entered again, corrected.
    advise(&b, "ADV-1", "360.00", "").await.assert_ok();
    assert_eq!(b.claim().await.status, ClaimStatus::Paid);
}

/// Only a claim with the payer takes a remittance, and only a holder of
/// `claims.manage` records it.
#[renox::test]
async fn a_remittance_needs_a_claim_with_the_payer_and_the_permission() {
    let b = billed().await;
    // A draft claim was never sent.
    advise(&b, "ADV-1", "360.00", "").await.assert_status(422);
    assert_eq!(b.claim().await.status, ClaimStatus::Draft);

    b.send().await;
    let line = b.lines().await.remove(0);
    let paid_field = format!("line_{}_paid", line.id);
    b.d.post(
        &b.d.rita,
        &b.path("remittance"),
        &[("reference", "ADV-9"), (paid_field.as_str(), "360.00")],
    )
    .await
    .assert_forbidden();
    assert_eq!(b.claim().await.status, ClaimStatus::Submitted);
    assert_eq!(
        Claim::of_visit(b.db(), b.claim.visit_id)
            .await
            .unwrap()
            .len(),
        1
    );
}
