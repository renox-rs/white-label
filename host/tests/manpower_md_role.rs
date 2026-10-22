//! BL-MPW-15 (#429), 09 §3.1 `MPW.ACL.14`, `MPW.ACL.17`, `PLT.RUL.04`: the host
//! of a manpower agency defines the role `md` (for this deployment only), so a
//! user holding it approves the contract release and termination the account
//! manager asks for (segregation of duties still applies) and is alerted when a
//! compliance-critical rule fails. The company admin stays the fallback.

mod manpower_support;
mod support;

use std::time::Duration;

use host::roles::ADMIN;
use manpower::{ClientContract, ContractHold, ContractStatus, HoldReason};
use manpower_support::{Account, account, boot_seeded, post};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use support::{day, person};
use wl_kernel::approvals::{ApprovalRequest, approve};
use wl_kernel::{RuleRunSpec, RunOutcome, Subject};

/// A contract held for an overdue invoice of AED 1,000, held from `Active`.
async fn held(app: &renox::testing::TestApp, a: &Account) -> ClientContract {
    let contract = &contract_in(app, a, "CC-0003", ContractStatus::OnHold).await;
    ContractHold::create(
        app.db(),
        ContractHold {
            client_contract_id: contract.id,
            reason: HoldReason::Overdue,
            label: Some("INV-1".into()),
            amount: 100_000,
            ..ContractHold::default()
        },
    )
    .await
    .unwrap();
    ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap()
}

async fn waiting(app: &renox::testing::TestApp, contract: &ClientContract) -> Vec<ApprovalRequest> {
    ApprovalRequest::pending_about(app.db(), &contract.subject())
        .await
        .unwrap()
}

/// A user holding `md` approves a release while money is owed; the requester
/// (the account manager) cannot approve their own request.
#[renox::test]
async fn an_md_role_user_approves_a_release() {
    let app = boot_seeded().await;
    let a = account(&app).await;
    let mona = person(&app, "Mona", "md").await;
    let contract = held(&app, &a).await;

    post(
        &app,
        &a.amal,
        &format!("/manpower/contracts/{}/release", contract.id),
        &[],
    )
    .await
    .assert_status(303);
    let requests = waiting(&app, &contract).await;
    assert_eq!(requests.len(), 1);
    let refused = approve(app.state(), requests[0].id, &a.amal, "").await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");

    approve(app.state(), requests[0].id, &mona, "payment plan")
        .await
        .unwrap();
    let released = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(released.status, ContractStatus::Active);
}

/// A user holding `md` approves a termination; the company admin still can
/// approve the same step (the fallback).
#[renox::test]
async fn an_md_role_user_approves_a_termination_and_the_admin_still_can() {
    let app = boot_seeded().await;
    let a = account(&app).await;
    let mona = person(&app, "Mona", "md").await;
    let admin = person(&app, "Ada", ADMIN).await;
    let end = day(60).to_string();
    let notice = day(0).to_string();
    let form = [
        ("initiated_by", "client"),
        ("reason", "budget cut"),
        ("notice_on", notice.as_str()),
        ("end_date", end.as_str()),
    ];
    let path = format!("/manpower/contracts/{}/terminate", a.contract.id);

    post(&app, &a.amal, &path, &form).await.assert_status(303);
    let requests = waiting(&app, &a.contract).await;
    assert_eq!(requests.len(), 1);
    approve(app.state(), requests[0].id, &mona, "")
        .await
        .unwrap();
    let ended = ClientContract::find_or_404(app.db(), a.contract.id)
        .await
        .unwrap();
    assert_eq!(ended.status, ContractStatus::Terminating);

    // The admin approves a second contract's termination.
    let second = contract_in(&app, &a, "CC-0002", ContractStatus::Active).await;
    post(
        &app,
        &a.amal,
        &format!("/manpower/contracts/{}/terminate", second.id),
        &form,
    )
    .await
    .assert_status(303);
    let requests = waiting(&app, &second).await;
    assert_eq!(requests.len(), 1);
    approve(app.state(), requests[0].id, &admin, "")
        .await
        .unwrap();
    assert_eq!(
        ClientContract::find_or_404(app.db(), second.id)
            .await
            .unwrap()
            .status,
        ContractStatus::Terminating
    );
}

/// Another contract of the same client in `status`, with the same account
/// manager (`on_hold` comes from `active`).
async fn contract_in(
    app: &renox::testing::TestApp,
    a: &Account,
    reference: &str,
    status: ContractStatus,
) -> ClientContract {
    let mut contract = ClientContract {
        reference: reference.into(),
        status_before_hold: (status == ContractStatus::OnHold).then_some(ContractStatus::Active),
        client_party_id: a.client.0,
        start_date: day(-200),
        end_date: day(400),
        payment_terms_days: 30,
        notice_period_days: 30,
        account_manager_user_id: Some(a.amal.id),
        status,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, status, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    contract
}

/// `PLT.RUL.04`: a critical rule of the agency fails for good and the owner
/// alert reaches the user holding `md` (and the admin).
#[renox::test]
async fn a_critical_rule_alert_reaches_a_user_with_the_role_md() {
    let app = boot_seeded().await;
    let mona = person(&app, "Mona", "md").await;
    let clerk = person(&app, "Clark", "member").await;
    let rule = manpower::seed::CRITICAL_RULES[0];
    let run = |fail: bool| {
        let spec = RuleRunSpec::new(rule, Subject::new("hr.payroll_run", 4), "run:1");
        app.at_travelled_time(wl_kernel::automation::run_rule(
            app.state(),
            spec,
            move |_state| async move {
                if fail {
                    Err(Error::Status(
                        StatusCode::BAD_GATEWAY,
                        "bank file rejected".to_owned(),
                    ))
                } else {
                    Ok(())
                }
            },
        ))
    };
    for wait in [60_u64, 300, 900, 3600] {
        assert_eq!(run(true).await.unwrap(), RunOutcome::Retrying);
        app.travel(Duration::from_secs(wait));
    }
    assert_eq!(run(true).await.unwrap(), RunOutcome::Failed);

    let alerts = |user: &User| {
        let user = user.clone();
        let db = app.db().clone();
        async move {
            user.notifications(&db, 50)
                .await
                .unwrap()
                .into_iter()
                .filter(|n| n.kind == wl_kernel::compliance::service::ALERT_KIND)
                .count()
        }
    };
    assert_eq!(alerts(&mona).await, 1, "the MD role is alerted");
    assert_eq!(alerts(&clerk).await, 0);
}
