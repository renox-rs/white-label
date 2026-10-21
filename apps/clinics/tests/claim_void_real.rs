//! Voiding a claim created in error (BL-CLN-10, #358, `CLN.ACL.14`) with the
//! **real** wl-invoicing payer bill: the claim keeps its number and is shown
//! void, its payer bill is voided, the visit's payer split is removed and the
//! visit is split again to the right policy (a new claim) or flagged
//! *coverage to resolve*; a claim that was ever sent is never voided; a
//! duplicate names the claim it duplicates.

mod support;

use renox::AuthUser;
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::{PayerAgreement, VatRecipient};
use wl_kernel::PartyId;
use wl_kernel::approvals::{ApprovalPolicy, ConditionKind, Step};
use wl_kernel::documents::today;
use wl_kernel::machines;
use wl_kernel::payer_bills::PayerBillStatus;
use wl_kernel::voids::VoidRecord;

use clinics::claim_void::{self, VoidRequest};
use clinics::contracts::{self, NewContract};
use clinics::{Claim, ClaimStatus, InsurancePolicy, PayerContract, Visit, claims};
use support::real::{Real, boot_real};
use support::{PATIENT, dubai};

fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The other payer, its agreement and payer contract, and the patient's
/// policy with it.
async fn payer_y(t: &Real) -> InsurancePolicy {
    let mut agreement = PayerAgreement {
        legal_entity_id: t.agreement.legal_entity_id,
        payer_party_id: 901,
        name: "Orient Health".to_owned(),
        active: true,
        vat_recipient: VatRecipient::Customer,
        first_submission_days: 30,
        resubmission_days: 30,
        resubmission_limit: 2,
        write_off_limit: 100_000,
        ..PayerAgreement::default()
    };
    agreement.insert(t.db()).await.expect("the agreement");
    contracts::create(
        t.state(),
        None,
        NewContract::new(PartyId(901), d("2026-01-01")).agreement(agreement.id),
    )
    .await
    .expect("the contract");
    let mut policy = InsurancePolicy {
        patient_party_id: PATIENT.0,
        payer_party_id: 901,
        plan: "Gold".to_owned(),
        member_id: "M-901".to_owned(),
        valid_from: d("2026-01-01"),
        ..InsurancePolicy::default()
    };
    policy.save(t.db()).await.expect("the policy");
    policy
}

/// A `ready` claim of AED 400.00 for a visit on 2026-11-01.
async fn ready(t: &Real, by: &AuthUser) -> Claim {
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    claims::validate(t.state(), by, claim.id, None, dubai("2026-11-02", 9, 0))
        .await
        .expect("the claim is valid");
    t.claim_now(claim.id).await
}

/// The table row of the list that holds `number`.
fn row_of(page: &str, number: &str) -> String {
    page.split("<tr")
        .find(|row| row.contains(number))
        .unwrap_or_else(|| panic!("a row for {number} in {page}"))
        .to_owned()
}

fn void_path(claim: &Claim) -> String {
    format!("/clinics/claims/{}/void", claim.id)
}

