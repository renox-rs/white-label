//! BL-MPW-01 (#415): putting a client contract on credit hold (`hold`,
//! `MPW.RUL.10`). The daily rule holds a contract with an invoice overdue more
//! than 30 days, or a bounced cheque of the client; the account manager holds
//! by hand with a reason. The account manager and the MD are told once with
//! the amount and the workers on site, the client nothing, and no deployment
//! changes.

mod support;

use manpower::notices::{CLIENT_NOTICE, NOTICE};
use manpower::{
    CREDIT_HOLD_TASK, ClientContract, ContractHold, ContractStatus, DeploymentStatus, HoldReason,
};
use wl_kernel::notify::{NotifyRecipient, NotifyRequest};

use support::{Booted, FOODS, TOWER, boot, date, users_of};

/// `count` workers `active` at the Tower site under `contract`.
async fn workers_on_site(t: &Booted, contract: &ClientContract, count: i64) -> Vec<i64> {
    let site = t.site(TOWER, "Tower A").await;
    let mut ids = Vec::new();
    for n in 1..=count {
        let worker = t.staff.employ(&t.parties, n, &format!("Worker {n}"), true);
        let deployment = t
            .deployment(
                contract,
                &site,
                worker,
                Some(date(2027, 6, 30)),
                DeploymentStatus::Active,
            )
            .await;
        ids.push(deployment.id);
    }
    ids
}

/// The messages to the staff that mention the hold.
fn hold_notices(t: &Booted) -> Vec<NotifyRequest> {
    t.notices
        .of_type(NOTICE)
        .into_iter()
        .filter(|request| {
            request.data["title"]
                .as_str()
                .is_some_and(|title| title.contains("hold"))
        })
        .collect()
}

/// The daily run of `MPW.RUL.10` on `day`.
async fn run_rule(t: &Booted, day: renox::chrono::NaiveDate) {
    t.on(day, async {
        t.app.run_scheduled(CREDIT_HOLD_TASK).await.unwrap();
    })
    .await;
}

/// AC1, AC2: an `active` contract with INV-1 of AED 42,000 due 2026-10-01 and
/// unpaid stays `active` on 2026-10-31 (30 days) and is `on_hold` on
/// 2026-11-01 (31 days), reason `overdue`, state before `active`; the account
/// manager and the MD get one message with AED 42,000 and the 12 workers on
/// site.
#[renox::test]
async fn an_invoice_overdue_more_than_thirty_days_holds_the_contract() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    workers_on_site(&t, &contract, 12).await;
    let invoice = t
        .invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));

    run_rule(&t, date(2026, 10, 31)).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert!(hold_notices(&t).is_empty());

    run_rule(&t, date(2026, 11, 1)).await;
    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(ContractStatus::Active));
    assert!(held.held_at.is_some());
    assert_eq!(held.held_by, None);
    let reasons = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].reason, HoldReason::Overdue);
    assert_eq!(reasons[0].invoice_id, Some(invoice.0));
    assert_eq!(reasons[0].amount, 4_200_000);
    assert_eq!(reasons[0].held_by, None);

    let told = hold_notices(&t);
    assert_eq!(told.len(), 1, "one message for both managers");
    let mut users = users_of(&told[0]);
    users.sort_unstable();
    assert_eq!(users, vec![amal.id, bader.id]);
    let body = told[0].data["body"].as_str().unwrap();
    assert!(body.contains("AED 42,000"), "{body}");
    assert!(body.contains("INV-1"), "{body}");
    assert!(body.contains("(12)"), "{body}");
    assert!(body.contains("Worker 1, guard at Tower A"), "{body}");
}

/// AC3: a `disputed` invoice does not hold the contract, nor does one that is
/// written off or paid.
#[renox::test]
async fn a_disputed_invoice_does_not_hold_the_contract() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let invoice = t
        .invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    t.invoices.dispute(invoice);
    let paid = t
        .invoices
        .issue(&contract, "INV-2", 100_000, date(2026, 9, 1));
    t.invoices.pay(paid, 100_000);

    run_rule(&t, date(2026, 11, 2)).await;
    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert!(hold_notices(&t).is_empty());
}

