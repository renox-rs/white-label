//! Editing a payer contract in place, and picking its payer agreement by name
//! (05 `CLN.INS.01`), with the real wl-invoicing behind the picker.

mod support;

use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_invoicing::permissions::PAYER_BILLS_VIEW;

use clinics::contracts::{self, NewContract};
use clinics::permissions::{AUTHORISATIONS_MANAGE, POLICIES_VIEW};
use support::real::{PAYER, boot_real};

fn d(text: &str) -> NaiveDate {
    text.parse().expect("a date")
}

/// The billing coordinator changes the terms of a contract on its page: the
/// copay, the deductible, the last day and the windows. The payer, the plan
/// and the first day stay, the change is audited with the terms before it, a
/// wrong term is answered in words, and a user without `insurance.manage` is
/// refused.
#[renox::test]
async fn the_coordinator_changes_the_terms_of_a_contract_in_place() {
    let t = boot_real().await;
    let bea = t
        .user(
            "Bea",
            &[AUTHORISATIONS_MANAGE, POLICIES_VIEW, PAYER_BILLS_VIEW],
        )
        .await;
    let viewer = t.user("Vic", &[POLICIES_VIEW]).await;
    let id = t.contract.id;
    let path = format!("/clinics/contracts/{id}");

    t.get(&bea, "/clinics/contracts")
        .await
        .assert_ok()
        .assert_see("Daman Gold")
        .assert_see("Change the terms");
    t.get(&viewer, "/clinics/contracts")
        .await
        .assert_ok()
        .assert_dont_see("Change the terms");
    t.post(&viewer, &path, &[("copay", "10")])
        .await
        .assert_forbidden();

    t.post(&bea, &path, &[("copay", "120")])
        .await
        .assert_status(422);
    let late = t
        .post(
            &bea,
            &path,
            &[("first_submission_days", "10"), ("reminder_days", "10")],
        )
        .await;
    late.assert_status(422);
    assert!(
        late.text()
            .contains("N must be below the submission window"),
        "{}",
        late.text()
    );

    t.post(
        &bea,
        &path,
        &[
            ("agreement", t.agreement.id.to_string().as_str()),
            ("copay", "15"),
            ("deductible", "50"),
            ("valid_to", "2030-12-31"),
            ("first_submission_days", "45"),
            ("resubmission_days", "20"),
            ("resubmission_limit", "3"),
            ("reminder_days", "10"),
            ("accepts_late_claims", "yes"),
        ],
    )
    .await
    .assert_ok();
    let now = clinics::contract::PayerContract::find_or_404(t.db(), id)
        .await
        .unwrap();
    assert_eq!(now.copay_bp, 1_500);
    assert_eq!(now.deductible, 5_000);
    assert_eq!(now.valid_to, Some(d("2030-12-31")));
    assert_eq!(
        (
            now.first_submission_days,
            now.resubmission_days,
            now.resubmission_limit
        ),
        (45, 20, 3)
    );
    assert_eq!(now.reminder_days, 10);
    assert!(now.accepts_late_claims);
    assert_eq!(now.agreement_id, Some(t.agreement.id));
    // What names the contract did not move.
    assert_eq!(now.payer_party_id, t.contract.payer_party_id);
    assert_eq!(now.valid_from, t.contract.valid_from);
    assert_eq!(now.plan, t.contract.plan);

    let log = renox::audit::for_subject(t.db(), "clinics.payer_contract", id, 10)
        .await
        .unwrap();
    let entry = log
        .iter()
        .find(|entry| entry.action == "clinics.contract_updated")
        .expect("the change is audited");
    assert_eq!(entry.user_id, Some(bea.id));
    assert_eq!(entry.data["before"]["copay_bp"], t.contract.copay_bp);

    // The page shows the new terms.
    t.get(&bea, "/clinics/contracts")
        .await
        .assert_ok()
        .assert_see("15%");
}

/// The service refuses a change of the payer, the plan or the first day: a
/// different one is a new contract.
#[renox::test]
async fn the_payer_the_plan_and_the_first_day_cannot_be_changed() {
    let t = boot_real().await;
    let id = t.contract.id;
    for new in [
        NewContract::new(wl_kernel::PartyId(PAYER.0 + 1), t.contract.valid_from),
        NewContract::new(PAYER, d("2020-01-01")),
        NewContract::new(PAYER, t.contract.valid_from).plan("Platinum"),
    ] {
        let refused = contracts::update(t.state(), None, id, new).await;
        assert!(refused.is_err());
    }
    let same = NewContract::new(PAYER, t.contract.valid_from)
        .plan(t.contract.plan.as_deref().unwrap_or_default())
        .copay_bp(500);
    assert_eq!(
        contracts::update(t.state(), None, id, same)
            .await
            .unwrap()
            .copay_bp,
        500
    );
}

/// The picker lists the active payer agreements by name and payer as the
/// person types, and gives the label of the one a contract holds; a user who
/// does not see payer bills is refused.
#[renox::test]
async fn the_agreement_is_picked_by_name() {
    let t = boot_real().await;
    let bea = t.user("Bea", &[PAYER_BILLS_VIEW]).await;
    let nobody = t.user("Nora", &[POLICIES_VIEW]).await;

    t.get(&bea, "/payer-agreements/options?q=daman")
        .await
        .assert_ok()
        .assert_see("Daman Gold");
    t.get(&bea, "/payer-agreements/options?q=zzz")
        .await
        .assert_ok()
        .assert_dont_see("Daman");
    t.get(
        &bea,
        &format!("/payer-agreements/options?values={}", t.agreement.id),
    )
    .await
    .assert_ok()
    .assert_see("Daman Gold");
    t.get(&nobody, "/payer-agreements/options?q=daman")
        .await
        .assert_forbidden();
}
