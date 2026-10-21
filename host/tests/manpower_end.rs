//! BL-MPW-06 (#420), `end` of a client contract, with the real wl-attendance
//! and wl-hr in the host of a manpower agency: the 00:05 run after the end
//! date ends the contract, the open timesheet of each worker is closed at the
//! end date and submitted at once (a week that ran past it is cut), and the
//! workers `active` in HR are on the bench while one in `notice` is not.
//!
//! The billing of the approved last timesheet by `MPW.RUL.05` is in
//! `manpower_billing.rs`.

mod support;

use host::roles::{ACCOUNT_MANAGER, ADMIN};
use manpower::bench::is_on_bench;
use manpower::permissions::client_scope;
use manpower::{ClientContract, ContractStatus, Deployment, DeploymentStatus, Site};
use renox::chrono::Days;
use renox::prelude::*;
use renox::state_machine::StateMachine;
use renox::testing::TestApp;
use support::{company, noon_clock, person, scoped_person, tenant};
use wl_attendance::timesheet::{ApprovalBasis, ApproverKind};
use wl_attendance::{Timesheet, TimesheetStatus};
use wl_hr::{EmployeeStatus, NewEmployee, employees};
use wl_kernel::ClientConfig;
use wl_kernel::documents::today;
use wl_kernel::workforce::EmployeeId;

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

/// An employee of HR in `status`.
async fn employee(app: &TestApp, name: &str, status: EmployeeStatus) -> EmployeeId {
    let party = tenant(app, name).await;
    let created = employees::create(
        app.state(),
        NewEmployee::new(party, "security_guard", today()).status(EmployeeStatus::Active),
    )
    .await
    .unwrap();
    if status == EmployeeStatus::Notice {
        // A fixture: HR starts a new employee in onboarding or active only.
        wl_hr::Employee::query()
            .where_(wl_hr::Employee::ID.eq(created.id))
            .update(app.db(), &[("status", &"notice")])
            .await
            .unwrap();
    }
    EmployeeId(created.id)
}

/// AC2 and AC3: contract K ended yesterday; D1 (worker `active`) has an open
/// timesheet for the week around the end date, D2 (worker in `notice`) has one
/// that is already submitted. At the run: K is `ended`, D1 and D2 are `ended`
/// for *contract end* with yesterday as the last day; D1's timesheet is cut to
/// yesterday and `submitted`, D2's is untouched; D1's worker is on the bench,
/// D2's is not.
#[renox::test]
async fn the_run_after_the_end_date_closes_the_last_timesheets_and_sends_the_workers_to_the_bench()
{
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    let _md = person(&app, "Rana", ADMIN).await;
    let client = company(&app, "Gulf Tower LLC").await;
    let amal = scoped_person(&app, "Amal", ACCOUNT_MANAGER, &client_scope(client.0)).await;
    let last_day = today().checked_sub_days(Days::new(1)).unwrap();
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: client.0,
        start_date: today().checked_sub_days(Days::new(200)).unwrap(),
        end_date: last_day,
        payment_terms_days: 30,
        notice_period_days: 30,
        account_manager_user_id: Some(amal.id),
        status: ContractStatus::Terminating,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Terminating, "fixture")
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
    let w1 = employee(&app, "Wasim Khan", EmployeeStatus::Active).await;
    let w2 = employee(&app, "Wajid Ali", EmployeeStatus::Notice).await;
    let mut deployments = Vec::new();
    for worker in [w1, w2] {
        let mut d = Deployment {
            employee_id: worker.0,
            client_contract_id: contract.id,
            client_party_id: client.0,
            site_id: site.id,
            role: "security_guard".into(),
            shift_type: "day".into(),
            start_date: today().checked_sub_days(Days::new(100)).unwrap(),
            status: DeploymentStatus::Active,
            ..Deployment::default()
        };
        let mut tx = app.db().begin().await.unwrap();
        d.insert_in_state(&mut tx, DeploymentStatus::Active, "fixture")
            .await
            .unwrap();
        tx.commit().await.unwrap();
        deployments.push(d);
    }
    // D1's week ran past the end date; D2's was submitted already.
    let week_start = last_day.checked_sub_days(Days::new(6)).unwrap();
    let week_end = today().checked_add_days(Days::new(2)).unwrap();
    let open = Timesheet::create(
        app.db(),
        Timesheet::regular(w1, week_start, week_end).in_context(&deployments[0].subject()),
    )
    .await
    .unwrap();
    let mut sent =
        Timesheet::regular(w2, week_start, last_day).in_context(&deployments[1].subject());
    sent.status = TimesheetStatus::Submitted;
    let mut tx = app.db().begin().await.unwrap();
    sent.insert_in_state(&mut tx, TimesheetStatus::Submitted, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    app.run_scheduled(manpower::CONTRACT_END_TASK)
        .await
        .unwrap();

    let ended = ClientContract::find_or_404(app.db(), contract.id)
        .await
        .unwrap();
    assert_eq!(ended.status, ContractStatus::Ended);
    for d in &deployments {
        let d = Deployment::find_or_404(app.db(), d.id).await.unwrap();
        assert_eq!(d.status, DeploymentStatus::Ended);
        assert_eq!(d.end_reason.as_deref(), Some("contract_end"));
        assert_eq!(d.end_date, Some(last_day));
    }
    let closed = Timesheet::find_or_404(app.db(), open.id).await.unwrap();
    assert_eq!(
        closed.status,
        TimesheetStatus::Submitted,
        "submitted at once"
    );
    assert_eq!(closed.period_to, last_day, "cut to the end date");
    assert!(closed.submitted_at.is_some());
    let untouched = Timesheet::find_or_404(app.db(), sent.id).await.unwrap();
    assert_eq!(untouched.status, TimesheetStatus::Submitted);
    assert_eq!(untouched.period_to, last_day);

    assert!(is_on_bench(app.state(), w1).await.unwrap(), "active in HR");
    assert!(!is_on_bench(app.state(), w2).await.unwrap(), "in notice");
}

