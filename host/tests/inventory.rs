//! The host mounts wl-inventory when `client.toml` enables `inventory`
//! (wl-kernel §5.1, wl-inventory §6, issue #220).

use host::roles::{self, ADMIN, MEMBER, PROCUREMENT, STOREKEEPER};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_inventory::{Item, Location, permissions as inventory};
use wl_kernel::ClientConfig;

/// The demo client with `inventory` on.
fn with_inventory() -> ClientConfig {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    ClientConfig::parse(&demo.replace(
        r#""sales", "hr", "portal"]"#,
        r#""sales", "hr", "portal", "inventory"]"#,
    ))
    .unwrap()
}

#[renox::test]
async fn a_client_with_inventory_boots_and_stores_items_and_locations() {
    let app = TestApp::new(host::app_for(with_inventory()).unwrap()).await;
    let mut item = Item::new("CHICKEN", "Chicken", "kg");
    item.save(app.db()).await.unwrap();
    let mut store = Location::new("STORE-1", "Site store");
    store.save(app.db()).await.unwrap();
    assert_eq!(item.average_cost, 0);
    assert_eq!(store.code, "STORE-1");
}

#[test]
fn the_roles_hold_the_inventory_permissions() {
    let holds = |role: &str, permission: &str| {
        roles::ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| grants.contains(&permission))
    };
    // The storekeeper receives; procurement approves an over-receipt.
    assert!(holds(STOREKEEPER, inventory::GOODS_RECEIPTS_POST));
    assert!(!holds(STOREKEEPER, inventory::PURCHASE_ORDERS_ISSUE));
    assert!(holds(PROCUREMENT, inventory::PURCHASE_ORDERS_ISSUE));
    assert!(!holds(PROCUREMENT, inventory::GOODS_RECEIPTS_POST));
    assert!(holds(ADMIN, inventory::GOODS_RECEIPTS_POST) && holds(ADMIN, inventory::PURCHASE_ORDERS_ISSUE));
    assert!(!holds(MEMBER, inventory::GOODS_RECEIPTS_POST));
}
