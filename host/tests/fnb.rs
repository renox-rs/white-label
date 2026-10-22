//! The host mounts the F&B vertical when `client.toml` names `app = "fnb"`
//! (06 F&B, issues #365 to #367): the demo client boots, its roles hold the
//! order permissions, the settings of 06 are in force, `fnb:demo` makes the
//! demo company once, with an order in every state of the stories (a failed
//! delivery, a closed one, a customer whose cash on delivery is off), and the
//! screens open for the roles that hold the permissions.

use fnb::order::{decision, side};
use fnb::permissions as perm;
use fnb::{CustomerFlag, Fnb, FnbOrder, FnbOrderStatus};
use host::roles::{self, ADMIN, CASHIER, DRIVER, KITCHEN_MANAGER, LINE_COOK};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of F6, as it is in `clients/demo-fnb`.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-fnb/client.toml"
);

fn holds(role: &str, permission: &str) -> bool {
    roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .is_some_and(|(_, grants)| grants.contains(&permission))
}

async fn booted() -> TestApp {
    let config = ClientConfig::load(DEMO).expect("the demo client");
    TestApp::new(host::app_for(config).unwrap()).await
}

#[test]
fn the_vertical_is_the_module_the_host_mounts() {
    assert_eq!(Fnb::new().name(), "fnb");
    assert!(host::VERTICALS.contains(&"fnb"));
}

#[test]
fn the_roles_hold_the_order_permissions() {
    // The kitchen manager runs the orders, the tickets and cash on delivery,
    // and approves a refund (`FNB.ACL.17`); the company admin holds them too.
    for role in [KITCHEN_MANAGER, ADMIN] {
        assert!(holds(role, perm::ORDERS_VIEW), "{role}");
        assert!(holds(role, perm::ORDERS_MANAGE), "{role}");
        assert!(holds(role, perm::CUSTOMERS_COD), "{role}");
        assert!(holds(role, perm::KDS_WORK), "{role}");
    }
    assert!(holds(KITCHEN_MANAGER, "refunds.approve"));
    assert!(holds(KITCHEN_MANAGER, "sales_orders.accept"));
    // It cancels and amends orders and approves the cancel of a started line.
    assert!(holds(KITCHEN_MANAGER, perm::ORDERS_CANCEL));
    assert!(holds(KITCHEN_MANAGER, perm::ORDERS_AMEND));
    assert!(holds(KITCHEN_MANAGER, "sales_orders.approve"));
    assert!(holds(ADMIN, perm::ORDERS_CANCEL));
    assert!(!holds(LINE_COOK, perm::ORDERS_CANCEL));
    // The line cook works the tickets and sees nothing else of the orders.
    assert!(holds(LINE_COOK, perm::KDS_WORK));
    assert!(!holds(LINE_COOK, perm::ORDERS_VIEW));
    assert!(!holds(LINE_COOK, perm::ORDERS_MANAGE));
    // The rider has no say in the orders: the delivery is the module's.
    assert!(!holds(DRIVER, perm::ORDERS_MANAGE));
    assert!(holds(DRIVER, "runs.drive"));
}

#[renox::test]
async fn the_cashier_asks_for_a_refund_and_never_approves_one() {
    let _app = booted().await;
    // The cashier of 06 holds the 06 permissions in this deployment only.
    let grants = roles::grants_of(CASHIER);
    assert!(grants.contains(&perm::ORDERS_REFUND_REQUEST));
    assert!(grants.contains(&perm::ORDERS_VIEW));
    assert!(!grants.contains(&perm::ORDERS_MANAGE));
    assert!(!grants.contains(&"refunds.approve"));
    // The cashier cancels an order or a line and amends a direct order; a
    // manager approves above AED 50 or for a started line.
    assert!(grants.contains(&perm::ORDERS_CANCEL));
    assert!(grants.contains(&perm::ORDERS_AMEND));
    assert!(!grants.contains(&"sales_orders.approve"));
}

#[renox::test]
async fn the_demo_client_boots_and_the_demo_company_is_made_once() {
    let app = booted().await;
    app.kernel()
        .call(host::fnb_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");
    let first = FnbOrder::query().count(app.db()).await.unwrap();
    assert!(first >= 9, "the stories' orders: {first}");
    // An order is `delivery_failed` for each side, one is closed with no refund.
    let failed = FnbOrder::query()
        .where_(FnbOrder::STATUS.eq(FnbOrderStatus::DeliveryFailed))
        .get(app.db())
        .await
        .unwrap();
    assert!(
        failed
            .iter()
            .any(|o| o.failure_side.as_deref() == Some(side::CUSTOMER))
    );
    assert!(
        failed
            .iter()
            .any(|o| o.failure_side.as_deref() == Some(side::OUTLET))
    );
    let closed = FnbOrder::query()
        .where_(FnbOrder::REFUND_DECISION.eq(Some(decision::NO_REFUND.to_owned())))
        .count(app.db())
        .await
        .unwrap();
    assert!(closed >= 1);
    // A customer is without cash on delivery.
    assert_eq!(CustomerFlag::turned_off(app.db()).await.unwrap().len(), 1);

    app.kernel()
        .call(host::fnb_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs again");
    assert_eq!(FnbOrder::query().count(app.db()).await.unwrap(), first);
    assert_eq!(CustomerFlag::turned_off(app.db()).await.unwrap().len(), 1);
}

#[renox::test]
async fn the_demo_screens_open_for_the_kitchen_manager_and_not_for_the_cook() {
    let app = booted().await;
    app.kernel()
        .call(host::fnb_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");
    let manager = User::find_by_email(app.db(), "kitchen@zaatar.example")
        .await
        .unwrap()
        .expect("the kitchen manager is made");
    let cook = User::find_by_email(app.db(), "cook@zaatar.example")
        .await
        .unwrap()
        .expect("the cook is made");
    let failed = FnbOrder::query()
        .where_(FnbOrder::STATUS.eq(FnbOrderStatus::DeliveryFailed))
        .first(app.db())
        .await
        .unwrap()
        .expect("a failed order");

    app.acting_as(&manager);
    app.get("/orders")
        .await
        .assert_ok()
        .assert_see("Delivery failed");
    app.get(&format!("/orders/{}", failed.id))
        .await
        .assert_ok()
        .assert_see("Close the failed delivery")
        .assert_see("No reattempt");
    app.get("/customers/cod")
        .await
        .assert_ok()
        .assert_see("Reem Farouk")
        .assert_see("Turn cash on delivery on");
    app.get("/kitchen").await.assert_ok();

    app.acting_as(&cook);
    app.get("/kitchen").await.assert_ok();
    app.get("/orders").await.assert_forbidden();
    app.get("/customers/cod").await.assert_forbidden();
}