/// Done when 1, 4: CLM-0300 `ready` for visit V1 with payer X: coordinator A
/// voids it with `wrong_payer` and V1 is re-split to payer Y: the claim is
/// `voided` with its number, shown in the list as void, its payer bill is
/// `voided` (no longer the bill of the visit) and a new claim exists for payer
/// Y that replaces it.
#[renox::test]
async fn a_ready_claim_is_voided_and_the_visit_is_split_again_to_the_right_payer() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let policy = payer_y(&t).await;
    let claim = ready(&t, &cora).await;
    assert_eq!(claim.status, ClaimStatus::Ready);

    let policy_id = policy.id.to_string();
    t.post(
        &cora,
        &void_path(&claim),
        &[
            ("code", "wrong_payer"),
            ("note", "billed to Daman"),
            ("policy", policy_id.as_str()),
        ],
    )
    .await
    .assert_ok();

    let voided = t.claim_now(claim.id).await;
    assert_eq!(voided.status, ClaimStatus::Voided);
    assert_eq!(voided.number(), claim.number(), "the number is kept");
    assert_eq!(voided.void_code.as_deref(), Some("wrong_payer"));
    assert_eq!(voided.void_note.as_deref(), Some("billed to Daman"));
    assert_eq!(voided.voided_by, Some(cora.id));
    assert_eq!(voided.voided_on, Some(today()));
    let record = VoidRecord::of_subject(t.db(), &voided.subject())
        .await
        .unwrap()
        .expect("the Void record keeps it");
    assert_eq!(record.number.as_deref(), Some(claim.number().as_str()));
    assert_eq!(record.state_before, "ready");
    assert_eq!(record.reason, "wrong_payer: billed to Daman");
    assert_eq!(t.bill_of(&voided).await.status, PayerBillStatus::Voided);

    // The visit is split again to payer Y: a new claim.
    let visit = Visit::find_or_404(t.db(), claim.visit_id).await.unwrap();
    assert_eq!(visit.coverage_policy_id, Some(policy.id));
    assert_eq!(visit.payer_total, 40_000);
    assert!(!visit.coverage_to_resolve);
    let all = Claim::of_visit(t.db(), visit.id).await.unwrap();
    assert_eq!(all.len(), 2);
    let made = all.iter().find(|c| c.id != claim.id).unwrap();
    assert_eq!(visit.payer_bill_id, Some(made.payer_bill_id));
    assert_eq!(made.status, ClaimStatus::Draft);
    assert_eq!(made.payer_party_id, 901);
    assert_eq!(made.claimed, 40_000);
    assert_eq!(made.replaces_claim_id, Some(claim.id));
    assert_ne!(made.payer_bill_id, claim.payer_bill_id);
    assert_eq!(t.bill_of(made).await.status, PayerBillStatus::Draft);

    // It stays in the list, marked void; its page points at the new claim.
    let response = t.get(&cora, "/clinics/claims").await;
    let list = response.assert_ok();
    list.assert_see(&claim.number());
    // The row of the voided claim itself: its number and the void badge.
    let row = row_of(&list.text(), &claim.number());
    assert!(row.contains("Void"), "{row}");
    assert!(!row.contains("Draft"), "{row}");
    let new_row = row_of(&list.text(), &made.number());
    assert!(new_row.contains("Draft"), "{new_row}");
    t.get(&cora, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see(&format!("Replaced by {}", made.number()));
    // The claim has its own approval action, not the module's.
    assert_eq!(
        machines::get("clinics.claim")
            .expect("the claim is a subject")
            .action_key("void"),
        "clinics.claim_void"
    );
}

/// A claim with no right policy: Done when 3: the visit has no payer split and
/// the coordinator gets a "coverage to resolve" task; nothing moves to the
/// patient; resolving it later splits the visit again and ends the task.
#[renox::test]
async fn with_no_right_policy_the_coverage_is_left_to_resolve() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let policy = payer_y(&t).await;
    let claim = ready(&t, &cora).await;

    t.post(&cora, &void_path(&claim), &[("code", "wrong_policy")])
        .await
        .assert_ok();
    let voided = t.claim_now(claim.id).await;
    assert_eq!(voided.status, ClaimStatus::Voided);
    let visit = Visit::find_or_404(t.db(), claim.visit_id).await.unwrap();
    assert!(visit.coverage_to_resolve);
    assert_eq!(visit.payer_bill_id, None);
    assert_eq!((visit.payer_total, visit.patient_total), (0, 0));
    assert_eq!(Claim::of_visit(t.db(), visit.id).await.unwrap().len(), 1);
    let key = claim_void::coverage_task_key(visit.id);
    let tasks = t.tasks.with_key(&key);
    assert_eq!(tasks.len(), 1);
    assert!(tasks[0].title.contains(&visit.number()));
    assert!(
        tasks[0]
            .assignee
            .as_ref()
            .is_some_and(|a| a == &wl_kernel::tickets::Assignee::user(cora.id))
    );
    t.get(&cora, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("coverage to resolve");

    // Resolved with the right policy: split again, a new claim, the task ends.
    let policy_id = policy.id.to_string();
    t.post(
        &cora,
        &format!("/clinics/claims/{}/resolve-coverage", claim.id),
        &[("policy", policy_id.as_str())],
    )
    .await
    .assert_ok();
    let visit = Visit::find_or_404(t.db(), claim.visit_id).await.unwrap();
    assert!(!visit.coverage_to_resolve);
    assert_eq!(visit.payer_total, 40_000);
    let all = Claim::of_visit(t.db(), visit.id).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[1].replaces_claim_id, Some(claim.id));
    let position = t
        .tasks
        .opened()
        .iter()
        .position(|task| task.source_key.as_deref() == Some(key.as_str()))
        .expect("the task was opened");
    assert!(t.tasks.is_resolved(i64::try_from(position + 1).unwrap()));
    // Nothing left to resolve.
    t.post(
        &cora,
        &format!("/clinics/claims/{}/resolve-coverage", claim.id),
        &[("policy", policy_id.as_str())],
    )
    .await
    .assert_status(422);
}

