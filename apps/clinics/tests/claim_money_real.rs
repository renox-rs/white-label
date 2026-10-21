//! The payer's money on a claim with the **real** wl-invoicing payer bill
//! (BL-CLN-08, #356, with BL-INV-17 and BL-KRN-01): an overpayment held for a
//! second person and the money to the payer credit balance on approval, a
//! recoupment approved by a second person that reopens the line as denied, a
//! written-off claim left by a recoupment, and a reversed payment that
//! recoups without approval.

mod support;

use renox::testing::TestResponse;
use wl_invoicing::payer_credit;
use wl_invoicing::payer_remittance::{PayerAllocation, PayerRemittance};
use wl_kernel::approvals;
use wl_kernel::documents::today;
use wl_kernel::payer_bills::PayerBillStatus;
use wl_kernel::payments::{PaymentId, PaymentReversed, PaymentSource, ReversalReason};

use clinics::claim_rules::denied_key;
use clinics::{AnswerKind, Claim, ClaimLine, ClaimRemittance, ClaimStatus};
use support::real::{PAYER, Real, boot_real};
use support::{day, dubai};

/// A claim of AED 400.00, validated and submitted by the coordinator.
async fn submitted(t: &Real, by: &User) -> Claim {
    let claim = t.claim(day(0), &[("consultation", 40_000)]).await;
    t.post(by, &path(&claim, "validate"), &[]).await.assert_ok();
    t.post(by, &path(&claim, "submit"), &[("reference", "CH-1")])
        .await
        .assert_ok();
    t.claim_now(claim.id).await
}

fn path(claim: &Claim, action: &str) -> String {
    format!("/clinics/claims/{}/{action}", claim.id)
}

/// The id of the first line of the claim.
async fn line_of(t: &Real, claim: &Claim) -> i64 {
    ClaimLine::of_claim(t.db(), claim.id)
        .await
        .unwrap()
        .remove(0)
        .id
}

/// `by` records an advice with `reference` paying `paid` (and denying
/// `denied` under R12) on the only line.
async fn remit(
    t: &Real,
    by: &User,
    claim: &Claim,
    reference: &str,
    paid: &str,
    denied: &str,
    extra: &[(&str, &str)],
) -> TestResponse {
    let line = line_of(t, claim).await;
    let date = day(0).to_string();
    let paid_field = format!("line_{line}_paid");
    let denied_field = format!("line_{line}_denied");
    let code_field = format!("line_{line}_code");
    let mut fields = vec![
        ("reference", reference),
        ("date", date.as_str()),
        (paid_field.as_str(), paid),
        (denied_field.as_str(), denied),
        (code_field.as_str(), "R12"),
    ];
    fields.extend_from_slice(extra);
    t.post(by, &path(claim, "remittance"), &fields).await
}

/// `by` records a recoupment of `amount` under R07 carried by the advice
/// dated `date`.
async fn recoup(t: &Real, by: &User, claim: &Claim, date: &str, amount: &str) -> TestResponse {
    let line = line_of(t, claim).await;
    let field = format!("line_{line}_recouped");
    t.post(
        by,
        &path(claim, "recoupment"),
        &[
            ("code", "R07"),
            ("reference", "ADV-10"),
            ("date", date),
            ("reason", "taken back"),
            (field.as_str(), amount),
        ],
    )
    .await
}

/// The credit balance the payer has with the clinic's legal entity.
async fn credit(t: &Real, claim: &Claim) -> i64 {
    let bill = t.bill_of(claim).await;
    payer_credit::balance(t.db(), PAYER, bill.legal_entity_id, "AED")
        .await
        .unwrap()
}

