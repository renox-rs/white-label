//! The demo client mounts wl-parties (#34 AC1).

use renox::prelude::*;
use renox::testing::TestApp;

use host::roles::{self, ADMIN, COMPLIANCE, MEMBER};

/// A verified user with `role`.
async fn person(app: &TestApp, email: &str, role: &str) -> User {
    let mut user = User::register(app.db(), "Rana", email, "secret123")
        .await
        .unwrap();
    user.email_verified_at = Some(renox::db::now());
    user.save(app.db()).await.unwrap();
    roles::define(app.db()).await.unwrap();
    user.assign_role(app.db(), role).await.unwrap();
    user
}

#[renox::test]
async fn the_parties_page_needs_parties_view() {
    let app = TestApp::new(host::app()).await;
    app.kernel().seed().await.unwrap();

    let member = person(&app, "parties.member@test.example", MEMBER).await;
    app.acting_as(&member);
    app.get("/parties").await.assert_forbidden();

    for (email, role) in [
        ("parties.admin@test.example", ADMIN),
        ("parties.mlro@test.example", COMPLIANCE),
    ] {
        let user = person(&app, email, role).await;
        app.acting_as(&user);
        app.get("/parties")
            .await
            .assert_ok()
            .assert_see("Al Noor Trading");
    }
}

#[test]
fn only_compliance_sees_restricted_block_reasons() {
    let holds = |grants: &[&str]| grants.contains(&wl_parties::permissions::BLOCK_REASON_VIEW);
    // Every role of the host, so a role added later cannot slip in.
    for (name, grants) in roles::ROLES {
        assert_eq!(
            holds(grants),
            *name == COMPLIANCE,
            "{name}: only the compliance role sees restricted block reasons"
        );
    }
    let names: Vec<&str> = roles::ROLES.iter().map(|(name, _)| *name).collect();
    for role in [
        COMPLIANCE,
        ADMIN,
        MEMBER,
        roles::PARTNER,
        roles::ENGAGEMENT_MANAGER,
        roles::BILLING,
        roles::TAX_SPECIALIST,
        roles::ACCOUNTANT,
        roles::HR,
    ] {
        assert!(names.contains(&role), "{role} is a host role");
    }
}
