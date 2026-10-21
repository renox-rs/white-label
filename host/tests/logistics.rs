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

#[test]
fn the_accountant_and_the_client_user_ask_for_a_bank_account_change_and_a_second_user_approves_it() {
    // `LOG.ACL.13`: the accountant (and the company admin) and a client user
    // in the portal ask; only the holders of `remittances.approve` decide.
    assert!(holds(roles::ACCOUNTANT, perm::REMITTANCES_REQUEST_ACCOUNT));
    assert!(holds(ADMIN, perm::REMITTANCES_REQUEST_ACCOUNT));
    assert!(holds(SHIPPING_CLIENT, perm::REMITTANCES_REQUEST_ACCOUNT));
    assert!(!holds(SHIPPING_CLIENT, perm::REMITTANCES_APPROVE));
    for role in [DISPATCHER, DRIVER, HUB_STAFF, OPERATIONS_MANAGER] {
        assert!(!holds(role, perm::REMITTANCES_REQUEST_ACCOUNT), "{role}");
    }
}

#[test]
fn customer_service_flags_statement_disputes_and_the_company_admin_approves_block_exceptions() {
    // `LOG.ACL.12`: customer service and the client user dispute a statement;
    // only the holders of `remittances.approve` resolve it.
    assert!(holds(roles::CUSTOMER_SERVICE, perm::REMITTANCES_DISPUTE));
    assert!(holds(roles::CUSTOMER_SERVICE, perm::REMITTANCES_VIEW));
    assert!(!holds(roles::CUSTOMER_SERVICE, perm::REMITTANCES_APPROVE));
    assert!(!holds(roles::CUSTOMER_SERVICE, perm::REMITTANCES_PAY));
    assert!(holds(SHIPPING_CLIENT, perm::REMITTANCES_DISPUTE));
    assert!(holds(roles::ACCOUNTANT, perm::REMITTANCES_DISPUTE));
    // `PTY.ACL.03`: 03 has no finance manager; the company admin approves an
    // exception and the accountant who asks for it does not.
    assert!(holds(ADMIN, wl_parties::permissions::BLOCK_EXCEPTION));
    assert!(!holds(roles::ACCOUNTANT, wl_parties::permissions::BLOCK_EXCEPTION));
    // Freight jobs are opened by the dispatcher and the operations manager.
    for role in [ADMIN, DISPATCHER, OPERATIONS_MANAGER] {
        assert!(holds(role, perm::FREIGHT_MANAGE), "{role}");
    }
    for role in [DRIVER, HUB_STAFF, SHIPPING_CLIENT] {
        assert!(!holds(role, perm::FREIGHT_MANAGE), "{role}");
    }
}

#[test]
fn the_logistics_client_has_the_company_admin_approve_the_block_exception() {
    let parties = host::parties_for("logistics");
    assert_eq!(parties.approver(), wl_kernel::COMPANY_ADMIN_ROLE);
}

#[test]
fn the_documentation_officer_works_and_closes_freight_jobs() {
    // `LOG.JOB.01`, BL-LOG-21 to BL-LOG-24: the documentation officer (and
    // the company admin) hold, release, deliver, close and reopen jobs; the
    // operations manager and the accountant only see them.
    for role in [ADMIN, roles::DOCUMENTATION_OFFICER] {
        for permission in [perm::JOBS_VIEW, perm::JOBS_MANAGE, perm::JOBS_DELIVER, perm::JOBS_CLOSE] {
            assert!(holds(role, permission), "{role} {permission}");
        }
    }
    for role in [OPERATIONS_MANAGER, roles::ACCOUNTANT] {
        assert!(holds(role, perm::JOBS_VIEW), "{role}");
        assert!(!holds(role, perm::JOBS_MANAGE), "{role}");
        assert!(!holds(role, perm::JOBS_CLOSE), "{role}");
    }
    for role in [DISPATCHER, DRIVER, HUB_STAFF, SHIPPING_CLIENT] {
        assert!(!holds(role, perm::JOBS_VIEW), "{role}");
    }
}
