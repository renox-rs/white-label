//! BL-CON-18 (#297): the 02 labour and compliance values stored by the seed
//! of a fresh Construction deployment: the compliance-critical rules and
//! their owner role, the onboarding checklists, the SOP library and the
//! roles of the HSE officer.

use host::roles::{self, ADMIN, HSE_OFFICER, PROJECT_MANAGER};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_documents::{Sop, SopReader};
use wl_hr::OnboardingTemplate;
use wl_kernel::ClientConfig;
use wl_kernel::compliance::registry;

const CLIENT: &str = r#"
[client]
name = "Gulf Build Contracting LLC"
app = "construction"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "sales", "hr", "attendance", "inventory"]

[[legal_entities]]
code = "GBC"
name_en = "Gulf Build Contracting LLC"
default = true
"#;

async fn booted() -> TestApp {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    app.kernel()
        .call("wl:install", Vec::<String>::new())
        .await
        .unwrap();
    app.kernel().seed().await.unwrap();
    app
}

/// Items 1 and 2: the hours, permit and midday rules are compliance-critical
/// and a failed run alerts the MD, the company admin.
#[renox::test]
async fn the_labour_rules_are_critical_and_owned_by_the_md() {
    let _app = booted().await;
    for rule in ["HRM.RUL.01", "HRM.RUL.13", "HRM.RUL.14"] {
        let found = registry::rule(rule).unwrap_or_else(|| panic!("{rule} is registered"));
        assert_eq!(found.owner_role_name(), Some(ADMIN), "{rule}");
        assert!(registry::is_critical(rule));
    }
}

/// Item 17: one checklist per role, each started for its job role, ending
/// with the SOPs, with tasks for HR, the manager and the employee.
#[renox::test]
async fn the_five_onboarding_checklists_are_stored_once() {
    let app = booted().await;
    for spec in &construction::seed::ONBOARDING {
        let found = OnboardingTemplate::query()
            .where_(OnboardingTemplate::NAME_EN.eq(spec.name))
            .count(app.db())
            .await
            .unwrap();
        assert_eq!(found, 1, "{}", spec.name);
        let owners: Vec<&str> = spec.steps.iter().map(|s| s.1).collect();
        for owner in ["hr", "manager", "employee"] {
            assert!(
                owners.contains(&owner),
                "{} has a step for {owner}",
                spec.name
            );
        }
    }
}

/// Item 18: the HSE officer owns the safety SOPs, the project manager the
/// site SOPs, and the readers are the supervisors and workers.
#[renox::test]
async fn the_sop_library_has_the_hse_officer_and_the_project_manager_as_owners() {
    let app = booted().await;
    for spec in &construction::seed::SOPS {
        let sop = Sop::query()
            .where_(Sop::CODE.eq(spec.code))
            .first(app.db())
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("the SOP {}", spec.code));
        assert_eq!(sop.owner_role, spec.owner_role);
        let readers = SopReader::query()
            .where_(SopReader::SOP_ID.eq(sop.id))
            .count(app.db())
            .await
            .unwrap();
        assert_eq!(readers, spec.readers.len() as u64, "{}", spec.code);
    }
    let owners: Vec<&str> = construction::seed::SOPS
        .iter()
        .map(|s| s.owner_role)
        .collect();
    assert!(owners.contains(&HSE_OFFICER) && owners.contains(&PROJECT_MANAGER));
}

/// The HSE officer is a role of the host, so the officer told of a midday
/// flag (item 4, 5) can hold the role.
#[test]
fn the_hse_officer_is_a_role_of_the_host() {
    assert!(roles::ROLES.iter().any(|(name, _)| *name == HSE_OFFICER));
}
