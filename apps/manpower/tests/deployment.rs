//! Base 09 (09 Manpower §7.1 *Deployment*, §3.2): a worker is planned at a
//! client site, `activate`d when the employee is active in HR and the contract
//! takes deployments (taking a position of the request it fills), `extend`ed
//! within the contract, `end`ed for the reasons a user may give, or
//! `cancel`led while planned; the daily run ends the deployments whose end
//! date has passed. `suspend`, `resume` and the other end reasons come with
//! BL-MPW-11 to -14, the checks of `MPW.ACL.07`, `MPW.ACL.14` (the hold),
//! `MPW.ACL.15` with BL-MPW-08, -02 and -10.

mod support;

use manpower::{ContractStatus, Deployment, DeploymentStatus, RequestStatus};
use renox::prelude::*;
use wl_kernel::Subject;
use wl_kernel::workforce::EmployeeId;

use support::{FOODS, TOWER, boot, date};

/// The form that plans `employee` under `contract` at `site`.
fn plan_form(contract: i64, site: i64, employee: i64) -> Vec<(&'static str, String)> {
    vec![
        ("employee_id", employee.to_string()),
        ("client_contract_id", contract.to_string()),
        ("site_id", site.to_string()),
        ("role", "Guard".into()),
        ("shift_type", "Day".into()),
        ("shift_start", "07:00".into()),
        ("shift_end", "19:00".into()),
        ("start_date", "2027-02-01".into()),
        ("end_date", "2027-07-31".into()),
    ]
}

async fn post_form(
    t: &support::Booted,
    by: &User,
    path: &str,
    fields: &[(&'static str, String)],
) -> renox::testing::TestResponse {
    let borrowed: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
    t.post(by, path, &borrowed).await
}

async fn all_deployments(t: &support::Booted) -> Vec<Deployment> {
    Deployment::query()
        .order_by(Deployment::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// Base 09: the account manager plans a worker: `planned`, with the role and
/// the shift of the request, for an employee HR knows (onboarding is allowed,
/// the activation is what needs an active one), also while the contract is on
/// hold or terminating (`MPW.ACL.14`: planned deployments may still be made).
#[renox::test]
async fn a_worker_is_planned_at_a_site_of_the_client() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", false);
    for status in [
        ContractStatus::Active,
        ContractStatus::OnHold,
        ContractStatus::Terminating,
    ] {
        let contract = t.contract(TOWER, status, Some(&amal)).await;
        post_form(
            &t,
            &amal,
            "/manpower/deployments",
            &plan_form(contract.id, site.id, ravi.0),
        )
        .await
        .assert_status(303);
    }
    let planned = all_deployments(&t).await;
    assert_eq!(planned.len(), 3);
    assert!(
        planned
            .iter()
            .all(|d| d.status == DeploymentStatus::Planned)
    );
    assert_eq!(planned[0].role, "guard");
    assert_eq!(planned[0].shift_type, "day");
    assert_eq!(planned[0].employee_id, 7);
    assert_eq!(planned[0].client_party_id, TOWER.0);
    assert_eq!(planned[0].start_date, date(2027, 2, 1));
    assert_eq!(planned[0].end_date, Some(date(2027, 7, 31)));
    assert_eq!(planned[0].activated_on, None);
}

/// A deployment needs a known employee, a site of the client, dates within
/// the contract and a contract that is not a draft, over or cancelled.
#[renox::test]
async fn a_deployment_with_missing_or_wrong_details_is_not_planned() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let foods_site = t.site(FOODS, "Plant 1").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let path = "/manpower/deployments";
    let change = |name: &'static str, value: &str| {
        let mut fields = plan_form(contract.id, site.id, ravi.0);
        fields.retain(|(field, _)| *field != name);
        fields.push((name, value.to_owned()));
        fields
    };
    for fields in [
        change("employee_id", "99"),
        change("site_id", &foods_site.id.to_string()),
        change("role", " "),
        change("start_date", "2026-12-01"),
        change("end_date", "2028-01-31"),
        change("end_date", "2027-01-15"),
    ] {
        post_form(&t, &amal, path, &fields).await.assert_status(422);
    }
    for status in [
        ContractStatus::Draft,
        ContractStatus::Ended,
        ContractStatus::Cancelled,
    ] {
        let closed = t.contract(TOWER, status, Some(&amal)).await;
        post_form(&t, &amal, path, &plan_form(closed.id, site.id, ravi.0))
            .await
            .assert_status(422);
    }
    assert!(all_deployments(&t).await.is_empty());
}

/// Base 09: `activate` of a planned deployment of an active employee under an
/// active contract: the worker is on site, the day is kept, the client is told
/// by name and `DeploymentActivated` is emitted.
#[renox::test]
async fn a_planned_deployment_is_activated_and_the_client_is_told() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let planned = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    t.on(date(2027, 2, 1), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/activate", planned.id),
            &[],
        )
        .await
        .assert_status(303);
    })
    .await;
    let active = t.reload_deployment(planned.id).await;
    assert_eq!(active.status, DeploymentStatus::Active);
    assert_eq!(active.activated_on, Some(date(2027, 2, 1)));
    let events = t.deployed.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].employee, ravi);
    assert_eq!(events[0].client, TOWER);
    assert_eq!(events[0].site, Subject::new("manpower.site", site.id));
    assert_eq!(events[0].request, None);
    let told = t.notices.of_type("manpower.client_notice");
    assert_eq!(told.len(), 1);
    assert!(
        told[0].data["title"]
            .as_str()
            .unwrap()
            .contains("Ravi Kumar")
    );
    assert!(told[0].data["body"].as_str().unwrap().contains("guard"));
}

