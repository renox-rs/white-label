//! Closing a claim that missed the first-submission window (BL-CLN-09, #357,
//! `CLN.ACL.16`, `FIN.ACL.14`) with the **real** wl-invoicing payer bill:
//! the claim is written off, or moved to the patient only where the owner
//! allowed it on the payer contract; above AED 1,000 the owner approves; the
//! window and the late-claims payer are respected; the amount is reported as
//! *not submitted in time*.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use renox::state_machine::TransitionError;
use wl_invoicing::PayerAgreement;
use wl_kernel::approvals::{self, ApprovalPolicy, Step};
use wl_kernel::payer_bills::{PayerBillStatus, payer_bills};

use clinics::claim_closing::{self, Closed};
use clinics::claims;
use clinics::contracts::{self, NewContract};
use clinics::{ClaimStatus, PayerContract, Visit};
use support::dubai;
use support::real::{PAYER, Real, boot_real};

fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The agreement allows billing the customer (the contract flag still says
/// whether the clinic does).
async fn allow_customer_billing(t: &Real) {
    PayerAgreement::query()
        .where_(PayerAgreement::ID.eq(t.agreement.id))
        .update(t.db(), &[("bill_customer_allowed", &true)])
        .await
        .unwrap();
}

/// Done when 1: a draft claim of AED 600.00 for a visit on 2026-09-01, a
/// 30-day window (ended 2026-10-01), a payer that does not accept late claims
/// and the flag off (the agreement would allow billing the patient): clinic
/// manager A writes it off with "missed window" through the route (today is
/// past the window); it is
/// `written_off`, the payer bill's `write_off` ran (nothing moved to the
/// patient) and the KPI shows AED 600.00 *not submitted in time*.
#[renox::test]
async fn a_draft_claim_that_missed_its_window_is_written_off_and_reported() {
    let t = boot_real().await;
    allow_customer_billing(&t).await;
    let mona = t.manager("Mona").await;
    let cora = t.coordinator().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 60_000)]).await;

    // Through the route the clinic manager uses, on today's clock: the window
    // of the visit of 2026-09-01 ended on 2026-10-01.
    t.post(
        &mona,
        &format!("/clinics/claims/{}/write-off", claim.id),
        &[("reason", "missed window")],
    )
    .await
    .assert_ok();
    let done = t.claim_now(claim.id).await;
    assert_eq!(done.status, ClaimStatus::WrittenOff);
    assert_eq!(done.close_how.as_deref(), Some("written_off"));
    assert_eq!(done.close_reason.as_deref(), Some("missed window"));
    assert_eq!(done.close_amount, 60_000);
    assert_eq!(done.closed_on, Some(wl_kernel::documents::today()));
    assert_eq!(done.closed_by, Some(mona.id));
    assert!(done.not_submitted_in_time);

    let bill = t.bill_of(&done).await;
    assert_eq!(bill.status, PayerBillStatus::Closed);
    assert_eq!(bill.close_outcome.as_deref(), Some("written_off"));
    assert!(bill.not_submitted_in_time);
    let lines = payer_bills(t.state())
        .lines(t.db(), done.payer_bill())
        .await
        .unwrap();
    assert_eq!(lines.iter().map(|l| l.written_off).sum::<i64>(), 60_000);
    assert_eq!(lines.iter().map(|l| l.transferred).sum::<i64>(), 0);
    let visit = Visit::find_or_404(t.db(), done.visit_id).await.unwrap();
    assert_eq!(visit.patient_total, 0, "nothing moved to the patient");

    let kpi = claim_closing::not_submitted_in_time(t.db(), None, None)
        .await
        .unwrap();
    assert_eq!((kpi.claims, kpi.amount), (1, 60_000));
    let response = t.get(&cora, "/clinics/claims").await;
    let page = response.assert_ok();
    page.assert_see("AED 600.00");
    page.assert_see("not submitted in time");
    t.get(&cora, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("Written off AED 600.00, not submitted in time");
}

