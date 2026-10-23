//! The host mounts the Manufacturing vertical when `client.toml` names
//! `app = "manufacturing"` (Base 07, 07 Manufacturing §3.1): the roles hold
//! the permissions of the spec, the pages open for them in the host's layout
//! and show each person only the actions they may run. The abort and the
//! unlink on the screens are in `manufacturing_abort_screens.rs` and
//! `manufacturing_unlink_screens.rs`.

use host::roles::{
    ADMIN, PRODUCTION_MANAGER, PRODUCTION_PLANNER, QC_INSPECTOR, STOREKEEPER, SUPERVISOR,
};
use manufacturing::permissions as perm;
use manufacturing::{Manufacturing, WorkOrder, WorkOrderStatus};
use renox::prelude::*;

mod manufacturing_support;

use manufacturing_support::{boot, browse, floor, person};

#[renox::test]
async fn a_manufacturing_client_boots_and_keeps_work_orders() {
    let app = boot().await;
    let f = floor(&app).await;
    let saved = WorkOrder::find_or_404(app.db(), f.order.id).await.unwrap();
    assert_eq!(saved.status, WorkOrderStatus::Released);
    assert_eq!(f.line.plant_id, f.plant.id);
}

#[test]
fn the_vertical_is_the_module_the_host_mounts() {
    assert_eq!(Manufacturing.name(), "manufacturing");
    assert!(host::VERTICALS.contains(&"manufacturing"));
}

#[test]
fn the_roles_hold_the_manufacturing_permissions() {
    let holds = |role: &str, permission: &str| {
        roles()
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| grants.contains(&permission))
    };
    // The company admin holds all of them.
    for permission in perm::ALL {
        assert!(holds(ADMIN, permission), "{permission}");
    }
    // The planner writes and releases work orders, asks for an abort and does
    // not approve it (`MFG.ACL.12`); no costs for the planner.
    assert!(holds(PRODUCTION_PLANNER, perm::WORK_ORDERS_CREATE));
    assert!(holds(PRODUCTION_PLANNER, perm::WORK_ORDERS_RELEASE));
    assert!(!holds(PRODUCTION_PLANNER, perm::WORK_ORDERS_ABORT_APPROVE));
    assert!(!holds(PRODUCTION_PLANNER, perm::WORK_ORDERS_COST_VIEW));
    // The production manager approves the abort and sees the costs.
    assert!(holds(PRODUCTION_MANAGER, perm::WORK_ORDERS_ABORT_APPROVE));
    assert!(holds(PRODUCTION_MANAGER, perm::WORK_ORDERS_COST_VIEW));
    assert!(holds(PRODUCTION_MANAGER, perm::WORK_ORDERS_CLOSE));
    // The storekeeper issues materials and does nothing else to a work order.
    assert!(holds(STOREKEEPER, perm::WORK_ORDERS_ISSUE));
    assert!(!holds(STOREKEEPER, perm::WORK_ORDERS_RELEASE));
    assert!(!holds(STOREKEEPER, perm::WORK_ORDERS_REPORT));
    assert!(!holds(STOREKEEPER, perm::WORK_ORDERS_COST_VIEW));
    // The line supervisor reports; no costs, no approval.
    assert!(holds(SUPERVISOR, perm::WORK_ORDERS_REPORT));
    assert!(!holds(SUPERVISOR, perm::WORK_ORDERS_ABORT_APPROVE));
    assert!(!holds(SUPERVISOR, perm::WORK_ORDERS_COST_VIEW));
    // The QC inspector only sees work orders, works the NCRs and inspects the
    // returned goods.
    assert!(holds(QC_INSPECTOR, perm::WORK_ORDERS_VIEW));
    for permission in [
        perm::NCRS_VIEW,
        perm::NCRS_RAISE,
        perm::NCRS_DISPOSITION,
        perm::NCRS_APPROVE,
    ] {
        assert!(holds(QC_INSPECTOR, permission), "{permission}");
        assert!(holds(PRODUCTION_MANAGER, permission), "{permission}");
    }
    for permission in perm::ALL {
        let ncr = permission.starts_with("ncrs.");
        // BL-MFG-07: QC also inspects the goods that came back (`MFG.RUL.10`).
        let inspection = permission.starts_with("inspections.");
        if *permission != perm::WORK_ORDERS_VIEW && !ncr && !inspection {
            assert!(!holds(QC_INSPECTOR, permission), "{permission}");
        }
    }
}

