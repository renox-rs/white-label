//! BL-MPW-06 (#420): the system ends a client contract at the first daily run
//! after its end date (`end`). Its `active` and `suspended` deployments end with
//! the reason *contract end* and the contract's end date, the planned ones and
//! the open requests are cancelled, a hold is closed, each worker's open
//! timesheet is closed through the kernel's contract (wl-attendance) and the
//! workers `active` in HR are back on the bench. The billing of the closed
//! hours by `MPW.RUL.05` is not built: the feed to invoicing is deferred.

mod support;

use manpower::notices::{CLIENT_NOTICE, NOTICE};
use manpower::{ClientContract, ContractHold, ContractStatus, DeploymentStatus, RequestStatus};
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus};
use wl_kernel::workforce::EmployeeId;

use support::{Booted, TOWER, boot, date, users_of};

/// A contract of Tower in `status` ending 2027-12-01, managed by `manager`.
async fn contract_ending_dec(t: &Booted, manager: &User, status: ContractStatus) -> ClientContract {
    let mut contract = t.contract(TOWER, status, Some(manager)).await;
    contract.end_date = date(2027, 12, 1);
    contract.save(t.db()).await.unwrap();
    contract
}

async fn worker(t: &Booted, n: i64, active: bool) -> EmployeeId {
    t.staff
        .employ(&t.parties, n, &format!("Worker {n}"), active)
}

async fn run_end(t: &Booted, day: renox::chrono::NaiveDate) {
    t.on(day, async {
        t.app
            .run_scheduled(manpower::CONTRACT_END_TASK)
            .await
            .unwrap();
    })
    .await;
}

/// AC1: K `terminating` with end date 2027-12-01, active D1, suspended D2,
/// planned D3 and open request R: on the day after, K is `ended`, D1 and D2 are
/// `ended` with the reason *contract end* and end date 2027-12-01, D3 and R are
/// `cancelled`, the client is told per deployment ended and the account manager
/// once.
#[renox::test]
async fn the_contract_ends_with_its_deployments_and_requests_the_day_after_the_end_date() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Terminating).await;
    let site = t.site(TOWER, "Tower A").await;
    let w1 = worker(&t, 1, true).await;
    let w2 = worker(&t, 2, true).await;
    let w3 = worker(&t, 3, true).await;
    let d1 = t
        .deployment(
            &contract,
            &site,
            w1,
            Some(date(2027, 12, 1)),
            DeploymentStatus::Active,
        )
        .await;
    let d2 = t
        .deployment(&contract, &site, w2, None, DeploymentStatus::Suspended)
        .await;
    let d3 = t
        .deployment(
            &contract,
            &site,
            w3,
            Some(date(2028, 3, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    let open = t.request(&contract, &site, 2, RequestStatus::Open).await;
    let partly = t
        .request(&contract, &site, 3, RequestStatus::PartiallyFilled)
        .await;
    let filled = t.request(&contract, &site, 1, RequestStatus::Filled).await;

    // The end date itself is a working day: nothing ends on it.
    run_end(&t, date(2027, 12, 1)).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Terminating
    );
    assert!(t.contracts_ended.all().is_empty());

    run_end(&t, date(2027, 12, 2)).await;

    let ended = t.reload_contract(contract.id).await;
    assert_eq!(ended.status, ContractStatus::Ended);
    assert!(ended.ended_at.is_some());
    for id in [d1.id, d2.id] {
        let d = t.reload_deployment(id).await;
        assert_eq!(d.status, DeploymentStatus::Ended, "{id}");
        assert_eq!(d.end_reason.as_deref(), Some("contract_end"));
        assert_eq!(d.end_date, Some(date(2027, 12, 1)));
        assert_eq!(d.ended_on, Some(date(2027, 12, 1)));
    }
    let d3 = t.reload_deployment(d3.id).await;
    assert_eq!(d3.status, DeploymentStatus::Cancelled);
    assert_eq!(d3.cancel_reason.as_deref(), Some("contract ended"));
    for id in [open.id, partly.id] {
        let r = t.reload_request(id).await;
        assert_eq!(r.status, RequestStatus::Cancelled, "{id}");
        assert_eq!(r.cancel_reason.as_deref(), Some("contract ended"));
    }
    assert_eq!(
        t.reload_request(filled.id).await.status,
        RequestStatus::Filled,
        "a filled request is not open"
    );

    // The client is told per deployment ended; the account manager once.
    assert_eq!(t.notices.of_type(CLIENT_NOTICE).len(), 2);
    let told: Vec<_> = t
        .notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("has ended"))
        .collect();
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), vec![amal.id]);
    assert_eq!(t.ended.all().len(), 2);
    assert!(t.ended.all().iter().all(|e| e.reason == "contract_end"));
    assert_eq!(t.cancelled.all().len(), 1);
    assert_eq!(t.contracts_ended.all().len(), 1);

    // A second run finds the contract `ended` and does nothing.
    run_end(&t, date(2027, 12, 3)).await;
    assert_eq!(t.notices.of_type(CLIENT_NOTICE).len(), 2);
    assert_eq!(t.contracts_ended.all().len(), 1);
    assert_eq!(t.ended.all().len(), 2);
}