/// Done when 2: with the owner's flag on, the same action runs
/// `transfer_to_customer` and the patient's share of the visit rises by AED
/// 600.00; the patient is told. Only the owner sets the flag.
#[renox::test]
async fn with_the_flag_on_the_share_moves_to_the_patient() {
    let t = boot_real().await;
    allow_customer_billing(&t).await;
    let mona = t.manager("Mona").await;
    let olga = t.owner().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 60_000)]).await;
    let contract = claim.contract_id.expect("the claim has its contract");
    let path = "/clinics/contract-missed-window";
    let id = contract.to_string();

    // The clinic manager cannot set it; the owner can.
    t.post(
        &mona,
        path,
        &[("contract", id.as_str()), ("allowed", "yes")],
    )
    .await
    .assert_forbidden();
    assert!(
        !PayerContract::find_or_404(t.db(), contract)
            .await
            .unwrap()
            .bill_missed_to_patient
    );
    t.post(
        &olga,
        path,
        &[("contract", id.as_str()), ("allowed", "yes")],
    )
    .await
    .assert_ok();
    assert!(
        PayerContract::find_or_404(t.db(), contract)
            .await
            .unwrap()
            .bill_missed_to_patient
    );

    let closed = claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "missed window",
        dubai("2026-10-05", 9, 0),
    )
    .await
    .expect("the claim is closed");
    let Closed::Done(done) = closed else {
        panic!("a move of AED 600.00 is not held");
    };
    assert_eq!(done.status, ClaimStatus::WrittenOff);
    assert_eq!(done.close_how.as_deref(), Some("transferred"));
    assert_eq!(done.close_amount, 60_000);
    assert!(done.not_submitted_in_time);
    let bill = t.bill_of(&done).await;
    assert_eq!(bill.status, PayerBillStatus::Closed);
    assert_eq!(bill.close_outcome.as_deref(), Some("transferred"));
    let lines = payer_bills(t.state())
        .lines(t.db(), done.payer_bill())
        .await
        .unwrap();
    assert_eq!(lines.iter().map(|l| l.transferred).sum::<i64>(), 60_000);
    assert_eq!(lines.iter().map(|l| l.written_off).sum::<i64>(), 0);
    let visit = Visit::find_or_404(t.db(), done.visit_id).await.unwrap();
    assert_eq!((visit.patient_total, visit.payer_total), (60_000, 0));
    assert!(
        t.messages
            .sent()
            .iter()
            .any(|r| r.type_key == "invoicing.payer_bill_transferred"),
        "the patient is told"
    );
}

/// With the flag on but the payer agreement not allowing it, the payer bill
/// refuses and nothing changes.
#[renox::test]
async fn the_payer_agreement_can_still_refuse_to_bill_the_patient() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let olga = t.owner().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 60_000)]).await;
    let id = claim.contract_id.unwrap().to_string();
    t.post(
        &olga,
        "/clinics/contract-missed-window",
        &[("contract", id.as_str()), ("allowed", "yes")],
    )
    .await
    .assert_ok();
    let refused = claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "missed window",
        dubai("2026-10-05", 9, 0),
    )
    .await;
    assert!(refused.is_err());
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Draft);
    assert_eq!(t.bill_of(&claim).await.status, PayerBillStatus::Draft);
}

/// Done when 3: AED 1,500.00 creates an approval request; the claim stays
/// `draft` until the owner approves; the clinic manager who asked, and a
/// second clinic manager, cannot approve.
#[renox::test]
async fn a_write_off_above_the_limit_waits_for_the_owner() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let max = t.manager("Max").await;
    let olga = t.owner().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 150_000)]).await;

    let closed = claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "missed window",
        dubai("2026-10-05", 9, 0),
    )
    .await
    .expect("the request is made");
    let Closed::Held(held) = closed else {
        panic!("AED 1,500.00 is above the limit");
    };
    assert_eq!(held.status, ClaimStatus::Draft);
    assert_eq!(held.held_kind.as_deref(), Some("write_off"));
    assert_eq!(held.held_by, Some(mona.id));
    let bill = t.bill_of(&held).await;
    assert_eq!(bill.status, PayerBillStatus::Draft);
    let requests = approvals::pending_for(t.db(), &bill.subject())
        .await
        .unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].action, "invoicing.payer_write_off");
    assert_eq!(requests[0].requester_id, mona.id);
    assert_eq!(held.held_request_id, Some(requests[0].id));
    t.get(&mona, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("waits for the owner");

    // Neither the one who asked nor another clinic manager decides.
    for clinic_manager in [&mona, &max] {
        assert!(
            approvals::approve(t.state(), requests[0].id, clinic_manager, "ok")
                .await
                .is_err()
        );
    }
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Draft);

    approvals::approve(t.state(), requests[0].id, &olga, "ok")
        .await
        .expect("the owner approves");
    let done = t.claim_now(claim.id).await;
    assert_eq!(done.status, ClaimStatus::WrittenOff);
    assert_eq!(done.close_how.as_deref(), Some("written_off"));
    assert_eq!(done.close_amount, 150_000);
    assert_eq!(done.close_reason.as_deref(), Some("missed window"));
    assert_eq!(done.closed_by, Some(mona.id));
    assert!(done.not_submitted_in_time);
    assert!(!done.is_held());
    assert_eq!(t.bill_of(&done).await.status, PayerBillStatus::Closed);
    let kpi = claim_closing::not_submitted_in_time(t.db(), None, None)
        .await
        .unwrap();
    assert_eq!((kpi.claims, kpi.amount), (1, 150_000));
}

