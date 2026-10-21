//! Returning a claim to draft after a technical rejection (`reject_submission`,
//! 05 §4.3 "When it goes wrong", §7.1 *Claim*; BL-CLN-07, #355): the acceptance
//! checks through the claim routes, with the stand-in payer bill. The real
//! payer bill is in `claim_rejection_real.rs`.

mod support;

use wl_kernel::documents::today;

use clinics::claim_rules::rejected_key;
use clinics::{AttemptKind, ClaimAttempt, ClaimStatus};
use support::claims::billed;
use support::{day, dubai};

/// Done when 1 and 2: the channel rejects a submitted claim with "E104
/// invalid member ID": the claim is `draft`, the attempt shows E104, the count
/// stays 0, the payer bill ran `submission_rejected`, and the coordinator has
/// a task with the first-submission deadline; after fixing, `validate` and
/// `submit` send it again as the same claim with the same payer bill number.
#[renox::test]
async fn a_technical_rejection_returns_the_claim_to_draft_and_it_is_sent_again() {
    let b = billed().await;
    b.send().await;
    let number = b.claim.number();
    let bill_number = b.bill().number.expect("the first submission numbers it");
    assert_eq!(b.bill().issued, 1);

    b.act("reject", &[("errors", "E104: invalid member ID")])
        .await
        .assert_ok();

    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Draft);
    assert_eq!((claim.attempts, claim.resubmission_count), (1, 0));
    assert!(!claim.has_remittance);
    let deadline = claim.visit_date + renox::chrono::Days::new(30);
    assert_eq!(claim.fix_deadline, Some(deadline));
    let attempts = ClaimAttempt::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].is_rejected());
    assert_eq!(attempts[0].rejected_on, Some(today()));
    assert_eq!(attempts[0].errors.0.len(), 1);
    assert_eq!(attempts[0].errors.0[0].code, "E104");
    assert_eq!(attempts[0].errors.0[0].message, "invalid member ID");
    let bill = b.bill();
    assert_eq!(bill.status, wl_kernel::payer_bills::PayerBillStatus::Draft);
    assert_eq!(bill.rejections.len(), 1);
    assert_eq!(bill.rejections[0][0].code, "E104");
    assert_eq!(bill.resubmissions, 0);
    assert_eq!(bill.number.as_deref(), Some(bill_number.as_str()));

    // The coordinator's task names the errors and the last day to send it.
    let tasks = b.d.t.tasks.with_key("clinics.claim_rejected:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(rejected_key(claim.id, 1).as_str())
    );
    assert!(tasks[0].description.contains("E104 invalid member ID"));
    assert!(tasks[0].description.contains(&deadline.to_string()));
    assert_eq!(tasks[0].due_at, Some(dubai(&deadline.to_string(), 23, 59)));
    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("E104 invalid member ID");

    // Fixed, validated and submitted again: the same claim, the same payer
    // bill number, not a resubmission.
    b.validate().await;
    assert_eq!(b.claim().await.status, ClaimStatus::Ready);
    b.submit("CH-2").await;
    let again = b.claim().await;
    assert_eq!(again.status, ClaimStatus::Submitted);
    assert_eq!(again.number(), number);
    assert_eq!((again.attempts, again.resubmission_count), (2, 0));
    assert_eq!(again.fix_deadline, None);
    let bill = b.bill();
    assert_eq!(bill.number.as_deref(), Some(bill_number.as_str()));
    assert_eq!(bill.issued, 2);
    assert_eq!(bill.resubmissions, 0);
    let attempts = ClaimAttempt::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[1].kind, AttemptKind::Submission);
    assert!(attempts[1].errors.0.is_empty());
    assert_eq!(b.d.t.tasks.with_key("clinics.claim_rejected:").len(), 1);
}

/// A rejection needs an error with its code and message; nothing changes
/// without one, and only a claim with the channel can be rejected.
#[renox::test]
async fn a_rejection_without_an_error_changes_nothing() {
    let b = billed().await;
    // A draft claim was never sent.
    b.act("reject", &[("errors", "E104: invalid member ID")])
        .await
        .assert_status(422);
    b.send().await;

    for errors in ["", "  \n ", "  "] {
        b.act("reject", &[("errors", errors)])
            .await
            .assert_status(422);
    }
    assert_eq!(b.claim().await.status, ClaimStatus::Submitted);
    assert_eq!(b.bill().status, wl_kernel::payer_bills::PayerBillStatus::Issued);
    assert!(b.bill().rejections.is_empty());
    assert!(b.d.t.tasks.with_key("clinics.claim_rejected:").is_empty());
    assert!(ClaimAttempt::of_claim(b.db(), b.claim.id).await.unwrap()[0]
        .errors
        .0
        .is_empty());
}