/// A policy that is not the patient's, or that would change the patient's
/// share, is refused before anything is voided.
#[renox::test]
async fn a_policy_that_cannot_be_used_is_refused_before_anything_changes() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = ready(&t, &cora).await;
    let mut other = InsurancePolicy {
        patient_party_id: 71,
        payer_party_id: 901,
        plan: "Gold".to_owned(),
        member_id: "M-71".to_owned(),
        valid_from: d("2026-01-01"),
        ..InsurancePolicy::default()
    };
    other.save(t.db()).await.unwrap();
    let id = other.id.to_string();
    let refused = t
        .post(
            &cora,
            &void_path(&claim),
            &[("code", "wrong_policy"), ("policy", id.as_str())],
        )
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("another patient"),
        "{}",
        refused.text()
    );
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Ready);
    assert_eq!(t.bill_of(&claim).await.status, PayerBillStatus::Draft);
}

/// Done when 2: a claim back in `draft` after a technical rejection was
/// submitted, so `void` is refused; so is a submitted one.
#[renox::test]
async fn a_claim_that_was_ever_sent_is_not_voided() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = ready(&t, &cora).await;
    claims::submit(
        t.state(),
        &cora,
        claim.id,
        Some("CH-1"),
        dubai("2026-11-03", 9, 0),
    )
    .await
    .unwrap();
    let rejected = claims::reject_submission(
        t.state(),
        &cora,
        claim.id,
        vec![clinics::ClaimError {
            code: "E104".to_owned(),
            message: "invalid member ID".to_owned(),
        }],
        dubai("2026-11-04", 9, 0),
    )
    .await
    .expect("the channel rejected it");
    assert_eq!(rejected.status, ClaimStatus::Draft);

    let refused = t
        .post(&cora, &void_path(&claim), &[("code", "wrong_payer")])
        .await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("sent to the payer"),
        "{}",
        refused.text()
    );
    let after = t.claim_now(claim.id).await;
    assert_eq!(after.status, ClaimStatus::Draft);
    let bill = t.bill_of(&after).await;
    assert_eq!(bill.status, PayerBillStatus::Draft);
    assert!(bill.number.is_some(), "its number is untouched");
    assert!(
        VoidRecord::of_subject(t.db(), &after.subject())
            .await
            .unwrap()
            .is_none()
    );

    // Submitted again, the claim is locked: not even offered a void.
    claims::validate(t.state(), &cora, claim.id, None, dubai("2026-11-05", 9, 0))
        .await
        .unwrap();
    claims::submit(
        t.state(),
        &cora,
        claim.id,
        Some("CH-2"),
        dubai("2026-11-06", 9, 0),
    )
    .await
    .unwrap();
    t.post(&cora, &void_path(&claim), &[("code", "wrong_payer")])
        .await
        .assert_status(422);
}

