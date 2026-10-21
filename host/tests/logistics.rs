//! The host mounts the Logistics vertical when `client.toml` names
//! `app = "logistics"` (03 Logistics, issues #299 and #300): the demo client
//! boots, its roles hold the shipment permissions, and `logistics:demo` makes
//! the demo company once.

use host::roles::{
    self, ADMIN, DISPATCHER, DRIVER, HUB_STAFF, OPERATIONS_MANAGER, SHIPPING_CLIENT,
};
use logistics::permissions as perm;
use logistics::{Logistics, Shipment};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::ClientConfig;

/// The demo client of F3, as it is in `clients/demo-logistics`.
const DEMO: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../clients/demo-logistics/client.toml"
);

fn holds(role: &str, permission: &str) -> bool {
    roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .is_some_and(|(_, grants)| grants.contains(&permission))
}

#[test]
fn the_vertical_is_the_module_the_host_mounts() {
    assert_eq!(Logistics.name(), "logistics");
    assert!(host::VERTICALS.contains(&"logistics"));
}

#[test]
fn the_roles_hold_the_shipment_permissions() {
    // Staff manage, void and cancel any shipment; only they send one back.
    for role in [ADMIN, DISPATCHER, OPERATIONS_MANAGER] {
        assert!(holds(role, perm::SHIPMENTS_MANAGE), "{role}");
        assert!(holds(role, perm::SHIPMENTS_VOID), "{role}");
        assert!(holds(role, perm::SHIPMENTS_CANCEL), "{role}");
    }
    // The operations manager and the company admin approve a loss off a
    // delivery (`LOG.ACL.14`); the dispatcher who requests it cannot.
    assert!(holds(OPERATIONS_MANAGER, perm::SHIPMENTS_APPROVE_LOSS));
    assert!(holds(ADMIN, perm::SHIPMENTS_APPROVE_LOSS));
    assert!(!holds(DISPATCHER, perm::SHIPMENTS_APPROVE_LOSS));
    assert!(!holds(HUB_STAFF, perm::SHIPMENTS_APPROVE_LOSS));
    assert!(!holds(DRIVER, perm::SHIPMENTS_APPROVE_LOSS));
    // The client user books, voids and cancels, and never manages.
    assert!(holds(SHIPPING_CLIENT, perm::SHIPMENTS_BOOK));
    assert!(holds(SHIPPING_CLIENT, perm::SHIPMENTS_VOID));
    assert!(holds(SHIPPING_CLIENT, perm::SHIPMENTS_CANCEL));
    assert!(!holds(SHIPPING_CLIENT, perm::SHIPMENTS_MANAGE));
    assert!(!holds(SHIPPING_CLIENT, perm::SHIPMENTS_SCAN));
    // Hub staff and drivers scan; the driver also records the return.
    assert!(holds(HUB_STAFF, perm::SHIPMENTS_SCAN));
    assert!(!holds(HUB_STAFF, perm::SHIPMENTS_MANAGE));
    assert!(holds(DRIVER, perm::SHIPMENTS_SCAN));
    assert!(holds(DRIVER, perm::SHIPMENTS_DELIVER));
}

#[test]
fn the_accountant_prepares_approves_and_pays_remittances_and_nobody_in_operations_does() {
    for permission in [
        perm::REMITTANCES_VIEW,
        perm::REMITTANCES_PREPARE,
        perm::REMITTANCES_APPROVE,
        perm::REMITTANCES_PAY,
    ] {
        assert!(holds(roles::ACCOUNTANT, permission), "{permission}");
        assert!(holds(ADMIN, permission), "{permission}");
        assert!(!holds(DISPATCHER, permission), "{permission}");
        assert!(!holds(DRIVER, permission), "{permission}");
        assert!(!holds(SHIPPING_CLIENT, permission), "{permission}");
    }
    assert!(holds(OPERATIONS_MANAGER, perm::REMITTANCES_VIEW));
    assert!(!holds(OPERATIONS_MANAGER, perm::REMITTANCES_APPROVE));
    assert!(!holds(OPERATIONS_MANAGER, perm::REMITTANCES_PAY));
}

#[renox::test]
async fn the_demo_logistics_client_boots_and_the_demo_company_is_made_once() {
    let config = ClientConfig::load(DEMO).expect("the demo client");
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel()
        .call(host::logistics_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs");
    let first = Shipment::query().count(app.db()).await.unwrap();
    assert_eq!(first, logistics::seed::DEMO_SHIPMENTS.len() as u64);
    app.kernel()
        .call(host::logistics_demo::COMMAND, Vec::<String>::new())
        .await
        .expect("the demo command runs again");
    assert_eq!(Shipment::query().count(app.db()).await.unwrap(), first);
}
