//! BL-MPW-04 (#418): terminating a client contract early (`terminate`,
//! `MPW.ACL.17`). The account manager requests it with who started it, the
//! reason, the notice date and the new end date; the MD approves; the
//! contract is `terminating`, deployments are cut to the new end date, the
//! ones that would start after it are cancelled and the client has the
//! confirmation. Nothing changes before the approval and nothing ends at once.

mod support;

use manpower::notices::{CLIENT_NOTICE, NOTICE, STATUTORY_NOTICE};
use manpower::termination::withdraw_pending;
use manpower::{
    ClientContract, ContractHold, ContractStatus, Deployment, DeploymentStatus, RequestStatus, Site,
};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use wl_kernel::approvals::{ApprovalRequest, ApprovalStatus, approve};

use support::{Booted, TOWER, boot, date, users_of};

/// A contract of Tower in `status` ending 2027-06-30, managed by `manager`.
async fn contract_ending_june(
    t: &Booted,
    manager: &User,
    status: ContractStatus,
) -> ClientContract {
    let mut contract = t.contract(TOWER, status, Some(manager)).await;
    contract.end_date = date(2027, 6, 30);
    contract.save(t.db()).await.unwrap();
    contract
}

/// A deployment of a new worker, with its start and end set.
async fn deployment(
    t: &Booted,
    contract: &ClientContract,
    site: &Site,
    n: i64,
    status: DeploymentStatus,
    start: NaiveDate,
    end: Option<NaiveDate>,
) -> Deployment {
    let worker = t.staff.employ(&t.parties, n, &format!("Worker {n}"), true);
    let mut deployment = t.deployment(contract, site, worker, end, status).await;
    deployment.start_date = start;
    deployment.save(t.db()).await.unwrap();
    deployment
}

/// The form of a request: client, "budget cut", notice 2026-11-01.
fn form(end: &'static str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("initiated_by", "client"),
        ("reason", "budget cut"),
        ("notice_on", "2026-11-01"),
        ("end_date", end),
    ]
}

fn path(contract: &ClientContract) -> String {
    format!("/manpower/contracts/{}/terminate", contract.id)
}

async fn pending(t: &Booted, contract: &ClientContract) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(t.db(), &contract.subject())
        .await
        .unwrap()
}