/// Done when 5: voiding with `duplicate` without picking a claim is refused;
/// the picker lists only the same patient's claims that are not voided; a
/// claim of another patient or a void one cannot be picked; the link is shown
/// on the voided claim; the visit of a duplicate is not flagged.
#[renox::test]
async fn a_duplicate_names_the_claim_it_duplicates() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let first = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    let second = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    let stranger = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    let gone = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    Claim::query()
        .where_(Claim::ID.eq(stranger.id))
        .update(t.db(), &[("patient_party_id", &71_i64)])
        .await
        .unwrap();
    t.post(&cora, &void_path(&gone), &[("code", "wrong_policy")])
        .await
        .assert_ok();

    // The picker: the other claim of the same patient, not the stranger's
    // nor the void one.
    let response = t
        .get(&cora, &format!("/clinics/claims/{}", second.id))
        .await;
    let page = response.assert_ok();
    page.assert_see(&first.number());
    page.assert_dont_see(&stranger.number());
    page.assert_dont_see(&gone.number());
    let candidates = claim_void::candidates(t.db(), &second).await.unwrap();
    assert_eq!(
        candidates.iter().map(|c| c.id).collect::<Vec<_>>(),
        [first.id]
    );

    // No claim picked: refused. A stranger's or a void claim: refused.
    let path = void_path(&second);
    t.post(&cora, &path, &[("code", "duplicate")])
        .await
        .assert_status(422);
    for other in [&stranger, &gone] {
        let id = other.id.to_string();
        t.post(
            &cora,
            &path,
            &[("code", "duplicate"), ("duplicate_of", id.as_str())],
        )
        .await
        .assert_status(422);
    }
    assert_eq!(t.claim_now(second.id).await.status, ClaimStatus::Draft);
    assert_eq!(t.bill_of(&second).await.status, PayerBillStatus::Draft);

    let id = first.id.to_string();
    t.post(
        &cora,
        &path,
        &[("code", "duplicate"), ("duplicate_of", id.as_str())],
    )
    .await
    .assert_ok();
    let voided = t.claim_now(second.id).await;
    assert_eq!(voided.status, ClaimStatus::Voided);
    assert_eq!(voided.void_duplicate_of, Some(first.id));
    let visit = Visit::find_or_404(t.db(), second.visit_id).await.unwrap();
    assert!(
        !visit.coverage_to_resolve,
        "the other claim carries the share"
    );
    assert_eq!(visit.payer_bill_id, None);
    assert!(
        t.tasks
            .with_key(&claim_void::coverage_task_key(visit.id))
            .is_empty()
    );
    t.get(&cora, &format!("/clinics/claims/{}", second.id))
        .await
        .assert_ok()
        .assert_see(&format!("Duplicate of {}", first.number()));
    // And the claim it duplicates shows the link too.
    t.get(&cora, &format!("/clinics/claims/{}", first.id))
        .await
        .assert_ok()
        .assert_see(&format!("Duplicated by {}", second.number()));

    // The service refuses a reason that is none of the three, too.
    let service = claim_void::void(t.state(), &cora, first.id, VoidRequest::new("tired")).await;
    assert!(service.is_err());
}

/// Voiding needs `claims.manage`; a user without it is refused.
#[renox::test]
async fn only_the_billing_coordinator_voids() {
    let t = boot_real().await;
    let claim = t.claim(d("2026-11-01"), &[("consultation", 40_000)]).await;
    let nobody = t.user("Nina", &[clinics::permissions::VISITS_VIEW]).await;
    t.post(&nobody, &void_path(&claim), &[("code", "wrong_payer")])
        .await
        .assert_forbidden();
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Draft);
}

/// A payer 902 whose contract has no payer agreement yet: a void that
/// re-splits the visit to it voids the claim and then fails at the new payer
/// bill.
async fn payer_without_agreement(t: &Real) -> (InsurancePolicy, i64) {
    let contract = contracts::create(
        t.state(),
        None,
        NewContract::new(PartyId(902), d("2026-01-01")),
    )
    .await
    .expect("the contract");
    let mut policy = InsurancePolicy {
        patient_party_id: PATIENT.0,
        payer_party_id: 902,
        plan: "Silver".to_owned(),
        member_id: "M-902".to_owned(),
        valid_from: d("2026-01-01"),
        ..InsurancePolicy::default()
    };
    policy.save(t.db()).await.expect("the policy");
    (policy, contract.id)
}

