//! A payer recoupment on a claim and the denial task (`record_recoupment`,
//! `CLN.RUL.05`, `FIN.ACL.16`; BL-CLN-08, #356): through the claim routes
//! with the stand-in payer bill. The approval by a second person and the
//! reversal of a payment are the real wl-invoicing's, in
//! `claim_money_real.rs`.

mod support;

use clinics::claim_rules::denied_key;
use clinics::{AnswerKind, ClaimRemittance, ClaimStatus};
use support::claims::{Billed, billed};
use support::{day, dubai};

/// The coordinator records an advice dated today on the only line.
async fn advise(b: &Billed, reference: &str, paid: &str, denied: &str) {
    let line = b.lines().await.remove(0);
    let date = day(0).to_string();
    let paid_field = format!("line_{}_paid", line.id);
    let denied_field = format!("line_{}_denied", line.id);
    let code_field = format!("line_{}_code", line.id);
    b.act(
        "remittance",
        &[
            ("reference", reference),
            ("date", date.as_str()),
            (paid_field.as_str(), paid),
            (denied_field.as_str(), denied),
            (code_field.as_str(), "R12"),
        ],
    )
    .await
    .assert_ok();
}

/// The coordinator records a recoupment of `amount` on the only line.
async fn recoup(
    b: &Billed,
    code: &str,
    date: &str,
    amount: &str,
) -> renox::testing::TestResponse {
    let line = b.lines().await.remove(0);
    let field = format!("line_{}_recouped", line.id);
    b.act(
        "recoupment",
        &[
            ("code", code),
            ("date", date),
            ("reference", "ADV-10"),
            ("reason", "taken back"),
            (field.as_str(), amount),
        ],
    )
    .await
}

/// `CLN.RUL.05`: a remittance that denies opens the coordinator's task with
/// the denied lines, the payer's code and the resubmission deadline (the
/// window counted from the advice); a remittance that pays everything opens
/// none.
#[renox::test]
async fn a_remittance_that_denies_opens_the_denial_task() {
    let paid = billed().await;
    paid.send().await;
    advise(&paid, "ADV-1", "360.00", "").await;
    assert!(paid.d.t.tasks.with_key("clinics.claim_denied:").is_empty());

    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-2", "200.00", "160.00").await;
    let tasks = b.d.t.tasks.with_key("clinics.claim_denied:");
    assert_eq!(tasks.len(), 1);
    let deadline = (day(0) + renox::chrono::Days::new(30)).to_string();
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(denied_key(b.claim.id, 1).as_str())
    );
    assert!(tasks[0].description.contains("R12"));
    assert!(tasks[0].description.contains("AED 160.00"));
    assert!(tasks[0].description.contains(&deadline));
    assert_eq!(tasks[0].due_at, Some(dubai(&deadline, 23, 59)));
}

/// Done when 2: a `paid` claim and a later advice recouping the whole line
/// with code "R07", dated 2026-12-10, with a payer resubmission window of 30
/// days: the claim is `denied`, the line shows R07, the coordinator has a task
/// with the resubmission deadline 2027-01-09.
#[renox::test]
async fn a_recoupment_reopens_the_line_as_denied_with_the_payers_code() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "360.00", "").await;
    assert_eq!(b.claim().await.status, ClaimStatus::Paid);

    recoup(&b, "R07", "2026-12-10", "360.00").await.assert_ok();

    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Denied);
    assert_eq!((claim.paid, claim.denied), (0, 36_000));
    assert_eq!(claim.last_remittance_on, Some("2026-12-10".parse().unwrap()));
    let line = b.lines().await.remove(0);
    assert_eq!((line.paid, line.denied, line.recouped), (0, 36_000, 36_000));
    assert_eq!(line.denial_code.as_deref(), Some("R07"));
    let answers = ClaimRemittance::of_claim(b.db(), claim.id).await.unwrap();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[1].kind, AnswerKind::Recoupment);
    assert_eq!(answers[1].code.as_deref(), Some("R07"));
    assert_eq!(answers[1].recouped, 36_000);
    assert_eq!(answers[1].recorded_by, Some(b.bea.id));
    assert!(!answers[1].reversal);

    let tasks = b.d.t.tasks.with_key("clinics.claim_denied:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(denied_key(claim.id, 2).as_str())
    );
    assert!(tasks[0].description.contains("R07"));
    assert!(tasks[0].description.contains("2027-01-09"));
    assert_eq!(tasks[0].due_at, Some(dubai("2027-01-09", 23, 59)));
    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("R07");
}

