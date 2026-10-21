//! The host mounts the Construction vertical when `client.toml` names
//! `app = "construction"` (02 Construction, issue #280).

use construction::{Construction, Project, ProjectStatus};
use host::roles::{self, ADMIN, MEMBER, PROJECT_MANAGER, QUANTITY_SURVEYOR};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, PartyId};

/// A contracting client: attendance and stock on, the vertical on.
const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

#[renox::test]
async fn a_construction_client_boots_and_keeps_projects() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    let mut project = Project::new("TWR-01", "Marina Tower fit-out", PartyId(3));
    project.save(app.db()).await.unwrap();
    assert_eq!(project.status, ProjectStatus::Setup);
    assert!(project.retention_due_on_termination);
}

#[test]
fn the_vertical_is_the_module_the_host_mounts() {
    assert_eq!(Construction.name(), "construction");
}

#[test]
fn the_roles_hold_the_project_permissions() {
    let holds = |role: &str, permission: &str| {
        roles::ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| grants.contains(&permission))
    };
    // The project manager asks for a termination, the MD (the company admin)
    // approves it and cancels a project before work (`CON.ACL.15`).
    assert!(holds(
        PROJECT_MANAGER,
        construction::permissions::PROJECTS_TERMINATE
    ));
    assert!(holds(
        PROJECT_MANAGER,
        construction::permissions::PROJECTS_MANAGE
    ));
    assert!(!holds(
        PROJECT_MANAGER,
        construction::permissions::PROJECTS_CANCEL
    ));
    assert!(holds(ADMIN, construction::permissions::PROJECTS_CANCEL));
    // The project manager submits an IPA, the QS records its certificate.
    assert!(holds(
        PROJECT_MANAGER,
        construction::permissions::IPAS_SUBMIT
    ));
    assert!(holds(
        QUANTITY_SURVEYOR,
        construction::permissions::IPAS_CERTIFY
    ));
    assert!(!holds(
        PROJECT_MANAGER,
        construction::permissions::IPAS_CERTIFY
    ));
    // The QS records the deductions, the project manager decides them.
    assert!(holds(
        QUANTITY_SURVEYOR,
        construction::permissions::IPAS_DEDUCTIONS_RECORD
    ));
    assert!(!holds(
        QUANTITY_SURVEYOR,
        construction::permissions::IPAS_DEDUCTIONS_DECIDE
    ));
    assert!(holds(
        PROJECT_MANAGER,
        construction::permissions::IPAS_DEDUCTIONS_DECIDE
    ));
    // The QS prices a variation and records the client's decision, only the
    // project manager submits it (`CON.ACL.06`).
    assert!(holds(
        QUANTITY_SURVEYOR,
        construction::permissions::VARIATIONS_RECORD
    ));
    assert!(!holds(
        QUANTITY_SURVEYOR,
        construction::permissions::VARIATIONS_SUBMIT
    ));
    assert!(holds(
        PROJECT_MANAGER,
        construction::permissions::VARIATIONS_SUBMIT
    ));
    assert!(!holds(
        PROJECT_MANAGER,
        construction::permissions::VARIATIONS_RECORD
    ));
    assert!(!holds(MEMBER, construction::permissions::PROJECTS_VIEW));
}

#[test]
fn an_unbuilt_vertical_is_refused_by_name() {
    let config = ClientConfig::parse(&CLIENT.replace("construction", "manpower")).unwrap();
    let Err(err) = host::app_for(config) else {
        panic!("manpower is not built");
    };
    assert!(err.0.contains("app `manpower` is not built yet"), "{err}");
}
