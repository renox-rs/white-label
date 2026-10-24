//! The bill of materials in the real host (Base 07, `STK.BOM.01`): wl-inventory
//! provides the kernel's `BillsOfMaterials`, so a planner can release a work
//! order made from a released version, and a person who keeps BOMs sees the
//! BOM pages under the roles of the plant.

use host::roles::{ADMIN, PRODUCTION_MANAGER, PRODUCTION_PLANNER, QC_INSPECTOR};
use manufacturing::{PickLine, WorkOrder, WorkOrderStatus};
use renox::prelude::*;
use wl_inventory::{Bom, BomStatus};

mod manufacturing_support;

use manufacturing_support::{boot, browse, floor, person, post};

/// The id a redirect landed on.
fn landed(response: &renox::testing::TestResponse) -> i64 {
    response
        .header("location")
        .and_then(|location| location.rsplit('/').next())
        .and_then(|id| id.parse().ok())
        .unwrap_or_else(|| panic!("a redirect to a page: {:?}", response.header("location")))
}

/// A work order whose BOM is a draft cannot be released; once the production
/// manager of the plant releases the version, the planner releases the work
/// order: the standard cost comes from the version and the pick list from its
/// components.
#[renox::test]
async fn a_work_order_is_released_from_a_bom_the_manager_released() {
    let app = boot().await;
    let f = floor(&app).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;

    // The manager, assigned to the plant only, keeps the BOM.
    let made = post(
        &app,
        &manager,
        "/boms",
        &[("item", &f.panel.id.to_string()), ("hours", "0.5")],
    )
    .await;
    let bom = landed(&made);
    post(
        &app,
        &manager,
        &format!("/boms/{bom}/lines"),
        &[("item", &f.resin.id.to_string()), ("quantity", "2")],
    )
    .await
    .assert_ok();

    // The planner writes the work order from the draft: it cannot be released.
    let created = post(
        &app,
        &planner,
        "/manufacturing/work-orders",
        &[
            ("plant_id", &f.plant.id.to_string()),
            ("line_id", &f.line.id.to_string()),
            ("item_id", &f.panel.id.to_string()),
            ("bom_id", &bom.to_string()),
            ("quantity", "10"),
        ],
    )
    .await;
    let order = landed(&created);
    let refused = post(
        &app,
        &planner,
        &format!("/manufacturing/work-orders/{order}/release"),
        &[],
    )
    .await;
    refused.assert_status(422);
    assert!(refused.text().contains("BOM version is not released"));

    // The planner does not release BOMs; the manager does.
    post(&app, &planner, &format!("/boms/{bom}/release"), &[])
        .await
        .assert_status(403);
    post(&app, &manager, &format!("/boms/{bom}/release"), &[])
        .await
        .assert_ok();
    assert_eq!(
        Bom::find_or_404(app.db(), bom).await.unwrap().status,
        BomStatus::Released
    );

    post(
        &app,
        &planner,
        &format!("/manufacturing/work-orders/{order}/release"),
        &[],
    )
    .await
    .assert_status(303);
    let released = WorkOrder::find_or_404(app.db(), order).await.unwrap();
    assert_eq!(released.status, WorkOrderStatus::Released);
    // 2 kg of resin at 8.00 a unit, half an hour at the line's 120.00 an hour.
    assert_eq!(released.std_material_cost, 1_600);
    assert_eq!(released.std_unit_cost, 1_600 + 6_000);
    let pick = manufacturing::pick_list::of_order(app.db(), order)
        .await
        .unwrap();
    assert_eq!(pick.len(), 1);
    let PickLine {
        item_id,
        required_milli,
        ..
    } = &pick[0];
    assert_eq!((*item_id, *required_milli), (f.resin.id, 20_000));
}

/// The BOM pages open for the roles that hold a BOM permission, not for the
/// inspector, and the planner sees the versions without the buttons to
/// change them.
#[renox::test]
async fn the_bom_pages_open_for_the_roles_of_the_plant() {
    let app = boot().await;
    let f = floor(&app).await;
    let admin = person(&app, "Rana", ADMIN, None).await;
    let manager = person(&app, "Mona", PRODUCTION_MANAGER, Some(f.plant.scope())).await;
    let planner = person(&app, "Amal", PRODUCTION_PLANNER, Some(f.plant.scope())).await;
    let inspector = person(&app, "Qasim", QC_INSPECTOR, Some(f.plant.scope())).await;

    let made = post(&app, &admin, "/boms", &[("item", &f.panel.id.to_string())]).await;
    let bom = landed(&made);
    for user in [&admin, &manager, &planner] {
        let page = browse(&app, user, "/boms").await;
        page.assert_ok();
        page.assert_see("Bills of materials");
        page.assert_see("rx-table-wrap");
        browse(&app, user, &format!("/boms/{bom}"))
            .await
            .assert_ok();
    }
    browse(&app, &manager, "/boms").await.assert_see("New bill");
    browse(&app, &planner, "/boms")
        .await
        .assert_dont_see("New bill");
    browse(&app, &inspector, "/boms").await.assert_forbidden();
    // The sidebar links the BOMs for the manager.
    browse(&app, &manager, "/manufacturing/work-orders")
        .await
        .assert_see("Bills of materials");
}
