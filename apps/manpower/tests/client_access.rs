//! Base 09 (09 Manpower §3.3 `MPW.ACL.01`, `MPW.ACL.02`, §3.2): a client user
//! sees only the workers deployed to their own company, and only their name,
//! role, site, shift and dates; a site supervisor sees the deployments of
//! their site; the account manager those of their client companies.

mod support;

use manpower::{ContractStatus, DeploymentStatus};

use support::{FOODS, TOWER, boot, date};

/// The keys of what a client user may see of a deployment.
const CLIENT_KEYS: [&str; 8] = [
    "end_date",
    "role",
    "shift_type",
    "site",
    "start_date",
    "status",
    "status_kind",
    "worker",
];

/// `MPW.ACL.01` and `MPW.ACL.02`: the client viewer and the client approver of
/// Tower see Tower's workers on site (active, suspended and ended, not the
/// planned or cancelled), by name, role, site, shift and dates only: no id of
/// the agency's records, no reason.
#[renox::test]
async fn a_client_user_sees_only_the_workers_of_their_own_company() {
    let t = boot().await;
    let tower = t.contract(TOWER, ContractStatus::Active, None).await;
    let foods = t.contract(FOODS, ContractStatus::Active, None).await;
    let tower_site = t.site(TOWER, "Tower A").await;
    let foods_site = t.site(FOODS, "Plant 1").await;
    for (id, name) in [
        (7, "Ravi Kumar"),
        (8, "Sana Ali"),
        (9, "Joel Smith"),
        (10, "Mei Lin"),
    ] {
        t.staff.employ(&t.parties, id, name, true);
    }
    use wl_kernel::workforce::EmployeeId;
    t.deployment(
        &tower,
        &tower_site,
        EmployeeId(7),
        Some(date(2027, 7, 31)),
        DeploymentStatus::Active,
    )
    .await;
    t.deployment(
        &tower,
        &tower_site,
        EmployeeId(8),
        None,
        DeploymentStatus::Planned,
    )
    .await;
    t.deployment(
        &tower,
        &tower_site,
        EmployeeId(9),
        Some(date(2027, 3, 1)),
        DeploymentStatus::Ended,
    )
    .await;
    t.deployment(
        &foods,
        &foods_site,
        EmployeeId(10),
        None,
        DeploymentStatus::Active,
    )
    .await;
    let viewer = t.client_user("Vik", TOWER, &["viewer"]).await;
    let approver = t.client_user("Hana", TOWER, &["approver"]).await;
    for user in [&viewer, &approver] {
        let rows: Vec<renox::serde_json::Value> = t.get(user, "/manpower/deployments").await.json();
        let names: Vec<_> = rows
            .iter()
            .map(|row| row["worker"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["Ravi Kumar", "Joel Smith"]);
        for row in &rows {
            let mut keys: Vec<_> = row
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            keys.sort_unstable();
            assert_eq!(keys, CLIENT_KEYS);
            assert_eq!(row["site"], "Tower A");
            assert_eq!(row["role"], "guard");
        }
        assert_eq!(rows[0]["status"], "Active");
    }
    let page = t.browse(&viewer, "/manpower/deployments").await;
    page.assert_ok();
    page.assert_see("Ravi Kumar");
    page.assert_dont_see("Mei Lin");
    page.assert_dont_see("Sana Ali");
}

/// `MPW.ACL.01`: a client user whose company has no worker on site yet gets an
/// empty list; a user with no access at all is refused; another company's
/// user sees none of Tower's workers.
#[renox::test]
async fn a_client_user_with_nothing_on_site_gets_an_empty_list() {
    let t = boot().await;
    let tower = t.contract(TOWER, ContractStatus::Active, None).await;
    let _foods = t.contract(FOODS, ContractStatus::Active, None).await;
    let site = t.site(TOWER, "Tower A").await;
    t.staff.employ(&t.parties, 7, "Ravi Kumar", true);
    t.deployment(
        &tower,
        &site,
        wl_kernel::workforce::EmployeeId(7),
        None,
        DeploymentStatus::Active,
    )
    .await;
    let foods_user = t.client_user("Omar", FOODS, &["viewer"]).await;
    let nobody = t.user("Nadia").await;
    let rows: Vec<renox::serde_json::Value> =
        t.get(&foods_user, "/manpower/deployments").await.json();
    assert!(rows.is_empty());
    t.get(&nobody, "/manpower/deployments")
        .await
        .assert_forbidden();
}

/// §3.2: the site supervisor sees the deployments of their site only, with the
/// staff view; the account manager those of their client companies; both with
/// the ids of the agency's records.
#[renox::test]
async fn staff_see_the_deployments_of_their_site_or_their_client_companies() {
    let t = boot().await;
    let amal = t.account_manager("Amal", TOWER).await;
    let tower = t.contract(TOWER, ContractStatus::Active, Some(&amal)).await;
    let foods = t.contract(FOODS, ContractStatus::Active, None).await;
    let tower_a = t.site(TOWER, "Tower A").await;
    let tower_b = t.site(TOWER, "Tower B").await;
    let plant = t.site(FOODS, "Plant 1").await;
    use wl_kernel::workforce::EmployeeId;
    for (id, name) in [(7, "Ravi Kumar"), (8, "Sana Ali"), (9, "Joel Smith")] {
        t.staff.employ(&t.parties, id, name, true);
    }
    t.deployment(
        &tower,
        &tower_a,
        EmployeeId(7),
        None,
        DeploymentStatus::Active,
    )
    .await;
    t.deployment(
        &tower,
        &tower_b,
        EmployeeId(8),
        None,
        DeploymentStatus::Planned,
    )
    .await;
    t.deployment(
        &foods,
        &plant,
        EmployeeId(9),
        None,
        DeploymentStatus::Active,
    )
    .await;
    let sami = t.user("Sami").await;
    sami.assign_role_in(
        t.db(),
        manpower::permissions::SUPERVISOR_ROLE,
        &manpower::permissions::site_scope(tower_a.id),
    )
    .await
    .unwrap();

    let supervised: Vec<renox::serde_json::Value> =
        t.get(&sami, "/manpower/deployments").await.json();
    assert_eq!(supervised.len(), 1);
    assert_eq!(supervised[0]["worker"], "Ravi Kumar");
    assert_eq!(supervised[0]["site"], "Tower A");
    assert_eq!(supervised[0]["employee_id"], 7);

    let managed: Vec<renox::serde_json::Value> = t.get(&amal, "/manpower/deployments").await.json();
    let names: Vec<_> = managed
        .iter()
        .map(|row| row["worker"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Ravi Kumar", "Sana Ali"]);
    assert_eq!(managed[1]["status"], "Planned");
}