/// A rejected request leaves the claim and its payer bill as they were; the
/// clinic manager may ask again.
#[renox::test]
async fn a_rejected_write_off_changes_nothing() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let olga = t.owner().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 150_000)]).await;
    claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "missed window",
        dubai("2026-10-05", 9, 0),
    )
    .await
    .expect("the request is made");
    let request = t.claim_now(claim.id).await.held_request_id.expect("held");

    approvals::reject(t.state(), request, &olga, "send it late instead")
        .await
        .expect("the owner rejects");
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Draft);
    assert!(!after.is_held());
    assert_eq!(after.held_result.as_deref(), Some("rejected"));
    assert_eq!(t.bill_of(&after).await.status, PayerBillStatus::Draft);

    // Asked again, it waits again.
    let again = claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "still missed",
        dubai("2026-10-06", 9, 0),
    )
    .await
    .expect("asked again");
    assert!(matches!(again, Closed::Held(_)));
}

/// Done when 4: while the window is open (2026-09-30, and its last day,
/// 2026-10-01) `write_off` from `draft` is refused; so it is from `ready`.
#[renox::test]
async fn while_the_window_is_open_a_draft_is_not_written_off() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let cora = t.coordinator().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 60_000)]).await;

    for day in ["2026-09-30", "2026-10-01"] {
        let refused = claim_closing::write_off(
            t.state(),
            &mona,
            claim.id,
            "missed window",
            dubai(day, 9, 0),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&refused, TransitionError::Guard(m) if m.contains("2026-10-01")),
            "{day}: {refused:?}"
        );
    }
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Draft);
    assert_eq!(t.bill_of(&claim).await.status, PayerBillStatus::Draft);

    // A ready claim is the same; once the window passed it can go.
    claims::validate(t.state(), &cora, claim.id, None, dubai("2026-09-30", 9, 0))
        .await
        .expect("valid inside the window");
    assert!(
        claim_closing::write_off(t.state(), &mona, claim.id, "x", dubai("2026-10-01", 9, 0))
            .await
            .is_err()
    );
    let closed = claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "missed window",
        dubai("2026-10-02", 9, 0),
    )
    .await
    .expect("closed once the window passed");
    assert!(matches!(closed, Closed::Done(c) if c.not_submitted_in_time));
}

/// Done when 5: for a payer that accepts late claims `write_off` from `draft`
/// is refused and `validate` without a late reason is refused; with "system
/// outage" it passes.
#[renox::test]
async fn a_payer_that_accepts_late_claims_gets_the_claim_with_a_late_reason() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let cora = t.coordinator().await;
    let contract = contracts::create(
        t.state(),
        None,
        NewContract::new(PAYER, d("2026-01-01"))
            .agreement(t.agreement.id)
            .accepts_late_claims(),
    )
    .await
    .unwrap();
    let claim = t
        .claim_under(&contract, d("2026-09-01"), &[("consultation", 60_000)])
        .await;
    let on = dubai("2026-10-05", 9, 0);

    let refused = claim_closing::write_off(t.state(), &mona, claim.id, "missed window", on)
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, TransitionError::Guard(m) if m.contains("late")),
        "{refused:?}"
    );
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Draft);

    let no_reason = claims::validate(t.state(), &cora, claim.id, None, on)
        .await
        .unwrap_err();
    assert!(matches!(&no_reason, TransitionError::Guard(m) if m.contains("late reason")));
    claims::validate(t.state(), &cora, claim.id, Some("system outage"), on)
        .await
        .expect("a late reason opens the window");
    let ready = t.claim_now(claim.id).await;
    assert_eq!(ready.status, ClaimStatus::Ready);
    assert_eq!(ready.late_reason.as_deref(), Some("system outage"));
}

