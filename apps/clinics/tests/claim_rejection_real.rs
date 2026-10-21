//! A technical rejection of a claim with the **real** wl-invoicing payer bill
//! (BL-CLN-07, #355, with BL-INV-16): the payer bill goes back to `draft`
//! with the channel's errors, keeps its number and its resubmission count,
//! gives the deadline the claim's task carries, and a rejected resubmission
//! can be fixed and sent again.

mod support;

use renox::prelude::*;
use wl_invoicing::payer_rejection::PayerSubmissionError;
use wl_kernel::payer_bills::PayerBillStatus;

use clinics::claims;
use clinics::{ClaimAttempt, ClaimStatus};
use support::real::boot_real;
use support::{day, dubai};

fn d(text: &str) -> renox::chrono::NaiveDate {
    text.parse().expect("a date")
}

/// Done when 1 and 2 on the real payer bill: claim submitted on 2026-11-03 for
/// a visit on 2026-11-01; the coordinator records "E104 invalid member ID";
/// the claim is `draft`, the attempt shows E104, the count stays 0, the payer
/// bill is `draft` with its number and the error, the coordinator has a task
/// with the first-submission deadline (2026-12-01); after fixing, `validate`
/// and `submit` send it again with the same claim and payer bill number.
#[renox::test]
async fn the_real_payer_bill_is_rejected_and_sent_again_with_its_number() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    claims::validate(t.state(), &cora, claim.id, None, dubai("2026-11-02", 9, 0))
        .await
        .expect("the claim is valid");
    claims::submit(
        t.state(),
        &cora,
        claim.id,
        Some("CH-1"),
        dubai("2026-11-03", 9, 0),
    )
    .await
    .expect("the claim is submitted");
    let submitted = t.claim_now(claim.id).await;
    assert_eq!(submitted.status, ClaimStatus::Submitted);
    assert_eq!(submitted.submitted_on, Some(d("2026-11-03")));
    let bill = t.bill_of(&submitted).await;
    assert_eq!(bill.status, PayerBillStatus::Issued);
    let number = bill.number.clone().expect("issued: it has its number");
    assert_eq!(bill.issue_count, 1);

    t.post(
        &cora,
        &format!("/clinics/claims/{}/reject", claim.id),
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_ok();

    let rejected = t.claim_now(claim.id).await;
    assert_eq!(rejected.status, ClaimStatus::Draft);
    assert_eq!(rejected.resubmission_count, 0);
    assert_eq!(rejected.fix_deadline, Some(d("2026-12-01")));
    let attempts = ClaimAttempt::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(attempts[0].errors.0[0].code, "E104");
    let bill = t.bill_of(&rejected).await;
    assert_eq!(bill.status, PayerBillStatus::Draft);
    assert_eq!(bill.resubmission_count, 0);
    assert_eq!(bill.number.as_deref(), Some(number.as_str()));
    let errors = PayerSubmissionError::of_bill(t.db(), bill.id)
        .await
        .unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!((errors[0].attempt, errors[0].code.as_str()), (1, "E104"));

    // The clinic's task, with the deadline the payer bill gave.
    let tasks = t.tasks.with_key("clinics.claim_rejected:");
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].description.contains("E104 invalid member ID"));
    assert!(tasks[0].description.contains("2026-12-01"));
    assert_eq!(tasks[0].due_at, Some(dubai("2026-12-01", 23, 59)));
    // And the payer bill's own, for the same rejection.
    assert_eq!(t.tasks.with_key("invoicing.payer_bill_rejected:").len(), 1);

    // Fixed and sent again as the same claim with the same payer bill number.
    claims::validate(t.state(), &cora, claim.id, None, dubai("2026-11-04", 9, 0))
        .await
        .expect("it is validated again");
    claims::submit(
        t.state(),
        &cora,
        claim.id,
        Some("CH-2"),
        dubai("2026-11-05", 9, 0),
    )
    .await
    .expect("it is submitted again");
    let again = t.claim_now(claim.id).await;
    assert_eq!(again.status, ClaimStatus::Submitted);
    assert_eq!(again.number(), submitted.number());
    assert_eq!((again.attempts, again.resubmission_count), (2, 0));
    let bill = t.bill_of(&again).await;
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert_eq!(bill.number.as_deref(), Some(number.as_str()));
    assert_eq!((bill.issue_count, bill.resubmission_count), (2, 0));
}

