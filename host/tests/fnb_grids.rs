//! The lists of the F&B vertical on the demo company (#536): the orders, the
//! closings, the cold units and the other lists are `renox::grid`s that are
//! direct children of their pages, search, filter, sort and page through the
//! query string; the outlet channels stay one table per outlet with a search
//! and a state filter at the top.

mod grid_support;

use grid_support::{encode, grid_is_a_child_of_the_page, load, range, total};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of the F&B vertical.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-fnb/client.toml"
);

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

/// The demo company, made by the demo command, with the area manager signed
/// in.
async fn demo() -> TestApp {
    let config = ClientConfig::load(DEMO).expect("the demo client");
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .expect("wl:install runs");
    app.kernel().seed().await.expect("the seeds run");
    app.kernel()
        .call(host::fnb_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");
    let area = User::find_by_email(app.db(), "area@zaatar.example")
        .await
        .unwrap()
        .expect("the area manager");
    app.acting_as(&area);
    app
}

/// Every list of the issue is a grid that is a direct child of its page.
#[test]
fn every_fnb_list_is_a_grid_that_fills_the_page() {
    for template in [
        "fnb/views/fnb/orders.html",
        "fnb/views/fnb/kitchen.html",
        "fnb/views/fnb/closings.html",
        "fnb/views/fnb/payouts.html",
        "fnb/views/fnb/batches.html",
        "fnb/views/fnb/cold_units.html",
        "fnb/views/fnb/rider_faults.html",
        "fnb/views/fnb/cod_customers.html",
        "fnb/views/fnb/report_failed.html",
        "fnb/views/fnb/report_refunds.html",
    ] {
        grid_is_a_child_of_the_page(template);
    }
}

/// The orders: ten to a page, newest first, a filter by state and the search
/// on the order number.
#[renox::test]
async fn the_orders_page_sort_filter_and_search() {
    let app = demo().await;
    let orders = count(&app, "select count(*) from fnb_orders").await;
    assert!(orders >= 9, "the demo has {orders} orders");
    let first = load(&app, "/orders?per_page=10&sort=id").await;
    assert_eq!(range(&first), (1, orders.min(10), orders));
    if orders > 10 {
        let second = load(&app, "/orders?per_page=10&sort=id&page=2").await;
        assert_eq!(range(&second), (11, orders.min(20), orders));
    }
    let oldest = text_of(&app, "select number from fnb_orders order by id limit 1").await;
    let newest = text_of(
        &app,
        "select number from fnb_orders order by id desc limit 1",
    )
    .await;
    let by_newest = load(&app, "/orders?per_page=100&sort=-id").await;
    assert!(by_newest.find(&newest).unwrap() < by_newest.find(&oldest).unwrap());

    let status = text_of(&app, "select status from fnb_orders order by id limit 1").await;
    let of_status = count(
        &app,
        &format!("select count(*) from fnb_orders where status = '{status}'"),
    )
    .await;
    assert_eq!(
        total(&app, &format!("/orders?in.status={status}")).await,
        of_status
    );
    let found = load(&app, &format!("/orders?search={}", encode(&oldest))).await;
    assert!(range(&found).2 >= 1 && found.contains(&oldest));
    assert_eq!(total(&app, "/orders?search=zzqqxx-nobody").await, 0);
}

/// The cold units split by temperature state; the closings count their rows.
#[renox::test]
async fn the_cold_units_filter_by_state_and_the_closings_count() {
    let app = demo().await;
    let units = count(&app, "select count(*) from fnb_cold_units where active").await;
    assert_eq!(total(&app, "/cold-units?per_page=10").await, units);
    let split = total(&app, "/cold-units?in.condition=in_range").await
        + total(&app, "/cold-units?in.condition=out_of_range").await;
    assert_eq!(split, units);
    let closings = load(&app, "/closings?per_page=10").await;
    let (_, _, rows) = range(&closings);
    assert!(rows <= count(&app, "select count(*) from fnb_daily_closings").await);
}

/// The outlet channels narrow to an outlet by search and say so when nothing
/// matches.
#[renox::test]
async fn the_outlet_channels_search_and_filter_by_state() {
    let app = demo().await;
    let all = load(&app, "/outlet-channels").await;
    assert!(all.contains("Search outlets and channels"));
    let none = load(&app, "/outlet-channels?q=zzqqxx-nothing").await;
    assert!(none.contains("No channel matches"));
    let outlet = text_of(
        &app,
        "select o.name from fnb_outlets o join fnb_outlet_channels c on c.outlet_id = o.id \
         where o.active order by o.id limit 1",
    )
    .await;
    let named = load(&app, &format!("/outlet-channels?q={}", encode(&outlet))).await;
    assert!(named.contains(&outlet) && !named.contains("No channel matches"));
    load(&app, "/outlet-channels?state=paused").await;
    load(&app, "/outlet-channels?state=open").await;
}