/// AC1: A requests `terminate` (initiated by the client, "budget cut", notice
/// 2026-11-01, end 2026-12-01): an approval request goes to the MD and K is
/// unchanged, nothing is cut. AC2: when MD user B approves, K is `terminating`
/// ending 2026-12-01, D1 (end 2027-06-30) ends 2026-12-01, D2 (end 2026-11-20)
/// keeps its date, planned D3 starting 2026-12-10 is `cancelled`, open request
/// R starting 2026-12-15 is `cancelled`, and the client gets the
/// confirmation. Nothing is ended.
#[renox::test]
async fn the_md_approves_the_termination_and_the_deployments_are_cut() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let contract = contract_ending_june(&t, &amal, ContractStatus::Active).await;
    let site = t.site(TOWER, "Tower A").await;
    let d1 = deployment(
        &t,
        &contract,
        &site,
        1,
        DeploymentStatus::Active,
        date(2026, 8, 1),
        Some(date(2027, 6, 30)),
    )
    .await;
    let d2 = deployment(
        &t,
        &contract,
        &site,
        2,
        DeploymentStatus::Active,
        date(2026, 8, 1),
        Some(date(2026, 11, 20)),
    )
    .await;
    let d3 = deployment(
        &t,
        &contract,
        &site,
        3,
        DeploymentStatus::Planned,
        date(2026, 12, 10),
        Some(date(2027, 3, 31)),
    )
    .await;
    let d4 = deployment(
        &t,
        &contract,
        &site,
        4,
        DeploymentStatus::Planned,
        date(2026, 11, 20),
        Some(date(2027, 3, 31)),
    )
    .await;
    let d5 = deployment(
        &t,
        &contract,
        &site,
        5,
        DeploymentStatus::Suspended,
        date(2026, 8, 1),
        None,
    )
    .await;
    let d6 = deployment(
        &t,
        &contract,
        &site,
        6,
        DeploymentStatus::Planned,
        date(2026, 12, 20),
        Some(date(2027, 3, 31)),
    )
    .await;
    let mut r = t.request(&contract, &site, 2, RequestStatus::Open).await;
    r.start_date = date(2026, 12, 15);
    r.save(t.db()).await.unwrap();
    t.fills(&d6, &r).await;
    let mut kept = t.request(&contract, &site, 1, RequestStatus::Open).await;
    kept.start_date = date(2026, 11, 15);
    kept.save(t.db()).await.unwrap();

    t.post(&amal, &path(&contract), &form("2026-12-01"))
        .await
        .assert_status(303);

    // Nothing changes before the approval.
    let waiting = t.reload_contract(contract.id).await;
    assert_eq!(waiting.status, ContractStatus::Active);
    assert_eq!(waiting.end_date, date(2027, 6, 30));
    let requests = pending(&t, &contract).await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].requester_id, amal.id);
    assert_eq!(requests[0].action, "manpower.terminate");
    assert_eq!(
        t.reload_deployment(d1.id).await.end_date,
        Some(date(2027, 6, 30))
    );
    assert_eq!(
        t.reload_deployment(d3.id).await.status,
        DeploymentStatus::Planned
    );
    assert_eq!(t.reload_request(r.id).await.status, RequestStatus::Open);
    assert!(t.notices.of_type(CLIENT_NOTICE).is_empty());
    assert!(t.notices.of_type(STATUTORY_NOTICE).is_empty());

    approve(t.state(), requests[0].id, &bader, "agreed")
        .await
        .unwrap();

    let done = t.reload_contract(contract.id).await;
    assert_eq!(done.status, ContractStatus::Terminating);
    assert_eq!(done.end_date, date(2026, 12, 1));
    assert_eq!(done.original_end_date, Some(date(2027, 6, 30)));
    assert_eq!(done.termination_initiated_by.as_deref(), Some("client"));
    assert_eq!(done.termination_reason.as_deref(), Some("budget cut"));
    assert_eq!(done.termination_notice_on, Some(date(2026, 11, 1)));
    assert_eq!(done.termination_requested_by, Some(amal.id));
    assert!(done.terminated_at.is_some());
    // Active and suspended deployments end on the new end date at the latest.
    let d1 = t.reload_deployment(d1.id).await;
    assert_eq!(
        (d1.status, d1.end_date),
        (DeploymentStatus::Active, Some(date(2026, 12, 1)))
    );
    let d2 = t.reload_deployment(d2.id).await;
    assert_eq!(
        d2.end_date,
        Some(date(2026, 11, 20)),
        "an earlier end is kept"
    );
    let d5 = t.reload_deployment(d5.id).await;
    assert_eq!(
        (d5.status, d5.end_date),
        (DeploymentStatus::Suspended, Some(date(2026, 12, 1)))
    );
    // Planned: those starting after the end are cancelled, the others capped.
    let d3 = t.reload_deployment(d3.id).await;
    assert_eq!(d3.status, DeploymentStatus::Cancelled);
    assert_eq!(d3.cancel_reason.as_deref(), Some("contract terminated"));
    let d4 = t.reload_deployment(d4.id).await;
    assert_eq!(
        (d4.status, d4.end_date),
        (DeploymentStatus::Planned, Some(date(2026, 12, 1)))
    );
    // The request starting after it is cancelled with its planned deployment.
    let r = t.reload_request(r.id).await;
    assert_eq!(r.status, RequestStatus::Cancelled);
    assert_eq!(r.cancel_reason.as_deref(), Some("contract terminated"));
    assert_eq!(
        t.reload_deployment(d6.id).await.status,
        DeploymentStatus::Cancelled
    );
    assert_eq!(t.reload_request(kept.id).await.status, RequestStatus::Open);
    // Nothing is ended now; the cancelled deployments are announced.
    assert!(t.ended.all().is_empty());
    let mut cancelled: Vec<_> = t
        .cancelled
        .all()
        .iter()
        .map(|event| event.deployment.subject_id)
        .collect();
    cancelled.sort_unstable();
    assert_eq!(cancelled, vec![d3.id, d6.id]);
    // The client has the confirmation, once; the account manager is told.
    // The confirmation is a statutory notice (09 §7.2, BL-MPW-20).
    assert!(t.notices.of_type(CLIENT_NOTICE).is_empty());
    let letters = t.notices.of_type(STATUTORY_NOTICE);
    assert_eq!(letters.len(), 1);
    let title = letters[0].data["title"].as_str().unwrap();
    assert!(title.contains("2026-12-01"), "{title}");
    let told: Vec<_> = t
        .notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("ends early"))
        .collect();
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), vec![amal.id]);
}