/// §7.1 `activate`, effect: the request takes a position, `fill_partly` while
/// positions are left and `fill` for the last, and a filled request takes no
/// more workers.
#[renox::test]
async fn activating_deployments_fills_the_request_position_by_position() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let request = t.request(&contract, &site, 2, RequestStatus::Open).await;
    let mut planned = Vec::new();
    for id in [7, 8] {
        let worker = t
            .staff
            .employ(&t.parties, id, &format!("Worker {id}"), true);
        let mut fields = plan_form(contract.id, site.id, worker.0);
        fields.push(("request_id", request.id.to_string()));
        post_form(&t, &amal, "/manpower/deployments", &fields)
            .await
            .assert_status(303);
        planned.push(all_deployments(&t).await.pop().unwrap());
    }
    t.on(date(2027, 2, 1), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/activate", planned[0].id),
            &[],
        )
        .await
        .assert_status(303);
        let half = t.reload_request(request.id).await;
        assert_eq!(half.status, RequestStatus::PartiallyFilled);
        assert_eq!(half.filled_count, 1);
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/activate", planned[1].id),
            &[],
        )
        .await
        .assert_status(303);
    })
    .await;
    let full = t.reload_request(request.id).await;
    assert_eq!(full.status, RequestStatus::Filled);
    assert_eq!(full.filled_count, 2);
    // A third worker for a filled request is not planned.
    let third = t.staff.employ(&t.parties, 9, "Worker 9", true);
    let mut fields = plan_form(contract.id, site.id, third.0);
    fields.push(("request_id", request.id.to_string()));
    post_form(&t, &amal, "/manpower/deployments", &fields)
        .await
        .assert_status(422);
    // A deployment that ends does not reopen the request.
    t.on(date(2027, 2, 10), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/end", planned[0].id),
            &[("reason", "replaced")],
        )
        .await
        .assert_status(303);
    })
    .await;
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Filled
    );
}

/// §7.1 `activate`, guards: an employee who is not active in HR, a contract
/// that does not take deployments (here `on_hold`: `MPW.ACL.14`), an end date
/// that has passed and a request that no longer takes workers each refuse it,
/// and every failing check is listed; nothing changes.
#[renox::test]
async fn activation_lists_every_failing_check_and_changes_nothing() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let onboarding = t.staff.employ(&t.parties, 7, "New Hire", false);
    let held = t.contract(TOWER, ContractStatus::OnHold, Some(&amal)).await;
    let request = t.request(&held, &site, 1, RequestStatus::Cancelled).await;
    let planned = t
        .deployment(
            &held,
            &site,
            onboarding,
            Some(date(2027, 1, 15)),
            DeploymentStatus::Planned,
        )
        .await;
    // The request it fills was cancelled after it was planned.
    t.fills(&planned, &request).await;
    t.on(date(2027, 2, 1), async {
        let response = t
            .post(
                &amal,
                &format!("/manpower/deployments/{}/activate", planned.id),
                &[],
            )
            .await;
        response.assert_status(422);
        let body = response.text();
        assert!(body.contains("not active in HR"), "{body}");
        assert!(body.contains("does not take deployments now"), "{body}");
        assert!(
            body.contains("end date of the deployment has passed"),
            "{body}"
        );
        assert!(body.contains("no longer takes workers"), "{body}");
    })
    .await;
    assert_eq!(
        t.reload_deployment(planned.id).await.status,
        DeploymentStatus::Planned
    );
    assert!(t.deployed.all().is_empty());
    assert!(t.notices.of_type("manpower.client_notice").is_empty());
}

