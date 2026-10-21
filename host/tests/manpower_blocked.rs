//! BL-MPW-15 (#429), 09 §4.1 *The client is blocked as a party*, wl-parties
//! §5.6: with the real wl-parties, wl-hr and wl-ticketing in the host of a
//! manpower agency, a blocked client's workers on site continue, nothing new
//! starts (deployment `activate` and `extend`, contract `renew`, request
//! `submit`), staff without `parties.block_reason.view` are told only *on
//! hold*, the MD and the account manager get a task listing the client's
//! deployments, and the guards pass again after the unblock.

mod support;

use host::roles::{ACCOUNT_MANAGER, ADMIN, COMPLIANCE};
use manpower::blocked;
use manpower::permissions::client_scope;
use manpower::{
    ClientContract, ContractStatus, Deployment, DeploymentStatus, ManpowerRequest, RequestStatus,
    Site,
};
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, noon_clock, person, scoped_person, tenant};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_kernel::block_guard::ActionKind;
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{ClientConfig, PartyId};
use wl_ticketing::Ticket;

/// A manpower agency: the vertical on, with the shared modules it reaches.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

async fn boot() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// An employee `active` in HR.
async fn worker(app: &TestApp, name: &str) -> EmployeeId {
    let party = tenant(app, name).await;
    let employee = employees::create(
        app.state(),
        NewEmployee::new(party, "security_guard", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    EmployeeId(employee.id)
}

/// The client C with its account manager Amal, an active contract (the first
/// year of 2027 and the one before), a site, D active, D2 planned and a draft
/// request.
struct Account {
    client: PartyId,
    amal: User,
    contract: ClientContract,
    d: Deployment,
    d2: Deployment,
    request: ManpowerRequest,
}

async fn account(app: &TestApp) -> Account {
    let client = company(app, "Gulf Tower LLC").await;
    let amal = scoped_person(app, "Amal", ACCOUNT_MANAGER, &client_scope(client.0)).await;
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: client.0,
        start_date: renox::chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        end_date: renox::chrono::NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
        payment_terms_days: 30,
        notice_period_days: 30,
        account_manager_user_id: Some(amal.id),
        status: ContractStatus::Active,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let site = Site::create(
        app.db(),
        Site {
            client_party_id: client.0,
            name: "Tower A".into(),
            active: true,
            ..Site::default()
        },
    )
    .await
    .unwrap();
    let w = worker(app, "Wasim Khan").await;
    let w2 = worker(app, "Wajid Ali").await;
    let make = |employee: EmployeeId, status: DeploymentStatus| Deployment {
        employee_id: employee.0,
        client_contract_id: contract.id,
        client_party_id: client.0,
        site_id: site.id,
        role: "security_guard".into(),
        shift_type: "day".into(),
        start_date: today(),
        end_date: Some(renox::chrono::NaiveDate::from_ymd_opt(2027, 6, 30).unwrap()),
        status,
        ..Deployment::default()
    };
    let mut d = make(w, DeploymentStatus::Active);
    let mut d2 = make(w2, DeploymentStatus::Planned);
    let mut request = ManpowerRequest {
        client_contract_id: contract.id,
        client_party_id: client.0,
        site_id: site.id,
        role: "security_guard".into(),
        headcount: 1,
        shift_type: "day".into(),
        start_date: today(),
        end_date: Some(renox::chrono::NaiveDate::from_ymd_opt(2027, 6, 30).unwrap()),
        ..ManpowerRequest::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    d.insert_in_state(&mut tx, DeploymentStatus::Active, "fixture")
        .await
        .unwrap();
    d2.insert_in_state(&mut tx, DeploymentStatus::Planned, "fixture")
        .await
        .unwrap();
    request
        .insert_in_state(&mut tx, RequestStatus::Draft, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    Account {
        client,
        amal,
        contract,
        d,
        d2,
        request,
    }
}

/// `user` blocks the client for `fraud_suspected` (the compliance officer).
async fn block(app: &TestApp, client: PartyId) {
    let officer = person(app, "Comply", COMPLIANCE).await;
    app.acting_as(&officer);
    app.htmx()
        .post(
            &format!("/parties/{}/block", client.0),
            &[
                ("reason_code", "fraud_suspected"),
                ("note", "Forged cheques"),
            ],
        )
        .await
        .assert_ok();
}

async fn post(
    app: &TestApp,
    by: &User,
    path: &str,
    form: &[(&str, &str)],
) -> renox::testing::TestResponse {
    app.acting_as(by);
    app.htmx().post(path, form).await
}

/// The task about the client found by `key`.
async fn task(app: &TestApp, key: &str) -> Option<Ticket> {
    Ticket::query()
        .where_(Ticket::SOURCE_KEY.eq(key))
        .first(app.db())
        .await
        .unwrap()
}

/// AC1 and AC2: with client C blocked for `fraud_suspected`, the active D
/// continues (nothing changes state), and account manager A, who has no
/// `parties.block_reason.view`, is refused `activate` of the planned D2,
/// `extend` of D and `submit` of the request with *on hold* and no reason; the
/// contract `renew` is refused the same way; a holder of the permission is
/// told the real reason.
#[renox::test]
async fn a_blocked_client_keeps_its_workers_and_starts_nothing_new() {
    let app = boot().await;
    let a = account(&app).await;
    block(&app, a.client).await;

    // Nothing changed state.
    let d = Deployment::find_or_404(app.db(), a.d.id).await.unwrap();
    assert_eq!(d.status, DeploymentStatus::Active);
    let contract = ClientContract::find_or_404(app.db(), a.contract.id)
        .await
        .unwrap();
    assert_eq!(contract.status, ContractStatus::Active);

    let refusals = [
        post(
            &app,
            &a.amal,
            &format!("/manpower/deployments/{}/activate", a.d2.id),
            &[],
        )
        .await,
        post(
            &app,
            &a.amal,
            &format!("/manpower/deployments/{}/extend", a.d.id),
            &[("end_date", "2027-09-30")],
        )
        .await,
        post(
            &app,
            &a.amal,
            &format!("/manpower/requests/{}/submit", a.request.id),
            &[],
        )
        .await,
    ];
    for refused in &refusals {
        refused.assert_status(422);
        let body = refused.text();
        assert!(body.contains("This party is on hold"), "{body}");
        assert!(!body.to_lowercase().contains("fraud"), "{body}");
    }
    let d2 = Deployment::find_or_404(app.db(), a.d2.id).await.unwrap();
    assert_eq!(d2.status, DeploymentStatus::Planned);
    let d = Deployment::find_or_404(app.db(), a.d.id).await.unwrap();
    assert_eq!(d.end_date, a.d.end_date, "not extended");
    let request = ManpowerRequest::find_or_404(app.db(), a.request.id)
        .await
        .unwrap();
    assert_eq!(request.status, RequestStatus::Draft);

    // `renew` is BL-MPW-05's; it asks the same check.
    let renew = blocked::ensure(app.state(), Some(&a.amal), a.client, ActionKind::Renew).await;
    assert!(renew.is_err(), "renew is refused");

    // A holder of `parties.block_reason.view` sees why.
    let officer = scoped_person(&app, "Olga", ACCOUNT_MANAGER, &client_scope(a.client.0)).await;
    officer.assign_role(app.db(), COMPLIANCE).await.unwrap();
    let told = post(
        &app,
        &officer,
        &format!("/manpower/deployments/{}/activate", a.d2.id),
        &[],
    )
    .await;
    told.assert_status(422);
    let body = told.text();
    assert!(body.contains("Blocked:"), "{body}");
    assert!(!body.contains("This party is on hold"), "{body}");
}

/// AC3: the MD and the account manager get a task listing the client's
/// deployments; blocking again while they are open adds none; the client is
/// told nothing.
#[renox::test]
async fn the_md_and_the_account_manager_get_a_task_listing_the_deployments() {
    let app = boot().await;
    let _md = person(&app, "Rana", ADMIN).await;
    let a = account(&app).await;
    block(&app, a.client).await;

    let for_md = task(&app, &format!("manpower.client_blocked:{}:md", a.client.0))
        .await
        .expect("the MD's task");
    // Nobody holds the role `md`: the company admin gets it.
    let admins: Vec<i64> = renox::auth::permissions::users_with_role(app.db(), ADMIN)
        .await
        .unwrap()
        .iter()
        .map(|user| user.id)
        .collect();
    assert!(
        for_md
            .assignee_user_id
            .is_some_and(|id| admins.contains(&id)),
        "{:?} not in {admins:?}",
        for_md.assignee_user_id
    );
    let for_amal = task(
        &app,
        &format!("manpower.client_blocked:{}:user:{}", a.client.0, a.amal.id),
    )
    .await
    .expect("the account manager's task");
    assert_eq!(for_amal.assignee_user_id, Some(a.amal.id));
    for ticket in [&for_md, &for_amal] {
        assert!(
            ticket.description.contains("Wasim Khan"),
            "{}",
            ticket.description
        );
        assert!(
            ticket.description.contains("Wajid Ali"),
            "{}",
            ticket.description
        );
        assert!(
            ticket.description.contains("active"),
            "{}",
            ticket.description
        );
        assert!(
            ticket.description.contains("planned"),
            "{}",
            ticket.description
        );
        assert!(!ticket.description.to_lowercase().contains("fraud"));
    }
    // A second run while the tasks are open adds none.
    assert_eq!(blocked::open_tasks(app.state(), a.client).await.unwrap(), 0);
    let all = Ticket::query()
        .where_(Ticket::SOURCE_KEY.like("manpower.client_blocked:%"))
        .get(app.db())
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    // The client is not told.
    let to_client = wl_notify::Message::query()
        .where_(wl_notify::Message::TYPE_KEY.eq("manpower.client_notice"))
        .get(app.db())
        .await
        .unwrap();
    assert!(to_client.is_empty(), "{to_client:?}");
}

/// AC4: after the unblock, D2 can be activated; a client nobody has blocked
/// never met a refusal.
#[renox::test]
async fn after_the_unblock_the_planned_deployment_activates() {
    let app = boot().await;
    let a = account(&app).await;
    block(&app, a.client).await;
    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/activate", a.d2.id),
        &[],
    )
    .await
    .assert_status(422);

    let deputy = person(&app, "Dina", COMPLIANCE).await;
    app.acting_as(&deputy);
    app.htmx()
        .post_multipart(
            &format!("/parties/{}/unblock", a.client.0),
            &[("note", "Cleared by the lawyer")],
            &[],
        )
        .await
        .assert_ok();

    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/activate", a.d2.id),
        &[],
    )
    .await
    .assert_status(303);
    let d2 = Deployment::find_or_404(app.db(), a.d2.id).await.unwrap();
    assert_eq!(d2.status, DeploymentStatus::Active);
    post(
        &app,
        &a.amal,
        &format!("/manpower/deployments/{}/extend", a.d.id),
        &[("end_date", "2027-09-30")],
    )
    .await
    .assert_status(303);
    assert!(
        blocked::ensure(app.state(), Some(&a.amal), a.client, ActionKind::Renew)
            .await
            .is_ok()
    );
}

/// The block effect of a deployment is `continue`, decided by the MD.
#[renox::test]
async fn the_deployment_block_effect_is_continue_and_the_md_decides() {
    let app = boot().await;
    let _ = &app;
    let rule = wl_kernel::block_effects::effect_for(
        "deployment",
        &renox::auth::permissions::Scope::global(),
    );
    assert_eq!(rule.effect, wl_kernel::block_effects::BlockEffect::Continue);
    assert_eq!(rule.decider_role.as_deref(), Some("md"));
}