/// Done when 3 on the real payer bill: a resubmitted claim (count 1) is
/// rejected back to `draft` with count 1 (the payer bill allows a technical
/// rejection of a resubmission although it has its remittance on record),
/// validated against the resubmission window, and submitted again with count
/// 1; the deadline is counted from the remittance.
#[renox::test]
async fn a_rejected_resubmission_is_fixed_and_sent_again() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = t.claim(day(0), &[("consultation", 40_000)]).await;
    let path = |action: &str| format!("/clinics/claims/{}/{action}", claim.id);
    t.post(&cora, &path("validate"), &[]).await.assert_ok();
    t.post(&cora, &path("submit"), &[("reference", "CH-1")])
        .await
        .assert_ok();

    // The payer denies the only line.
    let line = first_line(&t, claim.id).await;
    let date = day(0).to_string();
    t.post(
        &cora,
        &path("remittance"),
        &[
            ("reference", "ADV-1"),
            ("date", date.as_str()),
            (format!("line_{line}_denied").as_str(), "400.00"),
            (format!("line_{line}_code").as_str(), "R12"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Denied);

    t.post(
        &cora,
        &path("resubmit"),
        &[("corrections", "member ID fixed")],
    )
    .await
    .assert_ok();
    let resubmitted = t.claim_now(claim.id).await;
    assert_eq!(resubmitted.status, ClaimStatus::Resubmitted);
    assert_eq!(t.bill_of(&resubmitted).await.resubmission_count, 1);

    t.post(
        &cora,
        &path("reject"),
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_ok();
    let rejected = t.claim_now(claim.id).await;
    assert_eq!(rejected.status, ClaimStatus::Draft);
    assert_eq!(rejected.resubmission_count, 1);
    let bill = t.bill_of(&rejected).await;
    assert_eq!(bill.status, PayerBillStatus::Draft);
    assert_eq!(bill.resubmission_count, 1);
    let deadline = day(0) + renox::chrono::Days::new(30);
    assert_eq!(rejected.fix_deadline, Some(deadline));
    let tasks = t.tasks.with_key("clinics.claim_rejected:");
    assert!(tasks[0].description.contains(&deadline.to_string()));

    // The first-submission window passed long ago; the resubmission window is
    // open.
    clinics::Claim::query()
        .where_(clinics::Claim::ID.eq(claim.id))
        .update(t.db(), &[("visit_date", &day(-100))])
        .await
        .unwrap();
    t.post(&cora, &path("validate"), &[]).await.assert_ok();
    t.post(&cora, &path("submit"), &[("reference", "CH-3")])
        .await
        .assert_ok();
    let again = t.claim_now(claim.id).await;
    assert_eq!(again.status, ClaimStatus::Submitted);
    assert_eq!(again.resubmission_count, 1);
    let bill = t.bill_of(&again).await;
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert_eq!(bill.resubmission_count, 1, "not a new resubmission");
}

/// The payer denies the only line of `claim` (AED 400.00) with the advice
/// `reference` dated today.
async fn deny(t: &support::real::Real, by: &User, claim: i64, reference: &str) {
    let line = first_line(t, claim).await;
    let date = day(0).to_string();
    t.post(
        by,
        &format!("/clinics/claims/{claim}/remittance"),
        &[
            ("reference", reference),
            ("date", date.as_str()),
            (format!("line_{line}_denied").as_str(), "400.00"),
            (format!("line_{line}_code").as_str(), "R12"),
        ],
    )
    .await
    .assert_ok();
}

/// Done when 4: the payer accepts 2 resubmissions. Two resubmissions through the
/// route on the real payer bill take the count to 2 (the payer bill counts
/// them too); a third is refused with the limit, and the claim and its payer
/// bill stay as they are.
#[renox::test]
async fn the_third_resubmission_is_refused_after_two_real_ones() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = t.claim(day(0), &[("consultation", 40_000)]).await;
    let path = |action: &str| format!("/clinics/claims/{}/{action}", claim.id);
    t.post(&cora, &path("validate"), &[]).await.assert_ok();
    t.post(&cora, &path("submit"), &[("reference", "CH-1")])
        .await
        .assert_ok();

    for (round, advice) in [(1, "ADV-1"), (2, "ADV-2")] {
        deny(&t, &cora, claim.id, advice).await;
        assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Denied);
        t.post(
            &cora,
            &path("resubmit"),
            &[("corrections", "member ID fixed")],
        )
        .await
        .assert_ok();
        let resubmitted = t.claim_now(claim.id).await;
        assert_eq!(resubmitted.status, ClaimStatus::Resubmitted);
        assert_eq!(resubmitted.resubmission_count, round);
        assert_eq!(t.bill_of(&resubmitted).await.resubmission_count, round);
    }

    deny(&t, &cora, claim.id, "ADV-3").await;
    let refused = t
        .post(&cora, &path("resubmit"), &[("corrections", "once more")])
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("2 resubmissions"),
        "{}",
        refused.text()
    );
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Denied);
    assert_eq!(after.resubmission_count, 2);
    let bill = t.bill_of(&after).await;
    assert_eq!(bill.resubmission_count, 2, "the payer bill was not called");
    assert_eq!(bill.status, PayerBillStatus::Denied);
}