/// A void that fails after the claim was voided (the new payer bill cannot be
/// made) is finished by the same call once the cause is gone: the claim is
/// void and not marked complete, the form is offered again, and the retry
/// completes the split once, with one Void record and one voided payer bill.
#[renox::test]
async fn a_void_that_failed_in_the_middle_is_finished_by_the_same_call() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = ready(&t, &cora).await;
    let (policy, contract) = payer_without_agreement(&t).await;
    let policy_id = policy.id.to_string();
    let fields = [
        ("code", "wrong_payer"),
        ("note", "billed to Daman"),
        ("policy", policy_id.as_str()),
    ];

    let failed = t.post(&cora, &void_path(&claim), &fields).await;
    failed.assert_status(422);
    // Half way: the payer bill and the claim are void, the split is off the
    // visit, nothing says the void is complete.
    let half = t.claim_now(claim.id).await;
    assert_eq!(half.status, ClaimStatus::Voided);
    assert_eq!(half.void_code, None);
    assert_eq!(t.bill_of(&half).await.status, PayerBillStatus::Voided);
    let visit = Visit::find_or_404(t.db(), claim.visit_id).await.unwrap();
    assert_eq!(visit.payer_bill_id, None);
    assert_eq!(Claim::of_visit(t.db(), visit.id).await.unwrap().len(), 1);
    // The page offers the void again.
    t.get(&cora, &format!("/clinics/claims/{}", claim.id))
        .await
        .assert_ok()
        .assert_see("claim-void-");

    // The cause is gone: the payer's contract has its agreement.
    PayerContract::query()
        .where_(PayerContract::ID.eq(contract))
        .update(t.db(), &[("agreement_id", &Some(t.agreement.id))])
        .await
        .unwrap();
    t.post(&cora, &void_path(&claim), &fields)
        .await
        .assert_ok();

    let done = t.claim_now(claim.id).await;
    assert_eq!(done.status, ClaimStatus::Voided);
    assert_eq!(done.void_code.as_deref(), Some("wrong_payer"));
    assert_eq!(done.void_note.as_deref(), Some("billed to Daman"));
    let visit = Visit::find_or_404(t.db(), claim.visit_id).await.unwrap();
    assert_eq!(visit.payer_total, 40_000);
    assert_eq!(visit.coverage_policy_id, Some(policy.id));
    let all = Claim::of_visit(t.db(), visit.id).await.unwrap();
    assert_eq!(all.len(), 2, "one new claim, not two");
    let made = all.iter().find(|c| c.id != claim.id).unwrap();
    assert_eq!(made.replaces_claim_id, Some(claim.id));
    assert_eq!(visit.payer_bill_id, Some(made.payer_bill_id));
    assert_eq!(made.contract_id, Some(contract), "billed under the new contract");
    assert_eq!(
        VoidRecord::query()
            .where_(VoidRecord::SUBJECT_ID.eq(claim.id))
            .get(t.db())
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.subject_type == "clinics.claim")
            .count(),
        1,
        "voided once"
    );
    // A complete void is not run again.
    t.post(&cora, &void_path(&claim), &fields)
        .await
        .assert_status(422);
}

/// The payer bill was voided and the claim was not (the first write of the
/// void went through, the second did not): voiding the claim again finishes
/// it instead of failing on the bill that is void already.
#[renox::test]
async fn a_claim_whose_payer_bill_is_void_already_is_voided() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = ready(&t, &cora).await;
    wl_kernel::payer_bills::payer_bills(t.state())
        .void(t.state(), claim.payer_bill(), "wrong_payer", &cora)
        .await
        .expect("the payer bill is void");
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Ready);

    t.post(&cora, &void_path(&claim), &[("code", "wrong_payer")])
        .await
        .assert_ok();

    let done = t.claim_now(claim.id).await;
    assert_eq!(done.status, ClaimStatus::Voided);
    assert_eq!(done.void_code.as_deref(), Some("wrong_payer"));
    assert!(
        Visit::find_or_404(t.db(), claim.visit_id)
            .await
            .unwrap()
            .coverage_to_resolve
    );
}

/// An admin who puts an approval policy on the void of a claim (or of its payer
/// bill) makes the void wait; this app does not follow the approval, so the
/// void is refused before anything changes, whichever of the two it is. The
/// refusal says why.
#[renox::test]
async fn a_void_that_would_wait_for_an_approval_is_refused_before_anything_changes() {
    let t = boot_real().await;
    let cora = t.coordinator().await;
    let claim = ready(&t, &cora).await;
    for action in ["clinics.claim_void", "invoicing.payer_bill_void"] {
        let mut policy = ApprovalPolicy {
            action: action.to_owned(),
            condition: ConditionKind::Always,
            steps: renox::db::Json(vec![Step::one_of(["admin"])]),
            sod: true,
            active: true,
            ..ApprovalPolicy::default()
        };
        policy.insert(t.db()).await.expect("the admin's policy");

        let refused = t
            .post(&cora, &void_path(&claim), &[("code", "wrong_payer")])
            .await;
        refused.assert_status(422);
        assert!(
            refused.text().contains("waits for an approval"),
            "{action}: {}",
            refused.text()
        );
        let after = t.claim_now(claim.id).await;
        assert_eq!(after.status, ClaimStatus::Ready, "{action}");
        assert_eq!(t.bill_of(&after).await.status, PayerBillStatus::Draft);
        assert!(
            VoidRecord::of_subject(t.db(), &after.subject())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            wl_kernel::approvals::pending_for(t.db(), &after.subject())
                .await
                .unwrap()
                .is_empty(),
            "no request was left behind"
        );
        policy.active = false;
        policy.save(t.db()).await.unwrap();
    }
    // With the policies off again the claim voids.
    t.post(&cora, &void_path(&claim), &[("code", "wrong_payer")])
        .await
        .assert_ok();
    assert_eq!(t.claim_now(claim.id).await.status, ClaimStatus::Voided);
}
