//! The lists of the Manufacturing vertical on the demo company (#556): each
//! is a `renox::grid` that is a direct child of the page (it fills the
//! screen), opens for the role that owns it, searches, sorts and pages
//! through the query string, keeps the dashboard's `?status=` links working,
//! and exports for the roles that may.

mod grid_support;
mod manufacturing_demo_support;
mod support;

use grid_support::{encode, grid_is_a_child_of_the_page, load, range, total};
use manufacturing_demo_support::demo;
use renox::prelude::*;
use renox::testing::TestApp;

async fn count(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap()
}

async fn text_of(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap()
}

/// Signs in the demo login `email`.
async fn sign_in(app: &TestApp, email: &str) {
    let user = User::find_by_email(app.db(), email)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("the demo login {email}"));
    app.acting_as(&user);
}

/// The lists of the issue that are grids. The recalls list reads its rows
/// from the lot-trace contract (wl-inventory's records), so it fills the
/// screen in one panel instead.
const LISTS: [(&str, &str); 7] = [
    ("/manufacturing/work-orders", "work_orders.html"),
    ("/manufacturing/ncrs", "ncrs.html"),
    ("/manufacturing/returns", "returns.html"),
    ("/manufacturing/exports", "exports.html"),
    ("/manufacturing/conformity", "conformity.html"),
    ("/manufacturing/inspections", "inspections.html"),
    ("/manufacturing/aborted", "aborted.html"),
];

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_manufacturing_list_is_a_grid_that_fills_the_page() {
    for (_, template) in LISTS {
        grid_is_a_child_of_the_page(&format!("manufacturing/views/manufacturing/{template}"));
    }
}

/// Each list answers 200 for the login of the screen map that owns it, draws
/// the grid with its search, and a search that matches nothing counts none.
#[renox::test]
async fn every_manufacturing_list_opens_as_a_grid_for_its_owner() {
    let app = demo().await;
    let screens = host::manufacturing_demo::screen_map(app.state())
        .await
        .unwrap();
    for (path, _) in LISTS {
        let login = screens
            .iter()
            .find(|screen| screen.url == path)
            .map_or(host::manufacturing_demo::MANAGER_EMAIL, |screen| {
                screen.login
            });
        sign_in(&app, login).await;
        let html = load(&app, path).await;
        assert!(html.contains("class=\"rx-grid"), "{path} draws a grid");
        assert!(html.contains("search"), "{path} has a search");
        let none = load(&app, &format!("{path}?search=zzqqxx-nothing")).await;
        assert_eq!(range(&none).2, 0, "{path} finds nothing for nonsense");
    }
}

/// The work orders: pages of one row, the sort, the search by number, the
/// status link of the dashboard and the export for the production manager.
#[renox::test]
async fn the_work_orders_grid_pages_sorts_searches_and_exports() {
    let app = demo().await;
    sign_in(&app, host::manufacturing_demo::MANAGER_EMAIL).await;
    let all = count(&app, "select count(*) from manufacturing_work_orders").await;
    let seen = total(&app, "/manufacturing/work-orders").await;
    assert!(seen >= 2 && seen <= all, "{seen} of {all} work orders");

    assert_eq!(
        range(&load(&app, "/manufacturing/work-orders?per_page=1").await),
        (1, 1, seen)
    );
    assert_eq!(
        range(&load(&app, "/manufacturing/work-orders?per_page=1&page=2").await),
        (2, 2, seen)
    );

    let first = text_of(
        &app,
        "select number from manufacturing_work_orders order by id limit 1",
    )
    .await;
    let last = text_of(
        &app,
        "select number from manufacturing_work_orders order by id desc limit 1",
    )
    .await;
    let oldest = load(&app, "/manufacturing/work-orders?sort=id&per_page=100").await;
    let newest = load(&app, "/manufacturing/work-orders?sort=-id&per_page=100").await;
    assert!(oldest.find(&first).unwrap() < oldest.find(&last).unwrap());
    assert!(newest.find(&last).unwrap() < newest.find(&first).unwrap());

    let found = load(
        &app,
        &format!("/manufacturing/work-orders?search={}", encode(&first)),
    )
    .await;
    assert!(range(&found).2 >= 1 && found.contains(&first), "{first} is found");

    // The dashboard's `?status=` links still narrow the list.
    let on_hold = count(
        &app,
        "select count(*) from manufacturing_work_orders where status = 'on_hold'",
    )
    .await;
    assert!(
        total(&app, "/manufacturing/work-orders?status=on_hold").await <= on_hold,
        "only the work orders on hold"
    );
    assert!(total(&app, "/manufacturing/work-orders?in.status=on_hold").await <= on_hold);

    let file = app
        .request()
        .header("accept", "text/html")
        .get("/manufacturing/work-orders?export=csv")
        .await;
    assert_eq!(file.status.as_u16(), 200);
    assert!(file.text().contains(&first), "the export lists the work orders");
}

/// The export of the work orders is refused to whom the grid does not offer
/// it (the operator reports output, they do not plan).
#[renox::test]
async fn the_operator_may_not_export_the_work_orders() {
    let app = demo().await;
    sign_in(&app, host::manufacturing_demo::OPERATOR_EMAIL).await;
    let page = app
        .request()
        .header("accept", "text/html")
        .get("/manufacturing/work-orders?export=csv")
        .await;
    assert_eq!(page.status.as_u16(), 403);
    // The list itself opens for the operator, without the export menu.
    let html = load(&app, "/manufacturing/work-orders").await;
    assert!(html.contains("class=\"rx-grid"));
}
