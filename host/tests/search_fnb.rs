//! The F&B records in the global search and the Related panel (BL-AGT-13):
//! on the demo company, an operations director finds an order by its own
//! number and by the aggregator's order number, a production batch by its
//! number and an aggregator payout by its number and reference, each with
//! its related records; a manager of one outlet finds that outlet's orders
//! only.

use host::roles::{self, AREA_MANAGER, OPERATIONS_DIRECTOR};
use renox::auth::permissions::Scope;
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::related::{self, PANEL_DEPTH};
use wl_kernel::search::{self, Access};
use wl_kernel::{ClientConfig, Subject};

const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-fnb/client.toml"
);

async fn booted() -> TestApp {
    let config = ClientConfig::load(CLIENT).expect("the demo client");
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
    search::rebuild(app.db()).await.unwrap();
    app
}

async fn person(app: &TestApp, name: &str, role: &str, scope: Option<&Scope>) -> User {
    let mut user = User::register(
        app.db(),
        name,
        &format!("{}@search.example", name.to_lowercase()),
        "secret123",
    )
    .await
    .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    match scope {
        Some(scope) => user.assign_role_in(app.db(), role, scope).await.unwrap(),
        None => user.assign_role(app.db(), role).await.unwrap(),
    }
    user
}

async fn scalar_i64(app: &TestApp, query: &str) -> i64 {
    renox::db::sql(query.to_owned())
        .scalar::<i64>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn scalar_text(app: &TestApp, query: &str) -> String {
    renox::db::sql(query.to_owned())
        .scalar::<String>(app.db())
        .await
        .unwrap_or_else(|e| panic!("{query}: {e:?}"))
}

async fn finds(app: &TestApp, access: &Access, typed: &str, subject: &Subject) -> bool {
    search::search(app.db(), access, typed, 50)
        .await
        .unwrap()
        .groups
        .iter()
        .flat_map(|group| group.hits.iter())
        .any(|hit| &hit.subject == subject)
}

async fn related_count(app: &TestApp, access: &Access, subject: &Subject, name: &str) -> u64 {
    related::related(app.db(), access, subject, PANEL_DEPTH)
        .await
        .unwrap()
        .iter()
        .find(|group| group.name == name)
        .map_or(0, |group| group.count)
}

#[renox::test]
async fn an_operations_director_finds_orders_batches_and_payouts_by_their_numbers() {
    let app = booted().await;
    let director = person(&app, "Dina", OPERATIONS_DIRECTOR, None).await;
    let access = Access::of_user(app.db(), &director).await.unwrap();

    // An aggregator's order paid on a payout: its own number and the
    // aggregator's number, typed in lower case.
    let order = scalar_i64(
        &app,
        "select o.id from fnb_orders o join fnb_payout_lines l on l.order_id = o.id order by o.id limit 1",
    )
    .await;
    let order_subject = Subject::new("fnb.order", order);
    let number = scalar_text(
        &app,
        &format!("select number from fnb_orders where id = {order}"),
    )
    .await;
    assert!(finds(&app, &access, &number.to_lowercase(), &order_subject).await);
    let external = scalar_text(
        &app,
        &format!(
            "select s.external_number from wl_sales_sales_orders s join fnb_orders o on o.sales_order_id = s.id where o.id = {order}"
        ),
    )
    .await;
    assert!(finds(&app, &access, &external.to_lowercase(), &order_subject).await);
    assert!(related_count(&app, &access, &order_subject, "payouts").await >= 1);

    // The payout by its number and the aggregator's reference.
    let statement = scalar_i64(
        &app,
        &format!(
            "select statement_id from fnb_payout_lines where order_id = {order} order by id limit 1"
        ),
    )
    .await;
    let statement_subject = Subject::new("fnb.payout_statement", statement);
    let (statement_number, reference) = (
        scalar_text(
            &app,
            &format!("select number from fnb_payout_statements where id = {statement}"),
        )
        .await,
        scalar_text(
            &app,
            &format!("select reference from fnb_payout_statements where id = {statement}"),
        )
        .await,
    );
    assert!(finds(&app, &access, &statement_number, &statement_subject).await);
    assert!(finds(&app, &access, &reference, &statement_subject).await);
    assert!(related_count(&app, &access, &statement_subject, "orders").await >= 1);

    // A production batch by its number, with its lot.
    let batch = scalar_i64(
        &app,
        "select id from fnb_batches where lot_id is not null order by id limit 1",
    )
    .await;
    let batch_subject = Subject::new("fnb.batch", batch);
    let batch_number = scalar_text(
        &app,
        &format!("select number from fnb_batches where id = {batch}"),
    )
    .await;
    assert!(finds(&app, &access, &batch_number.to_lowercase(), &batch_subject).await);
    assert_eq!(related_count(&app, &access, &batch_subject, "lot").await, 1);

    // The order page loads its Related panel.
    app.acting_as(&director);
    app.get(&format!("/orders/{order}"))
        .await
        .assert_ok()
        .assert_see(&format!("hx-get=\"/related/fnb.order:{order}/panel\""));
    app.get(&format!("/related/fnb.order:{order}/panel"))
        .await
        .assert_ok()
        .assert_see("data-group=\"payouts\"");
}

#[renox::test]
async fn a_manager_of_one_outlet_finds_that_outlets_orders_only() {
    let app = booted().await;
    let (outlet, other) = (
        scalar_i64(
            &app,
            "select outlet_id from fnb_orders where outlet_id is not null order by id limit 1",
        )
        .await,
        scalar_i64(
            &app,
            "select id from fnb_orders where outlet_id is not null order by id limit 1",
        )
        .await,
    );
    let elsewhere = renox::db::sql(format!(
        "select id from fnb_orders where outlet_id is not null and outlet_id <> {outlet} order by id limit 1"
    ))
    .scalar_optional::<i64>(app.db())
    .await
    .unwrap();
    let scope = Scope::new("fnb.outlet", outlet);
    let manager = person(&app, "Amal", AREA_MANAGER, Some(&scope)).await;
    let access = Access::of_user(app.db(), &manager).await.unwrap();
    let mine = scalar_text(
        &app,
        &format!("select number from fnb_orders where id = {other}"),
    )
    .await;
    assert!(finds(&app, &access, &mine, &Subject::new("fnb.order", other)).await);
    if let Some(elsewhere) = elsewhere {
        let theirs = scalar_text(
            &app,
            &format!("select number from fnb_orders where id = {elsewhere}"),
        )
        .await;
        assert!(
            !finds(
                &app,
                &access,
                &theirs,
                &Subject::new("fnb.order", elsewhere)
            )
            .await
        );
    }
}