/// Each attempt keeps its own errors, the count never changes, and each
/// rejection has its own task.
#[renox::test]
async fn repeated_rejections_keep_every_attempts_errors() {
    let b = billed().await;
    b.send().await;
    b.act("reject", &[("errors", "E104: invalid member ID")])
        .await
        .assert_ok();
    b.send().await;
    b.act("reject", &[("errors", "E7: missing field\nE9: bad date")])
        .await
        .assert_ok();

    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Draft);
    assert_eq!((claim.attempts, claim.resubmission_count), (2, 0));
    let attempts = ClaimAttempt::of_claim(b.db(), claim.id).await.unwrap();
    let codes: Vec<Vec<&str>> = attempts
        .iter()
        .map(|a| a.errors.0.iter().map(|e| e.code.as_str()).collect())
        .collect();
    assert_eq!(codes, [vec!["E104"], vec!["E7", "E9"]]);
    assert_eq!(b.bill().resubmissions, 0);
    assert_eq!(b.bill().rejections.len(), 2);
    let keys: Vec<_> = b
        .d
        .t
        .tasks
        .with_key("clinics.claim_rejected:")
        .into_iter()
        .map(|t| t.source_key.unwrap())
        .collect();
    assert_eq!(keys, [rejected_key(claim.id, 1), rejected_key(claim.id, 2)]);
}

/// Done when 3: a claim with count 1 in `resubmitted` is rejected back to
/// `draft` with count 1; it is validated against the resubmission window (not
/// the first-submission window, long past) and submitted again, still with
/// count 1; once that window has passed too it is refused.
#[renox::test]
async fn a_rejected_resubmission_is_validated_against_the_resubmission_window() {
    let b = billed().await;
    b.send().await;
    b.answered(ClaimStatus::Denied, day(0)).await;
    b.act("resubmit", &[("corrections", "member ID fixed")])
        .await
        .assert_ok();
    assert_eq!(b.claim().await.status, ClaimStatus::Resubmitted);

    b.act("reject", &[("errors", "E104: invalid member ID")])
        .await
        .assert_ok();
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Draft);
    assert_eq!(claim.resubmission_count, 1, "the count stays");
    assert_eq!(b.bill().resubmissions, 1);
    let deadline = day(0) + renox::chrono::Days::new(30);
    assert_eq!(claim.fix_deadline, Some(deadline), "counted from the remittance");
    let task = &b.d.t.tasks.with_key("clinics.claim_rejected:")[0];
    assert!(task.description.contains(&deadline.to_string()));

    // The first-submission window passed long ago; the resubmission window
    // from the remittance is open.
    b.set(&[("visit_date", &day(-100))]).await;
    b.validate().await;
    b.submit("CH-3").await;
    let again = b.claim().await;
    assert_eq!(again.status, ClaimStatus::Submitted);
    assert_eq!(again.resubmission_count, 1, "not a new resubmission");
    assert_eq!(b.bill().resubmissions, 1);
    let attempts = ClaimAttempt::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(attempts.last().unwrap().kind, AttemptKind::Resubmission);

    // Rejected again, after the resubmission window ended: refused.
    b.act("reject", &[("errors", "E104: invalid member ID")])
        .await
        .assert_ok();
    b.set(&[("last_remittance_on", &Some(day(-31)))]).await;
    let res = b.act("validate", &[]).await;
    res.assert_status(422);
    assert!(res.text().contains(&day(-1).to_string()), "{}", res.text());
    assert_eq!(b.claim().await.status, ClaimStatus::Draft);
}

/// Done when 5: a claim with a recorded remittance is not rejected.
#[renox::test]
async fn a_claim_with_a_recorded_remittance_is_not_rejected() {
    for status in [ClaimStatus::Paid, ClaimStatus::PartiallyPaid, ClaimStatus::Denied] {
        let b = billed().await;
        b.send().await;
        b.answered(status, day(0)).await;
        b.act("reject", &[("errors", "E104: invalid member ID")])
            .await
            .assert_status(422);
        assert_eq!(b.claim().await.status, status);
        assert!(b.bill().rejections.is_empty());
        assert!(b.d.t.tasks.with_key("clinics.claim_rejected:").is_empty());
    }

    // Also a claim still with the payer whose first submission was answered.
    let b = billed().await;
    b.send().await;
    b.set(&[("has_remittance", &true)]).await;
    let res = b
        .act("reject", &[("errors", "E104: invalid member ID")])
        .await;
    res.assert_status(422);
    assert!(res.text().contains("has answered"), "{}", res.text());
    assert_eq!(b.claim().await.status, ClaimStatus::Submitted);
    assert!(b.bill().rejections.is_empty());
}

/// Only a holder of `claims.manage` records the rejection, and only a claim
/// with the channel offers the action.
#[renox::test]
async fn the_rejection_is_offered_to_the_coordinator_of_a_sent_claim() {
    let b = billed().await;
    let page = format!("/clinics/claims/{}", b.claim.id);
    b.d.get(&b.bea, &page)
        .await
        .assert_ok()
        .assert_dont_see("refused the claim before the payer looked at it");
    b.send().await;
    b.d.get(&b.bea, &page)
        .await
        .assert_ok()
        .assert_see("refused the claim before the payer looked at it");
    b.d.post(
        &b.d.rita,
        &b.path("reject"),
        &[("errors", "E104: invalid member ID")],
    )
    .await
    .assert_forbidden();
    assert_eq!(b.claim().await.status, ClaimStatus::Submitted);
}