/// AC2, approval half: the client approver approves D1's closed and submitted
/// timesheet after the contract is `ended` (and the deployment with it): the
/// sheet is `approved` by them, the contract and the deployment stay `ended`.
/// The billing of those hours is tested in `manpower_billing.rs`.
#[renox::test]
async fn the_client_approves_the_closed_timesheet_after_the_contract_has_ended() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap().provide(noon_clock())).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    renox::auth::permissions::define_role(app.db(), "client_approver", &[])
        .await
        .unwrap();
    let carl = person(&app, "Carl", "client_approver").await;
    let client = company(&app, "Gulf Tower LLC").await;
    let last_day = today().checked_sub_days(Days::new(1)).unwrap();
    let mut contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: client.0,
        start_date: today().checked_sub_days(Days::new(200)).unwrap(),
        end_date: last_day,
        payment_terms_days: 30,
        notice_period_days: 30,
        status: ContractStatus::Terminating,
        ..ClientContract::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    contract
        .insert_in_state(&mut tx, ContractStatus::Terminating, "fixture")
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
    let worker = employee(&app, "Wasim Khan", EmployeeStatus::Active).await;
    let mut deployment = Deployment {
        employee_id: worker.0,
        client_contract_id: contract.id,
        client_party_id: client.0,
        site_id: site.id,
        role: "security_guard".into(),
        shift_type: "day".into(),
        start_date: today().checked_sub_days(Days::new(100)).unwrap(),
        status: DeploymentStatus::Active,
        ..Deployment::default()
    };
    let mut tx = app.db().begin().await.unwrap();
    deployment
        .insert_in_state(&mut tx, DeploymentStatus::Active, "fixture")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut new = Timesheet::regular(
        worker,
        last_day.checked_sub_days(Days::new(6)).unwrap(),
        today().checked_add_days(Days::new(2)).unwrap(),
    )
    .in_context(&deployment.subject());
    new.approver_kind = ApproverKind::External;
    new.approver_role = Some("client_approver".into());
    let open = Timesheet::create(app.db(), new).await.unwrap();

    app.run_scheduled(manpower::CONTRACT_END_TASK)
        .await
        .unwrap();
    assert_eq!(
        ClientContract::find_or_404(app.db(), contract.id)
            .await
            .unwrap()
            .status,
        ContractStatus::Ended
    );
    assert_eq!(
        Timesheet::find_or_404(app.db(), open.id)
            .await
            .unwrap()
            .status,
        TimesheetStatus::Submitted
    );

    app.acting_as(&carl);
    app.htmx()
        .post(&format!("/timesheets/{}/approve", open.id), &[])
        .await
        .assert_ok();

    let approved = Timesheet::find_or_404(app.db(), open.id).await.unwrap();
    assert_eq!(approved.status, TimesheetStatus::Approved);
    assert_eq!(approved.approval_basis, Some(ApprovalBasis::Approved));
    assert_eq!(approved.approved_by, Some(carl.id));
    assert_eq!(
        approved.period_to, last_day,
        "the hours stop at the end date"
    );
    assert_eq!(
        ClientContract::find_or_404(app.db(), contract.id)
            .await
            .unwrap()
            .status,
        ContractStatus::Ended
    );
    assert_eq!(
        Deployment::find_or_404(app.db(), deployment.id)
            .await
            .unwrap()
            .status,
        DeploymentStatus::Ended
    );
}
