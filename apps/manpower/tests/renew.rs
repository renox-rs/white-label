//! BL-MPW-05 (#419): warning before a client contract ends and renewing it
//! (`MPW.RUL.11`, `renew`, `MPW.ACL.16`). The account manager is told 60 and
//! 30 days before the end date which deployments will end; `renew` moves the
//! end date with a signed renewal and, for changed rates, a rate card version;
//! the notices start again from the new end date.

mod support;

use manpower::expiry::{self, ContractEndNotice};
use manpower::notices::NOTICE;
use manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus};
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};

use support::{Booted, TOWER, boot, date, users_of};

/// An active contract of Tower ending 2027-12-31, managed by `manager`, with
/// `n` active deployments.
async fn contract_with(t: &Booted, manager: &User, n: i64) -> (ClientContract, Vec<Deployment>) {
    let contract = t
        .contract(TOWER, ContractStatus::Active, Some(manager))
        .await;
    let site = t.site(TOWER, "Tower A").await;
    let mut deployments = Vec::new();
    for i in 1..=n {
        let worker = t.staff.employ(&t.parties, i, &format!("Worker {i}"), true);
        deployments.push(
            t.deployment(
                &contract,
                &site,
                worker,
                Some(date(2027, 12, 31)),
                DeploymentStatus::Active,
            )
            .await,
        );
    }
    (contract, deployments)
}

/// The notices about the end of `contract`, oldest first.
fn warnings(t: &Booted, contract: &ClientContract) -> Vec<renox::serde_json::Value> {
    t.notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|n| {
            n.data["title"].as_str().is_some_and(|title| {
                title.contains(&contract.reference) && title.contains("ends on")
            })
        })
        .map(|n| n.data)
        .collect()
}

fn path(contract: &ClientContract) -> String {
    format!("/manpower/contracts/{}/renew", contract.id)
}

/// The form of a renewal to 2028-12-31, signed on 2026-10-01.
fn form() -> Vec<(&'static str, &'static str)> {
    vec![
        ("end_date", "2028-12-31"),
        ("signed_on", "2026-10-01"),
        ("document", "RENEWAL-2028.pdf"),
    ]
}