/// A `terminating` contract still takes deployments; an active deployment is
/// not activated twice; an employee who went back to onboarding does not start.
#[renox::test]
async fn a_terminating_contract_takes_deployments_and_a_deployment_starts_once() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t
        .contract(TOWER, ContractStatus::Terminating, Some(&amal))
        .await;
    let planned = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    let path = format!("/manpower/deployments/{}/activate", planned.id);
    t.on(date(2027, 2, 1), async {
        t.post(&amal, &path, &[]).await.assert_status(303);
        t.post(&amal, &path, &[]).await.assert_status(422);
    })
    .await;
    assert_eq!(t.deployed.all().len(), 1);
    t.staff.set_active(7, false);
    let other = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Planned)
        .await;
    t.on(date(2027, 2, 1), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/activate", other.id),
            &[],
        )
        .await
        .assert_status(422);
    })
    .await;
}

/// Base 09: `extend` moves the end date of an active deployment later, within
/// the contract, while the contract takes deployments; the client is told.
#[renox::test]
async fn an_active_deployment_is_extended_within_the_contract() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let active = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let path = format!("/manpower/deployments/{}/extend", active.id);
    t.post(&amal, &path, &[("end_date", "2027-12-15")])
        .await
        .assert_status(303);
    let extended = t.reload_deployment(active.id).await;
    assert_eq!(extended.status, DeploymentStatus::Active);
    assert_eq!(extended.end_date, Some(date(2027, 12, 15)));
    let told = t.notices.of_type("manpower.client_notice");
    assert_eq!(told.len(), 1);
    assert!(
        told[0].data["body"]
            .as_str()
            .unwrap()
            .contains("2027-12-15")
    );

    // Not later, not past the contract (31 December 2027), not a planned one.
    for end in ["2027-12-15", "2027-10-01", "2028-01-15", "soon"] {
        t.post(&amal, &path, &[("end_date", end)])
            .await
            .assert_status(422);
    }
    assert_eq!(
        t.reload_deployment(active.id).await.end_date,
        Some(date(2027, 12, 15))
    );
    let planned = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/extend", planned.id),
        &[("end_date", "2027-09-01")],
    )
    .await
    .assert_status(422);
}

/// §7.1 `extend`: an open-ended deployment has nothing to extend, and a
/// contract on hold takes no extension (`MPW.ACL.14`).
#[renox::test]
async fn an_open_ended_deployment_or_a_held_contract_takes_no_extension() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let open = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Active)
        .await;
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/extend", open.id),
        &[("end_date", "2027-09-01")],
    )
    .await
    .assert_status(422);
    let held = t.contract(TOWER, ContractStatus::OnHold, Some(&amal)).await;
    let on_site = t
        .deployment(
            &held,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/extend", on_site.id),
        &[("end_date", "2027-09-01")],
    )
    .await
    .assert_status(422);
    assert_eq!(
        t.reload_deployment(on_site.id).await.end_date,
        Some(date(2027, 7, 31))
    );
}

/// Base 09: `end` with the reason *replaced* ends an active deployment today;
/// the reason, the day and the last day are kept, the client is told and
/// `DeploymentEnded` is emitted.
#[renox::test]
async fn an_active_deployment_is_ended_when_another_worker_takes_the_place() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let active = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    t.on(date(2027, 3, 10), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/end", active.id),
            &[("reason", "replaced")],
        )
        .await
        .assert_status(303);
    })
    .await;
    let ended = t.reload_deployment(active.id).await;
    assert_eq!(ended.status, DeploymentStatus::Ended);
    assert_eq!(ended.end_reason.as_deref(), Some("replaced"));
    assert_eq!(ended.ended_on, Some(date(2027, 3, 10)));
    assert_eq!(ended.end_date, Some(date(2027, 3, 10)));
    let events = t.ended.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].reason, "replaced");
    assert_eq!(events[0].last_day, date(2027, 3, 10));
    assert_eq!(events[0].employee, EmployeeId(7));
    assert_eq!(t.notices.of_type("manpower.client_notice").len(), 1);
}