/// AC3: with end 2026-11-15 (14 days after the notice) and no waiver the
/// request is refused; with a waiver file it is allowed. A later end date, a
/// missing initiator, reason or date are refused too, and nothing is written.
#[renox::test]
async fn a_short_notice_needs_a_waiver() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_june(&t, &amal, ContractStatus::Active).await;

    let short = t.post(&amal, &path(&contract), &form("2026-11-15")).await;
    short.assert_status(422);
    assert!(short.text().contains("30 days"), "{}", short.text());
    t.post(&amal, &path(&contract), &form("2027-07-01"))
        .await
        .assert_status(422);
    t.post(
        &amal,
        &path(&contract),
        &[
            ("initiated_by", "bored"),
            ("reason", " "),
            ("notice_on", "2026-11-01"),
            ("end_date", "2026-12-01"),
        ],
    )
    .await
    .assert_status(422);
    t.post(
        &amal,
        &path(&contract),
        &[("initiated_by", "agency"), ("reason", "x")],
    )
    .await
    .assert_status(422);
    assert!(pending(&t, &contract).await.is_empty());

    let mut waived = form("2026-11-15");
    waived.push(("waiver", "CLIENT-WAIVER.pdf"));
    t.post(&amal, &path(&contract), &waived)
        .await
        .assert_status(303);
    assert_eq!(pending(&t, &contract).await.len(), 1);
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
}

/// AC4: A cannot approve the request, not even holding the MD role; the MD
/// can. A manager of another client and a draft contract are refused.
#[renox::test]
async fn the_requester_cannot_approve_the_termination() {
    let t = boot().await;
    let bea = t.account_manager("Bea", TOWER).await;
    bea.assign_role(t.db(), manpower::permissions::MD_ROLE)
        .await
        .unwrap();
    let bader = t.md("Bader").await;
    let stranger = t.account_manager("Sam", support::FOODS).await;
    let contract = contract_ending_june(&t, &bea, ContractStatus::Active).await;
    t.post(&stranger, &path(&contract), &form("2026-12-01"))
        .await
        .assert_forbidden();
    let draft = contract_ending_june(&t, &bea, ContractStatus::Draft).await;
    t.post(&bea, &path(&draft), &form("2026-12-01"))
        .await
        .assert_status(422);

    t.post(&bea, &path(&contract), &form("2026-12-01"))
        .await
        .assert_status(303);
    let waiting = pending(&t, &contract).await;
    let refused = approve(t.state(), waiting[0].id, &bea, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );

    approve(t.state(), waiting[0].id, &bader, "").await.unwrap();
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Terminating
    );
}

/// AC5: from `on_hold`, after approval K is `terminating` and the hold is
/// cleared (the state before dropped, the reasons closed); a pending request
/// to release the hold is withdrawn.
#[renox::test]
async fn a_held_contract_can_be_terminated_and_the_hold_is_cleared() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let contract = contract_ending_june(&t, &amal, ContractStatus::Active).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", contract.id),
        &[("reason", "other"), ("text", "client in liquidation")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );

    t.post(&amal, &path(&contract), &form("2026-12-01"))
        .await
        .assert_status(303);
    let waiting = pending(&t, &contract).await;
    assert_eq!(waiting.len(), 1);
    approve(t.state(), waiting[0].id, &bader, "").await.unwrap();

    let done = t.reload_contract(contract.id).await;
    assert_eq!(done.status, ContractStatus::Terminating);
    assert_eq!(done.status_before_hold, None);
    assert_eq!(done.held_at, None);
    let reasons = ContractHold::of_contract(t.db(), contract.id)
        .await
        .unwrap();
    assert!(reasons.iter().all(|row| !row.is_open() && !row.covered));
    assert_eq!(reasons[0].resolution.as_deref(), Some("terminated"));
}

