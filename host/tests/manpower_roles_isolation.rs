//! BL-MPW-15 (#429), audit risk 2: the extra grants of a vertical (the role
//! `md` of a manpower agency) belong to the deployment that was built, not to
//! the process. Building the app of another vertical afterwards, in the same
//! process, leaves `md` unknown.

use host::roles::{grants_of, role_names};
use wl_kernel::ClientConfig;

fn client(app: &str, modules: &str) -> ClientConfig {
    ClientConfig::parse(&format!(
        r#"
[client]
name = "Test Company LLC"
app = "{app}"

[modules]
enabled = [{modules}]

[[legal_entities]]
code = "TST"
name_en = "Test Company LLC"
default = true
"#
    ))
    .unwrap()
}

/// Only the manpower deployment knows `md`, and gives it the approvals the MD
/// holds; a property host built after it in the same process does not.
#[test]
fn only_the_manpower_deployment_knows_the_role_md() {
    let manpower = host::app_for(client("manpower", "\"parties\", \"hr\"")).unwrap();
    assert!(role_names().contains(&"md"));
    let grants = grants_of("md");
    for permission in [
        manpower::permissions::CONTRACTS_VIEW,
        wl_invoicing::permissions::RATE_CARDS_APPROVE,
        wl_hr::permissions::EMPLOYEES_EXIT_APPROVE,
        wl_hr::permissions::SETTLEMENTS_APPROVE,
        wl_hr::permissions::PAY_ACCOUNTS_APPROVE,
        wl_ticketing::permissions::CONFIDENTIAL_VIEW,
    ] {
        assert!(grants.contains(&permission), "{permission}");
    }
    drop(manpower);

    let property = host::app_for(client("property", "\"parties\"")).unwrap();
    assert!(!role_names().contains(&"md"), "md leaked into property");
    assert!(grants_of("md").is_empty());
    assert!(
        !grants_of(host::roles::ACCOUNTANT).contains(&wl_ticketing::permissions::CONFIDENTIAL_VIEW),
        "the manpower grants leaked into property"
    );
    drop(property);

    let _again = host::app_for(client("manpower", "\"parties\", \"hr\"")).unwrap();
    assert!(
        role_names().contains(&"md"),
        "a manpower build has it again"
    );
}