/// Done when 5: a recoupment above what the line was paid is refused; so is
/// one without the payer's code or without an amount, and nothing changes.
#[renox::test]
async fn a_recoupment_above_what_was_paid_is_refused() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "360.00", "").await;

    recoup(&b, "R07", "2026-12-10", "500.00")
        .await
        .assert_status(422);
    recoup(&b, " ", "2026-12-10", "100.00").await.assert_status(422);
    recoup(&b, "R07", "2026-12-10", "0").await.assert_status(422);
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::Paid);
    assert_eq!(claim.paid, 36_000);
    assert_eq!(b.bill().status, wl_kernel::payer_bills::PayerBillStatus::Paid);
    assert_eq!(
        ClaimRemittance::of_claim(b.db(), claim.id).await.unwrap().len(),
        1
    );
}

/// A part of what was paid taken back leaves the claim `partially_paid`; it
/// can then be resubmitted for the reopened amount.
#[renox::test]
async fn a_partial_recoupment_leaves_the_claim_partly_paid_and_resubmittable() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "360.00", "").await;

    recoup(&b, "R07", &day(0).to_string(), "100.00")
        .await
        .assert_ok();
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::PartiallyPaid);
    assert_eq!((claim.paid, claim.denied), (26_000, 10_000));
    assert_eq!(b.d.t.tasks.with_key("clinics.claim_denied:").len(), 1);

    b.act("resubmit", &[("corrections", "recouped lines corrected")])
        .await
        .assert_ok();
    assert_eq!(b.claim().await.status, ClaimStatus::Resubmitted);
}

/// Done when 3: a `written_off` claim with a line paid is recouped: it is
/// `partially_paid` and the write-off amount is still shown.
#[renox::test]
async fn a_written_off_claim_is_left_by_a_recoupment_and_the_write_off_stays_shown() {
    let b = billed().await;
    b.send().await;
    advise(&b, "ADV-1", "360.00", "").await;
    // The write-off of AED 40.00 happened on the payer bill earlier.
    b.d.t.claims.force(b.claim.payer_bill_id, |bill| {
        bill.lines[0].written_off = 4_000;
    });
    b.set(&[("status", &"written_off".to_owned())]).await;

    recoup(&b, "R07", &day(0).to_string(), "100.00")
        .await
        .assert_ok();
    let claim = b.claim().await;
    assert_eq!(claim.status, ClaimStatus::PartiallyPaid);
    assert_eq!(claim.paid, 26_000);
    b.d.get(&b.bea, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Written off or moved to the patient")
        .assert_see("AED 40.00");
}

/// Done when 6: from `paid` the claim offers the recoupment and neither a
/// resubmission nor a new remittance; the recoupment needs `claims.manage`.
#[renox::test]
async fn a_paid_claim_offers_only_the_recoupment() {
    let b = billed().await;
    b.send().await;
    let page = format!("/clinics/claims/{}", b.claim.id);
    b.d.get(&b.bea, &page)
        .await
        .assert_ok()
        .assert_dont_see("Record recoupment");
    advise(&b, "ADV-1", "360.00", "").await;
    b.d.get(&b.bea, &page)
        .await
        .assert_ok()
        .assert_see("Record recoupment")
        .assert_dont_see("Send the corrected claim")
        .assert_dont_see("remittance advice. An amount above");
    let line = b.lines().await.remove(0);
    let field = format!("line_{}_recouped", line.id);
    b.d.post(
        &b.d.rita,
        &b.path("recoupment"),
        &[("code", "R07"), (field.as_str(), "100.00")],
    )
    .await
    .assert_forbidden();
    assert_eq!(b.claim().await.status, ClaimStatus::Paid);
}