/// AC2 (the contract's side): each worker's open timesheet is closed at the end
/// date through the kernel's contract, once per deployment ended, with the
/// deployment as the context and the contract's end date as the last day.
#[renox::test]
async fn the_open_timesheet_of_each_deployment_is_closed_at_the_end_date() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Active).await;
    let site = t.site(TOWER, "Tower A").await;
    let w1 = worker(&t, 1, true).await;
    let w2 = worker(&t, 2, true).await;
    let d1 = t
        .deployment(&contract, &site, w1, None, DeploymentStatus::Active)
        .await;
    let d2 = t
        .deployment(&contract, &site, w2, None, DeploymentStatus::Suspended)
        .await;
    let planned = worker(&t, 3, true).await;
    let _d3 = t
        .deployment(&contract, &site, planned, None, DeploymentStatus::Planned)
        .await;

    run_end(&t, date(2027, 12, 2)).await;

    let calls = t.closings.all();
    assert_eq!(calls.len(), 2, "a planned deployment has no timesheet");
    assert_eq!(calls[0], (d1.subject(), w1, date(2027, 12, 1)));
    assert_eq!(calls[1], (d2.subject(), w2, date(2027, 12, 1)));
    let told: Vec<_> = t
        .notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("has ended"))
        .collect();
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("2 last timesheets were closed"),
        "{}",
        told[0].data["body"]
    );
}

/// AC3: D1's worker (`active` in HR) is on the bench; D2's worker (in `notice`)
/// is not, nor is a worker who is on another site.
#[renox::test]
async fn the_workers_still_active_go_back_to_the_bench() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Active).await;
    let other = t
        .contract(support::FOODS, ContractStatus::Active, Some(&amal))
        .await;
    let site = t.site(TOWER, "Tower A").await;
    let foods = t.site(support::FOODS, "Foods HQ").await;
    let w1 = worker(&t, 1, true).await;
    let w2 = worker(&t, 2, false).await;
    let w3 = worker(&t, 3, true).await;
    for w in [w1, w2, w3] {
        t.deployment(&contract, &site, w, None, DeploymentStatus::Active)
            .await;
    }
    // W3 is also on a site of another contract.
    t.deployment(&other, &foods, w3, None, DeploymentStatus::Active)
        .await;

    run_end(&t, date(2027, 12, 2)).await;

    let events = t.contracts_ended.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].bench, vec![w1]);
    assert_eq!(events[0].end_date, date(2027, 12, 1));
    assert!(manpower::bench::is_on_bench(t.state(), w1).await.unwrap());
    assert!(!manpower::bench::is_on_bench(t.state(), w2).await.unwrap());
    assert!(!manpower::bench::is_on_bench(t.state(), w3).await.unwrap());
    let body = t
        .notices
        .of_type(NOTICE)
        .into_iter()
        .map(|n| n.data["body"].as_str().unwrap().to_owned())
        .find(|body| body.contains("Back on the bench"))
        .unwrap();
    assert!(body.contains("Back on the bench: Worker 1."), "{body}");
}

/// AC4: K renewed on 2027-11-30 to 2028-11-30: nothing ends on 2027-12-02, and
/// the deployments stay on site.
#[renox::test]
async fn a_contract_renewed_before_the_run_does_not_end() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Active).await;
    let site = t.site(TOWER, "Tower A").await;
    let w1 = worker(&t, 1, true).await;
    let d1 = t
        .deployment(&contract, &site, w1, None, DeploymentStatus::Active)
        .await;
    t.on(date(2027, 11, 30), async {
        t.post(
            &amal,
            &format!("/manpower/contracts/{}/renew", contract.id),
            &[
                ("end_date", "2028-11-30"),
                ("signed_on", "2026-10-01"),
                ("document", "RENEWAL.pdf"),
            ],
        )
        .await
        .assert_status(303);
    })
    .await;

    run_end(&t, date(2027, 12, 2)).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert_eq!(
        t.reload_deployment(d1.id).await.status,
        DeploymentStatus::Active
    );
    assert!(t.closings.all().is_empty());
    assert!(t.contracts_ended.all().is_empty());
}

