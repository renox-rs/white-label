//! BL-MPW-19 (#433), `HRM.RUL.15` (the contract's side): the vertical's
//! `ApproverResolver` answers for the timesheets of a deployment from the
//! client contract: the account manager, the escalation contact, the client
//! approvers of the portal and the deemed-approval days in force when the
//! timesheet was submitted. The account manager fills the fields on the
//! contract.

mod support;

use manpower::ContractStatus;
use manpower::approvers::ContractTerms;
use renox::prelude::*;
use wl_kernel::notify::NotifyRecipient;
use wl_kernel::timesheets::approver_resolver;

use support::{Booted, FOODS, TOWER, boot, date};

/// A deployment of a worker under `contract`; the timesheet context.
async fn context_of(t: &Booted, contract: &manpower::ClientContract) -> wl_kernel::Subject {
    let site = t.site(TOWER, "Tower A").await;
    let worker = t.staff.employ(&t.parties, 1, "Worker 1", true);
    t.deployment(
        contract,
        &site,
        worker,
        Some(date(2027, 6, 30)),
        manpower::DeploymentStatus::Active,
    )
    .await
    .subject()
}

/// AC1: contract K with deemed days 10 and an escalation contact: the resolver
/// returns both for a timesheet of a K deployment, with the account manager of
/// K and the client approvers of the portal (the roles `owner` and `approver`,
/// not `viewer`).
#[renox::test]
async fn the_resolver_returns_the_contract_values_for_a_deployment_timesheet() {
    let t = boot().await;
    let am = t.account_manager("Amal", TOWER).await;
    let k = t.contract(TOWER, ContractStatus::Active, Some(&am)).await;
    let context = context_of(&t, &k).await;
    let owner = t.client_user("Olga", TOWER, &["owner"]).await;
    let approver = t.client_user("Carl", TOWER, &["approver"]).await;
    t.client_user("Vera", TOWER, &["viewer"]).await;
    t.client_user("Otto", FOODS, &["approver"]).await;

    t.post(
        &am,
        &format!("/manpower/contracts/{}/timesheet-terms", k.id),
        &[
            ("deemed_approval_days", "10"),
            ("escalation_contact_party_id", &FOODS.0.to_string()),
        ],
    )
    .await
    .assert_status(303);

    let resolver = approver_resolver(t.state());
    assert_eq!(
        resolver
            .deemed_approval_days(t.state(), Some(&context))
            .await
            .unwrap(),
        Some(10)
    );
    assert_eq!(
        resolver
            .escalation_contacts(t.state(), Some(&context))
            .await
            .unwrap(),
        vec![NotifyRecipient::Party { id: FOODS }]
    );
    assert_eq!(
        resolver
            .account_manager(t.state(), Some(&context))
            .await
            .unwrap(),
        Some(am.id)
    );
    let mut approvers = resolver
        .client_approvers(t.state(), Some(&context))
        .await
        .unwrap();
    approvers.sort_by_key(|r| format!("{r:?}"));
    let mut wanted = vec![
        NotifyRecipient::User { id: owner.id },
        NotifyRecipient::User { id: approver.id },
    ];
    wanted.sort_by_key(|r| format!("{r:?}"));
    assert_eq!(approvers, wanted);
}

/// AC2 and AC3: a contract without deemed days and without an escalation
/// contact: the resolver returns none of them (nothing is ever deemed), and a
/// context that is not a deployment gets no answer at all.
#[renox::test]
async fn a_contract_without_the_fields_has_no_clause_and_no_contact() {
    let t = boot().await;
    let am = t.account_manager("Amal", TOWER).await;
    let k = t.contract(TOWER, ContractStatus::Active, Some(&am)).await;
    let context = context_of(&t, &k).await;
    let resolver = approver_resolver(t.state());
    assert_eq!(
        resolver
            .deemed_approval_days(t.state(), Some(&context))
            .await
            .unwrap(),
        None
    );
    assert!(
        resolver
            .escalation_contacts(t.state(), Some(&context))
            .await
            .unwrap()
            .is_empty()
    );
    let elsewhere = wl_kernel::Subject::new("construction.project", 4);
    assert_eq!(
        resolver
            .deemed_approval_days(t.state(), Some(&elsewhere))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        resolver
            .account_manager(t.state(), Some(&elsewhere))
            .await
            .unwrap(),
        None
    );
}

/// AC4: K deems after 10 days; a timesheet submitted on 2026-11-01 is judged by
/// 10 days after K is changed to 5 on 2026-11-05, while one submitted on
/// 2026-11-06 is judged by 5. Emptying the field removes the clause for
/// timesheets submitted afterwards only.
#[renox::test]
async fn a_change_of_the_days_does_not_change_a_timesheet_already_submitted() {
    let t = boot().await;
    let am = t.account_manager("Amal", TOWER).await;
    let k = t.contract(TOWER, ContractStatus::Active, Some(&am)).await;
    let context = context_of(&t, &k).await;
    let path = format!("/manpower/contracts/{}/timesheet-terms", k.id);
    let resolver = approver_resolver(t.state());

    t.on(date(2026, 11, 1), async {
        t.post(&am, &path, &[("deemed_approval_days", "10")])
            .await
            .assert_status(303);
    })
    .await;
    let first = t.on(date(2026, 11, 2), async { renox::db::now() }).await;
    t.on(date(2026, 11, 5), async {
        t.post(&am, &path, &[("deemed_approval_days", "5")])
            .await
            .assert_status(303);
    })
    .await;
    let second = t.on(date(2026, 11, 6), async { renox::db::now() }).await;
    t.on(date(2026, 11, 9), async {
        t.post(&am, &path, &[("deemed_approval_days", "")])
            .await
            .assert_status(303);
    })
    .await;
    let third = t.on(date(2026, 11, 10), async { renox::db::now() }).await;

    let at = |when| resolver.deemed_approval_days_at(t.state(), Some(&context), when);
    assert_eq!(at(first).await.unwrap(), Some(10));
    assert_eq!(at(second).await.unwrap(), Some(5));
    assert_eq!(at(third).await.unwrap(), None);
    assert_eq!(t.reload_contract(k.id).await.deemed_approval_days, None);
    // Four rows: the contract as it was made, then each of the three changes.
    let rows = ContractTerms::query()
        .where_(ContractTerms::CLIENT_CONTRACT_ID.eq(k.id))
        .count(t.db())
        .await
        .unwrap();
    assert_eq!(rows, 4);
}

/// The fields are the account manager's to fill: days below 1 and an unknown
/// party are refused, and a user without `contracts.manage` for the client gets
/// 403.
#[renox::test]
async fn the_terms_need_whole_days_a_known_contact_and_the_permission() {
    let t = boot().await;
    let am = t.account_manager("Amal", TOWER).await;
    let other = t.account_manager("Olaf", FOODS).await;
    let k = t.contract(TOWER, ContractStatus::Active, Some(&am)).await;
    let path = format!("/manpower/contracts/{}/timesheet-terms", k.id);

    t.post(&am, &path, &[("deemed_approval_days", "0")])
        .await
        .assert_status(422);
    t.post(&am, &path, &[("deemed_approval_days", "-3")])
        .await
        .assert_status(422);
    t.post(&am, &path, &[("escalation_contact_party_id", "9999")])
        .await
        .assert_status(422);
    t.post(&other, &path, &[("deemed_approval_days", "7")])
        .await
        .assert_status(403);
    assert_eq!(t.reload_contract(k.id).await.deemed_approval_days, None);
}