/// A claim the payer denied is written off whatever the window (it is not
/// *not submitted in time*); the reason is required; only `claims.writeoff`
/// closes a claim; the route answers the same.
#[renox::test]
async fn a_denied_claim_is_written_off_by_the_clinic_manager_alone() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let cora = t.coordinator().await;
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    claims::validate(t.state(), &cora, claim.id, None, dubai("2026-11-02", 9, 0))
        .await
        .unwrap();
    claims::submit(
        t.state(),
        &cora,
        claim.id,
        Some("CH-1"),
        dubai("2026-11-03", 9, 0),
    )
    .await
    .unwrap();
    let line = clinics::ClaimLine::of_claim(t.db(), claim.id)
        .await
        .unwrap()
        .remove(0)
        .id;
    t.post(
        &cora,
        &format!("/clinics/claims/{}/remittance", claim.id),
        &[
            ("reference", "ADV-1"),
            ("date", "2026-11-20"),
            (format!("line_{line}_denied").as_str(), "400.00"),
            (format!("line_{line}_code").as_str(), "R12"),
        ],
    )
    .await
    .assert_ok();
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Denied);
    let path = format!("/clinics/claims/{}/write-off", claim.id);

    // The coordinator manages claims but does not close them.
    t.post(&cora, &path, &[("reason", "payer will not pay")])
        .await
        .assert_forbidden();
    // The reason is required.
    t.post(&mona, &path, &[("reason", " ")])
        .await
        .assert_status(422);
    t.post(&mona, &path, &[("reason", "payer will not pay")])
        .await
        .assert_ok();

    let done = t.claim_now(claim.id).await;
    assert_eq!(done.status, ClaimStatus::WrittenOff);
    assert!(!done.not_submitted_in_time, "the payer saw it");
    assert_eq!((done.close_amount, done.denied), (40_000, 0));
    assert_eq!(t.bill_of(&done).await.status, PayerBillStatus::Closed);
    let kpi = claim_closing::not_submitted_in_time(t.db(), None, None)
        .await
        .unwrap();
    assert_eq!(kpi.claims, 0);
    // Written off is left only by a recoupment; it is not written off twice.
    assert!(
        claim_closing::write_off(
            t.state(),
            &mona,
            claim.id,
            "again",
            dubai("2026-12-01", 9, 0)
        )
        .await
        .is_err()
    );
}

/// The write-off above the limit is the owner's: the policy stored for
/// `invoicing.payer_write_off` is wl-invoicing's own default, built with the
/// approver roles this app set on the module, not a second default registered
/// over it, so it does not depend on the order the modules boot in.
#[renox::test]
async fn the_stored_policy_of_a_payer_bill_write_off_names_the_owner_alone() {
    let t = boot_real().await;
    let policy = ApprovalPolicy::for_action(
        t.db(),
        "invoicing.payer_write_off",
        &renox::serde_json::json!({ "amount": 150_000 }),
    )
    .await
    .unwrap()
    .expect("a write-off above the limit needs an approval");
    assert_eq!(
        policy.steps.0,
        [Step::one_of([
            clinics::permissions::OWNER_ROLE,
            wl_kernel::COMPANY_ADMIN_ROLE
        ])]
    );
    assert!(policy.sod, "never the requester");
    assert_eq!(policy.expiry_hours, Some(168));
    // Exactly one policy: the module's own default, with the app's roles.
    let all = ApprovalPolicy::query()
        .where_(ApprovalPolicy::ACTION.eq("invoicing.payer_write_off"))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
}

/// Edge case: the open amount changes while the write-off waits for the
/// owner (here the payer bill's balance moves from AED 1,500.00 to AED
/// 1,400.00). The owner's approval does not write off the new amount: the
/// request fails, the payer bill and the claim are as they were, and the
/// claim does not go on waiting for it.
#[renox::test]
async fn an_amount_that_changed_while_the_write_off_waited_fails_the_request() {
    let t = boot_real().await;
    let mona = t.manager("Mona").await;
    let olga = t.owner().await;
    let claim = t.claim(d("2026-09-01"), &[("consultation", 150_000)]).await;
    claim_closing::write_off(
        t.state(),
        &mona,
        claim.id,
        "missed window",
        dubai("2026-10-05", 9, 0),
    )
    .await
    .expect("the request is made");
    let request = t.claim_now(claim.id).await.held_request_id.expect("held");

    wl_invoicing::PayerBill::query()
        .where_(wl_invoicing::PayerBill::ID.eq(claim.payer_bill_id))
        .update(t.db(), &[("balance", &140_000_i64)])
        .await
        .unwrap();
    // The decision is recorded; the write-off it was for does not run.
    let _ = approvals::approve(t.state(), request, &olga, "ok").await;

    let stored = approvals::ApprovalRequest::find_or_404(t.db(), request)
        .await
        .unwrap();
    assert_eq!(stored.status, approvals::ApprovalStatus::Failed);
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Draft);
    assert!(!after.is_held(), "the claim does not wait for a dead request");
    assert_eq!(t.bill_of(&after).await.status, PayerBillStatus::Draft);
    let lines = payer_bills(t.state())
        .lines(t.db(), after.payer_bill())
        .await
        .unwrap();
    assert_eq!(lines.iter().map(|l| l.written_off).sum::<i64>(), 0);
}