/// A contract `on_hold` ends too: its hold reasons are closed with the contract
/// and the hold fields cleared; an invoice overdue stays as it is, in
/// receivables.
#[renox::test]
async fn a_contract_on_hold_ends_and_its_hold_reasons_are_closed() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Active).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", contract.id),
        &[("reason", "other"), ("text", "Liquidation")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );

    run_end(&t, date(2027, 12, 2)).await;

    let ended = t.reload_contract(contract.id).await;
    assert_eq!(ended.status, ContractStatus::Ended);
    assert!(ended.status_before_hold.is_none() && ended.held_at.is_none());
    let reasons = ContractHold::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].resolution.as_deref(), Some("contract_end"));
    assert!(!reasons[0].is_open());
}

/// A request to terminate the contract that waits for the MD when the contract
/// ends is withdrawn (`termination::withdraw_pending`), and the account manager
/// is told; the contract is `ended`.
#[renox::test]
async fn an_end_meanwhile_withdraws_the_request_to_terminate() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Active).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/terminate", contract.id),
        &[
            ("initiated_by", "client"),
            ("reason", "budget cut"),
            ("notice_on", "2027-10-01"),
            ("end_date", "2027-11-15"),
        ],
    )
    .await
    .assert_status(303);
    let waiting = ApprovalRequest::pending_about(t.db(), &contract.subject())
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);

    run_end(&t, date(2027, 12, 2)).await;

    let request = ApprovalRequest::find_or_404(t.db(), waiting[0].id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Withdrawn);
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Ended
    );
}

/// Another worker's deployment of the same contract that ended earlier by its
/// own end date is left as it is: only `active` and `suspended` ones end now.
#[renox::test]
async fn deployments_already_ended_or_cancelled_are_left_alone() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_dec(&t, &amal, ContractStatus::Active).await;
    let site = t.site(TOWER, "Tower A").await;
    let w1 = worker(&t, 1, true).await;
    let done = t
        .deployment(
            &contract,
            &site,
            w1,
            Some(date(2027, 6, 30)),
            DeploymentStatus::Ended,
        )
        .await;

    run_end(&t, date(2027, 12, 2)).await;

    let kept = t.reload_deployment(done.id).await;
    assert_eq!(kept.status, DeploymentStatus::Ended);
    assert!(
        kept.end_reason.is_none(),
        "not ended again: {:?}",
        kept.end_reason
    );
    assert!(t.closings.all().is_empty());
    assert_eq!(t.ended.all().len(), 0);
}

/// The deployment `end` closes the open timesheet too (§7.1 *Deployment* `end`:
/// "open timesheet closed for the period"): a user's `end` and the daily
/// end-date run each close the worker's timesheet at the last day.
#[renox::test]
async fn any_deployment_end_closes_the_workers_open_timesheet() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let site = t.site(TOWER, "Tower A").await;
    let w1 = worker(&t, 1, true).await;
    let w2 = worker(&t, 2, true).await;
    let by_user = t
        .deployment(
            &contract,
            &site,
            w1,
            Some(date(2027, 3, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let by_run = t
        .deployment(
            &contract,
            &site,
            w2,
            Some(date(2027, 4, 15)),
            DeploymentStatus::Active,
        )
        .await;

    t.on(date(2027, 4, 1), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/end", by_user.id),
            &[("reason", "end_date")],
        )
        .await
        .assert_status(303);
    })
    .await;
    let calls = t.closings.all();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], (by_user.subject(), w1, date(2027, 3, 31)));

    t.on(date(2027, 4, 16), async {
        t.app.run_scheduled(manpower::END_DATE_TASK).await.unwrap();
    })
    .await;
    let calls = t.closings.all();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1], (by_run.subject(), w2, date(2027, 4, 15)));
    assert_eq!(
        t.reload_deployment(by_run.id).await.status,
        DeploymentStatus::Ended
    );
}