/// §7.1 `end`: the reason *end date* needs the date to have come; a missing
/// reason or one that belongs to a later issue (client rejected, client
/// non-payment, entered in error, contract end, exit, absconded, deceased) is
/// refused; a planned deployment is cancelled, not ended.
#[renox::test]
async fn a_deployment_is_not_ended_without_a_reason_a_user_may_give() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let active = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let path = format!("/manpower/deployments/{}/end", active.id);
    t.on(date(2027, 3, 10), async {
        for reason in [
            "",
            "because",
            "end_date",
            "client_rejected",
            "client_non_payment",
            "entered_in_error",
            "contract_end",
            "exit",
            "absconded",
            "deceased",
        ] {
            t.post(&amal, &path, &[("reason", reason)])
                .await
                .assert_status(422);
        }
    })
    .await;
    assert_eq!(
        t.reload_deployment(active.id).await.status,
        DeploymentStatus::Active
    );
    // Once the end date has come it may be given.
    t.on(date(2027, 7, 31), async {
        t.post(&amal, &path, &[("reason", "end_date")])
            .await
            .assert_status(303);
    })
    .await;
    assert_eq!(
        t.reload_deployment(active.id).await.end_reason.as_deref(),
        Some("end_date")
    );
    let planned = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Planned)
        .await;
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/end", planned.id),
        &[("reason", "replaced")],
    )
    .await
    .assert_status(422);
}

/// Base 09: `cancel` of a planned deployment needs a reason and keeps it;
/// an active one is ended, not cancelled; `DeploymentCancelled` is emitted.
#[renox::test]
async fn a_planned_deployment_is_cancelled_with_a_reason() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let planned = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Planned)
        .await;
    let path = format!("/manpower/deployments/{}/cancel", planned.id);
    t.post(&amal, &path, &[("reason", " ")])
        .await
        .assert_status(422);
    t.post(&amal, &path, &[("reason", "worker found another job")])
        .await
        .assert_status(303);
    let cancelled = t.reload_deployment(planned.id).await;
    assert_eq!(cancelled.status, DeploymentStatus::Cancelled);
    assert_eq!(
        cancelled.cancel_reason.as_deref(),
        Some("worker found another job")
    );
    assert_eq!(t.cancelled.all().len(), 1);
    assert_eq!(t.cancelled.all()[0].reason, "worker found another job");
    let active = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Active)
        .await;
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/cancel", active.id),
        &[("reason", "mistake")],
    )
    .await
    .assert_status(422);
}

/// §7.1 *Manpower request* `cancel`, effect: the planned deployments of the
/// request are cancelled with it by the system; a deployment already active
/// stays.
#[renox::test]
async fn cancelling_a_request_cancels_its_planned_deployments() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let request = t
        .request(&contract, &site, 3, RequestStatus::PartiallyFilled)
        .await;
    let mut fixtures = Vec::new();
    for (id, status) in [
        (7, DeploymentStatus::Active),
        (8, DeploymentStatus::Planned),
        (9, DeploymentStatus::Planned),
    ] {
        let worker = t
            .staff
            .employ(&t.parties, id, &format!("Worker {id}"), true);
        let deployment = t.deployment(&contract, &site, worker, None, status).await;
        t.fills(&deployment, &request).await;
        fixtures.push(deployment);
    }
    t.post(
        &amal,
        &format!("/manpower/requests/{}/cancel", request.id),
        &[("reason", "client changed plan")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Cancelled
    );
    let after: Vec<_> = {
        let mut rows = Vec::new();
        for d in &fixtures {
            rows.push(t.reload_deployment(d.id).await);
        }
        rows
    };
    assert_eq!(after[0].status, DeploymentStatus::Active);
    assert_eq!(after[1].status, DeploymentStatus::Cancelled);
    assert_eq!(after[2].status, DeploymentStatus::Cancelled);
    assert_eq!(after[1].cancel_reason.as_deref(), Some("request cancelled"));
    assert_eq!(t.cancelled.all().len(), 2);
}

