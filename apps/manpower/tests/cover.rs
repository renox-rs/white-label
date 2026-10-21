//! Base 09 (09 Manpower §4.1, §4.6, §7.1; `MPW.DEP.02`, `MPW.RUL.06`): a
//! replacement is a request of the kind `replacement`, opened at once for the
//! days a worker is away. An approved leave opens it when the client
//! contract requires cover (`MPW.RUL.06`); the account manager opens one by
//! hand; a worker planned for it is a cover deployment of the worker covered,
//! runs to the end of the cover period and fills the request.

mod support;

use manpower::cover::{Cover, CoverAsk, causes, open_cover};
use manpower::{
    ContractStatus, DeploymentKind, DeploymentStatus, ManpowerRequest, RequestKind, RequestStatus,
};
use renox::prelude::*;
use wl_kernel::workforce::EmployeeId;
use wl_kernel::{LeaveApproved, Subject};

use support::{FOODS, TOWER, boot, date};

fn leave(
    employee: i64,
    from: renox::chrono::NaiveDate,
    to: renox::chrono::NaiveDate,
) -> LeaveApproved {
    LeaveApproved {
        leave: Subject::new("hr.leave_request", 5),
        employee: EmployeeId(employee),
        from,
        to,
        unpaid: false,
    }
}

async fn requests(t: &support::Booted) -> Vec<ManpowerRequest> {
    ManpowerRequest::query()
        .order_by(ManpowerRequest::ID)
        .get(t.db())
        .await
        .unwrap()
}

/// `MPW.RUL.06`: an approved leave of a worker on site, under a contract that
/// requires cover, opens a replacement request for the leave period: one
/// worker, the role, site and shift of the deployment, `open` at once; the
/// account manager is told and `CoverRequested` is emitted.
#[renox::test]
async fn an_approved_leave_opens_a_cover_request_when_the_contract_requires_it() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let mut contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    contract.cover_required = true;
    contract.save(t.db()).await.unwrap();
    let covered = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    t.on(date(2027, 2, 20), async {
        t.state()
            .emit(leave(7, date(2027, 3, 1), date(2027, 3, 10)))
            .await
            .unwrap();
    })
    .await;
    let made = requests(&t).await;
    assert_eq!(made.len(), 1);
    let request = &made[0];
    assert_eq!(request.kind, RequestKind::Replacement);
    assert_eq!(request.status, RequestStatus::Open);
    assert_eq!(request.headcount, 1);
    assert_eq!(request.role, "guard");
    assert_eq!(request.site_id, site.id);
    assert_eq!(request.shift_type, "day");
    assert_eq!(request.shift_start.as_deref(), Some("07:00"));
    assert_eq!(request.start_date, date(2027, 3, 1));
    assert_eq!(request.end_date, Some(date(2027, 3, 10)));
    assert_eq!(request.cause.as_deref(), Some("leave"));
    assert_eq!(request.replaces_deployment_id, Some(covered.id));
    assert_eq!(request.client_party_id, TOWER.0);
    let events = t.covers.all();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].deployment, covered.subject());
    assert_eq!(events[0].cause, "leave");
    let told = t.notices.of_type("manpower.notice");
    assert_eq!(told.len(), 1);
    assert_eq!(support::users_of(&told[0]), [amal.id]);
    assert!(
        told[0].data["title"]
            .as_str()
            .unwrap()
            .contains("Ravi Kumar")
    );
}

/// `MPW.RUL.06`: a contract that does not require cover gets no request; a
/// repeated event changes nothing and tells nobody twice.
#[renox::test]
async fn the_rule_needs_the_cover_obligation_and_runs_once_per_leave() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let bare = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.deployment(&bare, &site, ravi, None, DeploymentStatus::Active)
        .await;
    t.on(date(2027, 2, 20), async {
        t.state()
            .emit(leave(7, date(2027, 3, 1), date(2027, 3, 10)))
            .await
            .unwrap();
    })
    .await;
    assert!(requests(&t).await.is_empty());
    assert!(t.notices.all().is_empty());

    let mut covered = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    covered.cover_required = true;
    covered.save(t.db()).await.unwrap();
    let other = t.staff.employ(&t.parties, 8, "Sana Ali", true);
    t.deployment(&covered, &site, other, None, DeploymentStatus::Active)
        .await;
    let event = LeaveApproved {
        employee: other,
        ..leave(8, date(2027, 3, 1), date(2027, 3, 10))
    };
    t.on(date(2027, 2, 20), async {
        t.state().emit(event.clone()).await.unwrap();
        t.state().emit(event.clone()).await.unwrap();
    })
    .await;
    assert_eq!(requests(&t).await.len(), 1);
    assert_eq!(t.notices.of_type("manpower.notice").len(), 1);
    assert_eq!(t.covers.all().len(), 1);
}

