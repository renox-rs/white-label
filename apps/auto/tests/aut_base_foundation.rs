//! The base of the 08 Auto app (base for #397 .. #414): the module boots with
//! the real scheduling, its permissions and role names are the spec's, and its
//! starter tables are what the host installs.

mod support;

use auto::{permissions, reasons, seed};
use support::boot;

/// The app boots next to wl-scheduling and the pages' layout.
#[renox::test]
async fn aut_base_the_app_boots_with_scheduling() {
    let t = boot().await;
    t.app.get("/health").await.assert_ok();
}

/// The role names the rules use are the roles of 08 §3.1; the technician,
/// the storekeeper and the purchaser reuse the host's own names.
#[test]
fn aut_base_the_roles_are_the_spec_roles() {
    assert_eq!(permissions::OWNER_ROLE, "owner");
    assert_eq!(permissions::BRANCH_MANAGER_ROLE, "branch_manager");
    assert_eq!(permissions::SERVICE_ADVISOR_ROLE, "service_advisor");
    assert_eq!(permissions::WORKSHOP_MANAGER_ROLE, "workshop_manager");
    assert_eq!(permissions::TECHNICIAN_ROLE, "technician");
    assert_eq!(permissions::COUNTER_SALES_ROLE, "counter_sales");
    assert_eq!(permissions::STOREKEEPER_ROLE, "storekeeper");
    assert_eq!(permissions::PURCHASER_ROLE, "procurement");
    assert_eq!(permissions::ACCOUNTANT_ROLE, "accountant");
    assert_eq!(
        permissions::INSURANCE_COORDINATOR_ROLE,
        "insurance_coordinator"
    );
    assert_eq!(
        permissions::FLEET_ACCOUNT_MANAGER_ROLE,
        "fleet_account_manager"
    );
}

/// The workshop reason codes: the duplicate check-in is the workshop's side,
/// the customer's decline and stop are the customer's.
#[test]
fn aut_base_the_reason_codes() {
    let codes: Vec<&str> = reasons::SCHEDULING_REASON_CODES
        .iter()
        .map(|(code, _)| *code)
        .collect();
    assert_eq!(
        codes,
        [
            "estimate_declined",
            "duplicate_check_in",
            "job_stopped",
            "fleet_contract_ended",
            "fleet_suspended"
        ]
    );
}

/// The starter service types are the periodic service, the diagnosis and the
/// accident estimate; the ticket categories include the warranty claim and
/// the vehicle incident (§4.4, §4.6).
#[test]
fn aut_base_the_starter_tables() {
    let codes: Vec<&str> = seed::SERVICE_TYPES.iter().map(|s| s.code).collect();
    assert_eq!(
        codes,
        ["periodic_service", "diagnosis", "accident_estimate"]
    );
    let keys: Vec<&str> = seed::CATEGORIES.iter().map(|c| c.key).collect();
    assert!(keys.contains(&"warranty"));
    assert!(keys.contains(&"vehicle_incident"));
}