fn roles() -> &'static [(&'static str, &'static [&'static str])] {
    host::roles::ROLES
}

/// The admin reaches the list through the sidebar; a person with no role of
/// the vertical is refused.
#[renox::test]
async fn the_list_opens_for_the_roles_of_the_plant_and_not_for_others() {
    let app = boot().await;
    let f = floor(&app).await;
    let admin = person(&app, "Rana", ADMIN, None).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let inspector = person(&app, "Qasim", QC_INSPECTOR, Some(f.plant.scope())).await;
    let nobody = person(&app, "Nadia", host::roles::MEMBER, None).await;

    for user in [&admin, &planner, &inspector] {
        let page = browse(&app, user, "/manufacturing/work-orders").await;
        page.assert_ok();
        page.assert_see("Work orders");
        page.assert_see("WO-HOST-1");
        page.assert_see("rx-table-wrap");
    }
    // The sidebar links the list for the admin.
    browse(&app, &admin, "/manufacturing/work-orders")
        .await
        .assert_see("/manufacturing/work-orders");
    browse(&app, &nobody, "/manufacturing/work-orders")
        .await
        .assert_forbidden();
    browse(
        &app,
        &nobody,
        &format!("/manufacturing/work-orders/{}", f.order.id),
    )
    .await
    .assert_forbidden();
    // Only the planner has the sheet to write a work order.
    browse(&app, &planner, "/manufacturing/work-orders")
        .await
        .assert_see("New work order");
    browse(&app, &inspector, "/manufacturing/work-orders")
        .await
        .assert_dont_see("New work order");
}

/// The record page shows each role the actions it may run: the planner
/// releases/cancels and asks for the abort, the storekeeper issues, the
/// supervisor of the line starts and reports, the inspector only looks.
#[renox::test]
async fn the_record_page_offers_each_role_its_own_actions() {
    let app = boot().await;
    let f = floor(&app).await;
    let path = format!("/manufacturing/work-orders/{}", f.order.id);
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let keeper = person(&app, "Karim", STOREKEEPER, Some(f.plant.scope())).await;
    let sami = person(&app, "Sami", SUPERVISOR, Some(f.line.scope())).await;
    let inspector = person(&app, "Qasim", QC_INSPECTOR, Some(f.plant.scope())).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;

    let planner_page = browse(&app, &planner, &path).await;
    planner_page.assert_ok();
    planner_page.assert_see("WO-HOST-1");
    planner_page.assert_see("Released");
    planner_page.assert_see("cancel-order");
    planner_page.assert_dont_see("issue-order");
    planner_page.assert_dont_see("start-order");
    // Nothing issued or reported: abort is not offered.
    planner_page.assert_dont_see("abort-order");

    let keeper_page = browse(&app, &keeper, &path).await;
    keeper_page.assert_ok();
    keeper_page.assert_see("issue-order");
    keeper_page.assert_dont_see("cancel-order");

    let supervisor_page = browse(&app, &sami, &path).await;
    supervisor_page.assert_ok();
    supervisor_page.assert_see("start-order");
    supervisor_page.assert_dont_see("issue-order");

    let inspector_page = browse(&app, &inspector, &path).await;
    inspector_page.assert_ok();
    inspector_page.assert_dont_see("cancel-order");
    inspector_page.assert_dont_see("start-order");
    inspector_page.assert_dont_see("issue-order");

    // The JSON of the page carries no costs for the planner, and does for the
    // production manager (`FIN.ACL.08`).
    app.acting_as(&planner);
    let json = app.get(&path).await;
    json.assert_ok();
    assert!(
        json.text().contains("\"show_costs\":false"),
        "{}",
        json.text()
    );
    app.acting_as(&manager);
    assert!(app.get(&path).await.text().contains("\"show_costs\":true"));
}