/// `MPW.RUL.06`: the cover is for the days of the overlap between the leave
/// and the deployment, each deployment on site gets its own, and a deployment
/// that is not on site (planned, ended) or does not overlap gets none.
#[renox::test]
async fn the_cover_is_for_the_days_the_leave_and_the_deployment_overlap() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let mut tower = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    tower.cover_required = true;
    tower.save(t.db()).await.unwrap();
    let mut foods = t.contract(FOODS, ContractStatus::Active, None).await;
    foods.cover_required = true;
    foods.save(t.db()).await.unwrap();
    let foods_site = t.site(FOODS, "Plant 1").await;
    // Ends in the middle of the leave.
    t.deployment(
        &tower,
        &site,
        ravi,
        Some(date(2027, 3, 5)),
        DeploymentStatus::Active,
    )
    .await;
    // Another client, open-ended.
    t.deployment(&foods, &foods_site, ravi, None, DeploymentStatus::Active)
        .await;
    // Not on site, or past.
    t.deployment(&tower, &site, ravi, None, DeploymentStatus::Planned)
        .await;
    t.deployment(
        &tower,
        &site,
        ravi,
        Some(date(2027, 2, 10)),
        DeploymentStatus::Ended,
    )
    .await;
    t.on(date(2027, 2, 20), async {
        t.state()
            .emit(leave(7, date(2027, 3, 1), date(2027, 3, 10)))
            .await
            .unwrap();
    })
    .await;
    let made = requests(&t).await;
    assert_eq!(made.len(), 2);
    assert_eq!(made[0].client_party_id, TOWER.0);
    assert_eq!(made[0].start_date, date(2027, 3, 1));
    assert_eq!(made[0].end_date, Some(date(2027, 3, 5)));
    assert_eq!(made[1].client_party_id, FOODS.0);
    assert_eq!(made[1].end_date, Some(date(2027, 3, 10)));

    // A leave after the deployment ended, and one already over, ask for none.
    t.on(date(2027, 3, 20), async {
        t.state()
            .emit(LeaveApproved {
                leave: Subject::new("hr.leave_request", 6),
                ..leave(7, date(2027, 4, 1), date(2027, 4, 3))
            })
            .await
            .unwrap();
    })
    .await;
    // The open-ended one at Foods is still on site, so April is covered there.
    assert_eq!(requests(&t).await.len(), 3);
    t.on(date(2027, 4, 20), async {
        t.state()
            .emit(LeaveApproved {
                leave: Subject::new("hr.leave_request", 7),
                ..leave(7, date(2027, 4, 5), date(2027, 4, 8))
            })
            .await
            .unwrap();
    })
    .await;
    assert_eq!(
        requests(&t).await.len(),
        3,
        "a leave that is over needs no cover"
    );
}

/// `MPW.DEP.02`: a worker planned for the cover request is a cover of the
/// worker covered, runs to the end of the cover period by default, fills the
/// request when activated, and ends on the daily run after that day, while the
/// worker covered stays.
#[renox::test]
async fn a_worker_planned_for_the_cover_request_covers_until_the_cover_ends() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let sana = t.staff.employ(&t.parties, 8, "Sana Ali", true);
    let mut contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    contract.cover_required = true;
    contract.save(t.db()).await.unwrap();
    let covered = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Active)
        .await;
    t.on(date(2027, 2, 20), async {
        t.state()
            .emit(leave(7, date(2027, 3, 1), date(2027, 3, 10)))
            .await
            .unwrap();
    })
    .await;
    let request = requests(&t).await.pop().unwrap();
    t.post(
        &amal,
        "/manpower/deployments",
        &[
            ("employee_id", &sana.0.to_string()),
            ("client_contract_id", &contract.id.to_string()),
            ("site_id", &site.id.to_string()),
            ("request_id", &request.id.to_string()),
            ("role", "guard"),
            ("shift_type", "day"),
            ("start_date", "2027-03-01"),
        ],
    )
    .await
    .assert_status(303);
    let cover = manpower::Deployment::query()
        .where_(manpower::Deployment::EMPLOYEE_ID.eq(8))
        .first(t.db())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(cover.kind, DeploymentKind::Cover);
    assert_eq!(cover.covers_deployment_id, Some(covered.id));
    assert_eq!(cover.end_date, Some(date(2027, 3, 10)));
    t.on(date(2027, 3, 1), async {
        t.post(
            &amal,
            &format!("/manpower/deployments/{}/activate", cover.id),
            &[],
        )
        .await
        .assert_status(303);
    })
    .await;
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Filled
    );
    t.on(date(2027, 3, 11), async {
        t.app.run_scheduled(manpower::END_DATE_TASK).await.unwrap();
    })
    .await;
    let after = t.reload_deployment(cover.id).await;
    assert_eq!(after.status, DeploymentStatus::Ended);
    assert_eq!(after.end_reason.as_deref(), Some("end_date"));
    assert_eq!(
        t.reload_deployment(covered.id).await.status,
        DeploymentStatus::Active
    );
}