/// AC4: a `terminating` contract and `ChequeBounced` for the client's cheque of
/// AED 15,000: the contract is `on_hold`, reason `bounced_cheque`, state before
/// `terminating`; a contract of another client is not touched, and a draft or
/// ended contract of the same client is not either.
#[renox::test]
async fn a_bounced_cheque_holds_the_terminating_contract() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t
        .contract(TOWER, ContractStatus::Terminating, Some(&amal))
        .await;
    let draft = t.contract(TOWER, ContractStatus::Draft, None).await;
    let ended = t.contract(TOWER, ContractStatus::Ended, None).await;
    let other = t.contract(FOODS, ContractStatus::Active, None).await;

    t.state()
        .emit(t.register.bounce(9, TOWER, "CHQ-9", 1_500_000))
        .await
        .unwrap();

    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(ContractStatus::Terminating));
    let reasons = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    assert_eq!(reasons.len(), 1);
    assert_eq!(reasons[0].reason, HoldReason::BouncedCheque);
    assert_eq!(reasons[0].cheque_id, Some(9));
    assert_eq!(reasons[0].amount, 1_500_000);
    for untouched in [&draft, &ended, &other] {
        assert_ne!(
            t.reload_contract(untouched.id).await.status,
            ContractStatus::OnHold
        );
    }
    let told = hold_notices(&t);
    assert_eq!(told.len(), 1);
    let body = told[0].data["body"].as_str().unwrap();
    assert!(
        body.contains("CHQ-9") && body.contains("AED 15,000"),
        "{body}"
    );
}

/// A bounced cheque of a client with several contracts holds every one that is
/// `active` or `terminating`.
#[renox::test]
async fn a_bounced_cheque_holds_every_contract_of_the_client() {
    let t = boot().await;
    let first = t.contract(TOWER, ContractStatus::Active, None).await;
    let second = t.contract(TOWER, ContractStatus::Terminating, None).await;
    t.state()
        .emit(t.register.bounce(4, TOWER, "CHQ-4", 500_000))
        .await
        .unwrap();
    for contract in [&first, &second] {
        assert_eq!(
            t.reload_contract(contract.id).await.status,
            ContractStatus::OnHold
        );
    }
}

/// AC5: user A holds an `active` contract with the reason `other` and the text
/// "client in liquidation": the contract is `on_hold`, held by A, and the
/// managers are told.
#[renox::test]
async fn the_account_manager_holds_a_contract_by_hand() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let bader = t.md("Bader").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;

    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", contract.id),
        &[("reason", "other"), ("text", "client in liquidation")],
    )
    .await
    .assert_status(303);

    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(ContractStatus::Active));
    assert_eq!(held.held_by, Some(amal.id));
    let reasons = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    assert_eq!(reasons[0].reason, HoldReason::Other);
    assert_eq!(
        reasons[0].reason_text.as_deref(),
        Some("client in liquidation")
    );
    assert_eq!(reasons[0].held_by, Some(amal.id));
    let told = hold_notices(&t);
    assert_eq!(told.len(), 1);
    let mut users = users_of(&told[0]);
    users.sort_unstable();
    assert_eq!(users, vec![amal.id, bader.id]);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("client in liquidation")
    );
    // The history of the contract names the reason.
    let page = t.get(&amal, "/manpower/contracts").await;
    let rows: Vec<renox::serde_json::Value> = page.json();
    assert_eq!(rows[0]["status"], "On hold");
    assert_eq!(rows[0]["hold"]["status_before"], "Active");
    assert_eq!(rows[0]["hold"]["held_by"], amal.id);
}