/// Done when 1: a claim line open AED 400.00 and an advice paying AED 450.00:
/// the advice is held (nothing is recorded, on the payer bill or the claim);
/// user B approves; AED 400.00 is allocated and AED 50.00 goes to the payer
/// credit balance; user A, who recorded it, cannot approve.
#[renox::test]
async fn an_overpayment_is_held_until_a_second_person_approves() {
    let t = boot_real().await;
    let ana = t.coordinator().await;
    let bea = t.approver().await;
    let claim = submitted(&t, &ana).await;

    remit(&t, &ana, &claim, "ADV-1", "450.00", "", &[])
        .await
        .assert_ok();

    // Held: nothing is recorded.
    let held = t.claim_now(claim.id).await;
    assert_eq!(held.status, ClaimStatus::Submitted);
    assert_eq!(held.held_kind.as_deref(), Some("remittance"));
    assert_eq!(held.held_by, Some(ana.id));
    assert!(!held.has_remittance);
    let bill = t.bill_of(&held).await;
    assert_eq!(bill.status, PayerBillStatus::Issued);
    assert!(!bill.has_remittance);
    assert!(
        PayerRemittance::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(credit(&t, &held).await, 0);
    assert!(
        ClaimRemittance::of_claim(t.db(), claim.id)
            .await
            .unwrap()
            .is_empty()
    );
    let requests = approvals::pending_for(t.db(), &bill.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(held.held_request_id, Some(requests[0].id));
    assert_eq!(requests[0].action, "invoicing.payer_money");
    assert_eq!(requests[0].requester_id, ana.id);
    assert_eq!(requests[0].payload.0["amount"], 5_000);
    t.get(&ana, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Nothing is recorded until it is given");

    // The one who recorded it cannot approve it.
    assert!(
        approvals::approve(t.state(), requests[0].id, &ana, "ok")
            .await
            .is_err()
    );
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Submitted);

    approvals::approve(t.state(), requests[0].id, &bea, "ok")
        .await
        .expect("a second person approves");

    let paid = t.claim_now(claim.id).await;
    assert_eq!(paid.status, ClaimStatus::Paid);
    assert_eq!((paid.paid, paid.denied), (40_000, 0));
    assert!(!paid.is_held());
    assert_eq!(paid.last_remittance_on, Some(day(0)));
    let line = ClaimLine::of_claim(t.db(), claim.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(line.paid, 40_000);
    let bill = t.bill_of(&paid).await;
    assert_eq!(bill.status, PayerBillStatus::Paid);
    let allocated: i64 = PayerAllocation::query()
        .get(t.db())
        .await
        .unwrap()
        .iter()
        .map(|a| a.amount)
        .sum();
    assert_eq!(allocated, 40_000);
    assert_eq!(credit(&t, &paid).await, 5_000, "AED 50.00 is credit");
    let answers = ClaimRemittance::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(answers.len(), 1);
    assert_eq!(answers[0].kind, AnswerKind::Remittance);
    assert_eq!(answers[0].reference.as_deref(), Some("ADV-1"));
    assert_eq!(answers[0].overpayment, 5_000);
    assert_eq!(answers[0].approval_request_id, Some(requests[0].id));
    assert_eq!(answers[0].recorded_by, Some(ana.id));
}

/// A held advice that is rejected records nothing; the coordinator can enter
/// it again, corrected.
#[renox::test]
async fn a_rejected_overpayment_records_nothing() {
    let t = boot_real().await;
    let ana = t.coordinator().await;
    let bea = t.approver().await;
    let claim = submitted(&t, &ana).await;
    remit(&t, &ana, &claim, "ADV-1", "450.00", "", &[])
        .await
        .assert_ok();
    let request = t
        .claim_now(claim.id)
        .await
        .held_request_id
        .expect("the advice is held");

    approvals::reject(t.state(), request, &bea, "the amount is wrong")
        .await
        .expect("a second person rejects it");

    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Submitted);
    assert!(!after.is_held());
    assert_eq!(after.held_result.as_deref(), Some("rejected"));
    assert!(!after.has_remittance);
    assert_eq!(t.bill_of(&after).await.status, PayerBillStatus::Issued);
    assert!(
        PayerRemittance::query()
            .get(t.db())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(credit(&t, &after).await, 0);

    remit(&t, &ana, &claim, "ADV-1", "400.00", "", &[])
        .await
        .assert_ok();
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Paid);
}

/// Done when 2: claim `paid` (one line AED 400.00) and a later advice dated
/// 2026-12-10 recouping AED 400.00 with code "R07", with a payer resubmission
/// window of 30 days: held until user B approves; then the claim is `denied`,
/// the line shows R07, and the coordinator has a task with the resubmission
/// deadline 2027-01-09. The recorder cannot approve.
#[renox::test]
async fn a_recoupment_is_approved_by_a_second_person_and_reopens_the_line() {
    let t = boot_real().await;
    let ana = t.coordinator().await;
    let bea = t.approver().await;
    let claim = submitted(&t, &ana).await;
    remit(&t, &ana, &claim, "ADV-1", "400.00", "", &[])
        .await
        .assert_ok();
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Paid);

    recoup(&t, &ana, &claim, "2026-12-10", "400.00")
        .await
        .assert_ok();

    // Held: the claim is as it was.
    let held = t.claim_now(claim.id).await;
    assert_eq!(held.status, ClaimStatus::Paid);
    assert_eq!(held.held_kind.as_deref(), Some("recoupment"));
    assert_eq!(held.held_date, Some("2026-12-10".parse().unwrap()));
    assert_eq!(t.bill_of(&held).await.status, PayerBillStatus::Paid);
    let request = held.held_request_id.expect("the recoupment is held");
    assert!(
        approvals::approve(t.state(), request, &ana, "ok")
            .await
            .is_err()
    );

    approvals::approve(t.state(), request, &bea, "ok")
        .await
        .expect("a second person approves");

    let denied = t.claim_now(claim.id).await;
    assert_eq!(denied.status, ClaimStatus::Denied);
    assert_eq!((denied.paid, denied.denied), (0, 40_000));
    assert_eq!(
        denied.last_remittance_on,
        Some("2026-12-10".parse().unwrap())
    );
    assert!(!denied.is_held());
    let line = ClaimLine::of_claim(t.db(), claim.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!((line.paid, line.denied, line.recouped), (0, 40_000, 40_000));
    assert_eq!(line.denial_code.as_deref(), Some("R07"));
    assert_eq!(t.bill_of(&denied).await.status, PayerBillStatus::Denied);
    let answers = ClaimRemittance::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[1].kind, AnswerKind::Recoupment);
    assert_eq!(answers[1].approval_request_id, Some(request));
    assert_eq!(answers[1].recorded_by, Some(ana.id));
    assert!(!answers[1].reversal);

    let tasks = t.tasks.with_key("clinics.claim_denied:");
    assert_eq!(tasks.len(), 1);
    assert_eq!(
        tasks[0].source_key.as_deref(),
        Some(denied_key(claim.id, 2).as_str())
    );
    assert!(tasks[0].description.contains("R07"));
    assert!(tasks[0].description.contains("2027-01-09"));
    assert_eq!(tasks[0].due_at, Some(dubai("2027-01-09", 23, 59)));
}

/// Done when 3 and 5: a `written_off` claim with a line paid AED 300.00 and
/// AED 100.00 written off: a recoupment of AED 100.00 makes it
/// `partially_paid` and the write-off amount is still recorded and shown;
/// a recoupment above what the line was paid is refused, also by the payer
/// bill.
#[renox::test]
async fn a_written_off_claim_is_left_by_a_recoupment() {
    let t = boot_real().await;
    let ana = t.coordinator().await;
    let bea = t.approver().await;
    let claim = submitted(&t, &ana).await;
    remit(&t, &ana, &claim, "ADV-1", "300.00", "100.00", &[])
        .await
        .assert_ok();
    assert_eq!(
        t.claim_now(claim.id).await.status,
        ClaimStatus::PartiallyPaid
    );

    // The AED 100.00 denied was written off (what BL-CLN-09 will do): the
    // payer bill is `closed`, the claim `written_off`.
    let bill = t.bill_of(&claim).await;
    wl_invoicing::PayerBillLine::query()
        .where_(wl_invoicing::PayerBillLine::PAYER_BILL_ID.eq(bill.id))
        .update(t.db(), &[("denied", &0_i64), ("written_off", &10_000_i64)])
        .await
        .unwrap();
    wl_invoicing::PayerBill::query()
        .where_(wl_invoicing::PayerBill::ID.eq(bill.id))
        .update(
            t.db(),
            &[
                ("status", &"closed".to_owned()),
                ("denied_total", &0_i64),
                ("balance", &0_i64),
            ],
        )
        .await
        .unwrap();
    Claim::query()
        .where_(Claim::ID.eq(claim.id))
        .update(
            t.db(),
            &[("status", &"written_off".to_owned()), ("denied", &0_i64)],
        )
        .await
        .unwrap();

    // Above what the line was paid: refused before anything is held.
    recoup(&t, &ana, &claim, &day(0).to_string(), "500.00")
        .await
        .assert_status(422);
    assert!(!t.claim_now(claim.id).await.is_held());

    recoup(&t, &ana, &claim, &day(0).to_string(), "100.00")
        .await
        .assert_ok();
    let request = t
        .claim_now(claim.id)
        .await
        .held_request_id
        .expect("the recoupment is held");
    approvals::approve(t.state(), request, &bea, "ok")
        .await
        .expect("a second person approves");

    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::PartiallyPaid);
    assert_eq!((after.paid, after.denied), (20_000, 10_000));
    let lines = wl_invoicing::payer_bill::lines_of(t.db(), bill.id)
        .await
        .unwrap();
    assert_eq!(lines[0].written_off, 10_000, "the write-off stays recorded");
    assert_eq!(lines[0].paid, 20_000);
    t.get(&ana, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Written off or moved to the patient");
}

/// Done when 4: a `PaymentReversed` for a payment allocated to the claim's
/// payer bill runs `record_recoupment` without approval; the claim is
/// `denied`; the same event again changes nothing.
#[renox::test]
async fn a_reversed_payment_recoups_the_claim_without_approval() {
    let t = boot_real().await;
    let ana = t.coordinator().await;
    let claim = submitted(&t, &ana).await;
    remit(
        &t,
        &ana,
        &claim,
        "ADV-1",
        "400.00",
        "",
        &[("payment", "77")],
    )
    .await
    .assert_ok();
    let paid = t.claim_now(claim.id).await;
    assert_eq!(paid.status, ClaimStatus::Paid);
    let bill = t.bill_of(&paid).await;

    let reversed = || PaymentReversed {
        payment: PaymentId(77),
        party: PAYER,
        legal_entity_id: bill.legal_entity_id,
        reason: ReversalReason::BankReturn,
        source: PaymentSource::Manual,
        statement_line: None,
        removed: vec![],
        prepaid: None,
        reversed_on: today(),
    };
    t.state()
        .emit(reversed())
        .await
        .expect("the event is handled");

    let denied = t.claim_now(claim.id).await;
    assert_eq!(denied.status, ClaimStatus::Denied);
    assert_eq!((denied.paid, denied.denied), (0, 40_000));
    assert!(!denied.is_held());
    assert!(
        approvals::pending_for(t.db(), &bill.subject())
            .await
            .unwrap()
            .is_empty(),
        "no approval is needed"
    );
    let answers = ClaimRemittance::of_claim(t.db(), claim.id).await.unwrap();
    assert_eq!(answers.len(), 2);
    assert_eq!(answers[1].kind, AnswerKind::Recoupment);
    assert!(answers[1].reversal);
    assert_eq!(answers[1].code.as_deref(), Some("payment_reversed"));
    assert_eq!(answers[1].approval_request_id, None);
    assert_eq!(t.tasks.with_key("clinics.claim_denied:").len(), 1);

    t.state()
        .emit(reversed())
        .await
        .expect("the same event again is handled");
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Denied);
    assert_eq!(
        ClaimRemittance::of_claim(t.db(), claim.id)
            .await
            .unwrap()
            .len(),
        2
    );
}

/// The same advice reference is not recorded twice for a payer: the second
/// claim stays as it was and nothing is left marked on it.
#[renox::test]
async fn the_same_advice_is_not_recorded_twice() {
    let t = boot_real().await;
    let ana = t.coordinator().await;
    let first = submitted(&t, &ana).await;
    let second = submitted(&t, &ana).await;
    remit(&t, &ana, &first, "ADV-1", "400.00", "", &[])
        .await
        .assert_ok();

    remit(&t, &ana, &second, "ADV-1", "400.00", "", &[])
        .await
        .assert_status(422);

    let after = t.claim_now(second.id).await;
    assert_eq!(after.status, ClaimStatus::Submitted);
    assert!(after.held_kind.is_none() && !after.is_held());
    assert!(
        ClaimRemittance::of_claim(t.db(), second.id)
            .await
            .unwrap()
            .is_empty()
    );
}