/// Done when 3: past the last day of the payer's resubmission window (30 days
/// from the remittance) the resubmission is refused and nothing changes; on
/// the last day it goes through.
#[renox::test]
async fn a_resubmission_after_the_window_is_refused_on_the_real_bill() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = t.claim(day(0), &[("consultation", 40_000)]).await;
    let path = |action: &str| format!("/clinics/claims/{}/{action}", claim.id);
    t.post(&cora, &path("validate"), &[]).await.assert_ok();
    t.post(&cora, &path("submit"), &[("reference", "CH-1")])
        .await
        .assert_ok();
    deny(&t, &cora, claim.id, "ADV-1").await;
    let on = |days: i64| {
        dubai(
            &(day(0) + renox::chrono::Days::new(days.unsigned_abs())).to_string(),
            9,
            0,
        )
    };

    let late = claims::resubmit(t.state(), &cora, claim.id, "fixed", None, on(31))
        .await
        .unwrap_err();
    assert!(
        matches!(&late, renox::state_machine::TransitionError::Guard(m)
            if m.contains("resubmission window ended on")),
        "{late:?}"
    );
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Denied);
    assert_eq!(t.bill_of(&after).await.resubmission_count, 0);

    claims::resubmit(t.state(), &cora, claim.id, "fixed", None, on(30))
        .await
        .expect("the last day of the window still counts");
    assert_eq!(
        t.claim_now(claim.id).await.status,
        ClaimStatus::Resubmitted
    );
}

/// Done when 5: a claim whose first submission the payer answered (a
/// remittance on record) is not sent back as rejected by the channel; the
/// real payer bill and the claim stay as they are and no task is made.
#[renox::test]
async fn a_claim_with_a_remittance_on_record_is_not_rejected() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = t.claim(day(0), &[("consultation", 40_000)]).await;
    let path = |action: &str| format!("/clinics/claims/{}/{action}", claim.id);
    t.post(&cora, &path("validate"), &[]).await.assert_ok();
    t.post(&cora, &path("submit"), &[("reference", "CH-1")])
        .await
        .assert_ok();
    deny(&t, &cora, claim.id, "ADV-1").await;

    let refused = t
        .post(
            &cora,
            &path("reject"),
            &[("errors", "E104: invalid member ID")],
        )
        .await;
    refused.assert_status(422);
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Denied);
    let bill = t.bill_of(&after).await;
    assert_eq!(bill.status, PayerBillStatus::Denied);
    assert!(
        PayerSubmissionError::of_bill(t.db(), bill.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(t.tasks.with_key("clinics.claim_rejected:").is_empty());
    assert!(t.tasks.with_key("invoicing.payer_bill_rejected:").is_empty());
}

/// The id of the first line of a claim, for the advice's field names.
async fn first_line(t: &support::real::Real, claim: i64) -> i64 {
    clinics::ClaimLine::of_claim(t.db(), claim)
        .await
        .unwrap()
        .remove(0)
        .id
}
