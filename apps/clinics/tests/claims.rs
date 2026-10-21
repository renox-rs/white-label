//! The claim lifecycle base (`CLN.CLM.01`, `CLN.ACL.09`; base for #355 to
//! #359): the claim a billed insured visit makes around its draft payer bill,
//! `validate`, `submit`, the lock of a submitted claim and `resubmit`, driven
//! through the routes of the billing coordinator with the stand-in payer
//! bill. The real payer bill is in `claim_rejection_real.rs` and
//! `claim_money_real.rs`.

mod support;

use renox::prelude::*;
use wl_kernel::documents::today;
use wl_kernel::payer_bills::PayerBillStatus;

use clinics::authorisations;
use clinics::{AttemptKind, Claim, ClaimAttempt, ClaimStatus, PayerContract};
use support::claims::billed;
use support::day;
use support::desk::PAYER;

/// A billed insured visit has a draft claim around its draft payer bill: the
/// payer's AED 360.00 of the AED 450.00 consultation, one line per payer bill
/// line, the contract whose windows apply.
#[renox::test]
async fn a_billed_visit_makes_a_draft_claim_around_its_payer_bill() {
    let b = billed().await;
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Draft);
    assert_eq!(claim.claimed, 36_000);
    assert!(claim.number().starts_with("CLM-"));
    let visit = b.d.visit(claim.visit_id).await;
    assert_eq!(Some(claim.payer_bill_id), visit.payer_bill_id);
    assert_eq!(claim.visit_date, visit.visit_date);
    assert_eq!(claim.payer_party_id, PAYER.0);
    assert!(claim.contract_id.is_some());
    let lines = b.lines().await;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].code, "consultation");
    assert_eq!(lines[0].claimed, 36_000);
    assert!(lines[0].payer_bill_line_id.is_some());
    assert_eq!(b.bill().status, PayerBillStatus::Draft);
    assert_eq!(Claim::of_visit(b.db(), visit.id).await.unwrap().len(), 1);

    b.d.get(&b.bea, "/clinics/claims")
        .await
        .assert_ok()
        .assert_see(&claim.number())
        .assert_see("AED 360.00");
    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see(&claim.number())
        .assert_see("consultation");
}

/// The claim pages are for the holders of `claims.manage`.
#[renox::test]
async fn the_claim_pages_need_the_permission() {
    let b = billed().await;
    b.d.get(&b.d.rita, "/clinics/claims")
        .await
        .assert_forbidden();
    b.d.post(&b.d.rita, &b.path("validate"), &[("late_reason", "")])
        .await
        .assert_forbidden();
    assert_eq!(b.claim().await.status, ClaimStatus::Draft);
}

/// `validate` then `submit`: the claim is `ready`, then `submitted` with its
/// reference and day, the attempt is recorded, the payer bill is issued with
/// its number, and a submitted claim is locked (`CLN.ACL.09`).
#[renox::test]
async fn a_claim_is_validated_submitted_and_then_locked() {
    let b = billed().await;
    b.validate().await;
    assert_eq!(b.claim().await.status, ClaimStatus::Ready);
    assert_eq!(b.bill().status, PayerBillStatus::Draft, "not sent yet");

    b.submit("CH-77").await;
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Submitted);
    assert_eq!(claim.submission_reference.as_deref(), Some("CH-77"));
    assert_eq!(claim.submitted_on, Some(today()));
    assert_eq!((claim.attempts, claim.resubmission_count), (1, 0));
    let bill = b.bill();
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert!(bill.number.is_some());
    let attempts = ClaimAttempt::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].kind, AttemptKind::Submission);
    assert_eq!(attempts[0].reference.as_deref(), Some("CH-77"));

    // Locked: nothing validates, submits or resubmits it again.
    b.act("validate", &[]).await.assert_status(422);
    b.act("submit", &[("reference", "CH-78")])
        .await
        .assert_status(422);
    b.act("resubmit", &[("corrections", "fixed")])
        .await
        .assert_status(422);
    assert_eq!(b.claim().await.status, ClaimStatus::Submitted);
    assert_eq!(b.bill().issued, 1);

    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("CH-77");
}

/// `validate` needs a service code on every line.
#[renox::test]
async fn a_line_without_a_code_stops_the_validation() {
    let b = billed().await;
    let line = b.lines().await.remove(0);
    clinics::ClaimLine::query()
        .where_(clinics::ClaimLine::ID.eq(line.id))
        .update(b.db(), &[("code", &"".to_owned())])
        .await
        .unwrap();
    let res = b.act("validate", &[]).await;
    res.assert_status(422);
    assert!(res.text().contains("service code"), "{}", res.text());
    assert_eq!(b.claim().await.status, ClaimStatus::Draft);
}

