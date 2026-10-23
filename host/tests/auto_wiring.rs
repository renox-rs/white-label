//! 08 Auto in the real host (base for #397 .. #414): the Auto client boots with
//! its service types, ticket categories and document kinds, and the roles of
//! 08 §3.1 hold the permissions of the app.

mod support;

use auto::permissions as aut;
use host::roles;
use renox::prelude::*;
use support::{auto_booted, person};
use wl_kernel::documents::{BlockMode, OverrideMode, guard};
use wl_scheduling::ServiceType;
use wl_ticketing::Category;

/// The grants of `role` in the host.
fn grants(role: &str) -> &'static [&'static str] {
    roles::ROLES
        .iter()
        .find(|(name, _)| *name == role)
        .map(|(_, grants)| *grants)
        .unwrap_or_else(|| panic!("no role {role}"))
}

/// `[client] app = "auto"` is a built vertical.
#[test]
fn aut_base_auto_is_a_built_vertical() {
    assert!(host::VERTICALS.contains(&"auto"));
}

/// The seed stores the service types (check-in required, no deposit) and the
/// ticket categories; a second install changes nothing.
#[renox::test]
async fn aut_base_the_service_types_and_categories_are_installed_once() {
    let app = auto_booted().await;

    let types = ServiceType::query()
        .order_by(ServiceType::CODE)
        .get(app.db())
        .await
        .unwrap();
    let codes: Vec<&str> = types.iter().map(|s| s.code.as_str()).collect();
    assert_eq!(
        codes,
        ["accident_estimate", "diagnosis", "periodic_service"]
    );
    for service in &types {
        assert!(service.check_in_required, "check-in opens the job card");
        assert_eq!(service.deposit_amount, 0);
    }
    for spec in auto::seed::CATEGORIES {
        assert!(
            Category::by_key(app.db(), spec.key)
                .await
                .unwrap()
                .is_some(),
            "{}",
            spec.key
        );
    }
    let again = host::auto_seed::install(app.state()).await.unwrap();
    assert_eq!(again.total(), 0);
}

/// The police accident report blocks the approval of a job card and of a
/// claim, a legal block with no override (`AUT.ACL.20`).
#[renox::test]
async fn aut_base_the_police_report_blocks_the_approvals_with_no_override() {
    let app = auto_booted().await;
    let documents = guard(app.state());

    let approve = documents
        .blocks_for(app.db(), "job_card.approve")
        .await
        .unwrap();
    assert_eq!(approve.len(), 1);
    assert_eq!(approve[0].kind, "police_report");
    assert_eq!(approve[0].mode, BlockMode::Block);
    assert_eq!(approve[0].override_mode, OverrideMode::Never);

    let claim = documents
        .blocks_for(app.db(), "insurance_claim.approve")
        .await
        .unwrap();
    assert_eq!(claim.len(), 1);
    assert_eq!(claim[0].override_mode, OverrideMode::Never);
}

/// The roles of 08 §3.1 exist and hold the permissions of the app: the
/// technician works but advises nothing, the advisor advises but does not
/// approve, the branch manager approves, the insurance coordinator and the
/// fleet account manager hold their own screens.
#[test]
fn aut_base_the_roles_hold_the_permissions_of_the_spec() {
    assert!(grants(roles::TECHNICIAN).contains(&aut::JOB_CARDS_WORK));
    assert!(!grants(roles::TECHNICIAN).contains(&aut::JOB_CARDS_VIEW));
    assert!(!grants(roles::TECHNICIAN).contains(&aut::JOB_CARDS_ADVISE));
    assert!(grants(roles::SERVICE_ADVISOR).contains(&aut::JOB_CARDS_ADVISE));
    assert!(!grants(roles::SERVICE_ADVISOR).contains(&aut::JOB_CARDS_APPROVE));
    assert!(grants(roles::BRANCH_MANAGER).contains(&aut::JOB_CARDS_APPROVE));
    assert!(grants(roles::WORKSHOP_MANAGER).contains(&aut::JOB_CARDS_QC));
    assert!(grants(roles::INSURANCE_COORDINATOR).contains(&aut::INSURANCE_MANAGE));
    assert!(grants(roles::FLEET_ACCOUNT_MANAGER).contains(&aut::FLEET_CONTRACTS_MANAGE));
    assert!(grants(roles::PROCUREMENT).contains(&aut::SUPPLIER_CLAIMS_MANAGE));
    assert!(grants(roles::STOREKEEPER).contains(&aut::JOB_CARDS_VIEW));
    assert!(grants(roles::ADMIN).contains(&aut::LABOUR_MENU_MANAGE));
}

/// A service advisor signs in to the Auto host and meets the dashboard, not
/// an error.
#[renox::test]
async fn aut_base_a_service_advisor_signs_in() {
    let app = auto_booted().await;
    let advisor = person(&app, "Sami", roles::SERVICE_ADVISOR).await;
    app.acting_as(&advisor);
    app.get("/dashboard").await.assert_ok();
}