/// `MPW.DEP.02`: the cover request of the other causes (a suspension, a client
/// rejection: BL-MPW-11, BL-MPW-14 call it): open-ended when no last day is
/// known, one per cause, nothing for a contract that is a draft or over.
#[renox::test]
async fn the_cover_function_serves_the_other_causes_once_each() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let mut contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    contract.cover_required = true;
    contract.save(t.db()).await.unwrap();
    let deployment = t
        .deployment(
            &contract,
            &site,
            ravi,
            Some(date(2027, 7, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let ask = |cause: &'static str, to| CoverAsk {
        deployment: &deployment,
        from: date(2027, 3, 1),
        to,
        cause,
        cause_ref: Some(format!("{cause}:{}", deployment.id)),
    };
    let first = open_cover(t.state(), ask(causes::SUSPENSION, None))
        .await
        .unwrap();
    let Cover::Opened(request) = first else {
        panic!("a request is opened: {first:?}")
    };
    assert_eq!(request.end_date, None);
    assert_eq!(request.cause.as_deref(), Some("suspension"));
    let again = open_cover(t.state(), ask(causes::SUSPENSION, None))
        .await
        .unwrap();
    assert!(matches!(again, Cover::Existing(ref found) if found.id == request.id));
    let other = open_cover(t.state(), ask(causes::REJECTION, Some(date(2027, 7, 31))))
        .await
        .unwrap();
    assert!(matches!(other, Cover::Opened(_)));
    assert_eq!(requests(&t).await.len(), 2);

    for status in [
        ContractStatus::Draft,
        ContractStatus::Ended,
        ContractStatus::Cancelled,
    ] {
        let mut closed = t.contract(TOWER, status, Some(&amal)).await;
        closed.cover_required = true;
        closed.save(t.db()).await.unwrap();
        let closed_deployment = t
            .deployment(&closed, &site, ravi, None, DeploymentStatus::Active)
            .await;
        let none = open_cover(
            t.state(),
            CoverAsk {
                deployment: &closed_deployment,
                from: date(2027, 3, 1),
                to: None,
                cause: causes::SUSPENSION,
                cause_ref: Some(format!("closed:{status:?}")),
            },
        )
        .await
        .unwrap();
        assert!(matches!(none, Cover::NotRequired), "{status:?}");
    }
    assert_eq!(requests(&t).await.len(), 2);
}

/// §4.1: the account manager may open a cover request by hand, also when the
/// contract does not require cover; only for a worker on site, with dates in
/// order, and only the account manager of the client.
#[renox::test]
async fn the_account_manager_opens_a_cover_request_by_hand() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let faris = t.account_manager("Faris", FOODS).await;
    let site = t.site(TOWER, "Tower A").await;
    let ravi = t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    assert!(!contract.cover_required);
    let active = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Active)
        .await;
    let planned = t
        .deployment(&contract, &site, ravi, None, DeploymentStatus::Planned)
        .await;
    let path = |id: i64| format!("/manpower/deployments/{id}/cover");
    t.post(&faris, &path(active.id), &[("from", "2027-03-01")])
        .await
        .assert_forbidden();
    t.post(&amal, &path(planned.id), &[("from", "2027-03-01")])
        .await
        .assert_status(422);
    t.post(
        &amal,
        &path(active.id),
        &[("from", "2027-03-10"), ("to", "2027-03-01")],
    )
    .await
    .assert_status(422);
    t.post(&amal, &path(active.id), &[("from", "tomorrow")])
        .await
        .assert_status(422);
    assert!(requests(&t).await.is_empty());
    t.post(
        &amal,
        &path(active.id),
        &[("from", "2027-03-01"), ("to", "2027-03-05")],
    )
    .await
    .assert_status(303);
    let made = requests(&t).await;
    assert_eq!(made.len(), 1);
    assert_eq!(made[0].cause.as_deref(), Some("manual"));
    assert_eq!(made[0].status, RequestStatus::Open);
    assert_eq!(made[0].replaces_deployment_id, Some(active.id));
    // A second request by hand for other days is a second request.
    t.post(&amal, &path(active.id), &[("from", "2027-04-01")])
        .await
        .assert_status(303);
    assert_eq!(requests(&t).await.len(), 2);
    let rows: Vec<renox::serde_json::Value> = t.get(&amal, "/manpower/requests").await.json();
    assert_eq!(rows[0]["replacement"], true);
    assert_eq!(rows[0]["cause"], "manual");
}