/// AC6: `hold` without a reason is refused, so is `other` without its text,
/// and a user who does not manage the client's contracts is forbidden; a
/// draft contract cannot be held. Nothing changes.
#[renox::test]
async fn a_hold_without_a_reason_is_refused() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let stranger = t.account_manager("Sam", FOODS).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let path = format!("/manpower/contracts/{}/hold", contract.id);

    t.post(&amal, &path, &[]).await.assert_status(422);
    t.post(&amal, &path, &[("reason", "")])
        .await
        .assert_status(422);
    t.post(&amal, &path, &[("reason", "bored")])
        .await
        .assert_status(422);
    t.post(&amal, &path, &[("reason", "other"), ("text", "  ")])
        .await
        .assert_status(422);
    t.post(&stranger, &path, &[("reason", "overdue")])
        .await
        .assert_forbidden();
    let draft = t.contract(TOWER, ContractStatus::Draft, Some(&amal)).await;
    t.post(
        &amal,
        &format!("/manpower/contracts/{}/hold", draft.id),
        &[("reason", "overdue")],
    )
    .await
    .assert_status(422);

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::Active
    );
    assert!(
        ContractHold::of_contract(t.db(), contract.id)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(hold_notices(&t).is_empty());
}

/// AC7: after the hold the client is told nothing (the `FIN.RUL.01` reminders
/// are wl-invoicing's and go on as they do) and the deployments are unchanged.
#[renox::test]
async fn the_client_is_told_nothing_and_the_deployments_stay() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let deployments = workers_on_site(&t, &contract, 3).await;
    t.invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));

    run_rule(&t, date(2026, 11, 1)).await;

    assert_eq!(
        t.reload_contract(contract.id).await.status,
        ContractStatus::OnHold
    );
    assert!(t.notices.of_type(CLIENT_NOTICE).is_empty());
    assert!(
        t.notices.all().iter().all(|request| request
            .recipients
            .iter()
            .all(|recipient| matches!(recipient, NotifyRecipient::User { .. }))),
        "nothing went to the client company"
    );
    for id in deployments {
        let deployment = t.reload_deployment(id).await;
        assert_eq!(deployment.status, DeploymentStatus::Active);
        assert_eq!(deployment.end_date, Some(date(2027, 6, 30)));
    }
    assert!(t.ended.all().is_empty() && t.cancelled.all().is_empty());
}

/// AC8: running the rule twice on 2026-11-01 sends one notification and makes
/// one reason.
#[renox::test]
async fn the_rule_run_twice_the_same_day_tells_once() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    t.md("Bader").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));

    run_rule(&t, date(2026, 11, 1)).await;
    run_rule(&t, date(2026, 11, 1)).await;

    assert_eq!(hold_notices(&t).len(), 1);
    assert_eq!(
        ContractHold::of_contract(t.db(), contract.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

/// A contract already `on_hold` has no second `hold`: a cheque that bounces
/// while it is held for an overdue invoice adds a reason, and both managers
/// are told once more; the same cheque again changes nothing.
#[renox::test]
async fn a_new_cause_on_a_held_contract_adds_a_reason() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    t.md("Bader").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    run_rule(&t, date(2026, 11, 1)).await;
    assert_eq!(hold_notices(&t).len(), 1);

    let bounce = t.register.bounce(9, TOWER, "CHQ-9", 1_500_000);
    t.state().emit(bounce.clone()).await.unwrap();
    t.state().emit(bounce).await.unwrap();

    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);
    assert_eq!(held.status_before_hold, Some(ContractStatus::Active));
    let reasons = ContractHold::open_of(t.db(), contract.id).await.unwrap();
    let kinds: Vec<_> = reasons.iter().map(|row| row.reason).collect();
    assert_eq!(kinds, vec![HoldReason::Overdue, HoldReason::BouncedCheque]);
    let told = hold_notices(&t);
    assert_eq!(told.len(), 2);
    let body = told[1].data["body"].as_str().unwrap();
    assert!(
        body.contains("AED 42,000") && body.contains("CHQ-9"),
        "{body}"
    );
}
