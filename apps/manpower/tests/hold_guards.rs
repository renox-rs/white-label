//! BL-MPW-02 (#416): while a client contract is `on_hold` no request is
//! submitted and no deployment is activated or extended (`MPW.ACL.14`); drafts
//! and planned deployments can still be made, workers on site stay, and the
//! client approver in the portal is told neither the hold nor its reason.
//! (`resume` of a suspended deployment is not refused by the hold: tested with
//! the transition in `resume.rs`, BL-MPW-12.)

mod support;

use manpower::{CREDIT_HOLD_TASK, ClientContract, ContractStatus, DeploymentStatus, RequestStatus};
use renox::chrono::NaiveDate;
use renox::prelude::*;
use support::{Booted, TOWER, boot, date};

/// A contract of Tower on hold since `day` for INV-1, through the rule.
async fn held_for_invoice(t: &Booted, day: NaiveDate) -> ClientContract {
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.cards
        .accept(&contract.subject(), date(2026, 1, 1), &[("guard", "day")]);
    t.invoices
        .issue(&contract, "INV-1", 4_200_000, date(2026, 10, 1));
    t.on(day, async {
        t.app.run_scheduled(CREDIT_HOLD_TASK).await.unwrap();
    })
    .await;
    let held = t.reload_contract(contract.id).await;
    assert_eq!(held.status, ContractStatus::OnHold);
    held
}

/// AC1: the client approver submits request R in the portal while K is
/// `on_hold`: refused with the neutral message, no word of the hold or its
/// reason, and R stays `draft`.
#[renox::test]
async fn the_portal_is_refused_without_a_word_of_the_hold() {
    let t = boot().await;
    let contract = held_for_invoice(&t, date(2026, 11, 5)).await;
    let site = t.site(TOWER, "Tower A").await;
    let request = t.request(&contract, &site, 1, RequestStatus::Draft).await;
    let approver = t.client_user("Cleo", TOWER, &["approver"]).await;

    let response = t
        .post(
            &approver,
            &format!("/manpower/requests/{}/submit", request.id),
            &[],
        )
        .await;
    response.assert_status(422);
    let body = response.text();
    assert!(
        body.contains("This request cannot be submitted now. Please contact your account manager."),
        "{body}"
    );
    let lower = body.to_lowercase();
    for word in ["hold", "overdue", "inv-1", "since"] {
        assert!(!lower.contains(word), "{word} in {body}");
    }
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Draft
    );
    assert!(t.notices.of_type("manpower.notice").iter().all(|n| {
        !n.data["title"]
            .as_str()
            .unwrap_or_default()
            .contains("New request")
    }));
}

/// Staff are told the hold, its date and its reason when they submit.
#[renox::test]
async fn staff_are_told_the_hold_when_they_submit() {
    let t = boot().await;
    let contract = held_for_invoice(&t, date(2026, 11, 5)).await;
    let site = t.site(TOWER, "Tower A").await;
    let request = t.request(&contract, &site, 1, RequestStatus::Draft).await;
    let amal = t.account_manager("Anna", TOWER).await;

    let response = t
        .post(
            &amal,
            &format!("/manpower/requests/{}/submit", request.id),
            &[],
        )
        .await;
    response.assert_status(422);
    let body = response.text();
    assert!(
        body.contains("Contract on credit hold since 2026-11-05: overdue (INV-1)"),
        "{body}"
    );
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Draft
    );
}

