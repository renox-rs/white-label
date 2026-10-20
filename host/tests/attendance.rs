//! The host mounts wl-attendance when `client.toml` enables `attendance`
//! (wl-kernel §5.1, wl-attendance §6, issue #178).

use host::roles::{self, ADMIN, HR, MEMBER, SUPERVISOR};
use renox::testing::TestApp;
use wl_attendance::{WorkLocation, permissions as attendance};
use wl_kernel::ClientConfig;

/// The demo client with `attendance` on, which needs `hr`.
fn with_attendance() -> ClientConfig {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    ClientConfig::parse(&demo.replace(
        r#""sales", "hr", "portal"]"#,
        r#""sales", "hr", "portal", "attendance"]"#,
    ))
    .unwrap()
}

#[renox::test]
async fn a_client_with_attendance_boots_and_stores_work_locations() {
    let app = TestApp::new(host::app_for(with_attendance()).unwrap()).await;
    let site = WorkLocation::create(app.db(), WorkLocation::named("Site 1"))
        .await
        .unwrap();
    assert_eq!(site.name, "Site 1");
}

#[test]
fn attendance_needs_hr() {
    let demo = std::fs::read_to_string(host::DEMO_CLIENT).unwrap();
    let without_hr = demo
        .replace(r#""sales", "hr", "portal"]"#, r#""sales", "portal", "attendance"]"#);
    let err = ClientConfig::parse(&without_hr)
        .and_then(|config| host::app_for(config).map(|_| ()))
        .unwrap_err();
    assert!(err.0.contains("attendance"), "{err}");
}

#[test]
fn the_roles_hold_the_attendance_permissions() {
    let holds = |role: &str, permission: &str| {
        roles::ROLES
            .iter()
            .find(|(name, _)| *name == role)
            .is_some_and(|(_, grants)| grants.contains(&permission))
    };
    assert!(holds(SUPERVISOR, attendance::MARK));
    assert!(holds(SUPERVISOR, attendance::CORRECT));
    assert!(!holds(SUPERVISOR, attendance::REVIEW));
    assert!(holds(HR, attendance::REVIEW) && holds(HR, attendance::CORRECT));
    assert!(!holds(HR, attendance::MARK));
    assert!(holds(ADMIN, attendance::REVIEW) && holds(ADMIN, attendance::SELF_CHECK_IN));
    assert!(!holds(MEMBER, attendance::MARK));
}
