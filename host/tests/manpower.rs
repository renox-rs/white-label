//! The host mounts the Manpower vertical when `client.toml` names
//! `app = "manpower"` (Base 09, 09 Manpower §3.1).

use host::roles::{self, ACCOUNT_MANAGER, ADMIN};
use manpower::{ClientContract, ContractStatus, Manpower};
use renox::prelude::*;
use renox::testing::TestApp;
use wl_kernel::{ClientConfig, PartyId};

/// A manpower agency: the vertical on, with the shared modules it reaches.
const CLIENT: &str = r#"
[client]
name = "Gulf Manpower Services LLC"
app = "manpower"

[modules]
enabled = ["parties", "documents", "notify", "invoicing", "ticketing", "payments", "hr", "attendance", "portal"]

[[legal_entities]]
code = "GMS"
name_en = "Gulf Manpower Services LLC"
default = true
"#;

#[renox::test]
async fn a_manpower_client_boots_and_keeps_contracts() {
    let config = ClientConfig::parse(CLIENT).unwrap();
    let app = TestApp::new(host::app_for(config).unwrap()).await;
    let contract = ClientContract {
        reference: "CC-0001".into(),
        client_party_id: PartyId(3).0,
        start_date: renox::chrono::NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        end_date: renox::chrono::NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
        ..ClientContract::default()
    };
    let saved = ClientContract::create(app.db(), contract).await.unwrap();
    assert_eq!(saved.status, ContractStatus::Draft);
}

#[test]
fn the_vertical_is_the_module_the_host_mounts() {
    assert_eq!(Manpower.name(), "manpower");
    assert!(host::VERTICALS.contains(&"manpower"));
}

#[test]
fn the_roles_hold_the_manpower_permissions() {
    let holds = |role: &str, permission: &str| {
        roles::ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| grants.contains(&permission))
    };
    // The account manager runs the client accounts, the MD (the company
    // admin) too; the account manager is assigned per client company.
    for permission in [
        manpower::permissions::CONTRACTS_VIEW,
        manpower::permissions::CONTRACTS_MANAGE,
        manpower::permissions::SITES_MANAGE,
    ] {
        assert!(holds(ACCOUNT_MANAGER, permission), "{permission}");
        assert!(holds(ADMIN, permission), "{permission}");
    }
}