/// AC2: staff user A's `activate` of planned deployment D is refused with the
/// hold reason and date; D stays `planned`, no request position is taken and
/// the client is told nothing.
#[renox::test]
async fn staff_cannot_activate_a_deployment_while_the_contract_is_held() {
    let t = boot().await;
    let contract = held_for_invoice(&t, date(2026, 11, 5)).await;
    let site = t.site(TOWER, "Tower A").await;
    let worker = t.staff.employ(&t.parties, 1, "Ravi Kumar", true);
    let planned = t
        .deployment(
            &contract,
            &site,
            worker,
            Some(date(2027, 6, 30)),
            DeploymentStatus::Planned,
        )
        .await;
    let request = t.request(&contract, &site, 1, RequestStatus::Open).await;
    t.fills(&planned, &request).await;
    let amal = t.account_manager("Anna", TOWER).await;

    t.on(date(2026, 11, 5), async {
        let response = t
            .post(
                &amal,
                &format!("/manpower/deployments/{}/activate", planned.id),
                &[],
            )
            .await;
        response.assert_status(422);
        let body = response.text();
        assert!(
            body.contains("Contract on credit hold since 2026-11-05: overdue (INV-1)"),
            "{body}"
        );
    })
    .await;
    assert_eq!(
        t.reload_deployment(planned.id).await.status,
        DeploymentStatus::Planned
    );
    assert_eq!(t.reload_request(request.id).await.filled_count, 0);
    assert!(t.deployed.all().is_empty());
    assert!(t.notices.of_type("manpower.client_notice").is_empty());
}

/// AC3: with K `on_hold` and active deployment D2 ending 2027-03-31, `extend`
/// to 2027-04-30 is refused and the end date stays.
#[renox::test]
async fn staff_cannot_extend_a_deployment_while_the_contract_is_held() {
    let t = boot().await;
    let contract = held_for_invoice(&t, date(2026, 11, 5)).await;
    let site = t.site(TOWER, "Tower A").await;
    let worker = t.staff.employ(&t.parties, 2, "Sunil Das", true);
    let active = t
        .deployment(
            &contract,
            &site,
            worker,
            Some(date(2027, 3, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let amal = t.account_manager("Anna", TOWER).await;

    let response = t
        .post(
            &amal,
            &format!("/manpower/deployments/{}/extend", active.id),
            &[("end_date", "2027-04-30")],
        )
        .await;
    response.assert_status(422);
    let body = response.text();
    assert!(body.contains("Contract on credit hold since"), "{body}");
    let kept = t.reload_deployment(active.id).await;
    assert_eq!(kept.status, DeploymentStatus::Active);
    assert_eq!(kept.end_date, Some(date(2027, 3, 31)));
}

/// AC4: with K `terminating`, `activate` of D is allowed and request `submit`
/// is refused, with the generic message (a terminating contract is not on
/// hold).
#[renox::test]
async fn a_terminating_contract_takes_deployments_but_no_request() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t
        .contract(TOWER, ContractStatus::Terminating, Some(&amal))
        .await;
    t.cards
        .accept(&contract.subject(), date(2026, 1, 1), &[("guard", "day")]);
    let site = t.site(TOWER, "Tower A").await;
    let worker = t.staff.employ(&t.parties, 1, "Ravi Kumar", true);
    let planned = t
        .deployment(
            &contract,
            &site,
            worker,
            Some(date(2027, 6, 30)),
            DeploymentStatus::Planned,
        )
        .await;
    let request = t.request(&contract, &site, 1, RequestStatus::Draft).await;

    t.post(
        &amal,
        &format!("/manpower/deployments/{}/activate", planned.id),
        &[],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.reload_deployment(planned.id).await.status,
        DeploymentStatus::Active
    );
    let response = t
        .post(
            &amal,
            &format!("/manpower/requests/{}/submit", request.id),
            &[],
        )
        .await;
    response.assert_status(422);
    assert!(
        response.text().contains("does not take new requests"),
        "generic refusal"
    );
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Draft
    );
}

/// AC5: with K `active` all three are allowed.
#[renox::test]
async fn an_active_contract_allows_submit_activate_and_extend() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let contract = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    t.cards
        .accept(&contract.subject(), date(2026, 1, 1), &[("guard", "day")]);
    let site = t.site(TOWER, "Tower A").await;
    let first = t.staff.employ(&t.parties, 1, "Ravi Kumar", true);
    let second = t.staff.employ(&t.parties, 2, "Sunil Das", true);
    let planned = t
        .deployment(
            &contract,
            &site,
            first,
            Some(date(2027, 6, 30)),
            DeploymentStatus::Planned,
        )
        .await;
    let active = t
        .deployment(
            &contract,
            &site,
            second,
            Some(date(2027, 3, 31)),
            DeploymentStatus::Active,
        )
        .await;
    let request = t.request(&contract, &site, 1, RequestStatus::Draft).await;

    t.post(
        &amal,
        &format!("/manpower/requests/{}/submit", request.id),
        &[],
    )
    .await
    .assert_status(303);
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/activate", planned.id),
        &[],
    )
    .await
    .assert_status(303);
    t.post(
        &amal,
        &format!("/manpower/deployments/{}/extend", active.id),
        &[("end_date", "2027-04-30")],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.reload_request(request.id).await.status,
        RequestStatus::Open
    );
    assert_eq!(
        t.reload_deployment(planned.id).await.status,
        DeploymentStatus::Active
    );
    assert_eq!(
        t.reload_deployment(active.id).await.end_date,
        Some(date(2027, 4, 30))
    );
}