/// AC1: contract K ends 2027-12-31 with 8 active deployments: on the day 60
/// days before, the account manager gets one notice listing the 8; on the day
/// 30 days before, a second; a run again the same day, or a day later, sends
/// nothing; the contract is not ended by the rule.
#[renox::test]
async fn the_account_manager_is_told_60_and_30_days_before_the_end_with_the_deployments() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, _deployments) = contract_with(&t, &amal, 8).await;

    // 61 days left: nothing yet.
    t.on(date(2027, 10, 31), async {
        expiry::run(t.state(), date(2027, 10, 31)).await.unwrap();
    })
    .await;
    assert!(warnings(&t, &contract).is_empty());

    t.on(date(2027, 11, 1), async {
        let told = expiry::run(t.state(), date(2027, 11, 1)).await.unwrap();
        assert_eq!(told, vec![(contract.id, 60)]);
        // The same day again: one notice per threshold per end date.
        assert!(
            expiry::run(t.state(), date(2027, 11, 1))
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
    let first = warnings(&t, &contract);
    assert_eq!(first.len(), 1);
    let body = first[0]["body"].as_str().unwrap();
    assert!(body.contains("60 days"), "{body}");
    assert!(body.contains("8 workers"), "{body}");
    for i in 1..=8 {
        assert!(body.contains(&format!("Worker {i},")), "{body}");
    }
    let told = t.notices.of_type(NOTICE);
    assert_eq!(users_of(told.last().unwrap()), vec![amal.id]);

    t.on(date(2027, 11, 20), async {
        assert!(
            expiry::run(t.state(), date(2027, 11, 20))
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
    // The scheduled task is the daily run: 30 days left.
    t.on(date(2027, 12, 1), async {
        t.app
            .run_scheduled(manpower::CONTRACT_EXPIRY_TASK)
            .await
            .unwrap();
    })
    .await;
    let both = warnings(&t, &contract);
    assert_eq!(both.len(), 2);
    assert!(both[1]["body"].as_str().unwrap().contains("30 days"));

    // Nothing more, and the rule never ends the contract, not even on the end date.
    t.on(date(2027, 12, 31), async {
        assert!(
            expiry::run(t.state(), date(2027, 12, 31))
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
    assert_eq!(warnings(&t, &contract).len(), 2);
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
}

/// A contract found inside the window (41 days left) gets the 60-day notice
/// at the next run and the 30-day one on time; one found with both behind it
/// (25 days left) gets one message for the nearer threshold and nothing more
/// the next day.
#[renox::test]
async fn a_missed_notice_goes_out_at_the_next_run_and_only_once() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (inside, _) = contract_with(&t, &amal, 1).await;
    t.on(date(2027, 11, 20), async {
        let told = expiry::run(t.state(), date(2027, 11, 20)).await.unwrap();
        assert_eq!(told, vec![(inside.id, 60)]);
    })
    .await;
    t.on(date(2027, 12, 1), async {
        let told = expiry::run(t.state(), date(2027, 12, 1)).await.unwrap();
        assert_eq!(told, vec![(inside.id, 30)]);
    })
    .await;
    assert_eq!(warnings(&t, &inside).len(), 2);

    let late = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.on(date(2027, 12, 6), async {
        let told = expiry::run(t.state(), date(2027, 12, 6)).await.unwrap();
        assert_eq!(told, vec![(late.id, 30)]);
        assert!(
            expiry::run(t.state(), date(2027, 12, 6))
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
    assert_eq!(warnings(&t, &late).len(), 1);
    let rows = ContractEndNotice::query()
        .where_(ContractEndNotice::CLIENT_CONTRACT_ID.eq(late.id))
        .get(t.db())
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "both thresholds are recorded as sent");
}

/// A contract that is not `active` or `on_hold` is not warned about, and a
/// contract nobody manages is not marked as told.
#[renox::test]
async fn only_active_and_held_contracts_are_warned_and_a_notice_needs_someone_to_tell() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let draft = t.contract(TOWER, ContractStatus::Draft, Some(&amal)).await;
    let terminating = t
        .contract(TOWER, ContractStatus::Terminating, Some(&amal))
        .await;
    let held = t.contract(TOWER, ContractStatus::OnHold, Some(&amal)).await;
    let unmanaged = t
        .contract(support::FOODS, ContractStatus::Active, None)
        .await;
    t.on(date(2027, 11, 1), async {
        let told = expiry::run(t.state(), date(2027, 11, 1)).await.unwrap();
        assert_eq!(told, vec![(held.id, 60)], "the held contract is warned");
    })
    .await;
    assert!(warnings(&t, &draft).is_empty());
    assert!(warnings(&t, &terminating).is_empty());
    let none = ContractEndNotice::query()
        .where_(ContractEndNotice::CLIENT_CONTRACT_ID.eq(unmanaged.id))
        .get(t.db())
        .await
        .unwrap();
    assert!(none.is_empty(), "nobody was told, so nothing is recorded");
}

/// AC2: the account manager renews K to 2028-12-31 with the signed renewal:
/// K stays `active`, keeps its deployments and their end dates, and the notices
/// start again from the new end date (the 2027 ones are not sent for it, the
/// next is due on 2028-11-01); the contract's old notices are not sent again.
#[renox::test]
async fn a_renewal_keeps_the_state_and_restarts_the_notices_from_the_new_end_date() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, deployments) = contract_with(&t, &amal, 2).await;
    t.on(date(2027, 11, 1), async {
        expiry::run(t.state(), date(2027, 11, 1)).await.unwrap();
    })
    .await;
    assert_eq!(warnings(&t, &contract).len(), 1);

    t.on(date(2027, 11, 5), async {
        t.post(&amal, &path(&contract), &form())
            .await
            .assert_status(303);
    })
    .await;
    let renewed = t.reload_contract(contract.id).await;
    assert_eq!(renewed.status, ContractStatus::Active);
    assert_eq!(renewed.end_date, date(2028, 12, 31));
    assert_eq!(renewed.renewal_previous_end_date, Some(date(2027, 12, 31)));
    assert_eq!(
        renewed.renewal_document.as_deref(),
        Some("RENEWAL-2028.pdf")
    );
    assert_eq!(renewed.renewal_signed_on, Some(date(2026, 10, 1)));
    assert_eq!(renewed.renewed_by, Some(amal.id));
    assert!(renewed.renewed_at.is_some());
    for d in &deployments {
        assert_eq!(
            t.reload_deployment(d.id).await.end_date,
            Some(date(2027, 12, 31)),
            "deployment end dates are not changed by the renewal"
        );
    }

    // The old end date's second notice is not sent; the new one starts later.
    t.on(date(2027, 12, 1), async {
        assert!(
            expiry::run(t.state(), date(2027, 12, 1))
                .await
                .unwrap()
                .is_empty()
        );
    })
    .await;
    assert_eq!(warnings(&t, &contract).len(), 1);
    t.on(date(2028, 11, 1), async {
        let told = expiry::run(t.state(), date(2028, 11, 1)).await.unwrap();
        assert_eq!(told, vec![(contract.id, 60)]);
    })
    .await;
    let again = warnings(&t, &contract);
    assert_eq!(again.len(), 2);
    let body = again[1]["body"].as_str().unwrap();
    assert!(
        body.contains("until 2027-12-31"),
        "the deployments to extend: {body}"
    );
}

/// AC3: a renewal without the signed renewal, or to 2027-12-15 (earlier than
/// the end date 2027-12-31), is refused with every failing check; nothing
/// changes. Without `contracts.manage` it is forbidden.
#[renox::test]
async fn a_renewal_needs_a_later_end_date_and_the_signed_renewal() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let nobody = t.user("Nora").await;
    let (contract, _) = contract_with(&t, &amal, 1).await;

    let unsigned = t
        .post(
            &amal,
            &path(&contract),
            &[("end_date", "2028-12-31"), ("signed_on", "2026-10-01")],
        )
        .await;
    unsigned.assert_status(422);
    assert!(
        unsigned.text().contains("signed by the client"),
        "{}",
        unsigned.text()
    );
    let earlier = t
        .post(
            &amal,
            &path(&contract),
            &[
                ("end_date", "2027-12-15"),
                ("signed_on", "2026-10-01"),
                ("document", "R.pdf"),
            ],
        )
        .await;
    earlier.assert_status(422);
    assert!(earlier.text().contains("later than the current end date"));
    let both = t
        .post(
            &amal,
            &path(&contract),
            &[("end_date", "2027-12-15"), ("signed_on", "2026-10-01")],
        )
        .await;
    both.assert_status(422);
    assert!(both.text().contains("later than") && both.text().contains("signed by the client"));
    t.post(&nobody, &path(&contract), &form())
        .await
        .assert_status(403);
    let kept = t.reload_contract(contract.id).await;
    assert_eq!(kept.end_date, date(2027, 12, 31));
    assert!(kept.renewed_at.is_none() && kept.renewal_document.is_none());
}

/// AC4: changed rates need a new rate card version, accepted by the client and
/// effective from the date given. A version accepted for another date does not
/// do; a pending version prices nothing (the card only answers for accepted
/// ones); the old version is never touched.
#[renox::test]
async fn changed_rates_need_a_rate_card_version_effective_from_the_date_given() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let (contract, _) = contract_with(&t, &amal, 1).await;
    t.cards
        .accept(&contract.subject(), date(2027, 1, 1), &[("guard", "day")]);
    let mut changed = form();
    changed.push(("rates_from", "2028-01-01"));

    // Only the version of 2027-01-01 is accepted.
    let refused = t.post(&amal, &path(&contract), &changed).await;
    refused.assert_status(422);
    assert!(
        refused.text().contains("takes effect on 2028-01-01"),
        "{}",
        refused.text()
    );
    assert!(t.reload_contract(contract.id).await.renewed_at.is_none());

    // A version for another date is no help.
    t.cards
        .accept(&contract.subject(), date(2028, 2, 1), &[("guard", "day")]);
    t.post(&amal, &path(&contract), &changed)
        .await
        .assert_status(422);

    // The new version, effective from 2028-01-01, is accepted.
    t.cards
        .accept(&contract.subject(), date(2028, 1, 1), &[("guard", "day")]);
    t.post(&amal, &path(&contract), &changed)
        .await
        .assert_status(303);
    let renewed = t.reload_contract(contract.id).await;
    assert_eq!(renewed.renewal_rates_from, Some(date(2028, 1, 1)));
    assert_eq!(renewed.end_date, date(2028, 12, 31));

    // The date of the new rates must lie inside the renewed contract.
    let early = t
        .post(
            &amal,
            &path(&contract),
            &[
                ("end_date", "2029-12-31"),
                ("signed_on", "2026-10-01"),
                ("document", "R2.pdf"),
                ("rates_from", "2027-01-01"),
            ],
        )
        .await;
    early.assert_status(422);
    assert!(
        early.text().contains("after the contract starts"),
        "{}",
        early.text()
    );
}

/// AC5: K `on_hold` is renewed and stays `on_hold`, with its hold reasons,
/// the state before the hold and the hold day. A `terminating` contract is not
/// renewed, nor is a `draft`.
#[renox::test]
async fn a_contract_on_hold_is_renewed_and_stays_on_hold() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", contract.id),
        &[("reason", "other"), ("text", "Liquidation")],
    )
    .await
    .assert_status(303);
    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);

    t.post(&amal, &path(&contract), &form())
        .await
        .assert_status(303);
    let renewed = t.reload_contract(contract.id).await;
    assert_eq!(renewed.status, ContractStatus::OnHold);
    assert_eq!(renewed.end_date, date(2028, 12, 31));
    assert_eq!(renewed.status_before_hold, Some(ContractStatus::Active));
    assert_eq!(renewed.held_at, held.held_at);
    assert_eq!(
        manpower::ContractHold::open_of(t.db(), contract.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let ending = t
        .contract(TOWER, ContractStatus::Terminating, Some(&amal))
        .await;
    t.post(&amal, &path(&ending), &form())
        .await
        .assert_status(422);
    let draft = t.contract(TOWER, ContractStatus::Draft, Some(&amal)).await;
    t.post(&amal, &path(&draft), &form())
        .await
        .assert_status(422);
    assert_eq!(
        t.reload_contract(ending.id).await.end_date,
        date(2027, 12, 31)
    );
}

/// A request to terminate the contract that still waits for the MD is
/// withdrawn by the renewal (`termination::withdraw_pending`, BL-MPW-04); the
/// event of the renewal is emitted.
#[renox::test]
async fn a_renewal_withdraws_the_request_to_terminate_that_is_waiting() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/terminate", contract.id),
        &[
            ("initiated_by", "client"),
            ("reason", "budget cut"),
            ("notice_on", "2027-10-01"),
            ("end_date", "2027-12-01"),
        ],
    )
    .await
    .assert_status(303);
    let waiting = ApprovalRequest::pending_about(t.db(), &contract.subject())
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);

    t.post(&amal, &path(&contract), &form())
        .await
        .assert_status(303);

    let request = ApprovalRequest::find_or_404(t.db(), waiting[0].id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Withdrawn);
    assert_eq!(t.renewed.all().len(), 1);
    assert_eq!(t.renewed.all()[0].end_date, date(2028, 12, 31));
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
}