/// AC6: A's request pending and K put `on_hold` by `MPW.RUL.10` before B
/// approves: B's approval still runs, K is `terminating` and the hold is
/// cleared; the rule can hold the terminating contract again.
#[renox::test]
async fn a_hold_meanwhile_does_not_stop_the_approval() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let contract = contract_ending_june(&t, &amal, ContractStatus::Active).await;
    t.invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    t.post(&amal, &path(&contract), &form("2026-12-01"))
        .await
        .assert_status(303);
    let waiting = pending(&t, &contract).await;

    t.on(date(2026, 11, 1), async {
        t.app
            .run_scheduled(manpower::CREDIT_HOLD_TASK)
            .await
            .unwrap();
    })
    .await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    // The request is still the only one pending: the hold did not end it.
    assert_eq!(pending(&t, &contract).await.len(), 1);

    approve(t.state(), waiting[0].id, &bader, "").await.unwrap();

    let done = t.reload_contract(contract.id).await;
    assert_eq!(done.status, ContractStatus::Terminating);
    assert_eq!(done.status_before_hold, None);
    assert!(
        ContractHold::open_of(t.db(), contract.id)
            .await
            .unwrap()
            .is_empty()
    );

    // INV-1 is still overdue, and the contract was not released by the MD:
    // the next run holds the terminating contract again.
    t.on(date(2026, 11, 2), async {
        t.app
            .run_scheduled(manpower::CREDIT_HOLD_TASK)
            .await
            .unwrap();
    })
    .await;
    let again = t.reload_contract(contract.id).await;
    assert_eq!(again.status, ContractStatus::OnHold);
    assert_eq!(again.status_before_hold, Some(ContractStatus::Terminating));
}

/// A request to release the hold that waits when the termination is approved
/// is withdrawn: the hold it would release is gone.
#[renox::test]
async fn a_pending_release_is_withdrawn_when_the_contract_terminates() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let contract = contract_ending_june(&t, &amal, ContractStatus::Active).await;
    t.invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    t.on(date(2026, 11, 1), async {
        t.app
            .run_scheduled(manpower::CREDIT_HOLD_TASK)
            .await
            .unwrap();
    })
    .await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);
    t.post(&amal, &path(&contract), &form("2026-12-01"))
        .await
        .assert_status(303);
    let all = pending(&t, &contract).await;
    assert_eq!(all.len(), 2);
    let termination = all
        .iter()
        .find(|r| r.action == "manpower.terminate")
        .unwrap();
    let release = all.iter().find(|r| r.action == "manpower.release").unwrap();

    approve(t.state(), termination.id, &bader, "")
        .await
        .unwrap();

    let release = ApprovalRequest::find_or_404(t.db(), release.id)
        .await
        .unwrap();
    assert_eq!(release.status, ApprovalStatus::Withdrawn);
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Terminating
    );
}

/// The contract `end`s or is `renew`ed while the request waits (BL-MPW-05,
/// BL-MPW-06 call `withdraw_pending`): the request is withdrawn and the
/// account manager is told; nothing was cut.
#[renox::test]
async fn an_end_or_a_renewal_meanwhile_withdraws_the_request() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = contract_ending_june(&t, &amal, ContractStatus::Active).await;
    t.post(&amal, &path(&contract), &form("2026-12-01"))
        .await
        .assert_status(303);
    let waiting = pending(&t, &contract).await;

    let count = withdraw_pending(t.state(), &contract, "the contract was renewed")
        .await
        .unwrap();

    assert_eq!(count, 1);
    assert!(pending(&t, &contract).await.is_empty());
    let request = ApprovalRequest::find_or_404(t.db(), waiting[0].id)
        .await
        .unwrap();
    assert_eq!(request.status, ApprovalStatus::Withdrawn);
    let told: Vec<_> = t
        .notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|n| n.data["title"].as_str().unwrap().contains("withdrawn"))
        .collect();
    assert_eq!(told.len(), 1);
    assert_eq!(users_of(&told[0]), vec![amal.id]);
    let kept = t.reload_contract(contract.id).await;
    assert_eq!(
        (kept.status, kept.end_date),
        (ContractStatus::Active, date(2027, 6, 30))
    );
    // A second call finds nothing to withdraw.
    assert_eq!(
        withdraw_pending(t.state(), &contract, "again")
            .await
            .unwrap(),
        0
    );
}

/// A request on a contract that is `terminating`, `ended` or `cancelled` is
/// refused and no approval request is made.
#[renox::test]
async fn only_an_active_or_held_contract_can_be_terminated() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    for status in [
        ContractStatus::Terminating,
        ContractStatus::Ended,
        ContractStatus::Cancelled,
    ] {
        let contract = contract_ending_june(&t, &amal, status).await;
        t.post(&amal, &path(&contract), &form("2026-12-01"))
            .await
            .assert_status(422);
        assert!(pending(&t, &contract).await.is_empty());
    }
}