/// A hold stops only what is new: a draft request can still be written and a
/// deployment planned, and the workers on site stay `active`. A cover request
/// by hand is refused with the hold's reason.
#[renox::test]
async fn drafts_and_planned_deployments_can_still_be_made_while_held() {
    let t = boot().await;
    let contract = held_for_invoice(&t, date(2026, 11, 5)).await;
    let site = t.site(TOWER, "Tower A").await;
    let on_site = t.staff.employ(&t.parties, 1, "Ravi Kumar", true);
    let next = t.staff.employ(&t.parties, 2, "Sunil Das", true);
    let active = t
        .deployment(
            &contract,
            &site,
            on_site,
            Some(date(2027, 6, 30)),
            DeploymentStatus::Active,
        )
        .await;
    let amal = t.account_manager("Anna", TOWER).await;

    t.post(
        &amal,
        "/manpower/requests",
        &[
            ("client_contract_id", &contract.id.to_string()),
            ("site_id", &site.id.to_string()),
            ("role", "guard"),
            ("headcount", "2"),
            ("shift_type", "day"),
            ("start_date", "2027-02-01"),
            ("end_date", "2027-03-31"),
        ],
    )
    .await
    .assert_status(303);
    t.post(
        &amal,
        "/manpower/deployments",
        &[
            ("employee_id", &next.0.to_string()),
            ("client_contract_id", &contract.id.to_string()),
            ("site_id", &site.id.to_string()),
            ("role", "guard"),
            ("shift_type", "day"),
            ("start_date", "2027-02-01"),
            ("end_date", "2027-03-31"),
        ],
    )
    .await
    .assert_status(303);
    assert_eq!(
        t.reload_deployment(active.id).await.status,
        DeploymentStatus::Active
    );

    let response = t
        .post(
            &amal,
            &format!("/manpower/deployments/{}/cover", active.id),
            &[("from", "2027-03-01"), ("to", "2027-03-10")],
        )
        .await;
    response.assert_status(422);
    assert!(
        response.text().contains("Contract on credit hold since"),
        "the hold is the reason"
    );
}

/// The cover the contract requires still opens for an approved leave while the
/// contract is held (the worker on site is the agency's to replace); the cover
/// worker's `activate` is what the hold refuses.
#[renox::test]
async fn the_cover_a_contract_requires_still_opens_while_it_is_held() {
    let t = boot().await;
    let contract = held_for_invoice(&t, date(2026, 11, 5)).await;
    let mut contract = t.reload_contract(contract.id).await;
    contract.cover_required = true;
    contract.save(t.db()).await.unwrap();
    let site = t.site(TOWER, "Tower A").await;
    let worker = t.staff.employ(&t.parties, 1, "Ravi Kumar", true);
    t.deployment(
        &contract,
        &site,
        worker,
        Some(date(2027, 6, 30)),
        DeploymentStatus::Active,
    )
    .await;

    t.state()
        .emit(wl_kernel::LeaveApproved {
            leave: wl_kernel::Subject::new("hr.leave_request", 5),
            employee: worker,
            leave_type: "annual".to_owned(),
            from: date(2027, 3, 1),
            to: date(2027, 3, 10),
            unpaid: false,
        })
        .await
        .unwrap();

    assert_eq!(t.covers.all().len(), 1);
}