/// Where the payer asks a prior authorisation for the service, the claim
/// carries its reference to be validated.
#[renox::test]
async fn a_service_that_needs_an_authorisation_needs_it_linked() {
    let b = billed().await;
    authorisations::require_for(b.d.t.state(), PAYER, "consultation")
        .await
        .unwrap();
    let res = b.act("validate", &[]).await;
    res.assert_status(422);
    assert!(res.text().contains("consultation"), "{}", res.text());
    assert_eq!(b.claim().await.status, ClaimStatus::Draft);

    b.set(&[("payer_reference", &Some("A-1".to_owned()))]).await;
    b.validate().await;
    assert_eq!(b.claim().await.status, ClaimStatus::Ready);
}

/// The first-submission window counts from the visit date and ends on its
/// last day; a payer that accepts late claims gets the claim with a late
/// reason, recorded on the claim.
#[renox::test]
async fn the_first_submission_window_and_the_late_reason() {
    let b = billed().await;
    // The last day of the window is today: a visit 30 days ago.
    b.set(&[("visit_date", &day(-30))]).await;
    b.validate().await;
    assert_eq!(b.claim().await.status, ClaimStatus::Ready);

    // One day later it is past; the payer does not accept late claims.
    let late = billed().await;
    late.set(&[("visit_date", &day(-31))]).await;
    let res = late.act("validate", &[]).await;
    res.assert_status(422);
    assert!(res.text().contains(&day(-1).to_string()), "{}", res.text());
    assert_eq!(late.claim().await.status, ClaimStatus::Draft);

    // The payer accepts late claims: a reason is needed.
    let contract_id = late.claim.contract_id.expect("the claim has its contract");
    PayerContract::query()
        .where_(PayerContract::ID.eq(contract_id))
        .update(late.db(), &[("accepts_late_claims", &true)])
        .await
        .unwrap();
    let res = late.act("validate", &[]).await;
    res.assert_status(422);
    assert!(res.text().contains("late reason"), "{}", res.text());
    late.act("validate", &[("late_reason", "payer portal was down")])
        .await
        .assert_ok();
    let claim = late.claim().await;
    assert_eq!(claim.status, ClaimStatus::Ready);
    assert_eq!(claim.late_reason.as_deref(), Some("payer portal was down"));
}

/// `resubmit` from `denied`: corrections written down, within the payer's
/// window and limit; the payer bill runs `resubmit` and the count goes up.
#[renox::test]
async fn a_denied_claim_is_resubmitted_within_the_window_and_the_limit() {
    let b = billed().await;
    b.send().await;
    b.answered(ClaimStatus::Denied, day(0)).await;

    // Corrections are needed.
    b.act("resubmit", &[("corrections", " ")])
        .await
        .assert_status(422);
    assert_eq!(b.claim().await.status, ClaimStatus::Denied);

    b.act(
        "resubmit",
        &[("corrections", "member ID fixed"), ("reference", "CH-2")],
    )
    .await
    .assert_ok();
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Resubmitted);
    assert_eq!(claim.resubmission_count, 1);
    assert_eq!(claim.corrections.as_deref(), Some("member ID fixed"));
    assert_eq!(claim.denied, 0, "the payer is asked for it again");
    let bill = b.bill();
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert_eq!(bill.resubmissions, 1);
    let attempts = ClaimAttempt::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].kind, AttemptKind::Resubmission);
}

/// Done when: with a payer limit of 2 and a count of 2, `resubmit` is
/// refused; so it is past the resubmission window.
#[renox::test]
async fn resubmit_is_refused_beyond_the_limit_and_the_window() {
    let b = billed().await;
    b.send().await;
    b.answered(ClaimStatus::Denied, day(0)).await;
    b.set(&[("resubmission_count", &2_i64)]).await;
    let res = b.act("resubmit", &[("corrections", "fixed again")]).await;
    res.assert_status(422);
    assert!(res.text().contains("2 resubmissions"), "{}", res.text());
    assert_eq!(b.claim().await.status, ClaimStatus::Denied);
    assert_eq!(b.bill().resubmissions, 0, "the payer bill was not called");

    b.set(&[
        ("resubmission_count", &0_i64),
        ("last_remittance_on", &Some(day(-31))),
    ])
    .await;
    let res = b.act("resubmit", &[("corrections", "fixed")]).await;
    res.assert_status(422);
    assert!(res.text().contains("resubmission window"), "{}", res.text());
    assert_eq!(b.claim().await.status, ClaimStatus::Denied);

    // The last day of the window still counts.
    b.set(&[("last_remittance_on", &Some(day(-30)))]).await;
    b.act("resubmit", &[("corrections", "fixed")])
        .await
        .assert_ok();
    assert_eq!(b.claim().await.status, ClaimStatus::Resubmitted);
}

/// A `paid` claim is left only by a recoupment: it offers no resubmission,
/// and `resubmit` is refused.
#[renox::test]
async fn a_paid_claim_offers_no_resubmission() {
    let b = billed().await;
    b.send().await;
    b.answered(ClaimStatus::Paid, day(0)).await;
    b.act("resubmit", &[("corrections", "fixed")])
        .await
        .assert_status(422);
    assert_eq!(b.claim().await.status, ClaimStatus::Paid);
    b.d.get(&b.bea, &format!("/clinics/claims/{}", b.claim.id))
        .await
        .assert_ok()
        .assert_dont_see("Send the corrected claim");
}