/// The daily run at 02:00 Asia/Dubai ends, by the system with the reason *end
/// date*, the deployments whose end date has passed; those running on or
/// open-ended stay; a second run ends nothing and tells nobody twice.
#[renox::test]
async fn the_daily_run_ends_the_deployments_whose_end_date_has_passed() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let mut ids = Vec::new();
    for (id, end, status) in [
        (7, Some(date(2027, 3, 31)), DeploymentStatus::Active),
        (8, Some(date(2027, 3, 31)), DeploymentStatus::Suspended),
        (9, Some(date(2027, 4, 1)), DeploymentStatus::Active),
        (10, None, DeploymentStatus::Active),
        (11, Some(date(2027, 3, 1)), DeploymentStatus::Planned),
    ] {
        let worker = t
            .staff
            .employ(&t.parties, id, &format!("Worker {id}"), true);
        ids.push(t.deployment(&contract, &site, worker, end, status).await.id);
    }
    // On 31 March nothing has passed yet.
    t.on(date(2027, 3, 31), async {
        t.app.run_scheduled(manpower::END_DATE_TASK).await.unwrap();
    })
    .await;
    assert!(t.ended.all().is_empty());
    for _ in 0..2 {
        t.on(date(2027, 4, 1), async {
            t.app.run_scheduled(manpower::END_DATE_TASK).await.unwrap();
        })
        .await;
    }
    let status = |id: i64| {
        let t = &t;
        async move { t.reload_deployment(id).await.status }
    };
    assert_eq!(status(ids[0]).await, DeploymentStatus::Ended);
    assert_eq!(status(ids[1]).await, DeploymentStatus::Ended);
    assert_eq!(status(ids[2]).await, DeploymentStatus::Active);
    assert_eq!(status(ids[3]).await, DeploymentStatus::Active);
    assert_eq!(status(ids[4]).await, DeploymentStatus::Planned);
    let first = t.reload_deployment(ids[0]).await;
    assert_eq!(first.end_reason.as_deref(), Some("end_date"));
    assert_eq!(first.ended_on, Some(date(2027, 4, 1)));
    assert_eq!(first.end_date, Some(date(2027, 3, 31)));
    assert_eq!(
        t.ended.all().len(),
        2,
        "one event per deployment, however many runs"
    );
    assert_eq!(t.notices.of_type("manpower.client_notice").len(), 2);
}

/// §3.2: a deployment is managed by the account manager of its client company
/// only; another client's account manager, a site supervisor and the MD with
/// only a view cannot plan, activate, extend, end or cancel.
#[renox::test]
async fn only_the_account_manager_of_the_client_manages_its_deployments() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let faris = t.account_manager("Faris", FOODS).await;
    let sami = t
        .user_with("Sami", manpower::permissions::SUPERVISOR_ROLE)
        .await;
    let md = t.md("Mona").await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let planned = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Planned,
        )
        .await;
    let active = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    for user in [&faris, &sami, &md] {
        post_form(
            &t,
            user,
            "/manpower/deployments",
            &plan_form(contract.id, site.id, ravi.0),
        )
        .await
        .assert_forbidden();
        t.post(
            user,
            &format!("/manpower/deployments/{}/activate", planned.id),
            &[],
        )
        .await
        .assert_forbidden();
        t.post(
            user,
            &format!("/manpower/deployments/{}/cancel", planned.id),
            &[("reason", "no")],
        )
        .await
        .assert_forbidden();
        t.post(
            user,
            &format!("/manpower/deployments/{}/extend", active.id),
            &[("end_date", "2027-09-01")],
        )
        .await
        .assert_forbidden();
        t.post(
            user,
            &format!("/manpower/deployments/{}/end", active.id),
            &[("reason", "replaced")],
        )
        .await
        .assert_forbidden();
    }
    assert_eq!(all_deployments(&t).await.len(), 2);
    assert_eq!(
        t.reload_deployment(planned.id).await.status,
        DeploymentStatus::Planned
    );
}
